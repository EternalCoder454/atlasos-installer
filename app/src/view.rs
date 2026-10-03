//! What the pages show, worked out from the helper's answers. Pure, so it is
//! unit-tested; QML gets the results as JSON.

use installer_core::GIB;
use installer_core::disks::{Disk, DiskList};
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
    list.disks.iter().map(disk_row).collect()
}

fn disk_row(d: &Disk) -> DiskRow {
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
    let lost = if d.contents.is_empty() {
        "it is empty".to_string()
    } else {
        format!("{} and all files on it are lost", d.description)
    };
    DiskRow {
        id: d.id.clone(),
        fingerprint: d.fingerprint.clone(),
        title: d.name.clone(),
        subtitle: format!("{size} · {what}"),
        icon,
        usb: d.usb,
        selectable,
        reason,
        erase_ok: d.erase.possible,
        erase_title: "Erase everything and install".into(),
        erase_text,
        free_ok: d.free_space.possible,
        free_title,
        free_text,
        note,
        review_erase: format!("{} ({size}) is erased: {lost}.", d.name),
        review_free: format!(
            "AtlasOS goes in {} of free space on {} ({size}). {keeps}",
            size_of_free(d.free_space.bytes),
            d.name
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
    pub warnings: Vec<String>,
    pub log: String,
}

/// The helper's Install answer.
#[derive(Debug, serde::Deserialize)]
struct Outcome {
    mok_password: Option<String>,
    windows_entry: bool,
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
        warnings: o.warnings,
        log: o.log,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "Crucial MX500 (466 GB) is erased: it is empty."
        );

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
            r#"{"mok_password":"12345678","windows_entry":true,"warnings":["w"],"log":"/run/atlas-installer/install.log"}"#,
        )
        .unwrap();
        assert_eq!(d.mok_password, "12345678");
        assert!(d.windows_entry);
        let d = done_from_outcome(
            r#"{"mok_password":null,"windows_entry":false,"warnings":[],"log":""}"#,
        )
        .unwrap();
        assert_eq!(d.mok_password, "");
        assert!(done_from_outcome("{}").is_err());
    }
}
