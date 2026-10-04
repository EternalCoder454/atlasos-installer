//! What the pages show, worked out from the helper's answers. Pure, so it is
//! unit-tested; QML gets the results as JSON.

use installer_core::disks::{Disk, DiskList};
use installer_core::{GIB, MIN_INSTALL_BYTES};
use serde::Serialize;

/// Sizes in binary units with the names Windows uses (a 1 TB disk is
/// "932 GB"), rounded to the nearest.
pub fn size(bytes: u64) -> String {
    let gb = bytes as f64 / GIB as f64;
    if gb >= 1000.0 {
        let tb = gb / 1024.0;
        if tb >= 10.0 {
            format!("{tb:.0} TB")
        } else {
            format!("{tb:.1} TB")
        }
    } else if gb >= 10.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB")
    }
}

/// One disk on the Disk page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskRow {
    pub id: String,
    pub fingerprint: String,
    pub title: String,
    /// "1.8 TB · Windows"
    pub subtitle: String,
    pub icon: &'static str,
    pub usb: bool,
    /// Nothing on it: erasing loses nothing, so no warning.
    pub empty: bool,
    /// At least one way to install is possible.
    pub selectable: bool,
    /// Why not, when it isn't.
    pub reason: String,
    pub erase_ok: bool,
    pub erase_title: String,
    pub erase_text: String,
    pub free_ok: bool,
    pub free_title: String,
    pub free_text: String,
    /// A note under the choices (BitLocker), or "".
    pub note: String,
    /// Review page: what happens to the disk, per mode.
    pub review_erase: String,
    pub review_free: String,
}

pub fn disk_rows(list: &DiskList) -> Vec<DiskRow> {
    list.disks
        .iter()
        .map(|d| disk_row(d, &disk_name(d, &list.disks)))
        .collect()
}

/// The disk's name, made unique when another disk has the same name and
/// size: "Samsung SSD 990 PRO, serial …4F2A" if the serials end
/// differently, else "Virtual disk, /dev/vdb".
fn disk_name(d: &Disk, all: &[Disk]) -> String {
    let twins: Vec<&Disk> = all
        .iter()
        .filter(|o| o.name == d.name && size(o.size) == size(d.size))
        .collect();
    if twins.len() < 2 {
        return d.name.clone();
    }
    let tail = |x: &Disk| -> Option<String> {
        let s = x.serial.as_deref()?;
        let n = s.chars().count();
        Some(s.chars().skip(n.saturating_sub(4)).collect())
    };
    let tails: Vec<_> = twins.iter().map(|x| tail(x)).collect();
    let distinct = tails.iter().all(Option::is_some)
        && tails
            .iter()
            .enumerate()
            .all(|(i, t)| !tails[..i].contains(t));
    match tail(d) {
        Some(t) if distinct => format!("{}, serial …{t}", d.name),
        _ => format!("{}, {}", d.name, d.path),
    }
}

