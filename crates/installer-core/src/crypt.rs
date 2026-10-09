//! Disk encryption: the choices, the checks on what the user typed and on
//! what cryptsetup and systemd-cryptenroll print, and the kernel arguments.
//! No I/O; the helper runs the commands.

use std::str::FromStr;

use crate::settings;

/// What protects the root partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encryption {
    None,
    /// Unlocked by the TPM at boot; the recovery key is the fallback.
    Tpm,
    /// Unlocked by the TPM and a PIN typed at boot; the recovery key is the
    /// fallback.
    TpmPin,
    /// Unlocked by a password typed at boot; the recovery key is the fallback.
    Password,
}

impl FromStr for Encryption {
    type Err = String;
    fn from_str(s: &str) -> Result<Encryption, String> {
        match s {
            "none" => Ok(Encryption::None),
            "tpm" => Ok(Encryption::Tpm),
            "tpm-pin" => Ok(Encryption::TpmPin),
            "password" => Ok(Encryption::Password),
            _ => Err(format!(
                "unknown encryption {s:?}: expected none, tpm, tpm-pin or password"
            )),
        }
    }
}

impl Encryption {
    pub fn as_str(self) -> &'static str {
        match self {
            Encryption::None => "none",
            Encryption::Tpm => "tpm",
            Encryption::TpmPin => "tpm-pin",
            Encryption::Password => "password",
        }
    }

    pub fn on(self) -> bool {
        self != Encryption::None
    }

    /// The TPM unlocks it (needs a TPM 2.0).
    pub fn uses_tpm(self) -> bool {
        matches!(self, Encryption::Tpm | Encryption::TpmPin)
    }

    /// Something is typed at boot, and `Request::password` carries it.
    pub fn has_secret(self) -> bool {
        matches!(self, Encryption::TpmPin | Encryption::Password)
    }
}

pub const PASSWORD_MIN: usize = 8;
pub const PASSWORD_MAX: usize = 256;
pub const PIN_MIN: usize = 4;
pub const PIN_MAX: usize = 64;

