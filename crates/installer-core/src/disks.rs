//! Which disks the installer offers, and what it says about them
//! (Plan: "Finding and hiding the boot media").

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::lsblk::{Device, Lsblk};
use crate::plan::{EspInfo, Mode, Plan, Unavailable, plan_erase, plan_free_space};
use crate::table::Table;
use crate::{ISO_LABELS, MIN_INSTALL_BYTES, gpt};

/// Where the live system's boot media is mounted by dmsquash-live.
pub const LIVE_MOUNT: &str = "/run/initramfs/live";

/// Everything the helper read from the system, handed in for a decision.
#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub lsblk: Option<Lsblk>,
    /// The boot media's devices, from [`live_sources`] (and resolved).
    pub live_sources: Vec<String>,
    /// `sfdisk --json` of each visible disk that has a partition table.
    pub tables: HashMap<String, Table>,
    /// Each EFI partition on a visible disk.
    pub esps: HashMap<String, EspInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub possible: bool,
    /// Why not, for the greyed-out choice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreeSpaceChoice {
    pub possible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Size of the largest free region, in bytes.
    pub bytes: u64,
    /// The existing EFI partition that would be shared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_esp: Option<String>,
}

/// A disk as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disk {
    /// What `Install` takes: the kernel name, e.g. `nvme0n1`.
    pub id: String,
    /// What `Install` takes too: see [`fingerprint`].
    pub fingerprint: String,
    pub path: String,
    /// Model from udev, e.g. "Samsung SSD 990 PRO".
    pub name: String,
    /// Serial number, to tell two disks of the same model apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    pub size: u64,
    pub usb: bool,
    pub removable: bool,
    /// What is on it, e.g. ["Windows", "Linux"]; empty for a blank disk.
    pub contents: Vec<String>,
    /// The same in words: "Windows", "Windows and Linux", "Empty".
    pub description: String,
    pub bitlocker: bool,
    /// Under 40 GiB: listed but greyed out.
    pub too_small: bool,
    pub erase: Choice,
    pub free_space: FreeSpaceChoice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hidden {
    pub path: String,
    pub reason: String,
}

/// `ListDisks`' answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskList {
    pub disks: Vec<Disk>,
    /// Devices never offered, and why (for logs and tests; the UI ignores it).
    pub hidden: Vec<Hidden>,
}

/// Where dmsquash-live mounts the partition holding the ISO file when it
/// boots one with `iso-scan/filename=`.
pub const ISOSCAN_MOUNT: &str = "/run/initramfs/isoscan";

/// The devices dmsquash-live mounted at [`LIVE_MOUNT`] or [`ISOSCAN_MOUNT`],
/// from `/proc/self/mounts`. The helper adds their resolved paths.
pub fn live_sources(mounts: &str) -> Vec<String> {
    mounts
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (dev, dir) = (f.next()?, unescape(f.next()?));
            (dir == LIVE_MOUNT || dir == ISOSCAN_MOUNT).then(|| unescape(dev))
        })
        .collect()
}

/// `/proc/mounts` writes space, tab, newline and backslash as `\ooo`.
pub fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            let v = u32::from(b[i + 1] - b'0') * 64
                + u32::from(b[i + 2] - b'0') * 8
                + u32::from(b[i + 3] - b'0');
            let Ok(v) = u8::try_from(v) else {
                out.push(b[i]);
                i += 1;
                continue;
            };
            out.push(v);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn has_iso_label(d: &Device) -> bool {
    [d.label(), d.partlabel()]
        .into_iter()
        .flatten()
        .any(|l| ISO_LABELS.contains(&l))
}

fn is_ventoy(d: &Device) -> bool {
    d.name == "/dev/mapper/ventoy"
        || [d.label(), d.partlabel()]
            .into_iter()
            .flatten()
            .any(|l| l == "Ventoy" || l == "VTOYEFI")
}

/// The device, or something on it, is the live system's boot media.
fn holds_live_source(d: &Device, live_sources: &[String]) -> bool {
    d.walk().iter().any(|x| {
        live_sources
            .iter()
            .any(|src| x.name == *src || x.kname.as_deref() == Some(src))
    })
}

