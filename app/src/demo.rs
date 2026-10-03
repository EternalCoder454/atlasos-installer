//! Demo mode, for screenshots and UI work: `ATLAS_INSTALLER_DEMO=1`, or a
//! comma-separated list of the flags below. Nothing touches the system bus,
//! the disks or the session; the disks come from `fixtures/disks.json`.
//!
//! | Flag | Effect |
//! |---|---|
//! | `wired` | Connected by cable: the Wi-Fi page is skipped |
//! | `nowifi` | No Wi-Fi adapter |
//! | `nodisks` | No disk can be offered |
//! | `onedisk` | Only the Windows disk, so it is chosen for you |
//! | `emptydisk` | Only the empty disk |
//! | `busy` | ListDisks answers Busy |
//! | `fail` | The install fails while copying |
//! | `hold` | The install stops moving at 42 % (to screenshot it) |
//! | `mok` | NVIDIA with Secure Boot on: the Restart page shows a MOK password |
//! | `warn` | The install succeeds with warnings |
//! | `fast` | The install takes 2 seconds instead of 20 |
//!
//! `ATLAS_INSTALLER_DEMO_PAGE=<step>` opens at a step (welcome, keyboard,
//! wifi, disk, review, progress, restart), with the first disk chosen.

use std::time::Duration;

use crate::helper;
use crate::network::{Network, Security, WifiState};
use crate::view::Done;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags {
    pub wired: bool,
    pub nowifi: bool,
    pub nodisks: bool,
    pub onedisk: bool,
    pub emptydisk: bool,
    pub busy: bool,
    pub fail: bool,
    pub hold: bool,
    pub mok: bool,
    pub warn: bool,
    pub fast: bool,
}

/// `None` unless `ATLAS_INSTALLER_DEMO` is set (and not empty or `0`).
/// Never on the installer media: a fake install there would tell the user
/// AtlasOS is installed when it isn't.
pub fn from_env() -> Option<Flags> {
    let f = parse(&std::env::var("ATLAS_INSTALLER_DEMO").ok()?)?;
    if crate::system::live_session() {
        eprintln!("atlas-installer: ignoring ATLAS_INSTALLER_DEMO in the live session");
        return None;
    }
    Some(f)
}

pub fn parse(v: &str) -> Option<Flags> {
    let v = v.trim();
    if v.is_empty() || v == "0" {
        return None;
    }
    let mut f = Flags::default();
    for w in v.split(',').map(str::trim) {
        match w {
            "wired" => f.wired = true,
            "nowifi" => f.nowifi = true,
            "nodisks" => f.nodisks = true,
            "onedisk" => f.onedisk = true,
            "emptydisk" => f.emptydisk = true,
            "busy" => f.busy = true,
            "fail" => f.fail = true,
            "hold" => f.hold = true,
            "mok" => f.mok = true,
            "warn" => f.warn = true,
            "fast" => f.fast = true,
            _ => {}
        }
    }
    Some(f)
}

