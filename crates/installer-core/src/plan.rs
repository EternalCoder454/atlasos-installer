//! The partition plan for the two choices (Plan: "Partition layouts").
//!
//! | | Erase | Use free space |
//! |---|---|---|
//! | Table | new GPT | existing GPT, untouched |
//! | EFI | new 600 MiB | the disk's ESP if ≥ 100 MiB with ≥ 40 MiB free, else a new 600 MiB one |
//! | /boot | 2 GiB ext4 | 2 GiB ext4 in the largest free region |
//! | / | rest of the disk, btrfs | rest of that region, btrfs |

use std::collections::HashMap;
use std::fmt::Write as _;
use std::str::FromStr;

use serde::Serialize;

use crate::lsblk::Device;
use crate::table::{Table, check_unchanged, partition_node, partition_number};
use crate::{
    BOOT_BYTES, ESP_BYTES, ESP_REUSE_MIN_FREE, ESP_REUSE_MIN_PART, GIB, MIN_INSTALL_BYTES, gpt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Erase,
    FreeSpace,
}

impl FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> Result<Mode, String> {
        match s {
            "erase" => Ok(Mode::Erase),
            "free-space" => Ok(Mode::FreeSpace),
            _ => Err(format!("unknown mode {s:?}: expected erase or free-space")),
        }
    }
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Erase => "erase",
            Mode::FreeSpace => "free-space",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Esp,
    Boot,
    Root,
}

/// A partition the plan creates. `start` and `size` are in sectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewPart {
    pub role: Role,
    pub node: String,
    pub start: u64,
    pub size: u64,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: &'static str,
}

/// What the helper found on an EFI partition, mounted read-only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EspInfo {
    /// Free bytes in its filesystem.
    pub fs_free: u64,
    /// It holds `EFI/Microsoft/Boot/bootmgfw.efi`.
    pub windows: bool,
    /// It holds `EFI/fedora` (another Fedora-family system).
    pub fedora: bool,
    /// Its filesystem UUID (`ABCD-1234`).
    pub fs_uuid: Option<String>,
}

/// Why a choice is not possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    TooSmall,
    InUse,
    NoTable,
    NotGpt,
    NoRoom {
        free: u64,
        needed: u64,
    },
    TableFull,
    /// What was read about the disk doesn't add up, or the plan failed its
    /// own check; never install from it.
    Unsafe(&'static str),
}

