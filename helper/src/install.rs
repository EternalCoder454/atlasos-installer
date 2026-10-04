//! What `ListDisks` and `Install` do: probe the disks, plan, partition,
//! format, run bootc, write the settings, fix the boot entry, queue the
//! NVIDIA key. The decisions are in installer-core; this runs them.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use installer_core::disks::{self, Probe};
use installer_core::lsblk::{self, Device, Lsblk};
use installer_core::plan::{EspInfo, Mode, Plan, Role};
use installer_core::progress::{BootcProgress, Progress, Stage};
use installer_core::settings::{self, Keymap};
use installer_core::table::{Table, partition_number};
use installer_core::{efi, gpt, grub};
use serde::Serialize;

use crate::run::{Cmd, Output, Runner, bin, lock, run};

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
}

impl Request {
    /// Check every argument before anything else happens. `wifi_uuid` may
    /// be empty (no Wi-Fi to carry over).
    pub fn new(
        disk_id: &str,
        fingerprint: &str,
        mode: &str,
        locale: &str,
        keymap: &str,
        wifi_uuid: &str,
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
        Ok(Request {
            disk_id: disk_id.into(),
            fingerprint,
            mode: mode.parse()?,
            locale: locale.into(),
            keymap: Keymap::parse(keymap)?,
            wifi_uuid,
        })
    }
}

/// `Install`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    /// The one-time password for MOK Manager, when the NVIDIA key was queued.
    pub mok_password: Option<String>,
    /// Windows was added to the boot menu.
    pub windows_entry: bool,
    /// What the installer was started from: "cd", "usb" or "", for
    /// "Remove the USB stick" (see [`disks::boot_media`]).
    pub boot_media: String,
    /// Things that went wrong without failing the install.
    pub warnings: Vec<String>,
    pub log: String,
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
pub fn list_disks(r: &dyn Runner, env: &Env) -> Result<disks::DiskList, String> {
    Ok(disks::list(&probe(r, env)?))
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
    /// See [`Outcome::boot_media`].
    pub boot_media: &'static str,
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
    if let Some(f) = &req.fingerprint
        && *f != disks::fingerprint(disk, probe.tables.get(&disk.name))
    {
        return Err(format!(
            "{} changed since the disks were listed (it was replugged, or its partitions changed). Nothing was written. Choose the disk again.",
            disk.name
        ));
    }
    let plan = disks::plan_for(disk, &probe, req.mode)
        .map_err(|u| format!("Can't install to {}: {}", disk.name, u.message(false)))?;

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
    Ok(Prepared {
        console_keymap: settings::console_keymap(&map, &req.keymap),
        plan,
        image,
        windows_esps,
        mok,
        wifi,
        boot_media: disks::boot_media(&probe),
    })
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

fn bootc(image: &str, target: &str) -> Cmd {
    Cmd::new(
        bin::BOOTC,
        [
            "install",
            "to-filesystem",
            "--source-imgref",
            &format!("containers-storage:{image}"),
            "--target-imgref",
            image,
            "--skip-fetch-check",
            "--karg",
            "rootflags=compress=zstd:1",
            target,
        ],
    )
    .timeout(60 * MINUTE)
}

/// `--plan`: what `Install` would do.
pub fn describe(p: &Prepared, req: &Request, env: &Env) -> String {
    let target = env.target.display().to_string();
    let mut s = p.plan.describe();
    s.push_str("format:\n");
    for part in &p.plan.parts {
        s.push_str(&format!("  {}\n", mkfs(part.role, &part.node).display()));
    }
    s.push_str(&format!(
        "install: {}\n",
        bootc(&p.image, &target).display()
    ));
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
    let result = prepare(&lr, env, req).and_then(|p| {
        log.note(&describe(&p, req, env));
        execute(&lr, env, req, &p, emit)
    });
    if let Err(e) = &result {
        log.note(&format!("FAILED: {e}"));
        let _ = unmount_target(&lr, env);
    }
    result
}

/// Unmount anything left at the target (a failed earlier attempt).
fn unmount_target(r: &dyn Runner, env: &Env) -> Result<(), String> {
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
    if busy {
        run(r, umount_target_cmd(target))?;
    }
    Ok(())
}

/// Unmounting writes back what bootc copied, which on a slow disk takes
/// minutes.
fn umount_target_cmd(target: &str) -> Cmd {
    Cmd::new(bin::UMOUNT, ["--recursive", target])
        .timeout(15 * MINUTE)
        .cleanup()
}

/// Write `base/rel` without following a symlink anywhere below `base` (the
/// new system's own links point into the live system). Returns the
/// directories it had to create, which need labels too.
fn write_file(base: &Path, rel: &str, contents: &str, mode: u32) -> Result<Vec<PathBuf>, String> {
    let path = base.join(rel);
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    let mut created = Vec::new();
    let mut dir = base.to_path_buf();
    let parts: Vec<&str> = rel.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        if part.is_empty() || *part == "." || *part == ".." {
            return Err(format!("bad path {rel}"));
        }
        dir.push(part);
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "cannot write {}: {} is not a directory",
                    path.display(),
                    dir.display()
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&dir).map_err(fail)?;
                created.push(dir.clone());
            }
            Err(e) => return Err(fail(e)),
        }
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(fail)?;
    f.write_all(contents.as_bytes()).map_err(fail)?;
    f.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(fail)?;
    f.sync_all().map_err(fail)?;
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