fn disk_row(d: &Disk, name: &str) -> DiskRow {
    let size = size(d.size);
    let what = if d.contents.is_empty() {
        "Empty".to_string()
    } else {
        d.description.clone()
    };
    let icon = if d.usb {
        "drive-removable-media-usb"
    } else if d.id.starts_with("nvme") {
        "drive-harddisk-solidstate"
    } else {
        "drive-harddisk"
    };
    const NOT_POSSIBLE: &str = "Not possible on this disk.";
    let erase_text = match d.erase.reason.as_deref() {
        Some(r) if !d.erase.possible => r.to_string(),
        None if !d.erase.possible => NOT_POSSIBLE.into(),
        _ if d.contents.is_empty() => "The disk is empty. AtlasOS uses all of it.".into(),
        _ => format!(
            "Everything on this disk is erased, including {}.",
            lower_first(&d.description)
        ),
    };
    let keeps = if d.contents.is_empty() {
        "What's on the disk stays as it is.".to_string()
    } else {
        format!("{} and your files stay as they are.", d.description)
    };
    let free_text = match d.free_space.reason.as_deref() {
        Some(r) if !d.free_space.possible => r.to_string(),
        None if !d.free_space.possible => NOT_POSSIBLE.into(),
        _ => format!(
            "Use {} of free space. {keeps}",
            size_of_free(d.free_space.bytes)
        ),
    };
    let free_title = if d.contents.is_empty() {
        "Install alongside what's on the disk".to_string()
    } else {
        format!("Install alongside {}", d.description)
    };
    let selectable = !d.too_small && (d.erase.possible || d.free_space.possible);
    let reason = if selectable {
        String::new()
    } else {
        d.erase
            .reason
            .clone()
            .or_else(|| d.free_space.reason.clone())
            .unwrap_or_else(|| "AtlasOS can't be installed on this disk.".into())
    };
    let note = if d.bitlocker {
        "Windows uses BitLocker on this disk. Have your recovery key ready: Windows may ask for it the next time it starts.".to_string()
    } else {
        String::new()
    };
    let review_erase = if d.contents.is_empty() {
        format!("{name} ({size}) is empty. AtlasOS uses all of it.")
    } else {
        format!(
            "{name} ({size}) is erased: {} and all files on it are lost.",
            d.description
        )
    };
    DiskRow {
        id: d.id.clone(),
        fingerprint: d.fingerprint.clone(),
        title: name.to_string(),
        subtitle: format!("{size} · {what}"),
        icon,
        usb: d.usb,
        empty: d.contents.is_empty(),
        selectable,
        reason,
        erase_ok: d.erase.possible,
        erase_title: "Erase everything and install".into(),
        erase_text,
        free_ok: d.free_space.possible,
        free_title,
        free_text,
        note,
        review_erase,
        review_free: format!(
            "AtlasOS goes in {} of free space on {name} ({size}). {keeps}",
            size_of_free(d.free_space.bytes),
        ),
    }
}

/// Free space is shown rounded down: "use 400 GB" must not promise more.
fn size_of_free(bytes: u64) -> String {
    if bytes >= 1000 * GIB {
        size(bytes)
    } else {
        format!("{} GB", bytes / GIB)
    }
}

/// "Windows and Linux" stays, "Unknown data" → "unknown data".
fn lower_first(s: &str) -> String {
    match s {
        "Files" => "files".into(),
        "Unknown data" => "unknown data".into(),
        _ => s.into(),
    }
}

/// What the Restart page shows after a successful install.
#[derive(Clone, Default, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Done {
    /// The MOK Manager password, or "" when the key wasn't queued.
    pub mok_password: String,
    pub windows_entry: bool,
    /// What to remove before restarting: "cd", "usb" or "".
    pub boot_media: String,
    pub warnings: Vec<String>,
    pub log: String,
    /// The disk's recovery key as the helper made it, or "" (not encrypted).
    pub recovery_key: String,
    /// The key for reading: rows of four groups (see `recovery_rows`).
    pub recovery_rows: Vec<String>,
}

