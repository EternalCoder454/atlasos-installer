//! Language, keyboard and Wi-Fi: checking what the UI sends, and the files
//! written into the installed system's `/etc` (the image has no locale,
//! keyboard or hostname handling of its own).

/// A locale such as `en_US.UTF-8` or `sr_RS.UTF-8@latin`. Only UTF-8 locales.
pub fn validate_locale(s: &str) -> Result<(), String> {
    let bad = || Err(format!("not a locale: {s:?}"));
    let (base, modifier) = match s.split_once('@') {
        Some((b, m)) => (b, Some(m)),
        None => (s, None),
    };
    let Some(lang_country) = base.strip_suffix(".UTF-8") else {
        return bad();
    };
    let (lang, country) = match lang_country.split_once('_') {
        Some((l, c)) => (l, Some(c)),
        None => (lang_country, None),
    };
    let lower = |x: &str, n: std::ops::RangeInclusive<usize>| {
        n.contains(&x.len()) && x.bytes().all(|b| b.is_ascii_lowercase())
    };
    if !lower(lang, 2..=3) {
        return bad();
    }
    if let Some(c) = country
        && !(c.len() == 2 && c.bytes().all(|b| b.is_ascii_uppercase())
            || c.len() == 3 && c.bytes().all(|b| b.is_ascii_digit()))
    {
        return bad();
    }
    if let Some(m) = modifier
        && !lower(m, 1..=16)
    {
        return bad();
    }
    Ok(())
}

/// An XKB layout with an optional variant, written `de` or `de(nodeadkeys)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    pub layout: String,
    pub variant: Option<String>,
}

impl Keymap {
    pub fn parse(s: &str) -> Result<Keymap, String> {
        let bad = || format!("not a keyboard layout: {s:?}");
        let (layout, variant) = match s.split_once('(') {
            Some((l, rest)) => (l, Some(rest.strip_suffix(')').ok_or_else(bad)?)),
            None => (s, None),
        };
        let ok_layout = (1..=16).contains(&layout.len())
            && layout
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_lowercase())
            && layout
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        let ok_variant = variant.is_none_or(|v| {
            (1..=48).contains(&v.len())
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        });
        if !ok_layout || !ok_variant {
            return Err(bad());
        }
        Ok(Keymap {
            layout: layout.into(),
            variant: variant.map(String::from),
        })
    }
}

/// The console keymap for an XKB layout, from systemd's kbd-model-map
/// (`consolelayout xlayout xmodel xvariant xoptions`). Without a match,
/// the XKB-converted console keymap of the same name (`de-nodeadkeys`),
/// which kbd ships for every layout.
pub fn console_keymap(kbd_model_map: &str, k: &Keymap) -> String {
    let want_variant = k.variant.as_deref().unwrap_or("-");
    for line in kbd_model_map.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() >= 4 && !f[0].starts_with('#') && f[1] == k.layout && f[3] == want_variant {
            return f[0].to_string();
        }
    }
    match &k.variant {
        Some(v) => format!("{}-{v}", k.layout),
        None => k.layout.clone(),
    }
}

/// `/etc/locale.conf`
pub fn locale_conf(locale: &str) -> String {
    format!("LANG={locale}\n")
}

/// `/etc/vconsole.conf`, with the XKB fields systemd-localed also keeps there.
pub fn vconsole_conf(console: &str, k: &Keymap) -> String {
    let mut s = format!("KEYMAP={console}\nXKBLAYOUT={}\nXKBMODEL=pc105\n", k.layout);
    if let Some(v) = &k.variant {
        s.push_str(&format!("XKBVARIANT={v}\n"));
    }
    s
}

/// `/etc/X11/xorg.conf.d/00-keyboard.conf`, as systemd-localed writes it.
pub fn x11_keyboard_conf(k: &Keymap) -> String {
    let mut s = String::from(
        "# Written by Atlas Installer, read by systemd-localed and Xorg.\n\
         # Use localectl(1) to change it.\n\
         Section \"InputClass\"\n\
         \x20       Identifier \"system-keyboard\"\n\
         \x20       MatchIsKeyboard \"on\"\n",
    );
    s.push_str(&format!("        Option \"XkbLayout\" \"{}\"\n", k.layout));
    s.push_str("        Option \"XkbModel\" \"pc105\"\n");
    if let Some(v) = &k.variant {
        s.push_str(&format!("        Option \"XkbVariant\" \"{v}\"\n"));
    }
    s.push_str("EndSection\n");
    s
}

