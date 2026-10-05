//! What `ListDisks` and `Install` do: probe the disks, plan, partition,
//! format, run bootc, write the settings, fix the boot entry, queue the
//! NVIDIA key. The decisions are in installer-core; this runs them.

use std::collections::HashMap;
use std::fs;
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use installer_core::crypt::{self, Encryption};
use installer_core::disks::{self, Probe};
use installer_core::lsblk::{self, Device, Lsblk};
use installer_core::plan::{EspInfo, Mode, Plan, Role};
use installer_core::progress::{BootcProgress, Progress, Stage};
use installer_core::settings::{self, Keymap};
use installer_core::table::{Table, partition_number};
use installer_core::{efi, gpt, grub};
use serde::Serialize;

use crate::run::{self, Cmd, Output, Runner, bin, lock, run};

const SECURE_BOOT_VAR: &str =
    "sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";
/// The AtlasOS module signing key; only the NVIDIA image has it.
const NVIDIA_KEY: &str = "usr/share/atlasos/nvidia/atlasos-module-signing.der";
const KBD_MODEL_MAP: &str = "usr/share/systemd/kbd-model-map";
const NM_DIRS: [&str; 2] = [
    "etc/NetworkManager/system-connections",
    "run/NetworkManager/system-connections",
];
/// The installed system tracks this tag, which Atlas Updater reads as its channel.
const IMAGE_TAG: &str = "stable";

const MINUTE: std::time::Duration = std::time::Duration::from_secs(60);

/// Where the TPM shows up: the kernel's TPM class (one directory per chip
/// with its spec version) and the resource-managed device systemd uses.
const TPM_CLASS: &str = "sys/class/tpm";
const TPM_DEVICE: &str = "dev/tpmrm0";
/// The disk's temporary key lives in this directory of the run dir (0700).
const KEY_DIR: &str = "luks";
const KEY_FILE: &str = "luks-key";
const KEY_BYTES: usize = 64;
const TPM_FAILED: &str = "This PC's security chip (TPM) didn't accept the disk key. Go back and turn encryption off, or try again.";

/// Where things are. Host files are read under `root`, so tests can use a
/// temporary directory; the commands get real paths.
#[derive(Debug, Clone)]
pub struct Env {
    pub root: PathBuf,
    /// The new system is mounted here.
    pub target: PathBuf,
    /// The install log, the ESP probe mount point and the MOK hash.
    pub run_dir: PathBuf,
}

impl Env {
    pub fn system() -> Env {
        Env {
            root: "/".into(),
            target: "/run/atlas-target".into(),
            run_dir: "/run/atlas-installer".into(),
        }
    }

    fn host(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn log_path(&self) -> PathBuf {
        self.run_dir.join("install.log")
    }
}

fn path_str(p: &Path) -> Result<&str, String> {
    p.to_str()
        .ok_or_else(|| format!("{} is not UTF-8", p.display()))
}

/// What `Install` was asked to do, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub disk_id: String,
    /// The disk's fingerprint from ListDisks; `None` only for `--plan`.
    pub fingerprint: Option<String>,
    pub mode: Mode,
    pub locale: String,
    pub keymap: Keymap,
    pub wifi_uuid: Option<String>,
    pub encryption: Encryption,
    /// The disk password (`Password`) or PIN (`TpmPin`); empty otherwise.
    pub password: Secret,
}

/// A secret that `Debug` doesn't show; it serializes as the real value, for
/// the one caller allowed to see it.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret(String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<hidden>")
    }
}

impl Serialize for Secret {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Secret {
        Secret(s)
    }
}

impl Drop for Secret {
    /// Overwrite the buffer before it is freed. Volatile writes and a fence
    /// keep the compiler from dropping them as dead stores.
    fn drop(&mut self) {
        crate::run::wipe(std::mem::take(&mut self.0));
    }
}

impl Secret {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Request {
    /// Check every argument before anything else happens. `wifi_uuid` may
    /// be empty (no Wi-Fi to carry over). `encryption` is `none`, `tpm`,
    /// `tpm-pin` or `password`; the last two take the `password` (the PIN).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        disk_id: &str,
        fingerprint: &str,
        mode: &str,
        locale: &str,
        keymap: &str,
        wifi_uuid: &str,
        encryption: &str,
        password: &str,
    ) -> Result<Request, String> {
        let id_ok = (1..=32).contains(&disk_id.len())
            && disk_id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        if !id_ok {
            return Err(format!("not a disk name: {disk_id:?}"));
        }
        let fingerprint = match fingerprint {
            "" => None,
            f if f.len() == 16
                && f.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) =>
            {
                Some(f.to_string())
            }
            f => return Err(format!("not a disk fingerprint: {f:?}")),
        };
        settings::validate_locale(locale)?;
        let wifi_uuid = match wifi_uuid {
            "" => None,
            u => {
                settings::validate_uuid(u)?;
                Some(u.to_string())
            }
        };
        let encryption: Encryption = encryption.parse()?;
        match encryption {
            Encryption::Password => crypt::validate_password(password)?,
            Encryption::TpmPin => crypt::validate_pin(password)?,
            _ if !password.is_empty() => {
                return Err("a disk password was given without password or PIN encryption".into());
            }
            _ => {}
        }
        Ok(Request {
            disk_id: disk_id.into(),
            fingerprint,
            mode: mode.parse()?,
            locale: locale.into(),
            keymap: Keymap::parse(keymap)?,
            wifi_uuid,
            encryption,
            password: Secret(password.into()),
        })
    }
}

/// `Install`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    /// The one-time password for MOK Manager, when the NVIDIA key was queued.
    pub mok_password: Option<Secret>,
    /// The disk's recovery key, when it was encrypted. Shown once, like the
    /// MOK password, and never logged.
    pub recovery_key: Option<Secret>,
    /// How the disk was encrypted (`none`, `tpm`, `tpm-pin`, `password`): a
    /// UI that gets no `recovery_key` (Status to a caller not allowed to
    /// install) knows one exists that it can't show, and how to make another.
    pub encryption: String,
    /// Windows was added to the boot menu.
    pub windows_entry: bool,
    /// What the installer was started from: "cd", "usb" or "", for
    /// "Remove the USB stick" (see [`disks::boot_media`]).
    pub boot_media: String,
    /// Things that went wrong without failing the install.
    pub warnings: Vec<String>,
    pub log: String,
}

/// `ListDisks`'s answer: the disks, and whether the PC has a usable TPM 2.0.
#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    #[serde(flatten)]
    pub disks: disks::DiskList,
    pub tpm2: bool,
    /// Secure Boot is on (a TPM-only disk is then bound to its state).
    pub secure_boot: bool,
    /// The boot loader that started the live system, when it wasn't the
    /// ISO's own: `ventoy` or `iso-file`. The TPM can't be used then.
    pub chain_loaded: Option<&'static str>,
}

/// A TPM 2.0 the system can use: the kernel lists a chip speaking spec
/// version 2 and its resource-managed device exists.
pub fn tpm2_present(env: &Env) -> bool {
    let Ok(chips) = fs::read_dir(env.host(TPM_CLASS)) else {
        return false;
    };
    let v2 = chips.flatten().any(|c| {
        let name = c.file_name();
        let name = name.to_string_lossy();
        name.strip_prefix("tpm")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            && fs::read_to_string(c.path().join("tpm_version_major")).is_ok_and(|v| v.trim() == "2")
    });
    v2 && env.host(TPM_DEVICE).exists()
}

/// Read the disks: lsblk, the live boot media, partition tables and EFI
/// partitions (mounted read-only) of the disks that may be offered.
pub fn probe(r: &dyn Runner, env: &Env) -> Result<Probe, String> {
    let out = run(r, Cmd::new(bin::LSBLK, lsblk::ARGS))?;
    let lsblk = Lsblk::parse(&out.stdout)?;
    let mut probe = Probe {
        live_sources: live_sources(env),
        ..Default::default()
    };
    for d in &lsblk.blockdevices {
        if disks::hidden_reason(d, &probe.live_sources).is_some() {
            continue;
        }
        if d.pttype.is_some()
            && let Ok(t) = run(r, Cmd::new(bin::SFDISK, ["--json", d.name.as_str()]))
                .and_then(|o| Table::parse(&o.stdout))
        {
            probe.tables.insert(d.name.clone(), t);
        }
        for p in d.walk() {
            if p.kind == "part"
                && gpt::is(p.parttype.as_deref(), gpt::ESP)
                && p.fstype() == Some("vfat")
                && let Some(info) = esp_info(r, env, p)?
            {
                probe.esps.insert(p.name.clone(), info);
            }
        }
    }
    probe.lsblk = Some(lsblk);
    Ok(probe)
}

/// The boot media's devices, as mounted and with symlinks resolved.
fn live_sources(env: &Env) -> Vec<String> {
    let mounts = fs::read_to_string(env.host("proc/self/mounts")).unwrap_or_default();
    let mut out = disks::live_sources(&mounts);
    for src in out.clone() {
        if let Ok(real) = fs::canonicalize(env.host(src.trim_start_matches('/')))
            && let Ok(rel) = real.strip_prefix(&env.root)
        {
            let real = format!("/{}", rel.display());
            if !out.contains(&real) {
                out.push(real);
            }
        }
    }
    out
}