/// The recovery key and the MOK password never show in `Debug` output.
impl std::fmt::Debug for Done {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hide = |s: &str| if s.is_empty() { "" } else { "<hidden>" };
        f.debug_struct("Done")
            .field("mok_password", &hide(&self.mok_password))
            .field("windows_entry", &self.windows_entry)
            .field("boot_media", &self.boot_media)
            .field("warnings", &self.warnings)
            .field("log", &self.log)
            .field("recovery_key", &hide(&self.recovery_key))
            .field(
                "recovery_rows",
                &self
                    .recovery_rows
                    .iter()
                    .map(|_| "<hidden>")
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// The helper's Install answer. No `Debug`: it holds the MOK password and
/// the recovery key.
#[derive(serde::Deserialize)]
struct Outcome {
    mok_password: Option<String>,
    windows_entry: bool,
    #[serde(default)]
    boot_media: String,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    log: String,
    #[serde(default)]
    recovery_key: Option<String>,
    /// "" from a helper older than this field: taken as not encrypted.
    #[serde(default)]
    encryption: String,
}

pub fn done_from_outcome(json: &str) -> Result<Done, String> {
    let o: Outcome =
        serde_json::from_str(json).map_err(|e| format!("bad answer from the installer: {e}"))?;
    let d = Done {
        mok_password: o.mok_password.unwrap_or_default(),
        windows_entry: o.windows_entry,
        boot_media: o.boot_media,
        warnings: o.warnings,
        log: o.log,
        recovery_rows: recovery_rows(o.recovery_key.as_deref().unwrap_or("")),
        recovery_key: o.recovery_key.unwrap_or_default(),
    };
    Ok(require_key(d, &o.encryption))
}

/// A Done for an encrypted install (`encryption` other than "none" or "")
/// must carry the recovery key. If it doesn't, say so in plain words, with
/// how to make a new one after the restart: systemd-cryptenroll unlocks with
/// the TPM only when told to, else it asks for the disk password, and wipes
/// the old recovery slot after enrolling, never the new one.
pub fn require_key(mut d: Done, encryption: &str) -> Done {
    const NO_KEY: &str = "recovery key couldn't be shown";
    let unlock = match encryption {
        "" | "none" => return d,
        "tpm" | "tpm-pin" => "--unlock-tpm2-device=auto ",
        _ => "",
    };
    if d.recovery_key.is_empty() && !d.warnings.iter().any(|w| w.contains(NO_KEY)) {
        d.warnings.push(format!(
            "AtlasOS is installed and encrypted, but its {NO_KEY}, so you don't have one yet. \
             After the restart, make a new one in a terminal with \
             \"sudo systemd-cryptenroll {unlock}--recovery-key --wipe-slot=recovery\" \
             followed by the AtlasOS partition (\"lsblk -f\" shows it as crypto_LUKS), \
             and keep it away from this PC."
        ));
    }
    d
}

/// The recovery key in rows of four groups, for the Restart page. Groups
/// keep their dashes, so a copied key is one the disk accepts.
pub fn recovery_rows(key: &str) -> Vec<String> {
    let groups: Vec<&str> = key.split('-').filter(|g| !g.is_empty()).collect();
    groups.chunks(4).map(|row| row.join("-")).collect()
}

/// The helper's rules for what is typed at start-up: printable ASCII only
/// (0x20 to 0x7e), because the start-up prompt can't reliably type
/// anything else. A password is 8 to 256 characters, a PIN 6 to 64.
pub const PASSWORD_MIN: usize = 8;
pub const PASSWORD_MAX: usize = 256;
pub const PIN_MIN: usize = 6;
pub const PIN_MAX: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretProblem {
    Empty,
    TooShort,
    TooLong,
    /// Not printable ASCII: an accent, another script, an emoji, a tab.
    BadChar,
}

fn secret_problem(s: &str, min: usize, max: usize) -> Option<SecretProblem> {
    if s.is_empty() {
        Some(SecretProblem::Empty)
    } else if !s.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        Some(SecretProblem::BadChar)
    } else if s.len() < min {
        Some(SecretProblem::TooShort)
    } else if s.len() > max {
        Some(SecretProblem::TooLong)
    } else {
        None
    }
}

pub fn password_problem(pw: &str) -> Option<SecretProblem> {
    secret_problem(pw, PASSWORD_MIN, PASSWORD_MAX)
}

pub fn pin_problem(pin: &str) -> Option<SecretProblem> {
    secret_problem(pin, PIN_MIN, PIN_MAX)
}

/// The problem with what is typed for this encryption mode ("password" or
/// "tpm-pin"; any other mode types nothing).
fn problem_for(mode: &str, s: &str) -> Option<SecretProblem> {
    match mode {
        "password" => password_problem(s),
        "tpm-pin" => pin_problem(s),
        _ => None,
    }
}

/// Continue may be pressed: a valid password or PIN, typed twice the same.
pub fn secret_ok(mode: &str, s: &str, confirm: &str) -> bool {
    problem_for(mode, s).is_none() && s == confirm
}

/// The line under the fields: what is wrong, or "" while nothing is (an
/// empty field is not nagged about; the hint below says the rule).
pub fn secret_hint(mode: &str, s: &str, confirm: &str) -> String {
    let pin = mode == "tpm-pin";
    match problem_for(mode, s) {
        Some(SecretProblem::Empty) => String::new(),
        Some(SecretProblem::TooShort) if pin => "Use at least 6 characters.".into(),
        Some(SecretProblem::TooShort) => "Use at least 8 characters.".into(),
        Some(SecretProblem::TooLong) => {
            format!("That {} is too long.", if pin { "PIN" } else { "password" })
        }
        Some(SecretProblem::BadChar) => {
            "Use letters without accents, numbers, spaces and symbols only.".into()
        }
        None if !confirm.is_empty() && s != confirm => {
            format!(
                "The two {} don't match.",
                if pin { "PINs" } else { "passwords" }
            )
        }
        None => String::new(),
    }
}

/// The disk is encrypted unless the user said otherwise: on when this PC
/// has a TPM 2.0 (it costs nothing then), off when it would need a password.
pub fn encryption_default(tpm2: bool) -> bool {
    tpm2
}

/// The PIN switch starts on when there is a TPM but Secure Boot is off:
/// TPM-only unlocking gives little protection then.
pub fn pin_default(tpm2: bool, secure_boot_off: bool) -> bool {
    tpm2 && secure_boot_off
}

/// What Install is told: "none", "tpm", "tpm-pin" or "password". `pin`
/// (ask for a PIN at start-up) counts only with a TPM.
pub fn encryption_mode(tpm2: bool, on: bool, pin: bool) -> &'static str {
    match (on, tpm2) {
        (false, _) => "none",
        (true, true) if pin => "tpm-pin",
        (true, true) => "tpm",
        (true, false) => "password",
    }
}