pub async fn list_disks(f: &Flags) -> Result<String, helper::Error> {
    tokio::time::sleep(Duration::from_millis(600)).await;
    if f.busy {
        return Err(helper::describe("net.eterneon.atlas.Error.Busy", None));
    }
    if f.nodisks {
        return Ok(r#"{"disks":[],"hidden":[]}"#.into());
    }
    let all = include_str!("../fixtures/disks.json");
    let keep = match (f.onedisk, f.emptydisk) {
        (true, _) => "nvme0n1",
        (_, true) => "sda",
        _ => return Ok(all.into()),
    };
    let mut list: serde_json::Value = serde_json::from_str(all).expect("fixture");
    if let Some(disks) = list["disks"].as_array_mut() {
        disks.retain(|d| d["id"] == keep);
    }
    Ok(list.to_string())
}

fn net(ssid: &str, strength: u8, security: Security) -> Network {
    Network {
        ssid: ssid.into(),
        strength,
        security,
        active: false,
    }
}

pub fn wifi(f: &Flags, connected: Option<&str>) -> WifiState {
    if f.nowifi {
        return WifiState {
            wired: f.wired,
            ..Default::default()
        };
    }
    let mut networks = vec![
        net("Atlas Home", 82, Security::Psk),
        net("Atlas Home 5G", 64, Security::Sae),
        net("Corner Cafe", 47, Security::Open),
        net("Office", 30, Security::Enterprise),
        net("DIRECT-printer", 18, Security::Psk),
    ];
    if let Some(c) = connected
        && let Some(n) = networks.iter_mut().find(|n| n.ssid == c)
    {
        n.active = true;
    }
    networks.sort_by(|a, b| b.active.cmp(&a.active).then(b.strength.cmp(&a.strength)));
    WifiState {
        available: true,
        wired: f.wired,
        networks,
        active_ssid: connected.unwrap_or("").into(),
        active_uuid: connected
            .map(|_| "6f1c9a52-0d4e-4b7a-9c3e-2a5f8e7d1b04".into())
            .unwrap_or_default(),
    }
}

/// "wrong" is the wrong password.
pub async fn connect(ssid: &str, password: &str) -> Result<String, String> {
    tokio::time::sleep(Duration::from_millis(1500)).await;
    if password == "wrong" {
        return Err(format!(
            "Couldn't connect to {ssid}. Check the password and try again."
        ));
    }
    Ok("6f1c9a52-0d4e-4b7a-9c3e-2a5f8e7d1b04".into())
}

const STEPS: &[(f64, &str)] = &[
    (0.00, "Getting ready"),
    (0.02, "Creating partitions"),
    (0.04, "Formatting"),
    (0.07, "Copying AtlasOS"),
    (0.85, "Installing the bootloader"),
    (0.93, "Applying your settings"),
    (0.97, "Finishing up"),
];

fn text_at(f: f64) -> &'static str {
    STEPS
        .iter()
        .rev()
        .find(|(start, _)| f >= *start)
        .map_or("Getting ready", |s| s.1)
}

/// About 20 seconds from start to finish (2 with `fast`).
pub async fn install(f: &Flags, progress: impl Fn(f64, String)) -> Result<String, helper::Error> {
    let mut x = 0.0;
    while x < 1.0 {
        if f.hold && x >= 0.42 {
            std::future::pending::<()>().await;
        }
        if f.fail && x >= 0.5 {
            return Err(helper::describe(
                "net.eterneon.atlas.Error.Failed",
                Some(
                    "bootc install failed: error: Installing to filesystem: Creating ostree deployment: No space left on device. The install log is at /run/atlas-installer/install.log",
                ),
            ));
        }
        progress(x, text_at(x).into());
        tokio::time::sleep(Duration::from_millis(if f.fast { 20 } else { 200 })).await;
        x = (x + 0.01).min(1.0);
    }
    progress(1.0, "Finishing up".into());
    let done = Done {
        mok_password: if f.mok {
            "48201937".into()
        } else {
            String::new()
        },
        windows_entry: true,
        warnings: if f.warn {
            vec!["The firmware boot entry couldn't be renamed to AtlasOS: it is still called Fedora.".into()]
        } else {
            Vec::new()
        },
        log: "/run/atlas-installer/install.log".into(),
    };
    Ok(serde_json::json!({
        "mok_password": if done.mok_password.is_empty() { None } else { Some(done.mok_password) },
        "windows_entry": done.windows_entry,
        "warnings": done.warnings,
        "log": done.log,
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("0"), None);
        assert_eq!(parse("1"), Some(Flags::default()));
        let f = parse("wired, mok,unknown").unwrap();
        assert!(f.wired && f.mok && !f.fail);
    }

    #[test]
    fn stage_texts() {
        assert_eq!(text_at(0.0), "Getting ready");
        assert_eq!(text_at(0.5), "Copying AtlasOS");
        assert_eq!(text_at(0.99), "Finishing up");
    }
}
