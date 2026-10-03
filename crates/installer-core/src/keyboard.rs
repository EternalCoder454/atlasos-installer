//! The keyboard layouts the UI offers, from xkeyboard-config's `base.lst`,
//! and the layout a language suggests.

use serde::Serialize;

use crate::settings::Keymap;

/// Where xkeyboard-config lists its layouts and variants.
pub const BASE_LST: &str = "/usr/share/X11/xkb/rules/base.lst";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Variant {
    /// `nodeadkeys`
    pub id: String,
    /// "German (no dead keys)"
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Layout {
    /// `de`
    pub id: String,
    /// "German"
    pub name: String,
    pub variants: Vec<Variant>,
}

/// The `! layout` and `! variant` sections of `base.lst`, sorted by name.
/// Only what the helper accepts ([`Keymap::parse`]) is kept.
pub fn parse_base_lst(text: &str) -> Vec<Layout> {
    let mut layouts: Vec<Layout> = Vec::new();
    let mut section = "";
    for line in text.lines() {
        if let Some(s) = line.strip_prefix('!') {
            section = s.trim();
            continue;
        }
        let Some((id, rest)) = line.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim();
        match section {
            "layout" => {
                if Keymap::parse(id).is_ok() {
                    layouts.push(Layout {
                        id: id.into(),
                        name: rest.into(),
                        variants: Vec::new(),
                    });
                }
            }
            "variant" => {
                // "  nodeadkeys      de: German (no dead keys)"
                let Some((layout, name)) = rest.split_once(": ") else {
                    continue;
                };
                if Keymap::parse(&format!("{layout}({id})")).is_err() {
                    continue;
                }
                if let Some(l) = layouts.iter_mut().find(|l| l.id == layout) {
                    l.variants.push(Variant {
                        id: id.into(),
                        name: name.trim().into(),
                    });
                }
            }
            _ => {}
        }
    }
    layouts.sort_by_cached_key(|l| l.name.to_lowercase());
    for l in &mut layouts {
        l.variants.sort_by_cached_key(|v| v.name.to_lowercase());
    }
    layouts
}

/// The layout a locale suggests: the country's layout when there is one
/// (`de_AT` → `at`), then the language's (`de` → `de`), then `us`.
pub fn default_layout(locale: &str, layouts: &[Layout]) -> String {
    let has = |id: &str| layouts.iter().any(|l| l.id == id);
    let base = locale.split(['.', '@']).next().unwrap_or("");
    let (lang, country) = match base.split_once('_') {
        Some((l, c)) => (l, Some(c.to_ascii_lowercase())),
        None => (base, None),
    };
    // Where xkb names a layout differently from the ISO code, or where the
    // country's layout is not the one its speakers of this language use.
    let special = match (lang, country.as_deref()) {
        ("en", Some("us" | "au" | "nz" | "ph" | "sg" | "hk")) => Some("us"),
        ("en", Some("ca")) => Some("us"),
        ("fr", Some("ca")) => Some("ca"),
        ("en", _) => None,
        ("de", Some("ch")) => Some("ch"),
        ("fr", Some("ch")) => Some("ch(fr)"),
        ("it", Some("ch")) => Some("ch"),
        ("nl", Some("be")) | ("fr", Some("be")) | ("de", Some("be")) => Some("be"),
        ("ca", _) => Some("es(cat)"),
        ("eu" | "gl", _) => Some("es"),
        ("pt", Some("br")) => Some("br"),
        ("pt", _) => Some("pt"),
        ("es", Some("es")) => Some("es"),
        ("es", _) => Some("latam"),
        ("ar", _) => Some("ara"),
        ("zh", Some("tw")) => Some("tw"),
        ("zh", _) => Some("cn"),
        // Belarusian, not the Belgian layout `be`
        ("be", _) => Some("by"),
        ("ja", _) => Some("jp"),
        ("ko", _) => Some("kr"),
        ("uk", _) => Some("ua"),
        ("el", _) => Some("gr"),
        ("he", _) => Some("il"),
        ("cs", _) => Some("cz"),
        ("da", _) => Some("dk"),
        ("sv", _) => Some("se"),
        ("nb" | "nn" | "no", _) => Some("no"),
        ("et", _) => Some("ee"),
        ("sl", _) => Some("si"),
        ("sr", _) if locale.contains("@latin") => Some("rs(latin)"),
        ("sr", _) => Some("rs"),
        ("fa", _) => Some("ir"),
        ("hi" | "mr" | "ta" | "te" | "bn" | "gu" | "kn" | "ml" | "pa", Some("in")) => Some("in"),
        _ => None,
    };
    if let Some(s) = special {
        let layout = s.split('(').next().unwrap_or(s);
        if has(layout) {
            return s.into();
        }
    }
    if lang != "en"
        && let Some(c) = &country
        && has(c)
    {
        return c.clone();
    }
    if has(lang) {
        return lang.into();
    }
    match country.as_deref() {
        Some("gb") if has("gb") => "gb".into(),
        Some("ie") if has("ie") => "ie".into(),
        Some("in") if has("in") => "in(eng)".into(),
        Some("za") if has("za") => "za".into(),
        _ => "us".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LST: &str = "! model\n  pc105           Generic 105-key PC\n\n\
        ! layout\n  us              English (US)\n  de              German\n  \
        at              German (Austria)\n  ch              German (Switzerland)\n  \
        gb              English (UK)\n  es              Spanish\n  latam           Spanish (Latin American)\n  \
        br              Portuguese (Brazil)\n  pt              Portuguese\n  fr              French\n  \
        ca              French (Canada)\n  BAD             Not a layout id\n\n\
        ! variant\n  nodeadkeys      de: German (no dead keys)\n  \
        mac             de: German (Macintosh)\n  bad(x)          de: Bad\n  \
        fr              ch: French (Switzerland)\n  cat             es: Catalan (Spain, with middle-dot L)\n  \
        intl            us: English (US, intl., with dead keys)\n  \
        x               nowhere: Variant of a missing layout\n\n\
        ! option\n  grp             Switching to another layout\n";

    fn layouts() -> Vec<Layout> {
        parse_base_lst(LST)
    }

    #[test]
    fn parses_layouts_and_variants_sorted() {
        let l = layouts();
        let names: Vec<&str> = l.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names.first(), Some(&"English (UK)"));
        assert!(!names.contains(&"Not a layout id"));
        assert!(!names.contains(&"Generic 105-key PC"));
        let de = l.iter().find(|l| l.id == "de").unwrap();
        assert_eq!(
            de.variants,
            vec![
                Variant {
                    id: "mac".into(),
                    name: "German (Macintosh)".into()
                },
                Variant {
                    id: "nodeadkeys".into(),
                    name: "German (no dead keys)".into()
                },
            ]
        );
        assert!(l.iter().all(|l| l.variants.iter().all(|v| v.id != "x")));
    }

    #[test]
    fn real_base_lst_when_present() {
        let Ok(text) = std::fs::read_to_string(BASE_LST) else {
            return;
        };
        let l = parse_base_lst(&text);
        assert!(l.len() > 50);
        for layout in &l {
            for v in &layout.variants {
                Keymap::parse(&format!("{}({})", layout.id, v.id)).unwrap();
            }
        }
        assert_eq!(default_layout("de_DE.UTF-8", &l), "de");
        assert_eq!(default_layout("en_US.UTF-8", &l), "us");
        assert_eq!(default_layout("en_GB.UTF-8", &l), "gb");
        assert_eq!(default_layout("ja_JP.UTF-8", &l), "jp");
        assert_eq!(default_layout("zh_TW.UTF-8", &l), "tw");
        assert_eq!(default_layout("be_BY.UTF-8", &l), "by");
        assert_eq!(default_layout("fr_CH.UTF-8", &l), "ch(fr)");
    }

    #[test]
    fn default_layouts() {
        let l = layouts();
        let d = |loc| default_layout(loc, &l);
        assert_eq!(d("de_DE.UTF-8"), "de");
        assert_eq!(d("de_AT.UTF-8"), "at");
        assert_eq!(d("de_CH.UTF-8"), "ch");
        assert_eq!(d("fr_CH.UTF-8"), "ch(fr)");
        assert_eq!(d("en_US.UTF-8"), "us");
        assert_eq!(d("en_GB.UTF-8"), "gb");
        assert_eq!(d("en_CA.UTF-8"), "us");
        assert_eq!(d("fr_CA.UTF-8"), "ca");
        assert_eq!(d("es_ES.UTF-8"), "es");
        assert_eq!(d("es_MX.UTF-8"), "latam");
        assert_eq!(d("pt_BR.UTF-8"), "br");
        assert_eq!(d("pt_PT.UTF-8"), "pt");
        assert_eq!(d("ca_ES.UTF-8"), "es(cat)");
        assert_eq!(d("fr_FR.UTF-8"), "fr");
        // no layout for the country or the language
        assert_eq!(d("xx_YY.UTF-8"), "us");
        assert_eq!(d("en_ZA.UTF-8"), "us");
    }
}
