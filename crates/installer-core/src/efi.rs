//! Firmware boot entries. bootupd names AtlasOS's entry "Fedora"; the
//! installer replaces it with one called "AtlasOS" pointing at the same file
//! (and replaces an "AtlasOS" entry left by an earlier install there too).

/// One `BootXXXX` line of `efibootmgr` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootEntry {
    /// Four hex digits, e.g. `0005`.
    pub num: String,
    pub label: String,
    /// The device path, e.g. `HD(1,GPT,<partuuid>,...)/\EFI\fedora\shimx64.efi`.
    pub path: String,
}

pub const LABEL: &str = "AtlasOS";
const BOOTUPD_LABEL: &str = "Fedora";
/// The loader bootupd registers, as efibootmgr's `--loader` takes it.
pub const SHIM: &str = r"\EFI\fedora\shimx64.efi";

/// The `BootOrder:` line of `efibootmgr`, as entry numbers.
pub fn boot_order(out: &str) -> Vec<String> {
    out.lines()
        .find_map(|line| line.strip_prefix("BootOrder:"))
        .map(|order| {
            order
                .split(',')
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Parse `efibootmgr`: `Boot0005* Fedora\tHD(...)`. efibootmgr 18 prints
/// the device path without `-v` (checked in the VM); older versions need
/// `-v`, and without a path no entry is ever found stale.
pub fn parse(out: &str) -> Vec<BootEntry> {
    out.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("Boot")?;
            let num = rest.get(..4)?;
            if !num.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let rest = rest[4..].strip_prefix('*').unwrap_or(&rest[4..]);
            let rest = rest.strip_prefix(' ')?;
            let (label, path) = rest.split_once('\t').unwrap_or((rest, ""));
            Some(BootEntry {
                num: num.into(),
                label: label.trim_end().into(),
                path: path.trim().into(),
            })
        })
        .collect()
}

/// The "Fedora" (bootupd's) and "AtlasOS" entries for shim on the EFI
/// partition `partuuid`. Entries on other partitions (another Fedora) are
/// left alone, and an EFI partition that already held `EFI/fedora` is never
/// shared, so these are always this install's own.
pub fn stale_entries<'a>(entries: &'a [BootEntry], partuuid: &str) -> Vec<&'a BootEntry> {
    let uuid = partuuid.to_ascii_lowercase();
    entries
        .iter()
        .filter(|e| e.label == BOOTUPD_LABEL || e.label == LABEL)
        .filter(|e| {
            let p = e.path.to_ascii_lowercase();
            p.contains(&format!(",{uuid},")) && p.contains(&SHIM.to_ascii_lowercase())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_order_lists_the_numbers() {
        assert_eq!(
            boot_order(OUT),
            ["0005", "0002", "0004", "0003", "0000", "0001"]
        );
        assert!(boot_order("BootCurrent: 0002\n").is_empty());
    }

    // From spike S3: Windows and AtlasOS sharing an ESP.
    const OUT: &str = "BootCurrent: 0002\nTimeout: 0 seconds\nBootOrder: 0005,0002,0004,0003,0000,0001\n\
        Boot0000* BootManagerMenuApp\tFvVol(7cb8bdc9-f8eb-4f34-aaea-3ee4af6516a1)/FvFile(eec25bdc-67f2-4d95-b1d5-f81b2039d11d)\n\
        Boot0002* UEFI QEMU DVD-ROM QM00003 \tPciRoot(0x0)/Pci(0x1f,0x2)/Sata(1,65535,0){auto_created_boot_option}\n\
        Boot0004* Windows Boot Manager\tHD(1,GPT,37df3229-55d9-43ef-902c-a5a6565f521a,0x800,0x32000)/\\EFI\\Microsoft\\Boot\\bootmgfw.efi5749\n\
        Boot0005* Fedora\tHD(1,GPT,37df3229-55d9-43ef-902c-a5a6565f521a,0x800,0x32000)/\\EFI\\fedora\\shimx64.efi\n\
        Boot0006  Fedora\tHD(1,GPT,99999999-55d9-43ef-902c-a5a6565f521a,0x800,0x32000)/\\EFI\\fedora\\shimx64.efi\n";

    #[test]
    fn parses_entries() {
        let e = parse(OUT);
        assert_eq!(e.len(), 5);
        assert_eq!(e[2].label, "Windows Boot Manager");
        assert_eq!(e[1].label, "UEFI QEMU DVD-ROM QM00003");
        assert_eq!(e[3].num, "0005");
        assert_eq!(e[4].num, "0006", "inactive entries count too");
    }

    #[test]
    fn only_our_partitions_fedora_entry() {
        let e = parse(OUT);
        let ours = stale_entries(&e, "37DF3229-55D9-43EF-902C-A5A6565F521A");
        assert_eq!(
            ours.iter().map(|x| x.num.as_str()).collect::<Vec<_>>(),
            ["0005"]
        );
        assert!(stale_entries(&e, "00000000-0000-0000-0000-000000000000").is_empty());
        // an earlier install's AtlasOS entry on the same partition goes too
        let again = parse(&OUT.replace("Boot0002* UEFI QEMU DVD-ROM QM00003 \tPciRoot", "Boot0002* AtlasOS\tHD(1,GPT,37df3229-55d9-43ef-902c-a5a6565f521a,0x800,0x32000)/\\EFI\\fedora\\shimx64.efi\nBoot0007* x\tPciRoot"));
        let ours = stale_entries(&again, "37df3229-55d9-43ef-902c-a5a6565f521a");
        assert_eq!(
            ours.iter().map(|x| x.num.as_str()).collect::<Vec<_>>(),
            ["0002", "0005"]
        );
    }
}