fn execute(
    r: &dyn Runner,
    env: &Env,
    req: &Request,
    p: &Prepared,
    emit: &mut dyn FnMut(&Progress),
) -> Result<Outcome, String> {
    let plan = &p.plan;
    let target = path_str(&env.target)?;
    unmount_target(r, env)?;
    fs::create_dir_all(&env.target).map_err(|e| format!("cannot create {target}: {e}"))?;

    // Partitions, then the safety check before anything is formatted.
    emit(&Progress::at(Stage::Partition, 0.0));
    plan.validate()
        .map_err(|e| format!("Stopped before changing the disk: {e}"))?;
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
    for (i, part) in plan.parts.iter().enumerate() {
        run(
            r,
            Cmd::new(bin::WIPEFS, ["--all", "--quiet", part.node.as_str()]).timeout(5 * MINUTE),
        )?;
        run(r, mkfs(part.role, &part.node))?;
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
    run(
        r,
        Cmd::new(
            bin::MOUNT,
            ["-o", "compress=zstd:1", node(Role::Root)?, target],
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
    let cmd = bootc(&p.image, target);
    let started = Instant::now();
    let mut bp = BootcProgress::default();
    r.run(&cmd, &mut |line| {
        let now = started.elapsed();
        let next = match line {
            Some(l) => bp.line(l, now),
            None => bp.tick(now),
        };
        if let Some(x) = next {
            emit(&x);
        }
    })?
    .check(&cmd)?;

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
    if !p.windows_esps.is_empty() {
        let mut cfg = write_file(
            &boot_dir,
            "grub2/custom.cfg",
            &grub::custom_cfg(&p.windows_esps)?,
            0o644,
        )?;
        cfg.push(boot_dir.join("grub2/custom.cfg"));
        if let Err(e) = relabel(r, &deploy, &env.target, &cfg) {
            warnings.push(label_warning(e));
        }
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
            Ok(pw) => mok_password = Some(pw),
            Err(e) => warnings.push(format!(
                "The NVIDIA key could not be queued ({e}). After restarting, run sudo /usr/libexec/atlasos/nvidia-enroll-key."
            )),
        }
    }
    // Installed by now: a failure here is not a failed install.
    if let Err(e) = run(r, umount_target_cmd(target)) {
        warnings.push(format!(
            "The new system could not be unmounted cleanly ({e}). Restart the computer to finish writing it."
        ));
    }
    emit(&Progress::at(Stage::Finish, 1.0));
    Ok(Outcome {
        mok_password,
        windows_entry: !p.windows_esps.is_empty(),
        boot_media: p.boot_media.into(),
        warnings,
        log: env.log_path().display().to_string(),
    })
}

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
