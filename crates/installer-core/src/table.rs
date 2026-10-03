//! Partition tables as `sfdisk --json` prints them, free space, and the
//! safety check that existing partitions were left alone.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Table {
    /// `gpt` or `dos`.
    pub label: String,
    #[serde(default)]
    pub id: Option<String>,
    pub device: String,
    /// First and last usable sectors (GPT only).
    #[serde(default)]
    pub firstlba: Option<u64>,
    #[serde(default)]
    pub lastlba: Option<u64>,
    #[serde(default = "default_sector_size")]
    pub sectorsize: u64,
    #[serde(default)]
    pub partitions: Vec<Partition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Partition {
    pub node: String,
    pub start: u64,
    pub size: u64,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

fn default_sector_size() -> u64 {
    512
}

#[derive(Deserialize)]
struct Wrapper {
    partitiontable: Table,
}

/// An aligned stretch of unallocated sectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub start: u64,
    pub sectors: u64,
}

impl Table {
    pub fn parse(json: &str) -> Result<Table, String> {
        serde_json::from_str::<Wrapper>(json)
            .map(|w| w.partitiontable)
            .map_err(|e| format!("cannot read sfdisk output: {e}"))
    }

    pub fn is_gpt(&self) -> bool {
        self.label == "gpt"
    }

    /// Sectors per alignment unit (1 MiB).
    pub fn align(&self) -> u64 {
        (crate::ALIGN_BYTES / self.sectorsize.max(1)).max(1)
    }

    /// Unallocated regions between the first and last usable sectors, each
    /// cut to 1 MiB boundaries at both ends. Empty for a non-GPT table.
    pub fn free_regions(&self) -> Vec<Region> {
        let (Some(first), Some(last)) = (self.firstlba, self.lastlba) else {
            return Vec::new();
        };
        if !self.is_gpt() || last < first {
            return Vec::new();
        }
        let align = self.align();
        let mut parts: Vec<(u64, u64)> =
            self.partitions.iter().map(|p| (p.start, p.size)).collect();
        parts.sort();
        let mut gaps = Vec::new();
        let mut cursor = first;
        for (start, size) in parts {
            if start > cursor && cursor <= last {
                gaps.push((cursor, (start - 1).min(last)));
            }
            cursor = cursor.max(start.saturating_add(size));
        }
        if cursor <= last {
            gaps.push((cursor, last));
        }
        gaps.into_iter()
            .filter_map(|(start, end)| {
                let start = start.div_ceil(align) * align;
                let end_excl = (end + 1) / align * align;
                (end_excl > start).then(|| Region {
                    start,
                    sectors: end_excl - start,
                })
            })
            .collect()
    }

    /// The largest free region (the first one if two are equal).
    pub fn largest_free(&self) -> Option<Region> {
        self.free_regions()
            .into_iter()
            .fold(None, |best: Option<Region>, r| match best {
                Some(b) if b.sectors >= r.sectors => Some(b),
                _ => Some(r),
            })
    }

    pub fn bytes(&self, sectors: u64) -> u64 {
        sectors.saturating_mul(self.sectorsize)
    }

    pub fn partition(&self, node: &str) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.node == node)
    }

    /// The lowest partition numbers not in use, `n` of them.
    pub fn free_numbers(&self, n: usize) -> Vec<u32> {
        let used: Vec<u32> = self
            .partitions
            .iter()
            .filter_map(|p| partition_number(&self.device, &p.node))
            .collect();
        (1..).filter(|i| !used.contains(i)).take(n).collect()
    }
}

/// Partition `n` of `disk`: `/dev/sda` + 3 is `/dev/sda3`, and a disk whose
/// name ends in a digit gets a `p` (`/dev/nvme0n1p3`), as the kernel names them.
pub fn partition_node(disk: &str, n: u32) -> String {
    if disk.ends_with(|c: char| c.is_ascii_digit()) {
        format!("{disk}p{n}")
    } else {
        format!("{disk}{n}")
    }
}

/// The number of partition `node` on `disk`, if it is one of its partitions.
pub fn partition_number(disk: &str, node: &str) -> Option<u32> {
    let rest = node.strip_prefix(disk)?;
    let rest = if disk.ends_with(|c: char| c.is_ascii_digit()) {
        rest.strip_prefix('p')?
    } else {
        rest
    };
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

fn same_guid(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    }
}