/// Why a top-level device is never offered, or `None` if it may be.
/// The helper asks this again before installing, whatever the UI sent.
pub fn hidden_reason(d: &Device, live_sources: &[String]) -> Option<&'static str> {
    let all = d.walk();
    if holds_live_source(d, live_sources) {
        return Some("the installer's boot media");
    }
    if all.iter().any(|x| has_iso_label(x)) {
        return Some("holds a Telamon OS installer");
    }
    if all.iter().any(|x| is_ventoy(x)) {
        return Some("Ventoy boot media");
    }
    let id = d.id();
    match d.kind.as_str() {
        "disk" => {}
        "rom" => return Some("optical drive"),
        "loop" => return Some("loop device"),
        _ => return Some("not a disk"),
    }
    if id.starts_with("zram") {
        return Some("zram");
    }
    if id.starts_with("ram") {
        return Some("RAM disk");
    }
    if id.starts_with("sr") {
        return Some("optical drive");
    }
    if d.ro {
        return Some("read-only");
    }
    if d.size == 0 {
        return Some("no media");
    }
    let raid = |x: &&Device| {
        x.kind.starts_with("raid")
            || x.fstype()
                .is_some_and(|f| f.ends_with("_raid_member") || f == "linux_raid_member")
    };
    if all.iter().any(raid) {
        return Some("RAID member");
    }
    if all
        .iter()
        .any(|x| x.kind == "mpath" || x.fstype() == Some("mpath_member"))
    {
        return Some("multipath member");
    }
    None
}

/// What is on a disk, from its filesystems and partition types.
fn contents(d: &Device, esps: &HashMap<String, EspInfo>) -> Vec<String> {
    let mut windows = false;
    let mut linux = false;
    let mut files = false;
    for p in d.walk().into_iter().skip(1) {
        let esp = gpt::is(p.parttype.as_deref(), gpt::ESP);
        match p.fstype() {
            Some("ntfs" | "BitLocker") => windows = true,
            Some(
                "ext2" | "ext3" | "ext4" | "btrfs" | "xfs" | "swap" | "LVM2_member" | "crypto_LUKS"
                | "f2fs",
            ) => linux = true,
            Some(_) if esp => windows |= esps.get(&p.name).is_some_and(|e| e.windows),
            Some(_) => files = true,
            None => {}
        }
        windows |= gpt::is(p.parttype.as_deref(), gpt::MS_RESERVED);
    }
    let mut out = Vec::new();
    if windows {
        out.push("Windows".to_string());
    }
    if linux {
        out.push("Linux".to_string());
    }
    if files {
        out.push("Files".to_string());
    }
    if out.is_empty() && (d.partitions().next().is_some() || d.fstype().is_some()) {
        out.push("Unknown data".to_string());
    }
    out
}