/// Free space, Windows Boot Manager and another Fedora on an EFI partition.
/// `Ok(None)` if it can't be read; `Err` if it was mounted and can't be
/// released (then nothing may be installed).
fn esp_info(r: &dyn Runner, env: &Env, p: &Device) -> Result<Option<EspInfo>, String> {
    let (dir, ours) = match p.mounts().next() {
        Some(m) if m.starts_with('/') => (env.root.join(m.trim_start_matches('/')), false),
        _ => {
            let dir = env.run_dir.join("esp-probe");
            if fs::create_dir_all(&dir).is_err() {
                return Ok(None);
            }
            let d = path_str(&dir)?;
            let mounted = run(
                r,
                Cmd::new(
                    bin::MOUNT,
                    [
                        "-o",
                        "ro,nosuid,nodev,noexec",
                        "-t",
                        "vfat",
                        p.name.as_str(),
                        d,
                    ],
                ),
            );
            if mounted.is_err() {
                return Ok(None);
            }
            (dir, true)
        }
    };
    let fs_free = rustix::fs::statvfs(&dir)
        .map(|s| s.f_bavail.saturating_mul(s.f_frsize))
        .unwrap_or(0);
    let windows = dir.join("EFI/Microsoft/Boot/bootmgfw.efi").is_file();
    // FAT is case-insensitive, but what Linux tools write is lower case
    let fedora = dir.join("EFI/fedora").is_dir();
    if ours {
        let d = path_str(&dir)?;
        if let Err(e) = run(r, Cmd::new(bin::UMOUNT, [d]).cleanup()) {
            let _ = run(r, Cmd::new(bin::UMOUNT, ["--lazy", d]).cleanup());
            return Err(format!("cannot release {} after reading it: {e}", p.name));
        }
    }
    Ok(Some(EspInfo {
        fs_free,
        windows,
        fedora,
        fs_uuid: p.uuid.clone(),
    }))
}

/// `ListDisks`.
pub fn list_disks(r: &dyn Runner, env: &Env) -> Result<Listing, String> {
    let probe = probe(r, env)?;
    Ok(Listing {
        disks: disks::list(&probe),
        tpm2: tpm2_present(env),
        secure_boot: secure_boot_on(env),
        chain_loaded: chain_loaded(env, &probe),
    })
}

fn chain_loaded(env: &Env, probe: &disks::Probe) -> Option<&'static str> {
    let mounts = fs::read_to_string(env.host("proc/self/mounts")).unwrap_or_default();
    disks::chain_loaded(probe, &mounts)
}

/// Everything decided before the first write.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub plan: Plan,
    /// What is installed and tracked, e.g. `ghcr.io/eternalcoder454/atlasos:stable`.
    pub image: String,
    /// FAT volume IDs of EFI partitions with Windows that survive the install.
    pub windows_esps: Vec<String>,
    /// Queue the NVIDIA module key for MOK enrollment.
    pub mok: bool,
    /// The Wi-Fi keyfile to copy: file name and contents.
    pub wifi: Option<(String, String)>,
    pub console_keymap: String,
    /// The new LUKS volume's UUID, when the root partition is encrypted.
    pub luks_uuid: Option<String>,
    /// Kernel arguments added to bootc's, for the encrypted root.
    pub kargs: Vec<String>,
    /// See [`Outcome::boot_media`].
    pub boot_media: &'static str,
    /// The disk's fingerprint as `prepare` saw it; checked again just before
    /// the first write.
    pub fingerprint: String,
}

fn secure_boot_on(env: &Env) -> bool {
    fs::read(env.host(SECURE_BOOT_VAR)).is_ok_and(|v| v.last() == Some(&1))
}

fn find_wifi(env: &Env, uuid: &str) -> Result<(String, String), String> {
    for dir in NM_DIRS {
        let Ok(entries) = fs::read_dir(env.host(dir)) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let small = e
                .metadata()
                .is_ok_and(|m| m.is_file() && m.len() < 64 * 1024);
            if !name.ends_with(".nmconnection") || name.starts_with('.') || !small {
                continue;
            }
            if let Some(copy) = fs::read_to_string(e.path())
                .ok()
                .and_then(|t| settings::keyfile_for_install(&t, uuid))
            {
                return Ok((name, copy));
            }
        }
    }
    Err("The Wi-Fi connection to carry over was not found.".into())
}

/// Probe again and decide everything, without writing to any disk. Refuses
/// the boot media and any disk not offered, whatever the caller sent.
pub fn prepare(r: &dyn Runner, env: &Env, req: &Request) -> Result<Prepared, String> {
    let probe = probe(r, env)?;
    if probe.live_sources.is_empty() {
        return Err(
            "The installer's boot media was not found, so it can't tell which disk to leave alone. Start the computer from the AtlasOS installer."
                .into(),
        );
    }
    let disk = disks::find_visible(&probe, &req.disk_id)?;
    let fingerprint = disks::fingerprint(disk, probe.tables.get(&disk.name));
    if req.fingerprint.as_ref().is_some_and(|f| *f != fingerprint) {
        return Err(format!(
            "{} changed since the disks were listed (it was replugged, or its partitions changed). Nothing was written. Choose the disk again.",
            disk.name
        ));
    }
    let plan = disks::plan_for(disk, &probe, req.mode)
        .map_err(|u| format!("Can't install to {}: {}", disk.name, u.message(false)))?;
    if req.encryption.uses_tpm() && !tpm2_present(env) {
        return Err(
            "This PC has no usable security chip (TPM 2.0), so the disk can't be unlocked by it. Choose a password instead, or turn encryption off."
                .into(),
        );
    }
    if req.encryption.uses_tpm()
        && let Some(loader) = chain_loaded(env, &probe)
    {
        let how = if loader == "ventoy" {
            "through Ventoy"
        } else {
            "from an ISO file, through another boot menu"
        };
        return Err(format!(
            "The installer was started {how}, so the disk can't be tied to this PC's security chip: it would ask for the recovery key at every start. Choose a password instead, or start the installer from a USB stick the AtlasOS ISO was written to."
        ));
    }

    if req.encryption != Encryption::None {
        for (tool, what) in [
            (bin::CRYPTSETUP, "cryptsetup"),
            (bin::CRYPTENROLL, "systemd-cryptenroll"),
            (bin::GRUB2_MKPASSWD, "grub2-mkpasswd-pbkdf2"),
        ] {
            let usable = fs::metadata(env.host(tool.trim_start_matches('/')))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
            if !usable {
                return Err(format!(
                    "Disk encryption needs {what}, which this installer doesn't have. Turn encryption off. Nothing was written."
                ));
            }
        }
    }

    let mut windows_esps: Vec<String> = probe
        .esps
        .iter()
        .filter(|(node, info)| {
            info.windows
                && !(plan.mode == Mode::Erase && partition_number(&plan.disk, node).is_some())
        })
        .filter_map(|(_, info)| info.fs_uuid.clone())
        .filter(|u| grub::validate_fat_uuid(u).is_ok())
        .collect();
    windows_esps.sort();
    windows_esps.dedup();

    let nvidia = env.host(NVIDIA_KEY).is_file();
    let image = format!(
        "ghcr.io/eternalcoder454/{}:{IMAGE_TAG}",
        if nvidia { "atlasos-nvidia" } else { "atlasos" }
    );
    let mok = nvidia && secure_boot_on(env) && {
        let key = env.host(NVIDIA_KEY);
        let out = r.run(
            &Cmd::new(bin::MOKUTIL, ["--test-key", path_str(&key)?]),
            &mut |_| {},
        )?;
        !format!("{}{}", out.stdout, out.stderr).contains("is already enrolled")
    };
    let wifi = match &req.wifi_uuid {
        Some(u) => Some(find_wifi(env, u)?),
        None => None,
    };
    let map = fs::read_to_string(env.host(KBD_MODEL_MAP)).unwrap_or_default();
    let console_keymap = settings::console_keymap(&map, &req.keymap);
    let luks_uuid = if req.encryption.on() {
        Some(new_uuid()?)
    } else {
        None
    };
    let kargs = match &luks_uuid {
        Some(u) => crypt::kargs(req.encryption, u, &console_keymap)?,
        None => Vec::new(),
    };
    Ok(Prepared {
        console_keymap,
        luks_uuid,
        kargs,
        plan,
        image,
        windows_esps,
        mok,
        wifi,
        boot_media: disks::boot_media(&probe),
        fingerprint,
    })
}

/// Look at the disk once more, right before the first command that writes
/// to it: unmounting and closing leftovers can take a while, and the disk
/// may have been swapped meanwhile.
fn recheck_disk(r: &dyn Runner, p: &Prepared) -> Result<(), String> {
    let disk = p.plan.disk.as_str();
    let changed = || {
        format!(
            "{disk} changed since it was checked (it was replugged, or its partitions changed). Nothing was written. Choose the disk again."
        )
    };
    let lsblk = Lsblk::parse(&run(r, Cmd::new(bin::LSBLK, lsblk::ARGS))?.stdout)?;
    let d = lsblk
        .blockdevices
        .iter()
        .find(|d| d.name == disk)
        .ok_or_else(changed)?;
    // As in `probe`: a table sfdisk can't read counts as none, so a disk
    // that was listed that way (a damaged table the user wants erased)
    // still matches. A table that was read then and not now doesn't, and
    // is named for what it is rather than called a replug.
    let (table, unread) = match d.pttype {
        Some(_) => match run(r, Cmd::new(bin::SFDISK, ["--json", disk]))
            .and_then(|o| Table::parse(&o.stdout))
        {
            Ok(t) => (Some(t), None),
            Err(e) => (None, Some(e)),
        },
        None => (None, None),
    };
    if disks::fingerprint(d, table.as_ref()) != p.fingerprint {
        return Err(match unread {
            Some(e) => {
                format!("Could not read the partition table on {disk} ({e}). Nothing was written.")
            }
            None => changed(),
        });
    }
    Ok(())
}

/// mkfs discards the whole partition first, which takes a while on some disks.
fn mkfs(role: Role, node: &str) -> Cmd {
    match role {
        Role::Esp => Cmd::new(bin::MKFS_VFAT, ["-F", "32", "-n", "EFI", node]),
        Role::Boot => Cmd::new(bin::MKFS_EXT4, ["-q", "-F", "-L", "boot", node]),
        Role::Root => Cmd::new(bin::MKFS_BTRFS, ["-q", "-f", "-L", "atlasos", node]),
    }
    .timeout(15 * MINUTE)
}

