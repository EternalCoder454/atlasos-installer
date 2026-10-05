//! Demo mode, for screenshots and UI work: `ATLAS_INSTALLER_DEMO=1`, or a
//! comma-separated list of the flags below. Nothing touches the system bus,
//! the disks or the session; the disks come from `fixtures/disks.json`.
//!
//! | Flag | Effect |
//! |---|---|
//! | `wired` | Connected by cable: the Wi-Fi page is skipped |
//! | `nowifi` | No Wi-Fi adapter |
//! | `crowd` | Many Wi-Fi networks, so the list scrolls |
//! | `nodisks` | No disk can be offered |
//! | `onedisk` | Only the Windows disk, so it is chosen for you |
//! | `emptydisk` | Only the empty disk |
//! | `twins` | Two empty disks of the same model and size |
//! | `busy` | ListDisks answers Busy |
//! | `fail` | The install fails while copying |
//! | `hold` | The install stops moving at 42 % (to screenshot it) |
//! | `mok` | NVIDIA with Secure Boot on: the Restart page shows a MOK password |
//! | `warn` | The install succeeds with warnings |
//! | `fast` | The install takes 2 seconds instead of 20 |
//! | `running` | The helper is already installing, at 42 %: the UI starts on the Progress page and follows it (as after a crash) |
//! | `finished` | The helper has finished installing: the UI starts on the Restart page (`mok`, `warn` and `cd` apply) |
//! | `notpm` | This PC has no TPM 2.0: encryption needs a password |
//! | `nosb` | Secure Boot is off on this PC (with a TPM, the PIN switch then starts on) |
//! | `pin` | The Disk page opens with the start-up PIN switched on (with a TPM) |
//! | `encrypt` | The Disk page opens with "Encrypt this disk" turned on |
//! | `cd` | Started from a disc, not a USB stick (the Restart page's wording) |
//!
//! `ATLAS_INSTALLER_DEMO_PAGE=<step>` opens at a step (welcome, keyboard,
//! wifi, disk, review, progress, restart), with the first disk chosen.
//! `wifi-hidden` opens the Wi-Fi page with the "Other Network" form open.

use std::time::Duration;

use crate::helper;
use crate::network::{Network, Security, WifiState};
use crate::view::Done;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags {
    pub wired: bool,
    pub nowifi: bool,
    pub crowd: bool,
    pub nodisks: bool,
    pub onedisk: bool,
    pub emptydisk: bool,
    pub twins: bool,
    pub busy: bool,
    pub fail: bool,
    pub hold: bool,
    pub mok: bool,
    pub warn: bool,
    pub fast: bool,
    pub cd: bool,
    pub running: bool,
    pub finished: bool,
    pub notpm: bool,
    pub encrypt: bool,
    pub nosb: bool,
    pub pin: bool,
}

/// `None` unless `ATLAS_INSTALLER_DEMO` is set (and not empty or `0`).
/// Never on the installer media: a fake install there would tell the user
/// AtlasOS is installed when it isn't.
pub fn from_env() -> Option<Flags> {
    // Decided once: main.cpp and the Backend both ask, and a refusal is
    // logged only the first time.
    static DECIDED: std::sync::OnceLock<Option<Flags>> = std::sync::OnceLock::new();
    DECIDED
        .get_or_init(|| {
            let v = std::env::var("ATLAS_INSTALLER_DEMO").ok()?;
            let f = parse(&v)?;
            decide(f, crate::system::live_session())
        })
        .clone()
}

/// Flags only if this isn't the live session.
fn decide(f: Flags, live: bool) -> Option<Flags> {
    if live {
        eprintln!("atlas-installer: ignoring ATLAS_INSTALLER_DEMO in the live session");
        return None;
    }
    Some(f)
}

/// The step the demo opens at, for `main.cpp`: empty unless demo mode is
/// on and allowed, so a refused demo never reaches the QML.
pub fn page_from_env() -> String {
    if from_env().is_none() {
        return String::new();
    }
    std::env::var("ATLAS_INSTALLER_DEMO_PAGE").unwrap_or_default()
}