impl Unavailable {
    /// The text shown under the greyed-out choice. `windows`: the disk holds
    /// Windows, so the way to make room is to shrink it.
    pub fn message(self, windows: bool) -> String {
        match self {
            Unavailable::TooSmall => "Too small: AtlasOS needs 40 GB.".into(),
            Unavailable::InUse => "A partition on this disk is in use. Restart the installer and try again.".into(),
            Unavailable::NoTable => "This disk has no partitions yet. Erase it to install AtlasOS.".into(),
            Unavailable::NotGpt => {
                "This disk uses an old MBR partition table, so AtlasOS can't install beside what's on it.".into()
            }
            Unavailable::NoRoom { free, needed } => {
                let have = format!("{} GB", free / GIB);
                let need = format!("{} GB", needed.div_ceil(GIB));
                if windows {
                    format!(
                        "Not enough free space ({have}, AtlasOS needs {need}). Shrink Windows in Disk Management to make room, then restart the installer."
                    )
                } else {
                    format!("Not enough free space ({have}, AtlasOS needs {need}).")
                }
            }
            Unavailable::TableFull => "This disk's partition table has no room for more partitions.".into(),
            Unavailable::Unsafe(why) => format!("The installer can't safely change this disk ({why})."),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// `/dev/...` of the whole disk.
    pub disk: String,
    pub mode: Mode,
    pub sector_size: u64,
    /// The table before (free space only), for the safety check.
    pub before: Option<Table>,
    /// The first and last sectors a partition may use.
    pub usable: (u64, u64),
    /// Erase: partitions to clear signatures from, then the disk itself.
    pub wipe: Vec<String>,
    /// Partitions to create, in disk order.
    pub parts: Vec<NewPart>,
    /// The EFI partition the bootloader goes on.
    pub esp: String,
    /// The ESP is one of `parts` (formatted), not an existing one (never formatted).
    pub esp_is_new: bool,
}

/// GPT reserves 128 entries of 128 bytes, plus a header, at both ends.
fn gpt_entry_sectors(sector_size: u64) -> u64 {
    (128 * 128u64).div_ceil(sector_size)
}

fn gpt_last_usable(total_sectors: u64, sector_size: u64) -> u64 {
    total_sectors.saturating_sub(2 + gpt_entry_sectors(sector_size))
}

fn layout(
    disk: &str,
    numbers: &[u32],
    start: u64,
    end_excl: u64,
    ss: u64,
    new_esp: bool,
) -> Vec<NewPart> {
    let mut parts = Vec::new();
    let mut at = start;
    let mut n = numbers.iter();
    let mut push = |role, size: u64, kind, name, at: &mut u64| {
        let node = partition_node(disk, *n.next().expect("enough numbers"));
        parts.push(NewPart {
            role,
            node,
            start: *at,
            size,
            kind,
            name,
        });
        *at += size;
    };
    if new_esp {
        push(
            Role::Esp,
            ESP_BYTES / ss,
            gpt::ESP,
            "EFI System Partition",
            &mut at,
        );
    }
    push(Role::Boot, BOOT_BYTES / ss, gpt::LINUX_FS, "boot", &mut at);
    let rest = end_excl - at;
    push(Role::Root, rest, gpt::ROOT_X86_64, "root", &mut at);
    parts
}

/// A disk with anything mounted on it (or active swap, LVM or encryption
/// on top of it) can't be partitioned.
pub fn in_use(disk: &Device) -> bool {
    disk.walk()
        .iter()
        .any(|d| d.mounts().next().is_some() || !matches!(d.kind.as_str(), "disk" | "part"))
}

/// Erase the whole disk.
pub fn plan_erase(disk: &Device) -> Result<Plan, Unavailable> {
    if disk.size < MIN_INSTALL_BYTES {
        return Err(Unavailable::TooSmall);
    }
    if in_use(disk) {
        return Err(Unavailable::InUse);
    }
    let ss = disk.sector_size();
    let align = (crate::ALIGN_BYTES / ss).max(1);
    let last = gpt_last_usable(disk.size / ss, ss);
    let end_excl = (last + 1) / align * align;
    if end_excl < align + (ESP_BYTES + BOOT_BYTES) / ss {
        return Err(Unavailable::TooSmall);
    }
    let parts = layout(&disk.name, &[1, 2, 3], align, end_excl, ss, true);
    let mut wipe: Vec<String> = disk.partitions().map(|p| p.name.clone()).collect();
    wipe.push(disk.name.clone());
    let plan = Plan {
        disk: disk.name.clone(),
        mode: Mode::Erase,
        sector_size: ss,
        before: None,
        usable: (2 + gpt_entry_sectors(ss), last),
        wipe,
        esp: parts[0].node.clone(),
        esp_is_new: true,
        parts,
    };
    plan.validate().map_err(Unavailable::Unsafe)?;
    Ok(plan)
}

/// An existing ESP on `table` that can be shared: big enough, and not
/// already used by another Fedora-family system, whose `EFI/fedora` files
/// and boot entry this install would replace.
pub fn reusable_esp<'a>(table: &'a Table, esps: &HashMap<String, EspInfo>) -> Option<&'a str> {
    table
        .partitions
        .iter()
        .filter(|p| gpt::is(Some(&p.kind), gpt::ESP))
        .filter(|p| table.bytes(p.size) >= ESP_REUSE_MIN_PART)
        .find(|p| {
            esps.get(&p.node)
                .is_some_and(|e| e.fs_free >= ESP_REUSE_MIN_FREE && !e.fedora)
        })
        .map(|p| p.node.as_str())
}