/// The image's size in containers storage, uncompressed: what the image
/// proxy hands bootc during the import. `None` if podman can't say; the
/// copy then moves the bar by time.
fn image_bytes(r: &dyn Runner, image: &str) -> Option<u64> {
    let cmd = Cmd::new(
        bin::PODMAN,
        ["image", "inspect", "--format", "{{.Size}}", image],
    )
    .timeout(MINUTE);
    let out = run(r, cmd).ok()?;
    out.stdout.trim().parse().ok().filter(|n| *n > 0)
}

/// How often the copy is measured.
const COPY_SAMPLE: Duration = Duration::from_secs(2);
/// How often the log gets a line about the copy.
const COPY_NOTE: Duration = Duration::from_secs(30);
/// Where ostree keeps one ref per imported layer, in the new system.
const BLOB_REFS: &str = "ostree/repo/refs/heads/ostree/container/blob";
/// How many times the image proxy writes each byte it hands bootc.
const SPOOLED: u64 = 2;

/// Counts what the image proxy (`skopeo experimental-image-proxy`, which
/// bootc starts) has handed bootc: the layers, uncompressed, as they import.
/// bootc has no progress output of its own for this.
///
/// The proxy writes each layer twice: from containers-storage into a
/// temporary file, then from that file to bootc (containers/image's
/// `storageImageSource.GetBlob`, to hold the storage lock briefly). So the
/// bytes it has written are [`SPOOLED`] times what bootc has had.
///
/// Only processes in the process group of a bootc that is the helper's own
/// child count (the runner starts bootc as a group leader). The live
/// session's user can name a process anything, but can't join that group,
/// which is in the helper's session.
struct CopyMeter {
    proc_dir: PathBuf,
    /// The helper's pid: bootc's parent.
    parent: u32,
    /// Bytes written by each proxy seen, by pid and start time (pids are
    /// reused): one that has exited keeps its last count.
    seen: HashMap<(u32, u64), u64>,
}

/// The fields of /proc/<pid>/stat after "pid (comm)": comm may hold spaces
/// and ")", so they start after the last ")".
fn stat_fields(stat: &str) -> Option<Vec<&str>> {
    Some(stat.rsplit_once(')')?.1.split_whitespace().collect())
}

impl CopyMeter {
    fn new(proc_dir: PathBuf, parent: u32) -> CopyMeter {
        CopyMeter {
            proc_dir,
            parent,
            seen: HashMap::new(),
        }
    }

    /// Bytes handed to bootc so far, by all of its proxies.
    fn sample(&mut self) -> u64 {
        if let Ok(dir) = fs::read_dir(&self.proc_dir) {
            for e in dir.flatten() {
                let Some(pid) = e.file_name().to_str().and_then(|n| n.parse().ok()) else {
                    continue;
                };
                if let Some((start, n)) = self.proxy_written(pid) {
                    let v = self.seen.entry((pid, start)).or_insert(0);
                    *v = (*v).max(n);
                }
            }
        }
        self.seen.values().fold(0u64, |a, n| a.saturating_add(*n)) / SPOOLED
    }

    fn read(&self, pid: u32, file: &str) -> Option<String> {
        fs::read_to_string(self.proc_dir.join(pid.to_string()).join(file)).ok()
    }

    /// The start time and `wchar` of `pid`, if it is one of bootc's image
    /// proxies.
    fn proxy_written(&self, pid: u32) -> Option<(u64, u64)> {
        if self.read(pid, "comm")?.trim_end() != "skopeo" {
            return None;
        }
        // state, ppid, pgrp, ..., starttime (field 22 of stat)
        let stat = self.read(pid, "stat")?;
        let f = stat_fields(&stat)?;
        let pgrp: u32 = f.get(2)?.parse().ok()?;
        let start: u64 = f.get(19)?.parse().ok()?;
        let leader = self.read(pgrp, "stat")?;
        let lf = stat_fields(&leader)?;
        if lf.get(1)?.parse::<u32>().ok()? != self.parent
            || lf.get(2)?.parse::<u32>().ok()? != pgrp
            || self.read(pgrp, "comm")?.trim_end() != "bootc"
        {
            return None;
        }
        // only now the command line, and no more of it than needed
        let mut cmdline = Vec::new();
        fs::File::open(self.proc_dir.join(pid.to_string()).join("cmdline"))
            .ok()?
            .take(4096)
            .read_to_end(&mut cmdline)
            .ok()?;
        if !cmdline
            .split(|b| *b == 0)
            .any(|a| a == b"experimental-image-proxy")
        {
            return None;
        }
        let written = self
            .read(pid, "io")?
            .lines()
            .find_map(|l| l.strip_prefix("wchar:"))?
            .trim()
            .parse()
            .ok()?;
        Some((start, written))
    }
}

/// The layers imported so far: one ref each in the new system.
fn layers_imported(refs: &Path) -> u64 {
    fs::read_dir(refs).map_or(0, |d| d.count() as u64)
}

/// A line in the install log every [`COPY_NOTE`] while the layers import,
/// and one when they are all in, so a slow copy can be read from the log
/// alone.
#[derive(Default)]
struct CopyLog {
    /// When the last line was written, and the byte count then.
    last: Option<(Duration, u64)>,
    /// The copy is in, and its last line written.
    ended: bool,
}

impl CopyLog {
    /// Notes the counts at `now`; `copied` once the layers are all in,
    /// which writes the last line.
    fn note(
        &mut self,
        log: &Log,
        bytes: Option<(u64, u64)>,
        layers: u64,
        copied: bool,
        now: Duration,
    ) {
        if self.ended {
            return;
        }
        self.ended = copied;
        let done = bytes.map_or(0, |(d, _)| d);
        if done == 0 && layers == 0 {
            return;
        }
        if let Some((then, _)) = self.last
            && !copied
            && now.saturating_sub(then) < COPY_NOTE
        {
            return;
        }
        if self.last.is_none() && !copied {
            self.last = Some((now, done));
            return;
        }
        // no speed on a copy that was in at the first look
        let speed = self.last.map_or(String::new(), |(then, before)| {
            let secs = now.saturating_sub(then).as_secs_f64().max(1.0);
            format!(
                ", {:.1} MB/s",
                done.saturating_sub(before) as f64 / secs / 1e6
            )
        });
        log.note(&match bytes {
            Some((done, total)) => format!(
                "# copy: {} of {} ({:.0} %){speed}, {layers} layers imported",
                gb(done),
                gb(total),
                100.0 * done as f64 / total as f64,
            ),
            None => format!("# copy: {layers} layers imported"),
        });
        self.last = Some((now, done));
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.2} GB", bytes as f64 / 1e9)
}

fn bootc(image: &str, target: &str, kargs: &[String]) -> Cmd {
    let mut args = vec![
        "install".to_string(),
        "to-filesystem".into(),
        "--source-imgref".into(),
        format!("containers-storage:{image}"),
        "--target-imgref".into(),
        image.into(),
        "--skip-fetch-check".into(),
        "--karg".into(),
        "rootflags=compress=zstd:1".into(),
    ];
    for k in kargs {
        args.push("--karg".into());
        args.push(k.clone());
    }
    args.push(target.into());
    Cmd::new(bin::BOOTC, args).timeout(60 * MINUTE)
}

/// A random (version 4) UUID, lower case.
fn new_uuid() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| format!("no randomness: {e}"))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    let u = format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    );
    settings::validate_uuid(&u)?;
    Ok(u)
}

/// The temporary key that formats and fills the LUKS volume. It is in a
/// directory only root can enter, and is deleted when this is dropped,
/// whatever way the install ends.
struct KeyFile {
    path: PathBuf,
}

impl KeyFile {
    fn create(env: &Env) -> Result<KeyFile, String> {
        let dir = env.run_dir.join(KEY_DIR);
        let fail = |e: std::io::Error| format!("cannot create the temporary disk key: {e}");
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.file_type().is_dir() => {
                use std::os::unix::fs::MetadataExt;
                if m.uid() != rustix::process::geteuid().as_raw() {
                    return Err(format!("{} belongs to someone else", dir.display()));
                }
            }
            Ok(_) => return Err(format!("{} is not a directory", dir.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::os::unix::fs::DirBuilderExt::mode(&mut fs::DirBuilder::new(), 0o700)
                    .create(&dir)
                    .map_err(fail)?;
            }
            Err(e) => return Err(fail(e)),
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(fail)?;
        let path = dir.join(KEY_FILE);
        // one left by a crash; remove_file doesn't follow a link
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(fail(e)),
        }
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(fail)?;
        let key = KeyFile { path };
        let mut bytes = [0u8; KEY_BYTES];
        getrandom::fill(&mut bytes).map_err(|e| format!("no randomness: {e}"))?;
        let written = f.write_all(&bytes).map_err(fail);
        wipe_bytes(&mut bytes);
        written?;
        Ok(key)
    }

    fn path(&self) -> Result<&str, String> {
        path_str(&self.path)
    }
}