/// The safety check: every partition in `before` is still in `after` with
/// the same node, start, size, type, UUID and name, and the table itself is
/// the same table (label, id and usable range).
pub fn check_unchanged(before: &Table, after: &Table) -> Result<(), String> {
    if before.label != after.label || !same_guid(&before.id, &after.id) {
        return Err("the partition table itself changed".into());
    }
    if before.sectorsize != after.sectorsize {
        return Err("the disk's sector size changed".into());
    }
    if before.firstlba != after.firstlba || before.lastlba != after.lastlba {
        return Err("the partition table's usable range changed".into());
    }
    for p in &before.partitions {
        let Some(q) = after.partition(&p.node) else {
            return Err(format!("{} is gone", p.node));
        };
        if p.start != q.start || p.size != q.size {
            return Err(format!("{} moved or changed size", p.node));
        }
        if !p.kind.eq_ignore_ascii_case(&q.kind) || !same_guid(&p.uuid, &q.uuid) {
            return Err(format!("{} changed type or UUID", p.node));
        }
        if p.name != q.name {
            return Err(format!("{} changed name", p.node));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS: &str = include_str!("../tests/fixtures/sfdisk-windows.json");

    #[test]
    fn parses_sfdisk_json() {
        let t = Table::parse(WINDOWS).unwrap();
        assert!(t.is_gpt());
        assert_eq!(t.partitions.len(), 4);
        assert_eq!(t.partitions[0].kind, crate::gpt::ESP);
        assert_eq!(t.align(), 2048);
    }

    #[test]
    fn largest_free_region_is_aligned_at_both_ends() {
        let t = Table::parse(WINDOWS).unwrap();
        let r = t.largest_free().unwrap();
        // after the recovery partition, up to the last usable sector
        assert_eq!(r.start, 126066688);
        assert_eq!(r.start % 2048, 0);
        assert_eq!(r.sectors % 2048, 0);
        assert!(r.start + r.sectors - 1 <= t.lastlba.unwrap());
        assert_eq!(t.bytes(r.sectors) / crate::MIB, 69515);
    }

    #[test]
    fn gaps_between_partitions_count_and_tiny_ones_vanish() {
        let mut t = Table::parse(WINDOWS).unwrap();
        // a 1000-sector gap after the MSR: smaller than 1 MiB once aligned
        t.partitions[2].start += 1000;
        t.partitions[2].size -= 1000;
        let regions = t.free_regions();
        assert_eq!(regions.len(), 1);
        // a 100 MiB gap is found, and the largest is still the tail
        t.partitions[2].start += 204800;
        t.partitions[2].size -= 204800;
        let regions = t.free_regions();
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].sectors, 204800);
        assert_eq!(t.largest_free().unwrap().start, 126066688);
    }

    #[test]
    fn mbr_has_no_usable_free_space() {
        let mut t = Table::parse(WINDOWS).unwrap();
        t.label = "dos".into();
        assert!(t.free_regions().is_empty());
    }

    #[test]
    fn empty_gpt_is_one_region() {
        let mut t = Table::parse(WINDOWS).unwrap();
        t.partitions.clear();
        assert_eq!(
            t.free_regions(),
            [Region {
                start: 2048,
                sectors: 268431360
            }]
        );
    }

    #[test]
    fn four_k_sectors_align_to_256() {
        let mut t = Table::parse(WINDOWS).unwrap();
        t.sectorsize = 4096;
        assert_eq!(t.align(), 256);
    }

    #[test]
    fn partition_names() {
        assert_eq!(partition_node("/dev/sda", 5), "/dev/sda5");
        assert_eq!(partition_node("/dev/nvme0n1", 5), "/dev/nvme0n1p5");
        assert_eq!(partition_node("/dev/mmcblk0", 1), "/dev/mmcblk0p1");
        assert_eq!(partition_number("/dev/sda", "/dev/sda12"), Some(12));
        assert_eq!(partition_number("/dev/nvme0n1", "/dev/nvme0n1p3"), Some(3));
        assert_eq!(partition_number("/dev/nvme0n1", "/dev/nvme0n13"), None);
        assert_eq!(partition_number("/dev/sda", "/dev/sdb1"), None);
        assert_eq!(partition_number("/dev/sda", "/dev/sda"), None);
    }

    #[test]
    fn free_numbers_fill_holes_first() {
        let mut t = Table::parse(WINDOWS).unwrap();
        t.device = "/dev/sda".into();
        for (i, p) in t.partitions.iter_mut().enumerate() {
            p.node = format!("/dev/sda{}", [1, 2, 3, 6][i]);
        }
        assert_eq!(t.free_numbers(3), [4, 5, 7]);
    }

    #[test]
    fn safety_check_catches_every_kind_of_change() {
        let before = Table::parse(WINDOWS).unwrap();
        let mut after = before.clone();
        after.partitions.push(Partition {
            node: "/dev/sda5".into(),
            start: 126066688,
            size: 4194304,
            kind: crate::gpt::LINUX_FS.into(),
            uuid: None,
            name: None,
        });
        assert_eq!(check_unchanged(&before, &after), Ok(()), "adding is fine");

        let change = |f: &dyn Fn(&mut Table)| {
            let mut a = after.clone();
            f(&mut a);
            check_unchanged(&before, &a)
        };
        assert!(change(&|a| a.partitions[2].start += 1).is_err());
        assert!(change(&|a| a.partitions[2].size -= 2048).is_err());
        assert!(change(&|a| a.partitions[1].kind = crate::gpt::LINUX_FS.into()).is_err());
        assert!(
            change(&|a| a.partitions[0].uuid = Some("00000000-0000-0000-0000-000000000000".into()))
                .is_err()
        );
        assert!(
            change(&|a| {
                a.partitions.remove(3);
            })
            .is_err()
        );
        assert!(change(&|a| a.id = Some("x".into())).is_err());
        assert!(change(&|a| a.label = "dos".into()).is_err());
        // lower-case GUIDs are the same GUIDs
        assert!(change(&|a| a.partitions[0].kind = a.partitions[0].kind.to_lowercase()).is_ok());
    }

    #[test]
    fn gaps_stop_at_the_last_usable_sector() {
        let mut t = Table::parse(WINDOWS).unwrap();
        let last = t.lastlba.unwrap();
        // a stray partition past the end: nothing beyond `last` is free
        t.partitions.push(Partition {
            node: "/dev/sda9".into(),
            start: last + 10_000,
            size: 2048,
            kind: "0FC63DAF-8483-4772-8E79-3D69D8477DE4".into(),
            uuid: None,
            name: None,
        });
        for r in t.free_regions() {
            assert!(r.start + r.sectors - 1 <= last, "{r:?}");
        }
    }
}