/// The Review page's row.
pub fn review_encryption(mode: &str) -> &'static str {
    match mode {
        "tpm" => "On, unlocks with this PC's security chip",
        "tpm-pin" => "On, unlocks with this PC's security chip and a PIN",
        "password" => "On, password at start-up",
        _ => "Off",
    }
}

/// ListDisks says Secure Boot is off (absent: unknown, so no warning).
pub fn secure_boot_off(list_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(list_json)
        .ok()
        .and_then(|v| v["secure_boot"].as_bool())
        == Some(false)
}

/// Whether ListDisks says this PC has a usable TPM 2.0.
pub fn tpm2_of(list_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(list_json)
        .ok()
        .and_then(|v| v["tpm2"].as_bool())
        .unwrap_or(false)
}

/// The smallest disk AtlasOS installs to, for the Disk page's texts: the
/// same minimum the helper's disk list uses.
pub fn min_disk_size() -> String {
    size(MIN_INSTALL_BYTES)
}

/// What the helper's Status says about the install, as the UI sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum Reattach {
    /// Nothing has been installed by this helper.
    Idle,
    Running {
        fraction: f64,
        text: String,
    },
    Done(Done),
    Failed(String),
}

impl Reattach {
    /// The page a UI that just started opens at.
    pub fn page(&self) -> &'static str {
        match self {
            Reattach::Idle => "welcome",
            Reattach::Running { .. } | Reattach::Failed(_) => "progress",
            Reattach::Done(_) => "restart",
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct StatusJson {
    state: String,
    #[serde(default)]
    fraction: f64,
    #[serde(default)]
    text: String,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<String>,
}

/// Reads the helper's Status answer.
pub fn reattach(json: &str) -> Result<Reattach, String> {
    let s: StatusJson =
        serde_json::from_str(json).map_err(|e| format!("bad answer from the installer: {e}"))?;
    match s.state.as_str() {
        "idle" => Ok(Reattach::Idle),
        "installing" => Ok(Reattach::Running {
            fraction: s.fraction,
            text: s.text,
        }),
        "done" => {
            // The install succeeded even if its report can't be read; the
            // mode, if it's there, still says whether a key is missing.
            let mode = s
                .result
                .as_ref()
                .and_then(|v| v.get("encryption"))
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            Ok(Reattach::Done(
                s.result
                    .ok_or_else(|| "no result".to_string())
                    .and_then(|v| done_from_outcome(&v.to_string()))
                    .unwrap_or_else(|m| {
                        let d = Done {
                            warnings: vec![format!(
                                "The installer's report couldn't be read ({m})."
                            )],
                            ..Default::default()
                        };
                        require_key(d, &mode)
                    }),
            ))
        }
        "failed" => Ok(Reattach::Failed(
            crate::helper::describe("net.eterneon.atlas.Error.Failed", s.error.as_deref()).message,
        )),
        other => Err(format!("unknown install state {other:?}")),
    }
}

/// What to do after asking Status while following an install whose call
/// was lost.
#[derive(Debug, Clone, PartialEq)]
pub enum Follow {
    Wait {
        fraction: f64,
        text: String,
    },
    Done(Done),
    Failed(String),
    /// The helper knows of no install: it was not started, or the helper
    /// restarted and the install died with it.
    Gone,
}

pub fn follow(r: Reattach) -> Follow {
    match r {
        Reattach::Idle => Follow::Gone,
        Reattach::Running { fraction, text } => Follow::Wait { fraction, text },
        Reattach::Done(d) => Follow::Done(d),
        Reattach::Failed(m) => Follow::Failed(m),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn done_debug_hides_the_secrets() {
        let d = super::Done {
            mok_password: "12345678".into(),
            recovery_key: "ulcbjnni-ehtlcfnl".into(),
            recovery_rows: vec!["ulcb jnni".into()],
            ..Default::default()
        };
        let s = format!("{d:?}");
        assert!(
            !s.contains("12345678") && !s.contains("ulcb") && s.contains("<hidden>"),
            "{s}"
        );
    }

    use super::*;

    #[test]
    fn an_encrypted_install_without_a_key_warns() {
        let d = require_key(Done::default(), "tpm");
        assert_eq!(d.warnings.len(), 1);
        assert!(d.warnings[0].contains("recovery key couldn't be shown"));
        assert!(d.warnings[0].contains("--unlock-tpm2-device=auto --recovery-key"));
        let d = require_key(Done::default(), "password");
        assert!(
            !d.warnings[0].contains("tpm2"),
            "a password disk asks for it"
        );
        assert!(require_key(Done::default(), "none").warnings.is_empty());
        assert!(require_key(Done::default(), "").warnings.is_empty());
        let keyed = Done {
            recovery_key: "a-b".into(),
            ..Default::default()
        };
        assert!(require_key(keyed, "tpm").warnings.is_empty());
        // Status to a caller who may not see the key: warned once, even when
        // the backend checks again
        let d = done_from_outcome(
            r#"{"mok_password":null,"windows_entry":false,"encryption":"tpm-pin"}"#,
        )
        .unwrap();
        assert_eq!(require_key(d, "tpm-pin").warnings.len(), 1);
        let d = done_from_outcome(r#"{"mok_password":null,"windows_entry":false}"#).unwrap();
        assert!(d.warnings.is_empty(), "an older helper's answer");
        // a report that can't be read still warns when its mode says encrypted
        let st = r#"{"state":"done","result":{"encryption":"password","windows_entry":"x"}}"#;
        match reattach(st).unwrap() {
            Reattach::Done(d) => {
                assert_eq!(d.warnings.len(), 2, "{:?}", d.warnings);
                assert!(d.warnings[1].contains("recovery key couldn't be shown"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_minimum_disk_size_is_the_cores() {
        assert_eq!(min_disk_size(), "40 GB");
        let rows = disk_rows(&fixture());
        assert!(
            rows[3].reason.contains(&min_disk_size()),
            "{}",
            rows[3].reason
        );
    }

    #[test]
    fn status_picks_the_page() {
        let page = |j: &str| reattach(j).unwrap().page();
        assert_eq!(page(r#"{"state":"idle"}"#), "welcome");
        assert_eq!(
            page(r#"{"state":"installing","step":"copy","fraction":0.4,"text":"Copying"}"#),
            "progress"
        );
        assert_eq!(
            page(r#"{"state":"done","result":{"mok_password":null,"windows_entry":false}}"#),
            "restart"
        );
        assert_eq!(
            page(r#"{"state":"failed","error":"bootc failed"}"#),
            "progress"
        );
        assert!(reattach(r#"{"state":"dancing"}"#).is_err());
        assert!(reattach("nope").is_err());
    }

    #[test]
    fn a_reattached_install_carries_its_progress_result_and_error() {
        let r = reattach(r#"{"state":"installing","fraction":0.42,"text":"Copying AtlasOS"}"#);
        assert_eq!(
            r,
            Ok(Reattach::Running {
                fraction: 0.42,
                text: "Copying AtlasOS".into()
            })
        );
        let Ok(Reattach::Done(d)) = reattach(
            r#"{"state":"done","fraction":1.0,"result":{"mok_password":"12345678","windows_entry":true,"boot_media":"cd","warnings":["w"],"log":"/l"}}"#,
        ) else {
            panic!("done")
        };
        assert_eq!(d.mok_password, "12345678");
        assert_eq!(d.boot_media, "cd");
        assert_eq!(d.warnings, ["w"]);
        // an unreadable result still counts as done, with a warning
        let Ok(Reattach::Done(d)) = reattach(r#"{"state":"done","result":null}"#) else {
            panic!("done")
        };
        assert!(d.warnings[0].contains("couldn't be read"));
        assert_eq!(
            reattach(r#"{"state":"failed","error":"the disk changed"}"#),
            Ok(Reattach::Failed("The disk changed".into()))
        );
    }

    #[test]
    fn a_lost_call_is_followed_until_the_install_ends() {
        let wait = follow(reattach(r#"{"state":"installing","fraction":0.5,"text":"t"}"#).unwrap());
        assert!(matches!(wait, Follow::Wait { fraction, .. } if fraction == 0.5));
        assert!(matches!(
            follow(reattach(r#"{"state":"done","result":{"windows_entry":false}}"#).unwrap()),
            Follow::Done(_)
        ));
        assert_eq!(
            follow(reattach(r#"{"state":"failed","error":"x"}"#).unwrap()),
            Follow::Failed("X".into())
        );
        assert_eq!(
            follow(reattach(r#"{"state":"idle"}"#).unwrap()),
            Follow::Gone
        );
    }

    fn fixture() -> DiskList {
        serde_json::from_str(include_str!("../fixtures/disks.json")).unwrap()
    }

    #[test]
    fn sizes() {
        assert_eq!(size(2_000_398_934_016), "1.8 TB");
        assert_eq!(size(1_000_204_886_016), "932 GB");
        assert_eq!(size(500_107_862_016), "466 GB");
        assert_eq!(size(32_015_679_488), "30 GB");
        assert_eq!(size(8 * GIB + GIB / 2), "8.5 GB");
        assert_eq!(size(16 * 1024 * GIB), "16 TB");
        assert_eq!(size_of_free(400 * GIB + GIB - 1), "400 GB");
    }

    #[test]
    fn windows_disk_offers_both() {
        let rows = disk_rows(&fixture());
        let w = &rows[0];
        assert_eq!(w.subtitle, "1.8 TB · Windows");
        assert!(w.selectable && w.erase_ok && w.free_ok);
        assert_eq!(
            w.erase_text,
            "Everything on this disk is erased, including Windows."
        );
        assert_eq!(w.free_title, "Install alongside Windows");
        assert_eq!(
            w.free_text,
            "Use 400 GB of free space. Windows and your files stay as they are."
        );
        assert_eq!(
            w.review_erase,
            "Samsung SSD 990 PRO 2TB (1.8 TB) is erased: Windows and all files on it are lost."
        );
        assert_eq!(w.icon, "drive-harddisk-solidstate");
        assert!(w.note.is_empty());
    }

    #[test]
    fn empty_disk_and_greyed_out_choices() {
        let rows = disk_rows(&fixture());
        let e = &rows[1];
        assert_eq!(e.subtitle, "466 GB · Empty");
        assert!(e.erase_ok && !e.free_ok);
        assert_eq!(e.erase_text, "The disk is empty. AtlasOS uses all of it.");
        assert!(e.free_text.starts_with("This disk has no partitions yet"));
        assert_eq!(
            e.review_erase,
            "Crucial MX500 (466 GB) is empty. AtlasOS uses all of it."
        );
        assert!(e.empty && !rows[0].empty);

        let b = &rows[2];
        assert!(b.note.contains("BitLocker"));
        assert!(b.free_text.starts_with("Not enough free space"));
    }

    #[test]
    fn small_usb_disk_is_not_selectable() {
        let rows = disk_rows(&fixture());
        let s = &rows[3];
        assert!(!s.selectable && s.usb);
        assert_eq!(s.reason, "Too small: AtlasOS needs 40 GB.");
        assert_eq!(s.icon, "drive-removable-media-usb");
    }

    #[test]
    fn outcome() {
        let d = done_from_outcome(
            r#"{"mok_password":"12345678","windows_entry":true,"boot_media":"usb","warnings":["w"],"log":"/run/atlas-installer/install.log"}"#,
        )
        .unwrap();
        assert_eq!(d.mok_password, "12345678");
        assert!(d.windows_entry);
        assert_eq!(d.boot_media, "usb");
        let d = done_from_outcome(
            r#"{"mok_password":null,"windows_entry":false,"warnings":[],"log":""}"#,
        )
        .unwrap();
        assert_eq!(d.mok_password, "");
        assert_eq!(d.boot_media, "", "an older helper doesn't say");
        assert!(done_from_outcome("{}").is_err());
    }

    #[test]
    fn identical_disks_are_told_apart() {
        let mut list = fixture();
        let empty = list.disks[1].clone();
        let mut twin = empty.clone();
        twin.id = "sdz".into();
        twin.path = "/dev/sdz".into();
        list.disks.push(twin);
        let titles =
            |l: &DiskList| -> Vec<String> { disk_rows(l).into_iter().map(|r| r.title).collect() };
        let plain = titles(&fixture());
        let t = titles(&list);
        assert_eq!(t[0], plain[0], "others keep their names");
        assert_eq!(t[1], format!("{}, {}", empty.name, empty.path));
        assert_eq!(t[4], format!("{}, /dev/sdz", empty.name));
        assert!(disk_rows(&list)[4].review_erase.starts_with(&t[4]));

        list.disks[1].serial = Some("S6Z1NF0W104F2A".into());
        list.disks[4].serial = Some("S6Z1NF0W1087B3".into());
        let t = titles(&list);
        assert_eq!(t[1], format!("{}, serial …4F2A", empty.name));
        assert_eq!(t[4], format!("{}, serial …87B3", empty.name));

        // same last four: the path, which always differs
        list.disks[4].serial = Some("XX4F2A".into());
        assert_eq!(titles(&list)[4], format!("{}, /dev/sdz", empty.name));
    }

    const KEY: &str = "abcdefgh-ijklmnop-qrstuvwx-yzabcdef-ghijklmn-opqrstuv-wxyzabcd-efghijkl";

    #[test]
    fn password_and_pin_rules_match_the_helper() {
        use SecretProblem::*;
        assert_eq!(password_problem(""), Some(Empty));
        assert_eq!(password_problem("1234567"), Some(TooShort));
        assert_eq!(password_problem("12345678"), None);
        assert_eq!(password_problem(&"a".repeat(256)), None);
        assert_eq!(password_problem(&"a".repeat(257)), Some(TooLong));
        assert_eq!(password_problem("pass word 1~"), None);
        // printable ASCII only: no accents, scripts, emoji, controls
        assert_eq!(password_problem("pässwörd"), Some(BadChar));
        assert_eq!(password_problem("密码密码密码密码"), Some(BadChar));
        assert_eq!(password_problem("password🙂"), Some(BadChar));
        assert_eq!(password_problem("abcd\tefgh"), Some(BadChar));
        assert_eq!(password_problem("abcdefgh\n"), Some(BadChar));
        assert_eq!(password_problem("abcdefgh\u{7f}"), Some(BadChar));
        assert_eq!(pin_problem("12345"), Some(TooShort));
        assert_eq!(pin_problem("123456"), None);
        assert_eq!(pin_problem(&"1".repeat(64)), None);
        assert_eq!(pin_problem(&"1".repeat(65)), Some(TooLong));
        assert_eq!(pin_problem("12345é"), Some(BadChar));
    }

    #[test]
    fn secrets_must_match() {
        assert!(secret_ok("password", "correct horse", "correct horse"));
        assert!(!secret_ok("password", "correct horse", "correct horsf"));
        assert!(!secret_ok("password", "correct horse", ""));
        assert!(!secret_ok("password", "short", "short"));
        assert!(secret_ok("tpm-pin", "123456", "123456"));
        assert!(!secret_ok("tpm-pin", "12345", "12345"));
        assert_eq!(secret_hint("password", "", ""), "");
        assert_eq!(
            secret_hint("password", "abc", ""),
            "Use at least 8 characters."
        );
        assert_eq!(
            secret_hint("tpm-pin", "123", ""),
            "Use at least 6 characters."
        );
        assert_eq!(secret_hint("password", "correct horse", ""), "");
        assert_eq!(
            secret_hint("password", "correct horse", "correct"),
            "The two passwords don't match."
        );
        assert_eq!(
            secret_hint("tpm-pin", "123456", "12345"),
            "The two PINs don't match."
        );
        assert_eq!(
            secret_hint("password", "correct horse", "correct horse"),
            ""
        );
        assert!(secret_hint("password", "pässwörd", "").contains("without accents"));
    }

    #[test]
    fn encryption_follows_the_tpm() {
        assert!(encryption_default(true));
        assert!(!encryption_default(false));
        assert!(pin_default(true, true));
        assert!(!pin_default(true, false));
        assert!(!pin_default(false, true));
        assert_eq!(encryption_mode(true, true, false), "tpm");
        assert_eq!(encryption_mode(true, true, true), "tpm-pin");
        assert_eq!(encryption_mode(false, true, false), "password");
        assert_eq!(encryption_mode(false, true, true), "password");
        assert_eq!(encryption_mode(true, false, true), "none");
        assert_eq!(encryption_mode(false, false, false), "none");
        assert_eq!(
            review_encryption("tpm"),
            "On, unlocks with this PC's security chip"
        );
        assert_eq!(
            review_encryption("tpm-pin"),
            "On, unlocks with this PC's security chip and a PIN"
        );
        assert_eq!(review_encryption("password"), "On, password at start-up");
        assert_eq!(review_encryption("none"), "Off");
        assert!(tpm2_of(r#"{"disks":[],"tpm2":true}"#));
        assert!(!tpm2_of(r#"{"disks":[],"tpm2":false}"#));
        assert!(!tpm2_of(r#"{"disks":[]}"#));
        assert!(!tpm2_of("nope"));
        assert!(secure_boot_off(r#"{"secure_boot":false}"#));
        assert!(!secure_boot_off(r#"{"secure_boot":true}"#));
        assert!(!secure_boot_off(r#"{"disks":[]}"#));
    }

    #[test]
    fn the_recovery_key_comes_in_two_rows() {
        let rows = recovery_rows(KEY);
        assert_eq!(
            rows,
            [
                "abcdefgh-ijklmnop-qrstuvwx-yzabcdef",
                "ghijklmn-opqrstuv-wxyzabcd-efghijkl"
            ]
        );
        assert!(recovery_rows("").is_empty());
    }

    #[test]
    fn the_recovery_key_survives_the_outcome_and_status() {
        let j = format!(r#"{{"mok_password":null,"windows_entry":false,"recovery_key":"{KEY}"}}"#);
        let d = done_from_outcome(&j).unwrap();
        assert_eq!(d.recovery_key, KEY);
        assert_eq!(d.recovery_rows.len(), 2);
        let d = done_from_outcome(r#"{"windows_entry":false,"recovery_key":null}"#).unwrap();
        assert!(d.recovery_key.is_empty() && d.recovery_rows.is_empty());
        // a UI that restarts reads it again from Status
        let st = format!(r#"{{"state":"done","result":{j}}}"#);
        match reattach(&st).unwrap() {
            Reattach::Done(d) => assert_eq!(d.recovery_key, KEY),
            other => panic!("{other:?}"),
        }
    }
}
