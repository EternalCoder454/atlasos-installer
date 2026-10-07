//! The apps the user picks during the install, which the new system adds at
//! its first start (the install itself is offline). The list is
//! `firstboot/apps.json`, shared with the first-start script: one line per
//! app. The install only records the chosen IDs, checked against the list.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Where the choices go in the new system (under its `/var`), root-owned.
pub const RECORD: &str = "lib/telamon/first-boot-apps.json";
/// Where they went until the rename (Atlas Installer, and the first-start
/// script of an image built before it): written too, with the same content,
/// for one release. The new first-start script reads either, and removes
/// both when it is done.
pub const LEGACY_RECORD: &str = "lib/atlasos/first-boot-apps.json";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    /// Pick at most one.
    Browser,
    /// Pick any.
    Developer,
    /// Local AI (a model runner and a chat app). Pick any.
    Ai,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub id: String,
    pub group: Group,
    pub name: String,
    pub summary: String,
    /// `flatpak:<app id>` (system-wide, from Flathub), `mise` (mise itself,
    /// for each account), `mise:<tool>` (through mise), or
    /// `toolbox:<packages>` (Fedora packages in a toolbox).
    pub install: String,
    /// Flatpak add-ons installed with a `flatpak:` app, system-wide.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<String>,
    /// Like `extra`, but only on a computer with an AMD graphics card
    /// (checked at the first start).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_amd: Vec<String>,
}

/// The list, in the order the page shows it.
pub fn catalog() -> &'static [App] {
    static CATALOG: OnceLock<Vec<App>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../../firstboot/apps.json"))
            .expect("firstboot/apps.json is valid (tested)")
    })
}

pub fn find(id: &str) -> Option<&'static App> {
    catalog().iter().find(|a| a.id == id)
}

/// Checks the IDs the UI sends: each in the list, none twice, at most one
/// browser. Returns them in the list's order.
pub fn validate(ids: &[String]) -> Result<Vec<&'static App>, String> {
    let mut chosen = Vec::new();
    for id in ids {
        let app = find(id).ok_or_else(|| format!("not an app on the list: {id:?}"))?;
        if chosen.iter().any(|a: &&App| a.id == app.id) {
            return Err(format!("an app was chosen twice: {id:?}"));
        }
        chosen.push(app);
    }
    if chosen.iter().filter(|a| a.group == Group::Browser).count() > 1 {
        return Err("more than one web browser was chosen".into());
    }
    let order = |a: &App| catalog().iter().position(|c| c.id == a.id);
    chosen.sort_by_key(|a| order(a));
    Ok(chosen)
}

/// The record's contents: `{"version": 1, "apps": [...]}`.
pub fn record(apps: &[&App]) -> String {
    let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
    format!("{}\n", serde_json::json!({ "version": 1, "apps": ids }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_list_is_well_formed() {
        let c = catalog();
        assert!(c.iter().any(|a| a.group == Group::Browser));
        for (i, a) in c.iter().enumerate() {
            assert!(
                !a.id.is_empty()
                    && a.id
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "{a:?}"
            );
            assert!(c[..i].iter().all(|b| b.id != a.id), "{} twice", a.id);
            assert!(!a.name.is_empty() && !a.summary.is_empty(), "{a:?}");
            let word = |s: &str| {
                !s.is_empty()
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            };
            let ok = match a.install.split_once(':') {
                None => a.install == "mise",
                Some(("flatpak", app)) => word(app) && app.contains('.'),
                Some(("mise", tool)) => word(tool),
                Some(("toolbox", pkgs)) => pkgs.split(' ').all(word),
                _ => false,
            };
            assert!(ok, "{a:?}");
            // Add-ons belong to a Flatpak app, and are Flatpak ids too.
            for r in a.extra.iter().chain(&a.extra_amd) {
                assert!(
                    a.install.starts_with("flatpak:") && word(r) && r.contains('.'),
                    "{a:?}"
                );
            }
            // A mise tool needs mise, which comes first on the list.
            if a.install.starts_with("mise:") {
                assert!(c[..i].iter().any(|b| b.install == "mise"), "{a:?}");
            }
        }
    }

    #[test]
    fn local_ai_is_two_ordinary_choices() {
        let ai: Vec<_> = catalog().iter().filter(|a| a.group == Group::Ai).collect();
        assert_eq!(
            ai.iter()
                .map(|a| (a.id.as_str(), a.install.as_str()))
                .collect::<Vec<_>>(),
            [
                ("ollama", "mise:ollama"),
                ("alpaca", "flatpak:com.jeffser.Alpaca")
            ]
        );
        let alpaca = find("alpaca").unwrap();
        assert_eq!(alpaca.extra, ["com.jeffser.Alpaca.Plugins.Ollama"]);
        assert_eq!(alpaca.extra_amd, ["com.jeffser.Alpaca.Plugins.AMD"]);
        // Both can be picked, with or without the other tools.
        let got = validate(&ids(&["alpaca", "firefox", "ollama", "mise"])).unwrap();
        assert_eq!(
            got.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["firefox", "mise", "ollama", "alpaca"]
        );
    }

    #[test]
    fn choices_are_checked() {
        let got = validate(&ids(&["gh", "firefox"])).unwrap();
        assert_eq!(
            got.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["firefox", "gh"]
        );
        assert!(validate(&[]).unwrap().is_empty());
        assert!(validate(&ids(&["firefox", "brave"])).is_err());
        assert!(validate(&ids(&["gh", "gh"])).is_err());
        assert!(validate(&ids(&["https://example.com/x.flatpakref"])).is_err());
        assert!(validate(&ids(&["../etc"])).is_err());
    }

    #[test]
    fn record_holds_only_ids() {
        let apps = validate(&ids(&["gh", "brave"])).unwrap();
        assert_eq!(
            record(&apps),
            "{\"apps\":[\"brave\",\"gh\"],\"version\":1}\n"
        );
    }
}
