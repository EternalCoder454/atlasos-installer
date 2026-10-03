//! The languages the UI offers: the UTF-8 locales the system has
//! (`localectl list-locales`), which are also the installed system's, since
//! the live session is the same image.

use crate::settings::validate_locale;

/// What the Welcome page selects first.
pub const DEFAULT_LOCALE: &str = "en_US.UTF-8";

/// Used when `localectl` can't be run.
const FALLBACK: &[&str] = &[
    "ar_EG.UTF-8",
    "cs_CZ.UTF-8",
    "da_DK.UTF-8",
    "de_AT.UTF-8",
    "de_CH.UTF-8",
    "de_DE.UTF-8",
    "el_GR.UTF-8",
    "en_AU.UTF-8",
    "en_CA.UTF-8",
    "en_GB.UTF-8",
    "en_IE.UTF-8",
    "en_IN.UTF-8",
    "en_NZ.UTF-8",
    "en_US.UTF-8",
    "es_ES.UTF-8",
    "es_MX.UTF-8",
    "fi_FI.UTF-8",
    "fr_CA.UTF-8",
    "fr_FR.UTF-8",
    "he_IL.UTF-8",
    "hi_IN.UTF-8",
    "hu_HU.UTF-8",
    "it_IT.UTF-8",
    "ja_JP.UTF-8",
    "ko_KR.UTF-8",
    "nb_NO.UTF-8",
    "nl_NL.UTF-8",
    "pl_PL.UTF-8",
    "pt_BR.UTF-8",
    "pt_PT.UTF-8",
    "ro_RO.UTF-8",
    "ru_RU.UTF-8",
    "sv_SE.UTF-8",
    "tr_TR.UTF-8",
    "uk_UA.UTF-8",
    "zh_CN.UTF-8",
    "zh_TW.UTF-8",
];

/// The output of `localectl list-locales`: one per line. Keeps the UTF-8
/// locales the helper accepts, without duplicates.
pub fn parse_list(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| validate_locale(l).is_ok())
        .map(String::from)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// [`parse_list`] of `localectl`'s output, or a short built-in list when it
/// gave nothing. The default locale is always in it.
pub fn offered(localectl_output: Option<&str>) -> Vec<String> {
    let mut list = localectl_output.map(parse_list).unwrap_or_default();
    if list.is_empty() {
        list = FALLBACK.iter().map(|s| s.to_string()).collect();
    }
    if !list.iter().any(|l| l == DEFAULT_LOCALE) {
        list.push(DEFAULT_LOCALE.into());
        list.sort();
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_valid_utf8_locales_once() {
        let text = "C.UTF-8\naa_DJ.UTF-8\nde_DE.UTF-8\nde_DE.UTF-8\nde_DE@euro\n\
            sr_RS.UTF-8@latin\nPOSIX\n\n en_US.UTF-8 \nyue_HK.UTF-8\n";
        assert_eq!(
            parse_list(text),
            [
                "aa_DJ.UTF-8",
                "de_DE.UTF-8",
                "en_US.UTF-8",
                "sr_RS.UTF-8@latin",
                "yue_HK.UTF-8"
            ]
        );
    }

    #[test]
    fn falls_back_and_always_has_the_default() {
        let f = offered(None);
        assert!(f.len() > 20 && f.iter().all(|l| validate_locale(l).is_ok()));
        assert_eq!(offered(Some("C.UTF-8\n")), f);
        assert_eq!(
            offered(Some("de_DE.UTF-8\n")),
            ["de_DE.UTF-8", "en_US.UTF-8"]
        );
    }
}