/// Called once from `main.cpp`: [`page_from_env`] as a C string that lives
/// for the whole run.
#[unsafe(no_mangle)]
pub extern "C" fn atlas_demo_page() -> *const std::ffi::c_char {
    static PAGE: std::sync::OnceLock<std::ffi::CString> = std::sync::OnceLock::new();
    PAGE.get_or_init(|| std::ffi::CString::new(page_from_env()).unwrap_or_default())
        .as_ptr()
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
            "crowd" => f.crowd = true,
            "nodisks" => f.nodisks = true,
            "onedisk" => f.onedisk = true,
            "emptydisk" => f.emptydisk = true,
            "twins" => f.twins = true,
            "busy" => f.busy = true,
            "fail" => f.fail = true,
            "hold" => f.hold = true,
            "mok" => f.mok = true,
            "cd" => f.cd = true,
            "running" => f.running = true,
            "finished" => f.finished = true,
            "warn" => f.warn = true,
            "fast" => f.fast = true,
            "notpm" => f.notpm = true,
            "encrypt" => f.encrypt = true,
            "nosb" => f.nosb = true,
            "pin" => f.pin = true,
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
    let all = include_str!("../fixtures/disks.json");
    let mut list: serde_json::Value = serde_json::from_str(all).expect("fixture");
    list["tpm2"] = (!f.notpm).into();
    list["secure_boot"] = (!f.nosb).into();
    if f.nodisks {
        list["disks"] = serde_json::json!([]);
        list["hidden"] = serde_json::json!([]);
        return Ok(list.to_string());
    }
    let keep = match (f.emptydisk || f.twins, f.onedisk) {
        (true, _) => Some("sda"),
        (_, true) => Some("nvme0n1"),
        _ => None,
    };
    if let (Some(keep), Some(disks)) = (keep, list["disks"].as_array_mut()) {
        disks.retain(|d| d["id"] == keep);
        if f.twins
            && let Some(first) = disks.first_mut()
        {
            first["serial"] = "2214E61A4F2A".into();
            let mut twin = first.clone();
            twin["id"] = "sdb".into();
            twin["path"] = "/dev/sdb".into();
            twin["serial"] = "2214E61A87B3".into();
            twin["fingerprint"] = "0000000000000b3b".into();
            disks.push(twin);
        }
    }
    Ok(list.to_string())
}

