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
#[derive(Debug, Clone, Default, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Done {
    /// The MOK Manager password, or "" when the key wasn't queued.
    pub mok_password: String,
    pub windows_entry: bool,
    /// What to remove before restarting: "cd", "usb" or "".
    pub boot_media: String,
    pub warnings: Vec<String>,
    pub log: String,
}

/// The helper's Install answer.
#[derive(Debug, serde::Deserialize)]
struct Outcome {
    mok_password: Option<String>,
    windows_entry: bool,
    #[serde(default)]
    boot_media: String,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    log: String,
}

pub fn done_from_outcome(json: &str) -> Result<Done, String> {
    let o: Outcome =
        serde_json::from_str(json).map_err(|e| format!("bad answer from the installer: {e}"))?;
    Ok(Done {
        mok_password: o.mok_password.unwrap_or_default(),
        windows_entry: o.windows_entry,
        boot_media: o.boot_media,
        warnings: o.warnings,
        log: o.log,
    })
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
        "done" => Ok(Reattach::Done(
            s.result
                .ok_or_else(|| "no result".to_string())
                .and_then(|v| done_from_outcome(&v.to_string()))
                // The install succeeded even if its report can't be read
                .unwrap_or_else(|m| Done {
                    warnings: vec![format!("The installer's report couldn't be read ({m}).")],
                    ..Default::default()
                }),
        )),
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
    use super::*;

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
}