/// Printable ASCII, space to tilde: what the initramfs console prompt can
/// type in any keyboard layout, with no dead keys or normalisation.
fn printable_ascii(s: &str) -> bool {
    s.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

fn validate_typed(s: &str, what: &str, min: usize, max: usize) -> Result<(), String> {
    if !(min..=max).contains(&s.len()) {
        return Err(format!("the {what} must be {min} to {max} characters long"));
    }
    if !printable_ascii(s) {
        return Err(format!(
            "the {what} can only use letters, digits, spaces and punctuation of the basic ASCII set"
        ));
    }
    Ok(())
}

/// A disk password: 8 to 256 printable ASCII characters. Anything else
/// can't be typed reliably at the boot prompt.
pub fn validate_password(p: &str) -> Result<(), String> {
    validate_typed(p, "disk password", PASSWORD_MIN, PASSWORD_MAX)
}

/// A TPM PIN: 4 to 64 printable ASCII characters. Short is enough: the
/// TPM locks out repeated wrong guesses, and the recovery key is the fallback.
pub fn validate_pin(p: &str) -> Result<(), String> {
    validate_typed(p, "disk PIN", PIN_MIN, PIN_MAX)
}

/// The device-mapper name of the opened root, which is what systemd's
/// `rd.luks.uuid` opens it as.
pub fn mapper_name(uuid: &str) -> String {
    format!("luks-{uuid}")
}

/// The UUID of a `luks-<uuid>` mapper name (without `/dev/mapper/`).
pub fn mapper_uuid(name: &str) -> Option<&str> {
    let u = name.strip_prefix("luks-")?;
    settings::validate_uuid(u).ok().map(|()| u)
}

/// A recovery key as `systemd-cryptenroll --recovery-key` prints it on a
/// pipe: its only output line. Eight groups of eight modhex characters.
pub fn parse_recovery_key(stdout: &str) -> Result<&str, String> {
    let mut lines = stdout.lines().map(str::trim).filter(|l| !l.is_empty());
    let (Some(key), None) = (lines.next(), lines.next()) else {
        return Err("systemd-cryptenroll did not print exactly one recovery key".into());
    };
    let groups: Vec<&str> = key.split('-').collect();
    let ok = groups.len() == 8
        && groups
            .iter()
            .all(|g| g.len() == 8 && g.bytes().all(|b| b"cbdefghijklnrtuv".contains(&b)));
    if ok {
        Ok(key)
    } else {
        Err("systemd-cryptenroll printed something that is not a recovery key".into())
    }
}

/// A console keymap name that is safe in a kernel command line.
pub fn validate_console_keymap(k: &str) -> Result<(), String> {
    let ok = (1..=64).contains(&k.len())
        && k.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+'));
    if ok {
        Ok(())
    } else {
        Err(format!("not a console keymap name: {k:?}"))
    }
}

/// The kernel arguments that unlock the root partition `uuid` in the
/// initramfs. What is typed at the prompt (the password, the PIN or the
/// recovery key) uses the user's own layout. No shell from the initramfs,
/// and a failed boot restarts so the boot counter can fall back. `discard`
/// is deliberate (see docs/helper-api.md). `tries=0` keeps asking, so typos
/// don't end in a restart loop.
pub fn kargs(enc: Encryption, uuid: &str, console_keymap: &str) -> Result<Vec<String>, String> {
    settings::validate_uuid(uuid)?;
    let options = match enc {
        Encryption::None => return Ok(Vec::new()),
        Encryption::Tpm => "discard,tpm2-device=auto,tries=0",
        Encryption::TpmPin => "discard,tpm2-device=auto,tries=0",
        Encryption::Password => "discard,tries=0",
    };
    validate_console_keymap(console_keymap)?;
    Ok(vec![
        format!("rd.luks.uuid={uuid}"),
        format!("rd.luks.options={uuid}={options}"),
        format!("vconsole.keymap={console_keymap}"),
        "rd.shell=0".into(),
        "rd.emergency=reboot".into(),
    ])
}

/// Whether the LUKS header (`cryptsetup luksDump --dump-json-metadata`)
/// holds exactly what is expected once the temporary key is gone: two key
/// slots (the recovery key and the TPM or the password), in every mode one
/// `systemd-recovery` token that points at a slot that exists, and for the
/// TPM one `systemd-tpm2` token that points at a slot that exists.
pub fn check_slots(json: &str, enc: Encryption) -> Result<(), String> {
    let bad = |what: &str| format!("the disk's key slots are not as expected: {what}");
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("cannot read the LUKS header: {e}"))?;
    let slots = v
        .get("keyslots")
        .and_then(|s| s.as_object())
        .ok_or_else(|| bad("no key slots"))?;
    if slots.len() != 2 {
        return Err(bad(&format!("{} slots instead of 2", slots.len())));
    }
    let empty = serde_json::Map::new();
    let tokens = match v.get("tokens") {
        None | Some(serde_json::Value::Null) => &empty,
        Some(t) => t.as_object().ok_or_else(|| bad("tokens"))?,
    };
    let (mut tpm, mut recovery, mut other) = (0, 0, 0);
    for t in tokens.values() {
        let points_at_a_slot = t
            .get("keyslots")
            .and_then(|k| k.as_array())
            .is_some_and(|k| {
                !k.is_empty()
                    && k.iter()
                        .all(|s| s.as_str().is_some_and(|s| slots.contains_key(s)))
            });
        match t.get("type").and_then(|t| t.as_str()) {
            Some("systemd-tpm2") if points_at_a_slot => tpm += 1,
            Some("systemd-recovery") if points_at_a_slot => recovery += 1,
            _ => other += 1,
        }
    }
    let want_tpm = usize::from(enc.uses_tpm());
    if tpm != want_tpm || recovery != 1 || other != 0 {
        return Err(bad(&format!(
            "{tpm} TPM tokens, {recovery} recovery tokens and {other} others, wanted {want_tpm}, 1 and 0"
        )));
    }
    Ok(())
}