/// Zero a buffer in a way the compiler can't drop as a dead store.
fn wipe_bytes(b: &mut [u8]) {
    for x in b.iter_mut() {
        // SAFETY: `x` is a valid, aligned, exclusive reference
        unsafe { std::ptr::write_volatile(x, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

impl Drop for KeyFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// `cryptsetup open --test-passphrase` with a key file. cryptsetup tries the
/// volume's tokens (the TPM) before the key it was given, so without
/// `--disable-external-tokens` a key that was removed would still "work".
fn passphrase_test<'a>(key_file: &'a str, node: &'a str) -> [&'a str; 6] {
    [
        "open",
        "--test-passphrase",
        "--disable-external-tokens",
        "--key-file",
        key_file,
        node,
    ]
}

/// cryptsetup and systemd-cryptenroll can take a while: key derivation uses
/// memory and time on purpose.
fn crypt_cmd<I, S>(program: &'static str, args: I) -> Cmd
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    Cmd::new(program, args).timeout(5 * MINUTE)
}

/// Run a command whose output or input is secret: a failure says what
/// failed and the end of stderr, never stdout.
fn run_secret(r: &dyn Runner, cmd: Cmd) -> Result<Output, String> {
    let out = r.run(&cmd, &mut |_| {})?;
    if out.ok() {
        return Ok(out);
    }
    let status = match out.code {
        Some(c) => format!("exit status {c}"),
        None => "stopped by a signal".into(),
    };
    let mut out = out;
    let err = run::tail(&out.stderr, 512);
    // a tool that printed a secret and then failed: don't keep it
    run::wipe(std::mem::take(&mut out.stdout));
    run::wipe(std::mem::take(&mut out.stderr));
    Err(if err.is_empty() {
        format!("{} failed ({status})", cmd.name())
    } else {
        format!("{} failed ({status}): {err}", cmd.name())
    })
}

/// A failed encryption step: what the user is told, with the detail only
/// in the log (it is the tools' own words, which may be long).
fn crypt_failed(env: &Env, log: &Log, what: &str, detail: &str) -> String {
    log.note(&format!("encryption failed while {what}: {detail}"));
    format!(
        "Setting up encryption failed while {what}. {}",
        log.details(env, detail)
    )
}

/// The root partition `node` as an encrypted volume `uuid`, opened as
/// `/dev/mapper/luks-<uuid>` and formatted. Leaves the volume open and
/// returns the recovery key. Only the recovery key and the TPM, the TPM and
/// PIN, or the password can unlock it afterwards.
fn format_encrypted_root(
    r: &dyn Runner,
    env: &Env,
    log: &Log,
    req: &Request,
    node: &str,
    uuid: &str,
) -> Result<String, String> {
    let step = |what: &str| {
        let what = what.to_string();
        move |e: String| crypt_failed(env, log, &what, &e)
    };
    let key = KeyFile::create(env).map_err(step("preparing the disk key"))?;
    let tmp = key.path().map_err(step("preparing the disk key"))?;
    let mapper = crypt::mapper_name(uuid);
    let mapper_dev = format!("/dev/mapper/{mapper}");
    run(
        r,
        crypt_cmd(
            bin::CRYPTSETUP,
            [
                "luksFormat",
                "--type",
                "luks2",
                "--batch-mode",
                "--uuid",
                uuid,
                "--label",
                "atlasos",
                "--key-file",
                tmp,
                node,
            ],
        ),
    )
    .map_err(step("formatting the disk"))?;
    run(
        r,
        crypt_cmd(
            bin::CRYPTSETUP,
            [
                "open",
                "--allow-discards",
                "--key-file",
                tmp,
                node,
                mapper.as_str(),
            ],
        ),
    )
    .map_err(step("opening the encrypted disk"))?;
    run(r, mkfs(Role::Root, &mapper_dev)).map_err(step("creating the filesystem"))?;

    // The recovery key: on a pipe it is the one thing the tool prints.
    let unlock = format!("--unlock-key-file={tmp}");
    let mut out = run_secret(
        r,
        crypt_cmd(bin::CRYPTENROLL, [unlock.as_str(), "--recovery-key", node]).secret(),
    )
    .map_err(step("creating the recovery key"))?;
    let parsed = crypt::parse_recovery_key(&out.stdout).map(str::to_string);
    // the tool's raw output held the key: wipe it, whatever the parse said
    run::wipe(std::mem::take(&mut out.stdout));
    run::wipe(std::mem::take(&mut out.stderr));
    let recovery = parsed.map_err(step("creating the recovery key"))?;

    match req.encryption {
        Encryption::Tpm | Encryption::TpmPin => {
            let mut enroll = vec![unlock.as_str(), "--tpm2-device=auto", "--tpm2-pcrs=7"];
            let pin = req.encryption == Encryption::TpmPin;
            if pin {
                enroll.push("--tpm2-with-pin=yes");
            }
            enroll.push(node);
            let mut cmd = crypt_cmd(bin::CRYPTENROLL, enroll);
            if pin {
                // the PIN goes in the environment (systemd-cryptenroll's
                // $NEWPIN), never in the arguments
                cmd = cmd.secret_env("NEWPIN", req.password.as_str());
            }
            let cmd = if pin { cmd.secret() } else { cmd };
            let enrolled = run_secret(r, cmd);
            // Proof that the TPM unlocks it, before the temporary key goes.
            // With a PIN, cryptsetup has no way to be given it without a
            // prompt, so only the recovery key is proven below.
            let proof = enrolled.and_then(|_| {
                if req.encryption == Encryption::TpmPin {
                    return Ok(());
                }
                run(
                    r,
                    crypt_cmd(
                        bin::CRYPTSETUP,
                        [
                            "open",
                            "--test-passphrase",
                            "--token-only",
                            "--token-type",
                            "systemd-tpm2",
                            node,
                        ],
                    ),
                )
                .map(drop)
            });
            if let Err(e) = proof {
                log.note(&format!("encryption failed at the TPM: {e}"));
                return Err(format!("{TPM_FAILED} {}", log.details(env, &e)));
            }
        }
        Encryption::Password => {
            run_secret(
                r,
                crypt_cmd(
                    bin::CRYPTSETUP,
                    ["luksAddKey", "--key-file", tmp, "--new-keyfile", "-", node],
                )
                .stdin(req.password.as_str())
                .secret(),
            )
            .map_err(step("adding the password"))?;
        }
        Encryption::None => return Err("encryption was not asked for".into()),
    }

    let check = step("checking the encrypted disk");
    run(
        r,
        crypt_cmd(bin::CRYPTSETUP, ["luksRemoveKey", "--key-file", tmp, node]),
    )
    .map_err(step("removing the temporary key"))?;
    // The temporary key must be useless now (exit 2: no key matched).
    let gone = r
        .run(
            &crypt_cmd(bin::CRYPTSETUP, passphrase_test(tmp, node)),
            &mut |_| {},
        )
        .map_err(step("removing the temporary key"))?;
    if gone.code != Some(2) {
        return Err(step("removing the temporary key")(
            "the temporary disk key still unlocks the disk".into(),
        ));
    }
    let dump = run(
        r,
        crypt_cmd(bin::CRYPTSETUP, ["luksDump", "--dump-json-metadata", node]),
    )
    .map_err(step("checking the encrypted disk"))?;
    crypt::check_slots(&dump.stdout, req.encryption).map_err(&check)?;
    // What the user will type must unlock it, before they're told it does.
    let mut proofs = vec![(recovery.as_str(), "the recovery key")];
    if req.encryption == Encryption::Password {
        proofs.push((req.password.as_str(), "the disk password"));
    }
    for (secret, what) in proofs {
        run_secret(
            r,
            crypt_cmd(bin::CRYPTSETUP, passphrase_test("-", node))
                .stdin(secret)
                .secret(),
        )
        .map_err(|e| check(format!("{what} doesn't unlock the new disk: {e}")))?;
    }
    Ok(recovery)
}

/// `--plan`: what `Install` would do.
pub fn describe(p: &Prepared, req: &Request, env: &Env) -> String {
    let target = env.target.display().to_string();
    let mut s = p.plan.describe();
    s.push_str("format:\n");
    for part in &p.plan.parts {
        if part.role == Role::Root && p.luks_uuid.is_some() {
            continue; // below, with the encryption
        }
        s.push_str(&format!("  {}\n", mkfs(part.role, &part.node).display()));
    }
    s.push_str(&format!(
        "install: {}\n",
        bootc(&p.image, &target, &p.kargs).display()
    ));
    s.push_str(&format!("encryption: {}\n", req.encryption.as_str()));
    if let Some(uuid) = &p.luks_uuid {
        let node = p.plan.part(Role::Root).map_or("?", |x| x.node.as_str());
        let mapper = crypt::mapper_name(uuid);
        let steps = [
            format!(
                "cryptsetup luksFormat --type luks2 --batch-mode --uuid {uuid} --label atlasos --key-file <temporary key> {node}"
            ),
            format!("cryptsetup open --allow-discards --key-file <temporary key> {node} {mapper}"),
            mkfs(Role::Root, &format!("/dev/mapper/{mapper}")).display(),
            "systemd-cryptenroll --unlock-key-file=<temporary key> --recovery-key (key is shown after the install)".into(),
            match req.encryption {
                Encryption::Tpm => format!(
                    "systemd-cryptenroll --unlock-key-file=<temporary key> --tpm2-device=auto --tpm2-pcrs=7 {node}, then cryptsetup open --test-passphrase --token-only --token-type systemd-tpm2 {node}"
                ),
                Encryption::TpmPin => format!(
                    "systemd-cryptenroll --unlock-key-file=<temporary key> --tpm2-device=auto --tpm2-pcrs=7 --tpm2-with-pin=yes {node} (PIN in $NEWPIN)"
                ),
                _ => format!(
                    "cryptsetup luksAddKey --key-file <temporary key> --new-keyfile - {node} (password on stdin)"
                ),
            },
            format!("cryptsetup luksRemoveKey --key-file <temporary key> {node}, then check the key slots"),
            "grub2-mkpasswd-pbkdf2 (random password, not kept) -> /boot/grub2/user.cfg: GRUB's menu editing is locked".into(),
        ];
        for step in steps {
            s.push_str(&format!("  {step}\n"));
        }
    }
    s.push_str(&format!(
        "settings: LANG={}, keyboard {} (console {}), Wi-Fi {}\n",
        req.locale,
        req.keymap
            .variant
            .as_ref()
            .map_or(req.keymap.layout.clone(), |v| format!(
                "{}({v})",
                req.keymap.layout
            )),
        p.console_keymap,
        p.wifi.as_ref().map_or("none".to_string(), |w| w.0.clone()),
    ));
    if p.windows_esps.is_empty() {
        s.push_str("boot menu: AtlasOS only\n");
    } else {
        s.push_str(&format!(
            "boot menu: Windows on {}\n",
            p.windows_esps.join(", ")
        ));
    }
    s.push_str(&format!(
        "firmware entry: \"{}\" on {}\n",
        efi::LABEL,
        p.plan.esp
    ));
    s.push_str(&format!(
        "NVIDIA key for MOK enrollment: {}\n",
        if p.mok { "queued" } else { "no" }
    ));
    s
}

/// The install log, which also gets every command and its output.
struct Log(Mutex<Option<fs::File>>);

/// How many characters of a tool's own words go into a message when there
/// is no log file to point to.
const DETAIL_TAIL: usize = 300;

impl Log {
    fn create(path: &Path) -> Log {
        let f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .ok();
        if let Some(f) = &f {
            let _ = f.set_permissions(fs::Permissions::from_mode(0o600));
        }
        Log(Mutex::new(f))
    }

    /// Where the user finds the detail of a failure: the log file, or, when
    /// it could not be opened, the end of the tool's own message.
    fn details(&self, env: &Env, detail: &str) -> String {
        if lock(&self.0).is_some() {
            format!(
                "The install log has the details: {}",
                env.log_path().display()
            )
        } else {
            format!("Details: {}", run::tail(detail, DETAIL_TAIL))
        }
    }

    fn note(&self, text: &str) {
        if let Some(f) = lock(&self.0).as_mut() {
            let _ = writeln!(f, "{text}");
        }
    }
}

struct LogRunner<'a> {
    inner: &'a dyn Runner,
    log: &'a Log,
}

impl Runner for LogRunner<'_> {
    fn run(&self, cmd: &Cmd, on_line: &mut dyn FnMut(Option<&str>)) -> Result<Output, String> {
        self.log.note(&format!("$ {}", cmd.display()));
        let result = self.inner.run(cmd, &mut |l| {
            if let (Some(l), false) = (l, cmd.secret) {
                self.log.note(l);
            }
            on_line(l);
        });
        match &result {
            Ok(o) => self.log.note(&format!(
                "# exit {}",
                o.code.map_or("signal".into(), |c| c.to_string())
            )),
            Err(e) => self.log.note(&format!("# {e}")),
        }
        result
    }
}

/// `Install`. `emit` gets progress as it happens.
pub fn install(
    r: &dyn Runner,
    env: &Env,
    req: &Request,
    emit: &mut dyn FnMut(&Progress),
) -> Result<Outcome, String> {
    fs::create_dir_all(&env.run_dir)
        .map_err(|e| format!("cannot create {}: {e}", env.run_dir.display()))?;
    let log = Log::create(&env.log_path());
    let lr = LogRunner {
        inner: r,
        log: &log,
    };
    emit(&Progress::at(Stage::Prepare, 0.0));
    // Once the disk is chosen and checked, a failure may leave its root
    // mounted and, if encrypted, open: unmount, then close.
    let mut disk = None;
    let mut nodes: Vec<String> = Vec::new();
    let result = prepare(&lr, env, req).and_then(|p| {
        log.note(&describe(&p, req, env));
        // Cleanup acts on this disk only once it has been seen to be the
        // one that was chosen: if this fails, nothing is unmounted or closed.
        recheck_disk(&lr, &p)?;
        disk = Some(p.plan.disk.clone());
        nodes = p.plan.parts.iter().map(|x| x.node.clone()).collect();
        execute(&lr, &log, env, req, &p, emit)
    });
    if let Err(e) = &result {
        log.note(&format!("FAILED: {e}"));
        if let Err(c) = unmount_target(&lr, &log, env, disk.as_deref(), &nodes, false) {
            log.note(&format!("cleanup: {c}"));
        }
    }
    result
}

/// Unmount anything left at the target (a failed earlier attempt), then
/// close the `luks-<uuid>` mappings of this helper on `disk` (a path such
/// as `/dev/sda`), which would keep wipefs and sfdisk from the disk. `strict`
/// (before the first write): a map whose close was only deferred is still
/// open and is refused; after the install a deferred close is enough.
fn unmount_target(
    r: &dyn Runner,
    log: &Log,
    env: &Env,
    disk: Option<&str>,
    nodes: &[String],
    strict: bool,
) -> Result<(), String> {
    let target = path_str(&env.target)?;
    let mounts = fs::read_to_string(env.host("proc/self/mounts")).unwrap_or_default();
    let busy = mounts
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .any(|m| {
            m == target
                || m.strip_prefix(target)
                    .is_some_and(|rest| rest.starts_with('/'))
        });
    let mut unmounted = Ok(());
    if busy && let Err(first) = run(r, umount_target_cmd(target)) {
        // busy: detach it now, the kernel finishes when the last user is gone
        unmounted = run(r, umount_lazy_cmd(target))
            .map(drop)
            .map_err(|e| format!("cannot unmount {target}: {first}; then lazily: {e}"));
    }
    if let Some(disk) = disk {
        // without the listing the names are unknown: release what is known
        let names = match run(r, Cmd::new(bin::LSBLK, lsblk::ARGS))
            .and_then(|out| Lsblk::parse(&out.stdout))
        {
            Ok(l) => l.luks_mappers_on(disk),
            Err(e) => {
                log.note(&format!(
                    "cannot list the disks to find encrypted volumes: {e}"
                ));
                Vec::new()
            }
        };
        let mut failed = false;
        // something else may hold them open (bootc leaves a mount of its own)
        let mut sources: Vec<String> = nodes.to_vec();
        sources.extend(names.iter().map(|n| format!("/dev/mapper/{n}")));
        release_mounts(r, log, &sources);
        for name in names {
            if close_mapper(r, log, &name).is_err() {
                failed = true;
            }
        }
        if strict && !failed {
            // a deferred close "succeeds" while the map is still open
            match run(r, Cmd::new(bin::LSBLK, lsblk::ARGS))
                .and_then(|out| Lsblk::parse(&out.stdout))
            {
                Ok(l) => failed = !l.luks_mappers_on(disk).is_empty(),
                Err(e) => {
                    log.note(&format!("cannot check that the volumes are closed: {e}"));
                    failed = true;
                }
            }
        }
        if failed {
            unmounted = unmounted.and(Err(format!(
                "An earlier encrypted volume on {disk} is still open and couldn't be closed. {}",
                log.details(env, "something still has the volume open")
            )));
        }
    }
    unmounted
}

/// The device behind a findmnt SOURCE: a btrfs subvolume shows as
/// `/dev/mapper/x[/root]`.
fn mount_device(source: &str) -> &str {
    source.split('[').next().unwrap_or(source)
}

/// The real path of a device node (`/dev/mapper/x` is a link to
/// `/dev/dm-N`), or `None` if it doesn't exist.
fn real_path(dev: &str) -> Option<String> {
    fs::canonicalize(dev)
        .ok()
        .and_then(|p| p.to_str().map(String::from))
}

/// The mount points whose source is one of `devices`, deepest first. A
/// source matches by its exact name or, if it is under /dev, by its real
/// path (`/dev/dm-3`, `/dev/disk/by-uuid/...`); `canon` gives the real path.
fn mounts_of(
    r: &dyn Runner,
    devices: &[String],
    canon: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<String>, String> {
    let out = run(
        r,
        Cmd::new(
            bin::FINDMNT,
            ["--json", "--list", "--output", "TARGET,SOURCE"],
        ),
    )
    .map_err(|e| format!("cannot list mounts: {e}"))?;
    let v: serde_json::Value = serde_json::from_str(&out.stdout)
        .map_err(|e| format!("cannot read the list of mounts: {e}"))?;
    let real: Vec<String> = devices.iter().filter_map(|d| canon(d)).collect();
    let mut targets: Vec<&str> = v["filesystems"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| Some((m["target"].as_str()?, mount_device(m["source"].as_str()?))))
        .filter(|(t, s)| {
            t.starts_with('/')
                && (devices.iter().any(|d| d == s)
                    || s.starts_with("/dev/") && canon(s).is_some_and(|c| real.contains(&c)))
        })
        .map(|(t, _)| t)
        .collect();
    // the list is in mount order: the last mounted comes off first, and
    // the stable sort then puts the deepest paths ahead of that
    targets.reverse();
    targets.sort_by_key(|t| std::cmp::Reverse(t.split('/').filter(|c| !c.is_empty()).count()));
    Ok(targets.into_iter().map(String::from).collect())
}

/// Unmount every mount whose source is one of `devices`, wherever it is,
/// deepest first. `bootc install to-filesystem` leaves the target mounted
/// at /run/bootc/storage, which keeps `cryptsetup close` from working and
/// is not under the target directory. What was unmounted is logged;
/// failures are logged too and never stop the caller, whose close then
/// falls back to a deferred one.
fn release_mounts(r: &dyn Runner, log: &Log, devices: &[String]) {
    release_mounts_with(r, log, devices, &real_path);
}

fn release_mounts_with(
    r: &dyn Runner,
    log: &Log,
    devices: &[String],
    canon: &dyn Fn(&str) -> Option<String>,
) {
    let targets = match mounts_of(r, devices, canon) {
        Ok(t) => t,
        Err(e) => {
            log.note(&e);
            return;
        }
    };
    if targets.is_empty() {
        log.note("no leftover mounts of the target disk");
        return;
    }
    for t in &targets {
        let done = match run(
            r,
            Cmd::new(bin::UMOUNT, ["--recursive", t])
                .timeout(15 * MINUTE)
                .cleanup(),
        ) {
            Ok(_) => Ok("unmounted"),
            Err(first) => run(r, umount_lazy_cmd(t))
                .map(|_| "detached lazily")
                .map_err(|e| format!("{first}; then lazily: {e}")),
        };
        match done {
            Ok(how) => log.note(&format!("{how} {t}, which kept the disk busy")),
            // a parent's recursive unmount may already have taken it
            Err(e) => log.note(&format!("cannot unmount {t} (it may be gone already): {e}")),
        }
    }
    // something mounted on top of one of them may still hold the disk
    match mounts_of(r, devices, canon) {
        Ok(left) if !left.is_empty() => log.note(&format!(
            "still mounted after unmounting: {}",
            left.join(", ")
        )),
        Ok(_) => {}
        Err(e) => log.note(&e),
    }
}

/// Close a `luks-<uuid>` mapping; if it is busy, have it closed when its
/// last user is gone.
fn close_mapper(r: &dyn Runner, log: &Log, name: &str) -> Result<(), String> {
    run(r, Cmd::new(bin::CRYPTSETUP, ["close", name]).cleanup())
        .or_else(|_| {
            let d = run(
                r,
                Cmd::new(bin::CRYPTSETUP, ["close", "--deferred", name]).cleanup(),
            );
            if d.is_ok() {
                log.note(&format!(
                    "{name} is still in use: the volume will close once it's no longer in use"
                ));
            }
            d
        })
        .map(drop)
}

fn umount_lazy_cmd(target: &str) -> Cmd {
    Cmd::new(bin::UMOUNT, ["--recursive", "--lazy", target])
        .timeout(MINUTE)
        .cleanup()
}

/// Unmounting writes back what bootc copied, which on a slow disk takes
/// minutes.
fn umount_target_cmd(target: &str) -> Cmd {
    Cmd::new(bin::UMOUNT, ["--recursive", target])
        .timeout(15 * MINUTE)
        .cleanup()
}

/// Whether the live system has a wired network with a cable in: a real
/// (not virtual) Ethernet device whose carrier is up (or, when the carrier
/// can't be read, whose operstate is `up`). NetworkManager brings such a
/// link up on the installed system by itself.
fn on_cable(env: &Env, log: &Log) -> bool {
    let Ok(dir) = fs::read_dir(env.host("sys/class/net")) else {
        return false;
    };
    dir.flatten().any(|e| {
        let p = e.path();
        let read = |f: &str| fs::read_to_string(p.join(f)).unwrap_or_default();
        if !(read("type").trim() == "1"
            && p.join("device").exists()
            && !p.join("wireless").exists()
            && !p.join("phy80211").exists())
        {
            return false;
        }
        // carrier reads EINVAL while the interface is down, or empty
        let carrier = read("carrier");
        let operstate = read("operstate");
        let up = match carrier.trim() {
            "" => operstate.trim() == "up",
            c => c == "1",
        };
        log.note(&format!(
            "network {}: carrier {:?}, operstate {:?}: {}",
            e.file_name().to_string_lossy(),
            carrier.trim(),
            operstate.trim(),
            if up { "wired" } else { "no cable" }
        ));
        up
    })
}

/// Write `base/rel` without following a symlink anywhere below `base` (the
/// new system's own links point into the live system). Walks down with
/// directory descriptors, so a directory swapped for a link meanwhile is
/// never followed, and replaces the file atomically (temporary name, then
/// rename). Returns the directories it had to create, which need labels too.
fn write_file(base: &Path, rel: &str, contents: &str, mode: u32) -> Result<Vec<PathBuf>, String> {
    use rustix::fs::{AtFlags, Mode, OFlags, openat, renameat, unlinkat};
    use rustix::io::Errno;
    let path = base.join(rel);
    let fail = |e: Errno| format!("cannot write {}: {e}", path.display());
    let not_dir = |dir: &Path| {
        format!(
            "cannot write {}: {} is not a directory",
            path.display(),
            dir.display()
        )
    };
    let parts: Vec<&str> = rel.split('/').collect();
    let (name, dirs) = parts
        .split_last()
        .ok_or_else(|| format!("bad path {rel}"))?;
    if name.is_empty() || *name == "." || *name == ".." {
        return Err(format!("bad path {rel}"));
    }
    let dir_flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut created = Vec::new();
    let mut dir = base.to_path_buf();
    let mut fd = openat(
        rustix::fs::CWD,
        base,
        dir_flags | OFlags::RDONLY,
        Mode::empty(),
    )
    .map_err(|e| {
        if matches!(e, Errno::LOOP | Errno::NOTDIR) {
            not_dir(base)
        } else {
            fail(e)
        }
    })?;
    for part in dirs {
        if part.is_empty() || *part == "." || *part == ".." {
            return Err(format!("bad path {rel}"));
        }
        dir.push(part);
        let open = |fd: &std::os::fd::OwnedFd| {
            openat(fd, *part, dir_flags | OFlags::RDONLY, Mode::empty())
        };
        fd = match open(&fd) {
            Ok(next) => next,
            // a link, or a file, where a directory should be
            Err(Errno::LOOP | Errno::NOTDIR) => return Err(not_dir(&dir)),
            Err(Errno::NOENT) => {
                match rustix::fs::mkdirat(&fd, *part, Mode::from_raw_mode(0o755)) {
                    Ok(()) => created.push(dir.clone()),
                    // made by someone else in between: use it if it is a directory
                    Err(Errno::EXIST) => {}
                    Err(e) => return Err(fail(e)),
                }
                open(&fd).map_err(|e| {
                    if matches!(e, Errno::LOOP | Errno::NOTDIR) {
                        not_dir(&dir)
                    } else {
                        fail(e)
                    }
                })?
            }
            Err(e) => return Err(fail(e)),
        };
    }
    // A temporary name beside the file, created new (never an existing
    // file or link), then renamed over the final name.
    let tmp = format!(".{name}.atlas-tmp-{}", std::process::id());
    let file_mode = Mode::from_raw_mode(mode & 0o7777);
    let create_tmp = || {
        openat(
            &fd,
            tmp.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            file_mode,
        )
    };
    let tmp_fd = match create_tmp() {
        // a leftover of a crashed run: remove it (a link is unlinked, not
        // followed) and try once more
        Err(Errno::EXIST) => {
            unlinkat(&fd, tmp.as_str(), AtFlags::empty()).map_err(fail)?;
            create_tmp()
        }
        r => r,
    }
    .map_err(fail)?;
    let written = (|| -> Result<(), String> {
        let mut f = fs::File::from(tmp_fd);
        f.write_all(contents.as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        rustix::fs::fchmod(&f, file_mode).map_err(fail)?;
        f.sync_all()
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        renameat(&fd, tmp.as_str(), &fd, *name).map_err(fail)?;
        // the rename itself must reach the disk
        rustix::fs::fsync(&fd).map_err(fail)
    })();
    if let Err(e) = written {
        let _ = unlinkat(&fd, tmp.as_str(), AtFlags::empty());
        return Err(e);
    }
    Ok(created)
}

/// The one deployment bootc made: `<target>/ostree/deploy/<stateroot>/deploy/<checksum>.<serial>`.
fn find_deployment(target: &Path) -> Result<PathBuf, String> {
    let mut found = Vec::new();
    let roots = fs::read_dir(target.join("ostree/deploy"))
        .map_err(|e| format!("no ostree deployment: {e}"))?;
    for root in roots.flatten() {
        let Ok(deps) = fs::read_dir(root.path().join("deploy")) else {
            continue;
        };
        for d in deps.flatten() {
            if d.file_type().is_ok_and(|t| t.is_dir()) {
                found.push(d.path());
            }
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        n => Err(format!("expected one deployment after bootc, found {n}")),
    }
}

/// Give files written into the new system the SELinux labels its own policy
/// wants (`root` is the path the files are at `/` under).
fn relabel(
    r: &dyn Runner,
    policy_root: &Path,
    root: &Path,
    files: &[PathBuf],
) -> Result<(), String> {
    let fc = policy_root.join("etc/selinux/targeted/contexts/files/file_contexts");
    if !fc.is_file() || files.is_empty() {
        return Ok(());
    }
    let mut args = vec![
        "-F".to_string(),
        "-r".into(),
        path_str(root)?.into(),
        path_str(&fc)?.into(),
    ];
    for f in files {
        args.push(path_str(f)?.into());
    }
    run(r, Cmd::new(bin::SETFILES, args)).map(|_| ())
}

/// Eight digits: MOK Manager reads the password with a US keyboard layout.
fn mok_password() -> Result<String, String> {
    let mut out = String::new();
    while out.len() < 8 {
        let mut b = [0u8; 16];
        getrandom::fill(&mut b).map_err(|e| format!("no randomness: {e}"))?;
        out.extend(
            b.iter()
                .filter(|x| **x < 250)
                .map(|x| char::from(b'0' + x % 10)),
        );
    }
    out.truncate(8);
    Ok(out)
}

/// The crypt hash from `mokutil --generate-hash`: its last line. With the
/// password on stdin it first prints its two prompts on stdout.
fn mok_hash(stdout: &str) -> Option<&str> {
    stdout
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .filter(|l| l.starts_with('$'))
}

fn queue_mok(r: &dyn Runner, env: &Env) -> Result<String, String> {
    let pw = mok_password()?;
    /* On stdin, twice as mokutil asks for it, so the password never shows
    in the process list. */
    let out = run(
        r,
        Cmd::new(bin::MOKUTIL, ["--generate-hash"])
            .stdin(format!("{pw}\n{pw}\n"))
            .secret(),
    )?;
    let hash = mok_hash(&out.stdout).ok_or("mokutil gave no password hash")?;
    write_file(&env.run_dir, "mok.hash", hash, 0o600)?;
    let hash_file = env.run_dir.join("mok.hash");
    let key = env.host(NVIDIA_KEY);
    let result = run(
        r,
        Cmd::new(
            bin::MOKUTIL,
            [
                "--import",
                path_str(&key)?,
                "--hash-file",
                path_str(&hash_file)?,
            ],
        ),
    );
    let _ = fs::remove_file(&hash_file);
    result.map(|_| pw)
}

/// Whether the medium the live system runs from is still there: false once
/// the USB stick is pulled (its block device goes away, and the by-label
/// link with it) or the disc is ejected (the drive stays, with a size of
/// 0). True when not running from a live medium at all.
pub fn live_medium_present(env: &Env) -> bool {
    let mounts = fs::read_to_string(env.host("proc/self/mounts")).unwrap_or_default();
    disks::live_sources(&mounts).iter().all(|src| {
        fs::canonicalize(env.host(src.trim_start_matches('/')))
            .ok()
            .and_then(|real| real.file_name().map(|n| n.to_string_lossy().into_owned()))
            .is_some_and(|name| device_present(env, &mounts, &name, 0))
    })
}

/// Whether block device `name` can still be read. A device-mapper device
/// (Ventoy's) and a loop device (an ISO file booted with iso-scan) outlive
/// the stick under them, so for those it is whether what they read from is
/// there.
fn device_present(env: &Env, mounts: &str, name: &str, depth: u32) -> bool {
    let dir = env.host(&format!("sys/class/block/{name}"));
    let sized = fs::read_to_string(dir.join("size"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .is_some_and(|size| size > 0);
    if !sized || depth > 8 {
        return sized;
    }
    let slaves: Vec<String> = fs::read_dir(dir.join("slaves"))
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    if !slaves.is_empty() || dir.join("dm").exists() {
        return !slaves.is_empty()
            && slaves
                .iter()
                .all(|s| device_present(env, mounts, s, depth + 1));
    }
    let Ok(file) = fs::read_to_string(dir.join("loop/backing_file")) else {
        return true;
    };
    let file = file.trim_end_matches('\n').trim_end_matches(" (deleted)");
    // The file's filesystem: the mount with the longest matching mount point.
    let holder = mounts
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (dev, dir) = (disks::unescape(f.next()?), disks::unescape(f.next()?));
            let inside = dir == "/" || file == dir || file.starts_with(&format!("{dir}/"));
            inside.then_some((dir.len(), dev))
        })
        .max_by_key(|&(len, _)| len)
        .map(|(_, dev)| dev);
    match holder
        .as_deref()
        .and_then(|dev| fs::canonicalize(env.host(dev.trim_start_matches('/'))).ok())
    {
        Some(real) => real
            .file_name()
            .is_some_and(|n| device_present(env, mounts, &n.to_string_lossy(), depth + 1)),
        // a filesystem with no device (tmpfs): nothing to pull out
        None => holder.is_some_and(|dev| !dev.starts_with("/dev/")),
    }
}

/// Restart the computer. With the live medium there, cleanly, through
/// systemd. Without it, systemctl can't start (it is on the medium), and
/// even systemd's own shutdown ends by starting systemd-shutdown from it,
/// which would leave the computer hanging. So then: unmount whatever of the
/// installed disk is still mounted (normally nothing: the install unmounts
/// it, but that can fail), write out the disks and restart through the
/// kernel. The live session itself keeps nothing.
pub fn restart(r: &dyn Runner, env: &Env) -> Result<(), String> {
    if live_medium_present(env) {
        match run(r, Cmd::new(bin::SYSTEMCTL, ["reboot"])) {
            Ok(_) => return Ok(()),
            // pulled since, or broken some other way
            Err(e) => eprintln!(
                "atlas-installer-helper: systemctl reboot failed, restarting directly: {e}"
            ),
        }
    }
    let mounts = fs::read_to_string(env.host("proc/self/mounts")).unwrap_or_default();
    let target = env.target.to_string_lossy();
    let mut under: Vec<String> = mounts
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(disks::unescape)
        .filter(|dir| *dir == *target || dir.starts_with(&format!("{target}/")))
        .collect();
    // The deepest first. A mount point can be stacked: one unmount per
    // mount, the top (later in the list) first.
    under.reverse();
    under.sort_by_key(|dir| std::cmp::Reverse(dir.len()));
    r.restart_now(&under)
}

/// Replace bootupd's "Fedora" firmware entry for our ESP with "AtlasOS".
/// The new entry is made first, so a failure never leaves none. `Err`: no
/// AtlasOS entry was made; `Ok` lists old entries that could not be removed.
fn rename_boot_entry(r: &dyn Runner, plan: &Plan, after: &Table) -> Result<Vec<String>, String> {
    let partuuid = after
        .partition(&plan.esp)
        .and_then(|p| p.uuid.clone())
        .ok_or("the EFI partition has no UUID")?;
    let n = partition_number(&plan.disk, &plan.esp).ok_or("the EFI partition has no number")?;
    let list = run(r, Cmd::new(bin::EFIBOOTMGR, Vec::<String>::new()))?;
    let entries = efi::parse(&list.stdout);
    let old: Vec<String> = efi::stale_entries(&entries, &partuuid)
        .iter()
        .map(|e| e.num.clone())
        .collect();
    if old.is_empty() {
        return Err("bootupd's firmware boot entry was not found".into());
    }
    let n = n.to_string();
    run(
        r,
        Cmd::new(
            bin::EFIBOOTMGR,
            [
                "--quiet",
                "--create",
                "--disk",
                plan.disk.as_str(),
                "--part",
                &n,
                "--loader",
                efi::SHIM,
                "--label",
                efi::LABEL,
            ],
        ),
    )?;
    let mut warnings = Vec::new();
    // Creating the entry puts it first in the boot order, but some firmware
    // still starts a USB stick first while one is plugged in. BootNext wins
    // over both, once, so the restart reaches AtlasOS even with the stick
    // left in. The new entry is the first in the order (efibootmgr may also
    // have reused an old one: then that one is kept, below).
    let new = run(r, Cmd::new(bin::EFIBOOTMGR, Vec::<String>::new()))
        .ok()
        .and_then(|list| {
            let first = efi::boot_order(&list.stdout).into_iter().next()?;
            let entries = efi::parse(&list.stdout);
            efi::stale_entries(&entries, &partuuid)
                .into_iter()
                .find(|e| e.num == first && e.label == efi::LABEL)
                .map(|e| e.num.clone())
        });
    let next = match &new {
        Some(num) => run(
            r,
            Cmd::new(bin::EFIBOOTMGR, ["--quiet", "--bootnext", num.as_str()]),
        )
        .map(drop),
        None => Err("the new entry is not first in the firmware's boot order".into()),
    };
    if let Err(e) = next {
        warnings.push(format!(
            "The firmware wasn't told to start AtlasOS next, so take out the USB stick or disc before the computer restarts: {e}"
        ));
    }
    for num in old.into_iter().filter(|num| Some(num) != new.as_ref()) {
        if let Err(e) = run(
            r,
            Cmd::new(
                bin::EFIBOOTMGR,
                ["--quiet", "--delete-bootnum", "--bootnum", num.as_str()],
            ),
        ) {
            warnings.push(format!(
                "The old firmware boot entry Boot{num} could not be removed, so the boot menu lists AtlasOS twice: {e}"
            ));
        }
    }
    Ok(warnings)
}

/// After partitioning: the kernel sees each new partition where the plan
/// put it (sysfs counts 512-byte sectors), so mkfs can't hit an old one.
fn check_kernel_sees(env: &Env, plan: &Plan) -> Result<(), String> {
    for p in &plan.parts {
        let name = p.node.rsplit('/').next().unwrap_or(&p.node);
        let read = |f: &str| {
            fs::read_to_string(env.host(&format!("sys/class/block/{name}/{f}")))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
        };
        let k = plan.sector_size / 512;
        if read("start") != Some(p.start * k) || read("size") != Some(p.size * k) {
            return Err(format!(
                "the kernel doesn't see {} where it was created",
                p.node
            ));
        }
    }
    Ok(())
}

/// Lock GRUB's menu editing and command line behind a password nobody
/// knows, so `e` and `init=/bin/sh` don't give a root shell on a disk the TPM
/// has unlocked. bootupd's static config reads `/boot/grub2/user.cfg`; the
/// boot entries and Windows (`--unrestricted`) stay bootable. Returns the
/// directories and file it made, which need labels.
fn lock_grub(r: &dyn Runner, boot_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| format!("no randomness: {e}"))?;
    let pw: String = b.iter().map(|x| format!("{x:02x}")).collect();
    wipe_bytes(&mut b);
    // twice on stdin, as it asks; the password is forgotten right here
    let out = run_secret(
        r,
        Cmd::new(bin::GRUB2_MKPASSWD, Vec::<String>::new())
            .stdin(format!("{pw}\n{pw}\n"))
            .secret(),
    );
    run::wipe(pw);
    let out = out?;
    let hash = grub::parse_mkpasswd(&out.stdout)?;
    let mut made = write_file(boot_dir, "grub2/user.cfg", &grub::user_cfg(hash)?, 0o600)?;
    made.push(boot_dir.join("grub2/user.cfg"));
    Ok(made)
}

fn execute(
    r: &dyn Runner,
    log: &Log,
    env: &Env,
    req: &Request,
    p: &Prepared,
    emit: &mut dyn FnMut(&Progress),
) -> Result<Outcome, String> {
    let plan = &p.plan;
    let target = path_str(&env.target)?;
    let nodes: Vec<String> = plan.parts.iter().map(|x| x.node.clone()).collect();
    // install() has checked this is the disk that was chosen; check again
    // below once its old mounts and maps are gone.
    unmount_target(r, log, env, Some(&plan.disk), &nodes, true)?;
    fs::create_dir_all(&env.target).map_err(|e| format!("cannot create {target}: {e}"))?;

    // Partitions, then the safety check before anything is formatted.
    emit(&Progress::at(Stage::Partition, 0.0));
    plan.validate()
        .map_err(|e| format!("Stopped before changing the disk: {e}"))?;
    recheck_disk(r, p)?;
    for dev in &plan.wipe {
        run(
            r,
            Cmd::new(bin::WIPEFS, ["--all", "--quiet", dev.as_str()]).timeout(5 * MINUTE),
        )?;
    }
    run(
        r,
        Cmd::new(bin::SFDISK, plan.sfdisk_args())
            .stdin(plan.sfdisk_script())
            .timeout(5 * MINUTE),
    )?;
    run(r, Cmd::new(bin::UDEVADM, ["settle", "--timeout=30"]))?;
    let after =
        Table::parse(&run(r, Cmd::new(bin::SFDISK, ["--json", plan.disk.as_str()]))?.stdout)?;
    plan.check_written(&after)
        .map_err(|e| format!("Stopped before formatting anything: {e}"))?;
    let mut wait = vec!["wait".to_string(), "--timeout=30".into()];
    wait.extend(plan.parts.iter().map(|x| x.node.clone()));
    run(r, Cmd::new(bin::UDEVADM, wait))?;
    check_kernel_sees(env, plan).map_err(|e| format!("Stopped before formatting anything: {e}"))?;

    emit(&Progress::at(Stage::Format, 0.0));
    let mut recovery_key = None;
    for (i, part) in plan.parts.iter().enumerate() {
        run(
            r,
            Cmd::new(bin::WIPEFS, ["--all", "--quiet", part.node.as_str()]).timeout(5 * MINUTE),
        )?;
        match (&p.luks_uuid, part.role) {
            (Some(uuid), Role::Root) => {
                recovery_key = Some(format_encrypted_root(r, env, log, req, &part.node, uuid)?);
            }
            _ => {
                run(r, mkfs(part.role, &part.node))?;
            }
        }
        emit(&Progress::at(
            Stage::Format,
            (i + 1) as f64 / plan.parts.len() as f64,
        ));
    }

    let node = |role| {
        plan.part(role)
            .map(|x| x.node.as_str())
            .ok_or("the plan has no such partition")
    };
    let boot_dir = env.target.join("boot");
    let esp_dir = boot_dir.join("efi");
    let mapper = p.luks_uuid.as_deref().map(crypt::mapper_name);
    let root_dev = match &mapper {
        Some(m) => format!("/dev/mapper/{m}"),
        None => node(Role::Root)?.to_string(),
    };
    run(
        r,
        Cmd::new(
            bin::MOUNT,
            ["-o", "compress=zstd:1", root_dev.as_str(), target],
        ),
    )?;
    fs::create_dir_all(&boot_dir).map_err(|e| format!("cannot create /boot: {e}"))?;
    run(
        r,
        Cmd::new(bin::MOUNT, [node(Role::Boot)?, path_str(&boot_dir)?]),
    )?;
    fs::create_dir_all(&esp_dir).map_err(|e| format!("cannot create /boot/efi: {e}"))?;
    run(
        r,
        Cmd::new(bin::MOUNT, [plan.esp.as_str(), path_str(&esp_dir)?]),
    )?;

    // bootc: the copy, the bootloader, then it trims and remounts read-only.
    emit(&Progress::at(Stage::Copy, 0.0));
    let total = image_bytes(r, &p.image);
    match total {
        Some(t) => log.note(&format!("# the image is {} uncompressed", gb(t))),
        None => log.note("# the image's size is unknown: the copy moves the bar by time"),
    }
    let cmd = bootc(&p.image, target, &p.kargs);
    let started = Instant::now();
    let mut bp = BootcProgress::default();
    let mut meter = CopyMeter::new(env.host("proc"), std::process::id());
    let refs = env.target.join(BLOB_REFS);
    let mut copy_log = CopyLog::default();
    let mut next_sample = Duration::ZERO;
    r.run(&cmd, &mut |line| {
        let now = started.elapsed();
        let next = match line {
            Some(l) => bp.line(l, now),
            None => {
                let mut moved = None;
                // the counts matter only until the layers are in
                if !copy_log.ended && now >= next_sample {
                    next_sample = now + COPY_SAMPLE;
                    let bytes = total.map(|t| (meter.sample(), t));
                    let layers = layers_imported(&refs);
                    moved = bp.counted(bytes, layers, now);
                    copy_log.note(log, bytes, layers, bp.copied(), now);
                }
                moved.or_else(|| bp.tick(now))
            }
        };
        if let Some(x) = next {
            emit(&x);
        }
    })?
    .check(&cmd)?;
    if let Some(total) = total {
        log.note(&format!(
            "# the image proxy handed bootc {} of the expected {}",
            gb(meter.sample()),
            gb(total)
        ));
    }

    emit(&Progress::at(Stage::Settings, 0.0));
    run(r, Cmd::new(bin::MOUNT, ["-o", "remount,rw", target]))?;
    run(
        r,
        Cmd::new(bin::MOUNT, ["-o", "remount,rw", path_str(&boot_dir)?]),
    )?;
    let deploy = find_deployment(&env.target)?;
    let mut files = vec![
        (
            "etc/locale.conf".to_string(),
            settings::locale_conf(&req.locale),
            0o644,
        ),
        (
            "etc/vconsole.conf".to_string(),
            settings::vconsole_conf(&p.console_keymap, &req.keymap),
            0o644,
        ),
        (
            "etc/X11/xorg.conf.d/00-keyboard.conf".to_string(),
            settings::x11_keyboard_conf(&req.keymap),
            0o644,
        ),
        (
            "etc/atlasos/installer.ini".to_string(),
            settings::installer_ini(
                &req.locale,
                &req.keymap,
                p.wifi.is_some() || on_cable(env, log),
            ),
            0o644,
        ),
    ];
    if let Some((name, contents)) = &p.wifi {
        files.push((
            format!("etc/NetworkManager/system-connections/{name}"),
            contents.clone(),
            0o600,
        ));
    }
    let mut written = Vec::new();
    for (rel, contents, mode) in &files {
        written.extend(write_file(&deploy, rel, contents, *mode)?);
        written.push(deploy.join(rel));
    }
    // A missing label doesn't stop the system from booting; say so and go on.
    let mut warnings = Vec::new();
    let label_warning = |e: String| {
        format!(
            "Some settings files may have the wrong SELinux label ({e}). After restarting, run sudo restorecon -R /etc."
        )
    };
    if let Err(e) = relabel(r, &deploy, &deploy, &written) {
        warnings.push(label_warning(e));
    }
    // /boot/grub2: the lock on GRUB's menu editing (encrypted installs) and
    // Windows in the menu, labelled together
    let mut boot_files = Vec::new();
    if p.luks_uuid.is_some() {
        match lock_grub(r, &boot_dir) {
            Ok(f) => boot_files.extend(f),
            Err(e) => {
                log.note(&format!("locking GRUB failed: {e}"));
                return Err(format!(
                    "Locking the boot menu failed. {}",
                    log.details(env, &e)
                ));
            }
        }
    }
    if !p.windows_esps.is_empty() {
        boot_files.extend(write_file(
            &boot_dir,
            "grub2/custom.cfg",
            &grub::custom_cfg(&p.windows_esps)?,
            0o644,
        )?);
        boot_files.push(boot_dir.join("grub2/custom.cfg"));
    }
    if let Err(e) = relabel(r, &deploy, &env.target, &boot_files) {
        warnings.push(label_warning(e));
    }

    emit(&Progress::at(Stage::Finish, 0.0));
    match rename_boot_entry(r, plan, &after) {
        Ok(w) => warnings.extend(w),
        Err(e) => warnings.push(format!(
            "The firmware boot entry is still called Fedora: {e}"
        )),
    }
    let mut mok_password = None;
    if p.mok {
        match queue_mok(r, env) {
            Ok(pw) => mok_password = Some(Secret(pw)),
            Err(e) => warnings.push(format!(
                "The NVIDIA key could not be queued ({e}). After restarting, run sudo /usr/libexec/atlasos/nvidia-enroll-key."
            )),
        }
    }
    // Installed by now: a failure here is not a failed install.
    let mut devices: Vec<String> = plan.parts.iter().map(|x| x.node.clone()).collect();
    devices.extend(mapper.iter().map(|m| format!("/dev/mapper/{m}")));
    match run(r, umount_target_cmd(target)) {
        Ok(_) => {
            release_mounts(r, log, &devices);
            if let Some(m) = &mapper
                && let Err(e) = close_mapper(r, log, m)
            {
                warnings.push(format!(
                    "The encrypted disk could not be closed cleanly ({e}). Restart the computer to finish."
                ));
            }
        }
        Err(e) => {
            warnings.push(format!(
                "The new system could not be unmounted cleanly ({e}). Restart the computer to finish writing it."
            ));
            // detach it, and have the volume closed once nothing uses it
            if let Err(e) = run(r, umount_lazy_cmd(target)) {
                log.note(&format!("lazy unmount failed: {e}"));
            }
            release_mounts(r, log, &devices);
            if let Some(m) = &mapper
                && let Err(e) = close_mapper(r, log, m)
            {
                log.note(&format!("closing {m} failed: {e}"));
            }
        }
    }
    emit(&Progress::at(Stage::Finish, 1.0));
    Ok(Outcome {
        mok_password,
        recovery_key: recovery_key.map(Secret),
        encryption: req.encryption.as_str().into(),
        windows_entry: !p.windows_esps.is_empty(),
        boot_media: p.boot_media.into(),
        warnings,
        log: env.log_path().display().to_string(),
    })
}

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
