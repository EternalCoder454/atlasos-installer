//! The pure logic behind Telamon Installer: which disks to offer, how to
//! partition them, the safety check, the files written into the installed
//! system, and progress. No I/O happens here; `telamon-installer-helper` runs
//! the commands and hands their output in, and the unit tests use fixtures.

pub mod apps;
pub mod crypt;
pub mod disks;
pub mod efi;
pub mod grub;
pub mod keyboard;
pub mod locales;
pub mod lsblk;
pub mod plan;
pub mod progress;
pub mod settings;
pub mod sigpolicy;
pub mod table;

#[cfg(test)]
mod props;

pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;

/// Smallest disk (erase) or free region (alongside) Telamon OS installs to.
pub const MIN_INSTALL_BYTES: u64 = 40 * GIB;
/// Partition sizes, Fedora's defaults.
pub const ESP_BYTES: u64 = 600 * MIB;
pub const BOOT_BYTES: u64 = 2 * GIB;
/// An existing EFI partition is shared when the partition is at least this
/// big and its filesystem has this much free. The partition size counts, not
/// the filesystem's: Windows' 100 MiB ESP holds a 96 MiB FAT.
pub const ESP_REUSE_MIN_PART: u64 = 100 * MIB;
pub const ESP_REUSE_MIN_FREE: u64 = 40 * MIB;
/// Partitions start and end on 1 MiB boundaries.
pub const ALIGN_BYTES: u64 = MIB;

/// Volume labels of the installer ISOs. A disk holding one is never offered.
pub const ISO_LABELS: [&str; 2] = ["ATLASOS", "ATLASOS-NV"];

/// GPT partition type GUIDs (upper case, as sfdisk prints them).
pub mod gpt {
    pub const ESP: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
    pub const MS_RESERVED: &str = "E3C9E316-0B5C-4DB8-817D-F92DF00215AE";
    pub const MS_BASIC_DATA: &str = "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7";
    pub const LINUX_FS: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";
    pub const ROOT_X86_64: &str = "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709";

    /// Case-insensitive GUID compare (lsblk prints lower case).
    pub fn is(guid: Option<&str>, want: &str) -> bool {
        guid.is_some_and(|g| g.eq_ignore_ascii_case(want))
    }
}