fn net(ssid: &str, strength: u8, security: Security) -> Network {
    Network {
        ssid: ssid.into(),
        ssid_hex: crate::network::ssid_hex(ssid.as_bytes()),
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
    if f.crowd {
        // An apartment block's worth: more than the list shows at once
        for (i, name) in [
            "Flat 2B",
            "Flat 3A",
            "VM-Hub-7731",
            "Garden Room",
            "TALKTALK-41C2",
            "Guest",
            "Studio Upstairs",
            "Pixel_8213",
        ]
        .into_iter()
        .enumerate()
        {
            let security = if i % 3 == 2 {
                Security::Sae
            } else {
                Security::Psk
            };
            networks.push(net(name, 60 - i as u8 * 6, security));
        }
    }
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

/// The helper's text at `f`: the stage it falls in.
fn text_at(f: f64) -> &'static str {
    use installer_core::progress::Stage;
    [
        Stage::Prepare,
        Stage::Partition,
        Stage::Format,
        Stage::Copy,
        Stage::Deploy,
        Stage::Bootloader,
        Stage::Settings,
        Stage::Finish,
    ]
    .into_iter()
    .rev()
    .find(|s| f >= s.range().0)
    .map_or("Getting ready", Stage::text)
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
    Ok(outcome(f))
}

/// What the helper's Status says: `running` and `finished` are installs
/// that began before the UI did.
pub fn status(f: &Flags) -> String {
    if f.finished {
        serde_json::json!({
            "state": "done", "step": "finish", "fraction": 1.0,
            "text": "Finishing up", "result": serde_json::from_str::<serde_json::Value>(&outcome(f)).expect("outcome"),
            "error": null,
        })
        .to_string()
    } else if f.running {
        serde_json::json!({
            "state": "installing", "step": "copy", "fraction": 0.42,
            "text": text_at(0.42), "result": null, "error": null,
        })
        .to_string()
    } else {
        serde_json::json!({
            "state": "idle", "step": "", "fraction": 0.0,
            "text": "", "result": null, "error": null,
        })
        .to_string()
    }
}

/// A made-up recovery key, so the Restart page's card can be shown.
const DEMO_RECOVERY_KEY: &str =
    "kqzvtnhw-bjmcpdre-xsfagyul-ewnqtkzh-dpcrvbjm-ayuxsgfl-tzhkwqne-mjbdrpcv";

/// Install's answer.
fn outcome(f: &Flags) -> String {
    let done = Done {
        mok_password: if f.mok {
            "48201937".into()
        } else {
            String::new()
        },
        windows_entry: true,
        boot_media: if f.cd { "cd" } else { "usb" }.into(),
        warnings: if f.warn {
            vec!["The firmware boot entry couldn't be renamed to AtlasOS: it is still called Fedora.".into()]
        } else {
            Vec::new()
        },
        log: "/run/atlas-installer/install.log".into(),
        recovery_key: String::new(),
        recovery_rows: Vec::new(),
    };
    serde_json::json!({
        "mok_password": if done.mok_password.is_empty() { None } else { Some(done.mok_password) },
        "windows_entry": done.windows_entry,
        "boot_media": done.boot_media,
        "warnings": done.warnings,
        "log": done.log,
        "recovery_key": if f.encrypt { Some(DEMO_RECOVERY_KEY) } else { None },
        "encryption": if f.encrypt { "tpm" } else { "none" },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_is_refused_in_the_live_session() {
        let f = parse("wired").unwrap();
        assert_eq!(decide(f.clone(), true), None);
        assert_eq!(decide(f.clone(), false), Some(f));
    }

    #[test]
    fn flags() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("0"), None);
        assert_eq!(parse("1"), Some(Flags::default()));
        let f = parse("wired, mok,unknown").unwrap();
        assert!(f.wired && f.mok && !f.fail);
    }

    #[test]
    fn status_of_an_install_that_began_before_the_ui() {
        let state = |flags: &str| -> serde_json::Value {
            serde_json::from_str(&status(&parse(flags).unwrap())).unwrap()
        };
        assert_eq!(state("1")["state"], "idle");
        assert_eq!(state("running")["state"], "installing");
        assert_eq!(state("running")["fraction"], 0.42);
        let done = state("finished,mok,encrypt");
        assert_eq!(done["state"], "done");
        assert_eq!(done["result"]["mok_password"], "48201937");
        assert_eq!(
            done["result"]["recovery_key"].as_str().map(str::len),
            Some(71)
        );
        // and the UI reads what the demo says
        assert_eq!(
            crate::view::reattach(&status(&parse("finished").unwrap()))
                .unwrap()
                .page(),
            "restart"
        );
    }

    #[tokio::test]
    async fn the_disk_list_says_whether_there_is_a_tpm() {
        let tpm2 = |flags: &str| {
            let f = parse(flags).unwrap();
            async move {
                let json = list_disks(&f).await.unwrap();
                serde_json::from_str::<serde_json::Value>(&json).unwrap()["tpm2"].as_bool()
            }
        };
        assert_eq!(tpm2("1").await, Some(true));
        assert_eq!(tpm2("notpm").await, Some(false));
        assert_eq!(tpm2("nodisks").await, Some(true));
        let list = list_disks(&parse("nosb").unwrap()).await.unwrap();
        assert!(crate::view::secure_boot_off(&list));
    }

    #[test]
    fn stage_texts() {
        assert_eq!(text_at(0.0), "Getting ready");
        assert_eq!(text_at(0.5), "Copying AtlasOS");
        assert_eq!(text_at(0.8), "Setting up AtlasOS");
        assert_eq!(text_at(0.99), "Finishing up");
    }
}