/// `/etc/atlasos/installer.ini`: what the installer already asked, so
/// AtlasOS's first-run wizard (plasma-setup) can skip those pages. `network`
/// is true when a Wi-Fi connection was carried over or the PC was on a
/// cable; the wizard still shows its Wi-Fi page when it finds itself offline.
/// The locale and keymap are validated, so neither can hold a newline.
pub fn installer_ini(locale: &str, k: &Keymap, network: bool) -> String {
    format!(
        "# Written by Atlas Installer. AtlasOS's first-run wizard reads it to\n\
         # skip the questions the installer already asked.\n\
         [Installer]\n\
         Version=1\n\
         Language={locale}\n\
         KeyboardLayout={}\n\
         KeyboardVariant={}\n\
         Network={network}\n",
        k.layout,
        k.variant.as_deref().unwrap_or("")
    )
}

/// A NetworkManager connection UUID (lower-case 8-4-4-4-12 hex).
pub fn validate_uuid(s: &str) -> Result<(), String> {
    let groups: Vec<&str> = s.split('-').collect();
    let lens = [8, 4, 4, 4, 12];
    let ok = groups.len() == 5
        && groups.iter().zip(lens).all(|(g, n)| {
            g.len() == n
                && g.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
    if ok {
        Ok(())
    } else {
        Err(format!("not a connection UUID: {s:?}"))
    }
}

/// If `keyfile` (a NetworkManager `.nmconnection`) is the connection `uuid`,
/// the copy for the installed system: the same, without `permissions=`,
/// which would tie it to the live session's user, and without
/// `interface-name=`, which NetworkManager adds on AddAndActivateConnection
/// and which would keep a USB adapter on another port from connecting.
pub fn keyfile_for_install(keyfile: &str, uuid: &str) -> Option<String> {
    let mut section = "";
    let mut found = false;
    let mut out = String::with_capacity(keyfile.len());
    for line in keyfile.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = &t[1..t.len() - 1];
        } else if section == "connection"
            && let Some((k, v)) = t.split_once('=')
        {
            match k.trim() {
                "uuid" => found |= v.trim().eq_ignore_ascii_case(uuid),
                "permissions" | "interface-name" => continue,
                _ => {}
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    found.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: &str = "# consolelayout\txlayout\txmodel\txvariant\txoptions\n\
        sg\t\t\tch\tpc105\t\tde_nodeadkeys\tterminate:ctrl_alt_bksp\n\
        uk\t\t\tgb\tpc105\t\t-\t\tterminate:ctrl_alt_bksp\n\
        de\t\t\tde\tpc105\t\t-\t\tterminate:ctrl_alt_bksp\n\
        de-latin1\t\tde\tpc105\t\t-\t\tterminate:ctrl_alt_bksp\n\
        de-latin1-nodeadkeys\tde\tpc105\t\tnodeadkeys\tterminate:ctrl_alt_bksp\n\
        us\t\t\tus\tpc105+inet\t-\t\tterminate:ctrl_alt_bksp\n";

    #[test]
    fn locales() {
        for ok in [
            "en_US.UTF-8",
            "de_DE.UTF-8",
            "sr_RS.UTF-8@latin",
            "ast_ES.UTF-8",
            "eo.UTF-8",
            "es_419.UTF-8",
        ] {
            assert!(validate_locale(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "en_US",
            "C.UTF-8",
            "en_us.UTF-8",
            "en_US.UTF-8\nX=1",
            "../x.UTF-8",
            "en_US.utf8",
            "EN_US.UTF-8",
        ] {
            assert!(validate_locale(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn keymaps_parse_and_reject_junk() {
        assert_eq!(
            Keymap::parse("us"),
            Ok(Keymap {
                layout: "us".into(),
                variant: None
            })
        );
        assert_eq!(
            Keymap::parse("de(nodeadkeys)"),
            Ok(Keymap {
                layout: "de".into(),
                variant: Some("nodeadkeys".into())
            })
        );
        assert!(Keymap::parse("us(dvorak-alt-intl)").is_ok());
        for bad in [
            "",
            "US",
            "us(",
            "us()",
            "us(x\")",
            "de nodeadkeys",
            "us\n",
            "\"us",
            "1us",
            "us(a)(b)",
        ] {
            assert!(Keymap::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn console_keymaps_come_from_the_map_or_the_xkb_name() {
        let k = |s: &str| Keymap::parse(s).unwrap();
        assert_eq!(console_keymap(MAP, &k("gb")), "uk");
        assert_eq!(console_keymap(MAP, &k("de")), "de", "the first match wins");
        assert_eq!(
            console_keymap(MAP, &k("de(nodeadkeys)")),
            "de-latin1-nodeadkeys"
        );
        assert_eq!(console_keymap(MAP, &k("ch(de_nodeadkeys)")), "sg");
        assert_eq!(console_keymap(MAP, &k("fr")), "fr");
        assert_eq!(console_keymap(MAP, &k("fr(bepo)")), "fr-bepo");
    }

    #[test]
    fn config_files() {
        assert_eq!(locale_conf("de_DE.UTF-8"), "LANG=de_DE.UTF-8\n");
        let k = Keymap::parse("de(nodeadkeys)").unwrap();
        assert_eq!(
            vconsole_conf("de-latin1-nodeadkeys", &k),
            "KEYMAP=de-latin1-nodeadkeys\nXKBLAYOUT=de\nXKBMODEL=pc105\nXKBVARIANT=nodeadkeys\n"
        );
        let x = x11_keyboard_conf(&k);
        assert!(x.contains("        Option \"XkbLayout\" \"de\"\n"));
        assert!(x.contains("        Option \"XkbVariant\" \"nodeadkeys\"\n"));
        assert!(x.ends_with("EndSection\n"));
        assert!(!x11_keyboard_conf(&Keymap::parse("us").unwrap()).contains("XkbVariant"));

        let ini = installer_ini("de_DE.UTF-8", &k, true);
        assert!(
            ini.ends_with(
                "[Installer]\nVersion=1\nLanguage=de_DE.UTF-8\nKeyboardLayout=de\n\
                 KeyboardVariant=nodeadkeys\nNetwork=true\n"
            ),
            "{ini}"
        );
        let ini = installer_ini("en_GB.UTF-8", &Keymap::parse("gb").unwrap(), false);
        assert!(
            ini.ends_with("KeyboardLayout=gb\nKeyboardVariant=\nNetwork=false\n"),
            "{ini}"
        );
    }

    #[test]
    fn uuids() {
        assert!(validate_uuid("0b4f6b8e-2a0c-4d5e-9f1a-3c2b1a0d9e8f").is_ok());
        for bad in [
            "",
            "0B4F6B8E-2A0C-4D5E-9F1A-3C2B1A0D9E8F",
            "0b4f6b8e2a0c4d5e9f1a3c2b1a0d9e8f",
            "../../etc/shadow",
            "0b4f6b8e-2a0c-4d5e-9f1a-3c2b1a0d9e8",
        ] {
            assert!(validate_uuid(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn keyfile_copy_matches_uuid_and_drops_permissions() {
        let kf = "[connection]\nid=Home\nuuid=0b4f6b8e-2a0c-4d5e-9f1a-3c2b1a0d9e8f\ntype=wifi\ninterface-name=wlan0\npermissions=user:atlas-installer;\n\n\
                  [wifi]\nssid=Home\n\n[wifi-security]\nkey-mgmt=wpa-psk\npsk=secret\n\n[vpn]\npermissions=keep\n";
        let out = keyfile_for_install(kf, "0b4f6b8e-2a0c-4d5e-9f1a-3c2b1a0d9e8f").unwrap();
        assert!(!out.contains("permissions=user"));
        assert!(!out.contains("interface-name"), "{out}");
        assert!(out.contains("psk=secret\n"));
        assert!(
            out.contains("[vpn]\npermissions=keep\n"),
            "only [connection] permissions go"
        );
        assert_eq!(
            keyfile_for_install(kf, "11111111-2a0c-4d5e-9f1a-3c2b1a0d9e8f"),
            None
        );
        // a uuid key outside [connection] doesn't count
        let other = "[connection]\nid=x\n[wifi]\nuuid=0b4f6b8e-2a0c-4d5e-9f1a-3c2b1a0d9e8f\n";
        assert_eq!(
            keyfile_for_install(other, "0b4f6b8e-2a0c-4d5e-9f1a-3c2b1a0d9e8f"),
            None
        );
    }
}