/// The service that ties the TPM unlock to Secure Boot on the installed
/// system's first start. The installer enrols the TPM with no PCRs, because
/// the live system's PCR 7 is not always the installed system's: Ventoy, or
/// a boot menu starting the ISO as a file, puts its own boot loader's key
/// in it. At the first start PCR 7 is the system's own, and the service
/// seals the key to it, then stops running.
pub const SEAL_UNIT: &str = "telamon-tpm-seal.service";
/// While this file exists, the service runs at every start; it is removed
/// once the seal worked. Writing it again (and the PIN, with a PIN) seals
/// the disk anew to the current PCR 7.
pub const SEAL_MARKER: &str = "/etc/telamon-installer/tpm-seal";
pub const SEAL_MARKER_TEXT: &str = "# While this file exists, telamon-tpm-seal.service ties the disk's TPM unlock\n# to this PC's Secure Boot state (PCR 7) at each start. It removes this file\n# once that worked.\n";
/// The PIN (no newline), for unlocking with the current TPM key and for the
/// new one: 0600, on the encrypted root. Removed with the marker.
pub const SEAL_PIN: &str = "/etc/telamon-installer/tpm-pin";

/// The unit file of [`SEAL_UNIT`] for the LUKS volume `uuid`.
///
/// The enrolment unlocks with the current TPM key (and the PIN), adds the
/// one bound to PCR 7, then wipes every other TPM key: a failure at any
/// point leaves at least one TPM key and the recovery key. It doesn't hold
/// up the start (no ordering against the boot targets), and it never asks
/// for anything: the PIN comes as a credential, and a prompt would time out.
pub fn seal_unit(enc: Encryption, uuid: &str) -> Result<String, String> {
    settings::validate_uuid(uuid)?;
    let pin = match enc {
        Encryption::Tpm => false,
        Encryption::TpmPin => true,
        _ => return Err("only a TPM unlock is sealed at the first start".into()),
    };
    let mut s = format!(
        "# Written by the Telamon OS installer: ties the disk's TPM unlock to this\n\
         # PC's Secure Boot state (PCR 7) at the first start. See {SEAL_MARKER}.\n\
         [Unit]\n\
         Description=Tie the disk's TPM unlock to Secure Boot\n\
         DefaultDependencies=no\n\
         ConditionPathExists={SEAL_MARKER}\n\
         Wants=tpm2.target\n\
         After=local-fs.target systemd-tmpfiles-setup.service tpm2.target\n\
         Before=shutdown.target\n\
         Conflicts=shutdown.target\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart=/usr/bin/systemd-cryptenroll --unlock-tpm2-device=auto --tpm2-device=auto --tpm2-pcrs=7{} --wipe-slot=tpm2 /dev/disk/by-uuid/{uuid}\n\
         ExecStartPost=/usr/bin/rm -f {SEAL_PIN} {SEAL_MARKER}\n",
        if pin { " --tpm2-with-pin=yes" } else { "" },
    );
    if pin {
        s.push_str(&format!(
            "LoadCredential=cryptenroll.tpm2-pin:{SEAL_PIN}\n\
             LoadCredential=cryptenroll.new-tpm2-pin:{SEAL_PIN}\n"
        ));
    }
    s.push_str(
        "TimeoutStartSec=2min\n\
         UMask=0077\n\
         ProtectSystem=strict\n\
         ReadWritePaths=/etc/telamon-installer -/run/cryptsetup\n\
         PrivateTmp=yes\n\
         PrivateNetwork=yes\n\
         ProtectHome=yes\n\
         ProtectProc=invisible\n\
         ProtectKernelTunables=yes\n\
         ProtectKernelModules=yes\n\
         ProtectKernelLogs=yes\n\
         ProtectControlGroups=yes\n\
         ProtectClock=yes\n\
         ProtectHostname=yes\n\
         CapabilityBoundingSet=CAP_DAC_OVERRIDE CAP_IPC_LOCK\n\
         NoNewPrivileges=yes\n\
         RestrictAddressFamilies=AF_UNIX AF_ALG\n\
         RestrictNamespaces=yes\n\
         RestrictRealtime=yes\n\
         RestrictSUIDSGID=yes\n\
         LockPersonality=yes\n\
         MemoryDenyWriteExecute=yes\n\
         SystemCallArchitectures=native\n\
         SystemCallFilter=@system-service\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
    );
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "0b4f6b8e-2a0c-4d5e-9f1a-3c2b1a0d9e8f";

    #[test]
    fn the_first_start_seal_unit_binds_pcr_7_and_never_asks() {
        let tpm = seal_unit(Encryption::Tpm, UUID).unwrap();
        let exec = format!(
            "ExecStart=/usr/bin/systemd-cryptenroll --unlock-tpm2-device=auto --tpm2-device=auto --tpm2-pcrs=7 --wipe-slot=tpm2 /dev/disk/by-uuid/{UUID}\n"
        );
        assert!(tpm.contains(&exec), "{tpm}");
        assert!(tpm.contains("ConditionPathExists=/etc/telamon-installer/tpm-seal\n"));
        assert!(
            tpm.contains("ExecStartPost=/usr/bin/rm -f /etc/telamon-installer/tpm-pin /etc/telamon-installer/tpm-seal\n")
        );
        assert!(tpm.contains("DefaultDependencies=no\n"));
        assert!(tpm.contains("WantedBy=multi-user.target\n"));
        assert!(!tpm.contains("LoadCredential"));
        assert!(!tpm.contains("with-pin"));

        let pin = seal_unit(Encryption::TpmPin, UUID).unwrap();
        assert!(pin.contains("--tpm2-pcrs=7 --tpm2-with-pin=yes --wipe-slot=tpm2 "));
        assert!(
            pin.contains("LoadCredential=cryptenroll.tpm2-pin:/etc/telamon-installer/tpm-pin\n")
        );
        assert!(
            pin.contains(
                "LoadCredential=cryptenroll.new-tpm2-pin:/etc/telamon-installer/tpm-pin\n"
            )
        );

        assert!(seal_unit(Encryption::Password, UUID).is_err());
        assert!(seal_unit(Encryption::None, UUID).is_err());
        assert!(seal_unit(Encryption::Tpm, "x\nExecStart=/bin/sh").is_err());
        // every line is a section, a comment, empty, or a key=value
        for l in pin.lines() {
            assert!(
                l.is_empty() || l.starts_with('#') || l.starts_with('[') || l.contains('='),
                "{l}"
            );
        }
    }

    /// The unit runs as root with the disk's key in reach, so it is the most
    /// sandboxed thing the installer leaves behind: a failed first start must
    /// not turn into anything else, and what is left off is left off for a
    /// reason (the TPM and block devices, the root of the system's files).
    #[test]
    fn the_seal_unit_is_sandboxed_and_holds_no_secret() {
        for enc in [Encryption::Tpm, Encryption::TpmPin] {
            let unit = seal_unit(enc, UUID).unwrap();
            let keys: Vec<(&str, &str)> = unit
                .lines()
                .filter(|l| !l.starts_with('#') && !l.starts_with('['))
                .filter_map(|l| l.split_once('='))
                .collect();
            let has = |k: &str, v: &str| keys.contains(&(k, v));
            for (k, v) in [
                ("Type", "oneshot"),
                ("NoNewPrivileges", "yes"),
                ("ProtectSystem", "strict"),
                ("ReadWritePaths", "/etc/telamon-installer -/run/cryptsetup"),
                ("PrivateTmp", "yes"),
                ("PrivateNetwork", "yes"),
                ("ProtectHome", "yes"),
                ("ProtectProc", "invisible"),
                ("ProtectKernelTunables", "yes"),
                ("ProtectKernelModules", "yes"),
                ("ProtectKernelLogs", "yes"),
                ("ProtectControlGroups", "yes"),
                ("ProtectClock", "yes"),
                ("ProtectHostname", "yes"),
                ("CapabilityBoundingSet", "CAP_DAC_OVERRIDE CAP_IPC_LOCK"),
                ("RestrictAddressFamilies", "AF_UNIX AF_ALG"),
                ("RestrictNamespaces", "yes"),
                ("RestrictRealtime", "yes"),
                ("RestrictSUIDSGID", "yes"),
                ("LockPersonality", "yes"),
                ("MemoryDenyWriteExecute", "yes"),
                ("SystemCallArchitectures", "native"),
                ("SystemCallFilter", "@system-service"),
                ("UMask", "0077"),
                ("TimeoutStartSec", "2min"),
                ("ConditionPathExists", SEAL_MARKER),
            ] {
                assert!(has(k, v), "{k}={v} in\n{unit}");
            }
            // it runs once: the marker and the PIN go when the seal worked, and only then
            let post = keys.iter().find(|(k, _)| *k == "ExecStartPost").unwrap().1;
            assert_eq!(post, format!("/usr/bin/rm -f {SEAL_PIN} {SEAL_MARKER}"));
            assert_eq!(
                keys.iter()
                    .filter(|(k, _)| k.starts_with("ExecStart"))
                    .count(),
                2
            );
            // the PIN is a credential, never in an argument or the environment
            assert!(!unit.contains("Environment"));
            assert!(!unit.contains("PIN="));
            let creds = keys.iter().filter(|(k, _)| *k == "LoadCredential").count();
            assert_eq!(creds, if enc == Encryption::TpmPin { 2 } else { 0 });
            // it does not ask: no prompt can come from a service with no terminal
            assert!(!unit.contains("StandardInput"));
            // no capability beyond reading the key files and locking memory
            assert!(!unit.contains("CAP_SYS_ADMIN") && !unit.contains("CAP_NET"));
        }
    }

    const KEY: &str = "ulcbjnni-ehtlcfnl-ntenkltt-vjuiicdf-hvdkerji-fjkurjhr-lckjntdb-kvkeeide";

    #[test]
    fn encryption_modes() {
        assert_eq!("none".parse(), Ok(Encryption::None));
        assert_eq!("tpm".parse(), Ok(Encryption::Tpm));
        assert_eq!("tpm-pin".parse(), Ok(Encryption::TpmPin));
        assert_eq!("password".parse(), Ok(Encryption::Password));
        for s in ["", "TPM", "yes", "tpm ", "none\n", "tpm_pin"] {
            assert!(s.parse::<Encryption>().is_err(), "{s:?}");
        }
        assert!(!Encryption::None.on() && Encryption::Tpm.on() && Encryption::Password.on());
        assert!(Encryption::TpmPin.uses_tpm() && !Encryption::Password.uses_tpm());
        assert!(Encryption::TpmPin.has_secret() && !Encryption::Tpm.has_secret());
    }

    #[test]
    fn passwords_and_pins_are_printable_ascii() {
        for ok in [
            "12345678",
            "with space  ",
            &"x".repeat(256),
            " ~!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~",
        ] {
            assert!(validate_password(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            "",
            "1234567",
            &"x".repeat(257),
            "pässwörd1",
            "日本語のパスワード",
            "abcdefg\n",
            "abcdefg\r",
            "abc\tdefgh",
            "abcdefg\0h",
            "abcdefg\u{7f}",
            "abcdefg\u{1b}",
            "abcdefg\u{85}",
            "abcdefg\u{a0}",
        ] {
            assert!(validate_password(bad).is_err(), "{bad:?}");
        }
        assert!(validate_pin("1234").is_ok() && validate_pin(&"7".repeat(64)).is_ok());
        for bad in ["", "123", &"7".repeat(65), "12345é", "1234\n56", "12345\t6"] {
            assert!(validate_pin(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn recovery_keys_are_strict() {
        assert_eq!(parse_recovery_key(&format!("{KEY}\n")), Ok(KEY));
        for bad in [
            "",
            "\n",
            KEY.to_uppercase().as_str(),
            &KEY[1..],
            &format!("{KEY}-aaaaaaaa"),
            &format!("{KEY}\n{KEY}\n"),
            &format!("key: {KEY}"),
            &KEY.replace('u', "a"),
            &KEY.replace('-', ""),
            "A secret recovery key has been generated for this volume:",
        ] {
            assert!(parse_recovery_key(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn mapper_names() {
        assert_eq!(mapper_name(UUID), format!("luks-{UUID}"));
        assert_eq!(mapper_uuid(&mapper_name(UUID)), Some(UUID));
        assert_eq!(mapper_uuid("luks-x"), None);
        assert_eq!(mapper_uuid("ventoy"), None);
        assert_eq!(mapper_uuid(&format!("luks-{UUID}/../x")), None);
    }

    #[test]
    fn kargs_per_mode() {
        assert_eq!(
            kargs(Encryption::None, UUID, "de").unwrap(),
            Vec::<String>::new()
        );
        let common = |options: &str| {
            vec![
                format!("rd.luks.uuid={UUID}"),
                format!("rd.luks.options={UUID}={options}"),
                "vconsole.keymap=de-latin1-nodeadkeys".to_string(),
                "rd.shell=0".to_string(),
                "rd.emergency=reboot".to_string(),
            ]
        };
        let k = |e| kargs(e, UUID, "de-latin1-nodeadkeys").unwrap();
        assert_eq!(
            k(Encryption::Tpm),
            common("discard,tpm2-device=auto,tries=0")
        );
        assert_eq!(
            k(Encryption::TpmPin),
            common("discard,tpm2-device=auto,tries=0")
        );
        assert_eq!(k(Encryption::Password), common("discard,tries=0"));
        for enc in [Encryption::Tpm, Encryption::TpmPin, Encryption::Password] {
            for bad in ["", "de latin", "-de", "de\nx", "de=1", &"a".repeat(65)] {
                assert!(kargs(enc, UUID, bad).is_err(), "{bad:?}");
            }
            assert!(kargs(enc, "not-a-uuid", "de").is_err());
        }
    }

    fn header(slots: &[&str], tokens: &str) -> String {
        let s: Vec<String> = slots
            .iter()
            .map(|s| format!("\"{s}\":{{\"type\":\"luks2\"}}"))
            .collect();
        format!("{{\"keyslots\":{{{}}},\"tokens\":{tokens}}}", s.join(","))
    }

    #[test]
    fn slots_must_be_exactly_the_expected_ones() {
        let rec = r#""1":{"type":"systemd-recovery","keyslots":["1"]}"#;
        let tpm = r#""0":{"type":"systemd-tpm2","keyslots":["2"]}"#;
        let with_rec = format!("{{{tpm},{rec}}}");
        let only_rec = format!("{{{rec}}}");
        assert!(check_slots(&header(&["1", "2"], &with_rec), Encryption::Tpm).is_ok());
        assert!(check_slots(&header(&["1", "2"], &with_rec), Encryption::TpmPin).is_ok());
        assert!(check_slots(&header(&["1", "2"], &only_rec), Encryption::TpmPin).is_err());
        assert!(check_slots(&header(&["1", "2"], &only_rec), Encryption::Password).is_ok());
        // the recovery token is required in every mode
        assert!(check_slots(&header(&["1", "2"], "null"), Encryption::Password).is_err());
        assert!(check_slots(&header(&["1", "2"], "{}"), Encryption::Password).is_err());
        let tpm_only = format!("{{{tpm}}}");
        assert!(check_slots(&header(&["1", "2"], &tpm_only), Encryption::Tpm).is_err());
        // a second recovery token, or one pointing nowhere
        let two = format!("{{{rec},\"2\":{{\"type\":\"systemd-recovery\",\"keyslots\":[\"2\"]}}}}");
        assert!(check_slots(&header(&["1", "2"], &two), Encryption::Password).is_err());
        let lost = r#"{"1":{"type":"systemd-recovery","keyslots":["9"]}}"#;
        assert!(check_slots(&header(&["1", "2"], lost), Encryption::Password).is_err());
        // the temporary slot is still there
        assert!(check_slots(&header(&["0", "1", "2"], &with_rec), Encryption::Tpm).is_err());
        assert!(check_slots(&header(&["1"], &with_rec), Encryption::Tpm).is_err());
        // a TPM token is missing, extra, dangling or somebody else's
        assert!(check_slots(&header(&["1", "2"], &only_rec), Encryption::Tpm).is_err());
        assert!(check_slots(&header(&["1", "2"], &with_rec), Encryption::Password).is_err());
        let dangling =
            format!("{{{rec},\"0\":{{\"type\":\"systemd-tpm2\",\"keyslots\":[\"7\"]}}}}");
        assert!(check_slots(&header(&["1", "2"], &dangling), Encryption::Tpm).is_err());
        let fido = format!("{{{rec},\"0\":{{\"type\":\"systemd-fido2\",\"keyslots\":[\"2\"]}}}}");
        assert!(check_slots(&header(&["1", "2"], &fido), Encryption::Password).is_err());
        assert!(check_slots("{}", Encryption::Tpm).is_err());
        assert!(check_slots("nope", Encryption::Tpm).is_err());
    }
}