fn describe(contents: &[String]) -> String {
    match contents {
        [] => "Empty".into(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

fn friendly_name(d: &Device) -> String {
    let model = d.model().map(|m| m.replace('_', " "));
    match (d.vendor(), model) {
        (Some(v), Some(m)) if !m.to_lowercase().starts_with(&v.to_lowercase()) && v != "ATA" => {
            format!("{v} {m}")
        }
        (_, Some(m)) => m,
        _ if d.tran() == Some("usb") => "USB disk".into(),
        _ if d.id().starts_with("vd") => "Virtual disk".into(),
        _ => "Disk".into(),
    }
}

/// What the installer was started from, for the Restart page: "cd", "usb",
/// or "" when it isn't one of those or wasn't found.
pub fn boot_media(probe: &Probe) -> &'static str {
    let Some(lsblk) = &probe.lsblk else {
        return "";
    };
    let media = lsblk
        .blockdevices
        .iter()
        .find(|d| holds_live_source(d, &probe.live_sources));
    match media {
        Some(d) if d.kind == "rom" => "cd",
        Some(d) if d.tran() == Some("usb") => "usb",
        _ => "",
    }
}

/// Which other boot loader started the live system, if one did: `ventoy`
/// (from a Ventoy stick) or `iso-file` (an ISO file booted from another boot
/// loader's menu, which dmsquash-live mounts at [`ISOSCAN_MOUNT`]). That
/// loader is measured into PCR 7, and Telamon OS's own boot isn't, so a TPM key
/// sealed to PCR 7 now would never unseal on the installed system.
pub fn chain_loaded(probe: &Probe, mounts: &str) -> Option<&'static str> {
    let media = probe.lsblk.as_ref().and_then(|l| {
        l.blockdevices
            .iter()
            .find(|d| holds_live_source(d, &probe.live_sources))
    });
    if media.is_some_and(|d| d.walk().iter().any(|x| is_ventoy(x)))
        || probe.live_sources.iter().any(|s| s == "/dev/mapper/ventoy")
    {
        return Some("ventoy");
    }
    let iso_file = mounts.lines().any(|line| {
        line.split_whitespace()
            .nth(1)
            .is_some_and(|dir| unescape(dir) == ISOSCAN_MOUNT)
    });
    iso_file.then_some("iso-file")
}

/// The plan for `mode` on `disk`, or why it isn't possible. Used both for
/// listing and, after a fresh probe, by `Install`.
pub fn plan_for(disk: &Device, probe: &Probe, mode: Mode) -> Result<Plan, Unavailable> {
    match mode {
        Mode::Erase => plan_erase(disk),
        Mode::FreeSpace => {
            if disk.size < MIN_INSTALL_BYTES {
                return Err(Unavailable::TooSmall);
            }
            plan_free_space(disk, probe.tables.get(&disk.name), &probe.esps)
        }
    }
}

/// The visible disks and the hidden devices.
pub fn list(probe: &Probe) -> DiskList {
    let mut out = DiskList::default();
    let Some(lsblk) = &probe.lsblk else {
        return out;
    };
    for d in &lsblk.blockdevices {
        if let Some(reason) = hidden_reason(d, &probe.live_sources) {
            out.hidden.push(Hidden {
                path: d.name.clone(),
                reason: reason.into(),
            });
            continue;
        }
        let contents = contents(d, &probe.esps);
        let windows = contents.iter().any(|c| c == "Windows");
        let erase = match plan_for(d, probe, Mode::Erase) {
            Ok(_) => Choice {
                possible: true,
                reason: None,
            },
            Err(u) => Choice {
                possible: false,
                reason: Some(u.message(windows)),
            },
        };
        let table = probe.tables.get(&d.name);
        let bytes = table
            .and_then(|t| t.largest_free().map(|r| t.bytes(r.sectors)))
            .unwrap_or(0);
        let free_space = match plan_for(d, probe, Mode::FreeSpace) {
            Ok(p) => FreeSpaceChoice {
                possible: true,
                reason: None,
                bytes,
                shared_esp: (!p.esp_is_new).then(|| p.esp.clone()),
            },
            Err(u) => FreeSpaceChoice {
                possible: false,
                reason: Some(u.message(windows)),
                bytes,
                shared_esp: None,
            },
        };
        out.disks.push(Disk {
            id: d.id().to_string(),
            fingerprint: fingerprint(d, probe.tables.get(&d.name)),
            path: d.name.clone(),
            name: friendly_name(d),
            serial: d.serial().map(String::from),
            size: d.size,
            usb: d.tran() == Some("usb"),
            removable: d.rm || d.hotplug,
            description: describe(&contents),
            bitlocker: d.walk().iter().any(|p| p.fstype() == Some("BitLocker")),
            contents,
            too_small: d.size < MIN_INSTALL_BYTES,
            erase,
            free_space,
        });
    }
    out
}

/// The disk as the user saw it in the list: which disk it is (model,
/// serial, WWN, size) and what is on it (table, partitions, filesystems).
/// Install refuses a disk whose fingerprint changed since ListDisks, so a
/// replugged disk that took over the name, or a partition added meanwhile,
/// is never written to. FNV-1a, 16 hex digits; not a secret.
pub fn fingerprint(d: &Device, table: Option<&Table>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |s: &str| {
        for b in s.bytes().chain([0xff]) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    let o = |s: &Option<String>| s.clone().unwrap_or_default();
    // Device-mapper maps come and go while the installer runs (it closes a
    // map left by an earlier attempt), so they and everything inside them
    // (LVM in LUKS) are not part of the identity.
    let mut nodes = vec![d];
    let mut i = 0;
    while i < nodes.len() {
        let x = nodes[i];
        nodes.extend(x.children.iter().filter(|c| c.kind != "crypt"));
        i += 1;
    }
    for x in nodes {
        for f in [
            x.name.clone(),
            x.kind.clone(),
            x.size.to_string(),
            o(&x.model),
            o(&x.serial),
            o(&x.wwn),
            o(&x.pttype),
            o(&x.ptuuid),
            o(&x.parttype),
            o(&x.partuuid),
            o(&x.fstype),
            o(&x.uuid),
        ] {
            add(&f);
        }
    }
    if let Some(t) = table {
        for p in &t.partitions {
            add(&format!("{} {} {}", p.node, p.start, p.size));
        }
    }
    format!("{h:016x}")
}

/// The visible disk `id` (a kernel name such as `sda`), or why it can't be
/// installed to. Never returns a hidden device.
pub fn find_visible<'a>(probe: &'a Probe, id: &str) -> Result<&'a Device, String> {
    let lsblk = probe.lsblk.as_ref().ok_or("no disks were found")?;
    let d = lsblk
        .blockdevices
        .iter()
        .find(|d| d.id() == id)
        .ok_or_else(|| format!("there is no disk {id:?}"))?;
    if let Some(reason) = hidden_reason(d, &probe.live_sources) {
        return Err(format!("{} can't be installed to: {reason}", d.name));
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GIB, MIB};

    const WIN_LSBLK: &str = include_str!("../tests/fixtures/lsblk-windows.json");
    const WIN_SFDISK: &str = include_str!("../tests/fixtures/sfdisk-windows.json");
    const MIXED: &str = include_str!("../tests/fixtures/lsblk-mixed.json");
    const VENTOY: &str = include_str!("../tests/fixtures/lsblk-ventoy.json");

    fn win_probe(esp_free: u64) -> Probe {
        Probe {
            lsblk: Some(Lsblk::parse(WIN_LSBLK).unwrap()),
            live_sources: vec!["/dev/sr0".into()],
            tables: HashMap::from([("/dev/sda".into(), Table::parse(WIN_SFDISK).unwrap())]),
            esps: HashMap::from([(
                "/dev/sda1".into(),
                EspInfo {
                    fs_free: esp_free,
                    windows: true,
                    fs_uuid: Some("4A1B-2C3D".into()),
                    ..Default::default()
                },
            )]),
        }
    }

    #[test]
    fn windows_disk_offers_both_choices() {
        let l = list(&win_probe(63 * MIB));
        assert_eq!(l.disks.len(), 1);
        let d = &l.disks[0];
        assert_eq!(d.id, "sda");
        assert_eq!(d.name, "QEMU HARDDISK");
        assert_eq!(d.contents, ["Windows"]);
        assert_eq!(d.description, "Windows");
        assert!(!d.usb && !d.too_small && !d.bitlocker);
        assert!(d.erase.possible);
        assert!(d.free_space.possible);
        assert_eq!(d.free_space.bytes / MIB, 69515);
        assert_eq!(d.free_space.shared_esp.as_deref(), Some("/dev/sda1"));
        assert_eq!(
            l.hidden,
            [Hidden {
                path: "/dev/sr0".into(),
                reason: "the installer's boot media".into()
            }]
        );
    }

    #[test]
    fn mixed_machine() {
        let mounts = "/dev/sdb1 /run/initramfs/live iso9660 ro 0 0\n";
        let p = Probe {
            lsblk: Some(Lsblk::parse(MIXED).unwrap()),
            live_sources: live_sources(mounts),
            ..Default::default()
        };
        let l = list(&p);
        let shown: Vec<_> = l.disks.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(shown, ["sdc", "sdd", "nvme0n1", "vda", "vdb"]);
        let hidden: Vec<_> = l
            .hidden
            .iter()
            .map(|h| (h.path.as_str(), h.reason.as_str()))
            .collect();
        assert_eq!(
            hidden,
            [
                ("/dev/loop0", "loop device"),
                ("/dev/sda", "holds a Telamon OS installer"),
                ("/dev/sdb", "the installer's boot media"),
                ("/dev/sde", "RAID member"),
                ("/dev/sdf", "read-only"),
                ("/dev/sdg", "no media"),
                ("/dev/sr0", "optical drive"),
                ("/dev/zram0", "zram"),
            ]
        );

        let usb = &l.disks[0];
        assert!(usb.usb && usb.removable);
        assert_eq!(usb.name, "SanDisk Extreme");
        assert_eq!(usb.description, "Files");

        let small = &l.disks[1];
        assert_eq!(small.name, "KINGSTON SA400S37");
        assert!(small.too_small);
        assert_eq!(small.size, 30 * GIB);
        assert_eq!(
            small.erase.reason.as_deref(),
            Some("Too small: Telamon OS needs 40 GB.")
        );
        assert_eq!(
            small.free_space.reason.as_deref(),
            Some("Too small: Telamon OS needs 40 GB.")
        );

        let nvme = &l.disks[2];
        assert_eq!(nvme.name, "Samsung SSD 990 PRO 1TB");
        assert_eq!(nvme.contents, ["Windows", "Linux"]);
        assert_eq!(nvme.description, "Windows and Linux");
        assert!(nvme.bitlocker && !nvme.usb);
        assert!(nvme.erase.possible);

        // an installed Telamon OS (btrfs label "atlasos") is not the ISO ("ATLASOS")
        let installed = &l.disks[3];
        assert_eq!(installed.contents, ["Linux"]);
        assert!(installed.erase.possible);

        let blank = &l.disks[4];
        assert_eq!(blank.name, "Virtual disk");
        assert_eq!(blank.description, "Empty");
        assert!(blank.erase.possible);
        assert_eq!(
            blank.free_space.reason.as_deref(),
            Some("This disk has no partitions yet. Erase it to install Telamon OS.")
        );
    }

    #[test]
    fn ventoy_stick_is_hidden_with_or_without_the_live_mount() {
        let l = Lsblk::parse(VENTOY).unwrap();
        for src in [Some("/dev/mapper/ventoy"), Some("/dev/dm-0"), None] {
            let p = Probe {
                lsblk: Some(l.clone()),
                live_sources: src.map(String::from).into_iter().collect(),
                ..Default::default()
            };
            let list = list(&p);
            assert_eq!(
                list.disks.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
                ["nvme0n1"]
            );
            assert_eq!(list.hidden.len(), 1);
        }
    }

    #[test]
    fn find_visible_refuses_hidden_and_unknown_devices() {
        let p = win_probe(63 * MIB);
        assert!(find_visible(&p, "sda").is_ok());
        let e = find_visible(&p, "sr0").unwrap_err();
        assert!(e.contains("boot media"), "{e}");
        assert!(find_visible(&p, "sdz").is_err());
        assert!(find_visible(&Probe::default(), "sda").is_err());
    }

    #[test]
    fn full_esp_means_a_new_one_and_more_space() {
        let l = list(&win_probe(10 * MIB));
        let d = &l.disks[0];
        assert!(d.free_space.possible, "67 GiB is enough for a new ESP too");
        assert_eq!(d.free_space.shared_esp, None);
    }

    #[test]
    fn boot_media_says_cd_or_usb() {
        assert_eq!(boot_media(&win_probe(63 * MIB)), "cd");
        let ventoy = |src: &str| Probe {
            lsblk: Some(Lsblk::parse(VENTOY).unwrap()),
            live_sources: vec![src.into()],
            ..Default::default()
        };
        assert_eq!(boot_media(&ventoy("/dev/mapper/ventoy")), "usb");
        assert_eq!(boot_media(&ventoy("/dev/nvme0n1")), "", "an internal disk");
        assert_eq!(boot_media(&ventoy("/dev/nope")), "");
        assert_eq!(boot_media(&Probe::default()), "");
    }

    #[test]
    fn a_live_system_another_loader_started_is_told_apart() {
        let ventoy = |src: &str| Probe {
            lsblk: Some(Lsblk::parse(VENTOY).unwrap()),
            live_sources: vec![src.into()],
            ..Default::default()
        };
        let live = "/dev/mapper/ventoy /run/initramfs/live iso9660 ro 0 0\n";
        assert_eq!(
            chain_loaded(&ventoy("/dev/mapper/ventoy"), live),
            Some("ventoy")
        );
        assert_eq!(chain_loaded(&ventoy("/dev/dm-0"), ""), Some("ventoy"));
        // without lsblk, by the device's name
        let bare = Probe {
            live_sources: vec!["/dev/mapper/ventoy".into()],
            ..Default::default()
        };
        assert_eq!(chain_loaded(&bare, ""), Some("ventoy"));
        // a Ventoy stick plugged in beside the real boot media: not it
        assert_eq!(chain_loaded(&ventoy("/dev/nvme0n1"), ""), None);
        // the ISO's own boot loader, from a stick or a CD
        assert_eq!(chain_loaded(&win_probe(63 * MIB), ""), None);
        assert_eq!(chain_loaded(&Probe::default(), ""), None);
        // an ISO file from another boot loader's menu
        let m = "/dev/sdb2 /run/initramfs/isoscan ext4 ro 0 0\n/dev/loop0 /run/initramfs/live iso9660 ro 0 0\n";
        let iso = Probe {
            live_sources: live_sources(m),
            ..Default::default()
        };
        assert_eq!(chain_loaded(&iso, m), Some("iso-file"));
    }

    #[test]
    fn live_sources_read_proc_mounts() {
        let m = "proc /proc proc rw 0 0\n/dev/sda1 /run/initramfs/live vfat ro 0 0\n";
        assert_eq!(live_sources(m), ["/dev/sda1"]);
        let m = "/dev/disk\\040x /run/initramfs/live iso9660 ro 0 0\n";
        assert_eq!(live_sources(m), ["/dev/disk x"]);
        assert!(live_sources("proc /proc proc rw 0 0\n").is_empty());
        // an ISO file booted from a partition: that partition's disk too
        let m = "/dev/sdb2 /run/initramfs/isoscan ext4 ro 0 0\n/dev/loop0 /run/initramfs/live iso9660 ro 0 0\n";
        assert_eq!(live_sources(m), ["/dev/sdb2", "/dev/loop0"]);
        // not an octal escape a byte can hold: kept as written
        assert_eq!(
            live_sources("/dev/a\\777 /run/initramfs/live x ro 0 0\n"),
            ["/dev/a\\777"]
        );
    }

    #[test]
    fn isoscan_partition_hides_its_disk() {
        let p = Probe {
            lsblk: Some(Lsblk::parse(WIN_LSBLK).unwrap()),
            live_sources: vec!["/dev/sda3".into()],
            ..Default::default()
        };
        let l = list(&p);
        assert!(l.disks.is_empty());
        assert!(
            l.hidden
                .iter()
                .any(|h| h.path == "/dev/sda" && h.reason.contains("boot media"))
        );
    }

    #[test]
    fn fingerprint_follows_identity_and_contents() {
        let p = win_probe(63 * MIB);
        let lsblk = p.lsblk.as_ref().unwrap();
        let d = lsblk.find("/dev/sda").unwrap();
        let t = p.tables.get("/dev/sda");
        let f = fingerprint(d, t);
        assert_eq!(f.len(), 16);
        assert_eq!(f, fingerprint(d, t), "stable");
        assert_eq!(list(&p).disks[0].fingerprint, f);
        let mut other = d.clone();
        other.serial = Some("OTHER".into());
        assert_ne!(
            fingerprint(&other, t),
            f,
            "another disk under the same name"
        );
        let mut changed = d.clone();
        changed.children.pop();
        assert_ne!(fingerprint(&changed, t), f, "a partition went away");
        let mut moved = t.unwrap().clone();
        moved.partitions[0].start += 2048;
        assert_ne!(fingerprint(d, Some(&moved)), f, "a partition moved");
        // what is mounted, or free on the ESP, doesn't matter
        let mut mounted = d.clone();
        mounted.children[0].mountpoints = vec![Some("/boot/efi".into())];
        assert_eq!(fingerprint(&mounted, t), f);
        // nor an open LUKS map, or LVM inside it, coming or going
        let mut opened = d.clone();
        let mut lv = d.children[0].clone();
        lv.name = "/dev/mapper/vg-root".into();
        lv.kind = "lvm".into();
        lv.children.clear();
        let mut map = lv.clone();
        map.name = "/dev/mapper/luks-1".into();
        map.kind = "crypt".into();
        map.children = vec![lv];
        opened.children[0].children.push(map);
        assert_eq!(fingerprint(&opened, t), f);
    }

    #[test]
    fn descriptions() {
        assert_eq!(describe(&[]), "Empty");
        assert_eq!(
            describe(&["Windows".into(), "Linux".into(), "Files".into()]),
            "Windows, Linux and Files"
        );
    }
}