/// Use the largest free region of `table` (the disk's current table).
pub fn plan_free_space(
    disk: &Device,
    table: Option<&Table>,
    esps: &HashMap<String, EspInfo>,
) -> Result<Plan, Unavailable> {
    let table = table.ok_or(Unavailable::NoTable)?;
    if !table.is_gpt() {
        return Err(Unavailable::NotGpt);
    }
    if in_use(disk) {
        return Err(Unavailable::InUse);
    }
    if table.device != disk.name {
        return Err(Unavailable::Unsafe(
            "sfdisk and lsblk name the disk differently",
        ));
    }
    if table.sectorsize != disk.sector_size() {
        return Err(Unavailable::Unsafe(
            "sfdisk and lsblk disagree on the sector size",
        ));
    }
    let (Some(first), Some(last)) = (table.firstlba, table.lastlba) else {
        return Err(Unavailable::Unsafe(
            "the partition table has no usable range",
        ));
    };
    let reuse = reusable_esp(table, esps);
    let needed = MIN_INSTALL_BYTES + if reuse.is_some() { 0 } else { ESP_BYTES };
    let region = table.largest_free();
    let free = region.map_or(0, |r| table.bytes(r.sectors));
    let Some(region) = region.filter(|_| free >= needed) else {
        return Err(Unavailable::NoRoom { free, needed });
    };
    let count = if reuse.is_some() { 2 } else { 3 };
    let numbers = table.free_numbers(count);
    if numbers.iter().any(|n| *n > 128) {
        return Err(Unavailable::TableFull);
    }
    let ss = table.sectorsize;
    let parts = layout(
        &disk.name,
        &numbers,
        region.start,
        region.start + region.sectors,
        ss,
        reuse.is_none(),
    );
    let esp = reuse.map_or_else(|| parts[0].node.clone(), str::to_string);
    let plan = Plan {
        disk: disk.name.clone(),
        mode: Mode::FreeSpace,
        sector_size: ss,
        before: Some(table.clone()),
        usable: (first, last),
        wipe: Vec::new(),
        esp,
        esp_is_new: reuse.is_none(),
        parts,
    };
    plan.validate().map_err(Unavailable::Unsafe)?;
    Ok(plan)
}

impl Plan {
    pub fn part(&self, role: Role) -> Option<&NewPart> {
        self.parts.iter().find(|p| p.role == role)
    }

    /// Before writing: every new partition lies in the usable range, starts
    /// on a 1 MiB boundary, and overlaps nothing, old or new; its node is a
    /// partition of the disk not in use. The plan functions check this, and
    /// the helper again right before sfdisk.
    pub fn validate(&self) -> Result<(), &'static str> {
        let align = (crate::ALIGN_BYTES / self.sector_size.max(1)).max(1);
        let (first, last) = self.usable;
        let old = self.before.as_ref().map_or(&[][..], |t| &t.partitions[..]);
        let mut taken: Vec<(u64, u64, &str)> = old
            .iter()
            .map(|p| (p.start, p.size, p.node.as_str()))
            .collect();
        for p in &self.parts {
            if p.size == 0 || p.start < first || p.start.saturating_add(p.size) - 1 > last {
                return Err("a new partition falls outside the usable sectors");
            }
            if p.start % align != 0 {
                return Err("a new partition is not aligned");
            }
            if partition_number(&self.disk, &p.node).is_none() {
                return Err("a new partition's name is not on this disk");
            }
            for &(start, size, node) in &taken {
                if node == p.node {
                    return Err("a new partition reuses an existing number");
                }
                if p.start < start.saturating_add(size) && start < p.start + p.size {
                    return Err("a new partition overlaps another");
                }
            }
            taken.push((p.start, p.size, &p.node));
        }
        Ok(())
    }

    /// sfdisk's arguments (the script goes on stdin).
    pub fn sfdisk_args(&self) -> Vec<String> {
        let mut a: Vec<String> = vec![
            "--quiet".into(),
            "--wipe-partitions".into(),
            "always".into(),
        ];
        match self.mode {
            Mode::Erase => a.extend(["--wipe".into(), "always".into()]),
            Mode::FreeSpace => a.push("--append".into()),
        }
        a.push(self.disk.clone());
        a
    }

    /// The sfdisk script. Every partition names its node, so the numbers
    /// are the planned ones.
    pub fn sfdisk_script(&self) -> String {
        let mut s = String::new();
        if self.mode == Mode::Erase {
            s.push_str("label: gpt\n");
        }
        for p in &self.parts {
            let _ = writeln!(
                s,
                "{} : start={}, size={}, type={}, name=\"{}\"",
                p.node, p.start, p.size, p.kind, p.name
            );
        }
        s
    }

    /// After writing: the new table holds exactly the old partitions,
    /// unchanged, plus the planned ones.
    pub fn check_written(&self, after: &Table) -> Result<(), String> {
        if !after.is_gpt() {
            return Err("the disk has no GPT partition table after partitioning".into());
        }
        if let Some(before) = &self.before {
            check_unchanged(before, after)?;
        }
        let kept = self.before.as_ref().map_or(0, |b| b.partitions.len());
        if after.partitions.len() != kept + self.parts.len() {
            return Err(format!(
                "expected {} partitions after partitioning, found {}",
                kept + self.parts.len(),
                after.partitions.len()
            ));
        }
        for p in &self.parts {
            let Some(q) = after.partition(&p.node) else {
                return Err(format!("{} was not created", p.node));
            };
            if q.start != p.start || q.size != p.size || !q.kind.eq_ignore_ascii_case(p.kind) {
                return Err(format!("{} was not created as planned", p.node));
            }
        }
        Ok(())
    }

    /// A readable summary, for `atlas-installer-helper --plan`.
    pub fn describe(&self) -> String {
        let mib = |sectors: u64| sectors * self.sector_size / crate::MIB;
        let mut s = String::new();
        let _ = writeln!(s, "disk {} ({})", self.disk, self.mode.as_str());
        if !self.wipe.is_empty() {
            let _ = writeln!(s, "wipe signatures: {}", self.wipe.join(" "));
        }
        if let Some(b) = &self.before {
            let _ = writeln!(
                s,
                "keep {} existing partitions unchanged:",
                b.partitions.len()
            );
            for p in &b.partitions {
                let _ = writeln!(
                    s,
                    "  {} start={} size={} ({} MiB)",
                    p.node,
                    p.start,
                    p.size,
                    mib(p.size)
                );
            }
        }
        let _ = writeln!(s, "create:");
        for p in &self.parts {
            let _ = writeln!(
                s,
                "  {} {:?} start={} size={} ({} MiB)",
                p.node,
                p.role,
                p.start,
                p.size,
                mib(p.size)
            );
        }
        let how = if self.esp_is_new {
            "new, formatted"
        } else {
            "existing, shared, not formatted"
        };
        let _ = writeln!(s, "EFI partition: {} ({how})", self.esp);
        let _ = writeln!(s, "sfdisk {}", self.sfdisk_args().join(" "));
        for line in self.sfdisk_script().lines() {
            let _ = writeln!(s, "  {line}");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsblk::Lsblk;
    use crate::{MIB, table::Partition};

    const LSBLK: &str = include_str!("../tests/fixtures/lsblk-windows.json");
    const SFDISK: &str = include_str!("../tests/fixtures/sfdisk-windows.json");

    fn windows() -> (Device, Table) {
        let l = Lsblk::parse(LSBLK).unwrap();
        (
            l.find("/dev/sda").unwrap().clone(),
            Table::parse(SFDISK).unwrap(),
        )
    }

    fn blank(gib: u64) -> Device {
        Device {
            name: "/dev/vda".into(),
            kind: "disk".into(),
            size: gib * GIB,
            log_sec: Some(512),
            ..Default::default()
        }
    }

    fn esp_free(mib: u64) -> HashMap<String, EspInfo> {
        HashMap::from([(
            "/dev/sda1".to_string(),
            EspInfo {
                fs_free: mib * MIB,
                windows: true,
                fs_uuid: Some("ABCD-1234".into()),
                fedora: false,
            },
        )])
    }

    #[test]
    fn never_shares_another_fedoras_esp() {
        let (d, t) = windows();
        let mut esps = esp_free(63);
        esps.get_mut("/dev/sda1").unwrap().fedora = true;
        let p = plan_free_space(&d, Some(&t), &esps).unwrap();
        assert!(p.esp_is_new);
        assert_eq!(p.esp, "/dev/sda5");
    }

    #[test]
    fn free_space_refuses_tables_that_dont_match_the_disk() {
        let (d, t) = windows();
        let mut other = t.clone();
        other.device = "/dev/sdb".into();
        assert!(matches!(
            plan_free_space(&d, Some(&other), &esp_free(63)),
            Err(Unavailable::Unsafe(_))
        ));
        let mut big = t.clone();
        big.sectorsize = 4096;
        assert!(matches!(
            plan_free_space(&d, Some(&big), &esp_free(63)),
            Err(Unavailable::Unsafe(_))
        ));
        let mut no_range = t;
        no_range.lastlba = None;
        assert!(matches!(
            plan_free_space(&d, Some(&no_range), &esp_free(63)),
            Err(Unavailable::Unsafe(_))
        ));
    }

    #[test]
    fn free_space_on_a_4k_disk() {
        let (mut d, mut t) = windows();
        d.log_sec = Some(4096);
        t.sectorsize = 4096;
        // the same layout in 4 KiB sectors: 128 GiB, partitions as before / 8
        for p in &mut t.partitions {
            p.start /= 8;
            p.size /= 8;
        }
        t.firstlba = Some(6);
        t.lastlba = Some(128 * GIB / 4096 - 6);
        let p = plan_free_space(&d, Some(&t), &esp_free(63)).unwrap();
        assert_eq!(p.parts[0].start % 256, 0, "1 MiB is 256 sectors");
        assert_eq!(p.parts[0].size * 4096, crate::BOOT_BYTES);
        assert!((p.parts[1].start + p.parts[1].size - 1) <= t.lastlba.unwrap());
        assert_eq!(p.validate(), Ok(()));
    }

    #[test]
    fn validate_catches_overlaps_and_strays() {
        let (d, t) = windows();
        let good = plan_free_space(&d, Some(&t), &esp_free(63)).unwrap();
        let mut overlap = good.clone();
        overlap.parts[0].start = t.partitions[2].start + 2048;
        assert_eq!(overlap.validate(), Err("a new partition overlaps another"));
        let mut past_end = good.clone();
        past_end.parts[1].size += 4096;
        assert_eq!(
            past_end.validate(),
            Err("a new partition falls outside the usable sectors")
        );
        let mut odd = good.clone();
        odd.parts[0].start += 1;
        assert_eq!(odd.validate(), Err("a new partition is not aligned"));
        let mut reused = good.clone();
        reused.parts[0].node = "/dev/sda2".into();
        assert_eq!(
            reused.validate(),
            Err("a new partition reuses an existing number")
        );
        let mut elsewhere = good.clone();
        elsewhere.parts[0].node = "/dev/sdb5".into();
        assert_eq!(
            elsewhere.validate(),
            Err("a new partition's name is not on this disk")
        );
        let mut twice = good;
        twice.parts[1].start = twice.parts[0].start;
        assert!(twice.validate().is_err());
        assert_eq!(plan_erase(&blank(64)).unwrap().validate(), Ok(()));
    }

    #[test]
    fn erase_layout_is_esp_boot_root_aligned() {
        let p = plan_erase(&blank(64)).unwrap();
        let roles: Vec<_> = p.parts.iter().map(|x| (x.role, x.node.as_str())).collect();
        assert_eq!(
            roles,
            [
                (Role::Esp, "/dev/vda1"),
                (Role::Boot, "/dev/vda2"),
                (Role::Root, "/dev/vda3")
            ]
        );
        assert_eq!(p.parts[0].start, 2048);
        assert_eq!(p.parts[0].size * 512, 600 * MIB);
        assert_eq!(p.parts[1].start, 2048 + 600 * 2048);
        assert_eq!(p.parts[1].size * 512, 2 * GIB);
        let root = &p.parts[2];
        assert_eq!(root.start, p.parts[1].start + p.parts[1].size);
        assert_eq!((root.start + root.size) % 2048, 0);
        // the root ends before the backup GPT (last 33 sectors)
        assert!(root.start + root.size <= 64 * GIB / 512 - 33);
        assert_eq!(p.wipe, ["/dev/vda"]);
        assert!(p.esp_is_new);
        assert_eq!(p.esp, "/dev/vda1");
    }

    #[test]
    fn erase_wipes_partitions_then_the_disk() {
        let (d, _) = windows();
        let p = plan_erase(&d).unwrap();
        assert_eq!(
            p.wipe,
            [
                "/dev/sda1",
                "/dev/sda2",
                "/dev/sda3",
                "/dev/sda4",
                "/dev/sda"
            ]
        );
    }

    #[test]
    fn erase_on_nvme_and_4k_disks() {
        let mut d = blank(100);
        d.name = "/dev/nvme0n1".into();
        d.log_sec = Some(4096);
        let p = plan_erase(&d).unwrap();
        assert_eq!(p.parts[0].node, "/dev/nvme0n1p1");
        assert_eq!(p.parts[0].start, 256);
        assert_eq!(p.parts[0].size * 4096, 600 * MIB);
        let root = &p.parts[2];
        assert!(root.start + root.size <= 100 * GIB / 4096 - 5);
    }

    #[test]
    fn erase_refuses_small_and_busy_disks() {
        assert_eq!(plan_erase(&blank(39)), Err(Unavailable::TooSmall));
        assert!(plan_erase(&blank(40)).is_ok());
        let mut d = blank(64);
        d.children.push(Device {
            name: "/dev/vda1".into(),
            kind: "part".into(),
            mountpoints: vec![Some("/mnt".into())],
            ..Default::default()
        });
        assert_eq!(plan_erase(&d), Err(Unavailable::InUse));
        d.children[0].mountpoints = vec![None];
        d.children[0].children.push(Device {
            name: "/dev/mapper/vg-lv".into(),
            kind: "lvm".into(),
            ..Default::default()
        });
        assert_eq!(plan_erase(&d), Err(Unavailable::InUse));
    }

    #[test]
    fn free_space_shares_windows_esp() {
        let (d, t) = windows();
        let p = plan_free_space(&d, Some(&t), &esp_free(63)).unwrap();
        assert!(!p.esp_is_new);
        assert_eq!(p.esp, "/dev/sda1");
        let roles: Vec<_> = p.parts.iter().map(|x| (x.role, x.node.as_str())).collect();
        assert_eq!(
            roles,
            [(Role::Boot, "/dev/sda5"), (Role::Root, "/dev/sda6")]
        );
        assert_eq!(p.parts[0].start, 126066688);
        assert_eq!(p.parts[1].start, 126066688 + 4194304);
        let r = t.largest_free().unwrap();
        assert_eq!(p.parts[1].start + p.parts[1].size, r.start + r.sectors);
        assert!(p.wipe.is_empty());
        assert_eq!(
            p.sfdisk_args(),
            [
                "--quiet",
                "--wipe-partitions",
                "always",
                "--append",
                "/dev/sda"
            ]
        );
        assert_eq!(
            p.sfdisk_script(),
            "/dev/sda5 : start=126066688, size=4194304, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=\"boot\"\n\
             /dev/sda6 : start=130260992, size=138172416, type=4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709, name=\"root\"\n"
        );
    }

    #[test]
    fn free_space_makes_a_new_esp_when_the_old_one_is_full_or_small() {
        let (d, t) = windows();
        for esps in [esp_free(39), HashMap::new()] {
            let p = plan_free_space(&d, Some(&t), &esps).unwrap();
            assert!(p.esp_is_new);
            assert_eq!(p.esp, "/dev/sda5");
            assert_eq!(p.parts.len(), 3);
        }
        // a 99 MiB ESP is too small whatever its free space
        let mut t2 = t.clone();
        t2.partitions[0].size = 99 * 2048;
        assert!(
            plan_free_space(&d, Some(&t2), &esp_free(90))
                .unwrap()
                .esp_is_new
        );
    }

    #[test]
    fn free_space_needs_40_gib_and_a_gpt() {
        let (d, mut t) = windows();
        // shrink the free tail to 39 GiB
        t.lastlba = Some(126066688 + 39 * 2 * 1024 * 1024 + 100);
        match plan_free_space(&d, Some(&t), &esp_free(63)) {
            Err(Unavailable::NoRoom { free, needed }) => {
                assert_eq!(free, 39 * GIB);
                assert_eq!(needed, 40 * GIB);
            }
            other => panic!("{other:?}"),
        }
        // exactly 40 GiB with the ESP shared is enough
        t.lastlba = Some(126066688 + 40 * 2 * 1024 * 1024 + 100);
        assert!(plan_free_space(&d, Some(&t), &esp_free(63)).is_ok());
        // ...but not when a new ESP is needed too
        assert!(matches!(
            plan_free_space(&d, Some(&t), &HashMap::new()),
            Err(Unavailable::NoRoom { needed, .. }) if needed == 40 * GIB + 600 * MIB
        ));
        assert_eq!(
            plan_free_space(&d, None, &HashMap::new()),
            Err(Unavailable::NoTable)
        );
        t.label = "dos".into();
        assert_eq!(
            plan_free_space(&d, Some(&t), &HashMap::new()),
            Err(Unavailable::NotGpt)
        );
    }

    #[test]
    fn check_written_accepts_the_plan_and_nothing_else() {
        let (d, t) = windows();
        let p = plan_free_space(&d, Some(&t), &esp_free(63)).unwrap();
        let mut after = t.clone();
        for np in &p.parts {
            after.partitions.push(Partition {
                node: np.node.clone(),
                start: np.start,
                size: np.size,
                kind: np.kind.to_string(),
                uuid: Some("11111111-2222-3333-4444-555555555555".into()),
                name: Some(np.name.into()),
            });
        }
        assert_eq!(p.check_written(&after), Ok(()));
        let mut moved = after.clone();
        moved.partitions[2].start += 2048;
        assert!(p.check_written(&moved).is_err(), "Windows partition moved");
        let mut extra = after.clone();
        extra.partitions.push(extra.partitions[5].clone());
        assert!(p.check_written(&extra).is_err());
        let mut wrong = after.clone();
        wrong.partitions[5].size -= 1;
        assert!(p.check_written(&wrong).is_err());
        let mut renamed = after.clone();
        renamed.partitions[1].name = Some("x".into());
        assert!(
            p.check_written(&renamed).is_err(),
            "a Windows partition renamed"
        );
        let mut missing = after;
        missing.partitions.pop();
        assert!(p.check_written(&missing).is_err());
    }

    #[test]
    fn modes_parse() {
        assert_eq!("erase".parse(), Ok(Mode::Erase));
        assert_eq!("free-space".parse(), Ok(Mode::FreeSpace));
        assert!("wipe".parse::<Mode>().is_err());
    }

    #[test]
    fn messages() {
        let m = Unavailable::NoRoom {
            free: 12 * GIB + 5,
            needed: 40 * GIB,
        }
        .message(true);
        assert!(m.starts_with("Not enough free space (12 GB, AtlasOS needs 40 GB)"));
        assert!(m.contains("Shrink Windows"));
        assert!(
            !Unavailable::NoRoom {
                free: 0,
                needed: 40 * GIB
            }
            .message(false)
            .contains("Windows")
        );
    }
}
