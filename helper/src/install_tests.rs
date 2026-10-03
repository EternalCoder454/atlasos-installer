//! The install sequence against a fake system: a fake runner that keeps a
//! partition table, and a temporary directory for the host and the target.

use installer_core::table::Partition;

use super::*;

const LSBLK: &str = include_str!("../../crates/installer-core/tests/fixtures/lsblk-windows.json");
const SFDISK: &str = include_str!("../../crates/installer-core/tests/fixtures/sfdisk-windows.json");
const UUID: &str = "0b4f6b8e-2a0c-4d5e-9f1a-3c2b1a0d9e8f";

/// bootc's output from spike S3.
const BOOTC_OUT: &str = "Installing image: docker://ghcr.io/eternalcoder454/atlasos:stable\n\
    Initializing ostree layout\n\
    layers already present: 0; layers needed: 128 (3.2\u{a0}GB)\n\
    Deploying container image...done (18 seconds)\n\
    Bootloader: grub\n\
    Installing bootloader via bootupd\n\
    Trimming root\n\
    Finalizing filesystem root\n\
    Installation complete!\n";

#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<Cmd>>,
    table: Mutex<Option<Table>>,
    /// The fake host: sysfs and /proc/self/mounts are kept up to date.
    root: PathBuf,
    target: PathBuf,
    /// The kernel keeps the old partitions (sysfs is not updated).
    stale_kernel: bool,
    /// The new system's /etc/X11 is a symlink to the host's.
    x11_symlink: bool,
    /// Commands ("name args...") that fail, by prefix.
    fail: Vec<&'static str>,
    /// sfdisk moves the Windows partition (the safety check must catch it).
    move_partition: bool,
    enrolled: bool,
}

fn ok(stdout: &str) -> Result<Output, String> {
    Ok(Output {
        code: Some(0),
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

/// The partitions an sfdisk script asks for.
fn script_parts(script: &str) -> Vec<Partition> {
    script
        .lines()
        .filter_map(|l| {
            let (node, rest) = l.split_once(" : ")?;
            let field = |k: &str| {
                rest.split(", ")
                    .find_map(|f| f.strip_prefix(&format!("{k}=")))
                    .map(|v| v.trim_matches('"').to_string())
            };
            Some(Partition {
                node: node.into(),
                start: field("start")?.parse().ok()?,
                size: field("size")?.parse().ok()?,
                kind: field("type")?,
                uuid: Some(format!(
                    "AAAAAAAA-0000-0000-0000-{:012}",
                    node.len() * 1000 + node.chars().last()? as usize
                )),
                name: field("name"),
            })
        })
        .collect()
}

impl Runner for Fake {
    fn run(&self, cmd: &Cmd, on_line: &mut dyn FnMut(Option<&str>)) -> Result<Output, String> {
        lock(&self.calls).push(cmd.clone());
        let args: Vec<&str> = cmd.args.iter().map(String::as_str).collect();
        let line = format!("{} {}", cmd.name(), args.join(" "));
        if self.fail.iter().any(|f| line.starts_with(f)) {
            return Ok(Output {
                code: Some(1),
                stdout: String::new(),
                stderr: "boom".into(),
            });
        }
        let mounts = self.root.join("proc/self/mounts");
        let target = self.target.to_str().unwrap();
        match (cmd.name(), args.as_slice()) {
            ("mount", [.., dev, dir]) if dir.starts_with(target) && !dev.starts_with("remount") => {
                let mut m = fs::read_to_string(&mounts).unwrap();
                m.push_str(&format!("{dev} {dir} x rw 0 0\n"));
                fs::write(&mounts, m).unwrap();
                ok("")
            }
            ("umount", ["--recursive", dir]) => {
                let m: String = fs::read_to_string(&mounts)
                    .unwrap()
                    .lines()
                    .filter(|l| !l.split(' ').nth(1).unwrap().starts_with(dir))
                    .map(|l| format!("{l}\n"))
                    .collect();
                fs::write(&mounts, m).unwrap();
                ok("")
            }
            ("lsblk", _) => ok(LSBLK),
            ("sfdisk", ["--json", "/dev/sda"]) => match &*lock(&self.table) {
                Some(t) => ok(&serde_json::json!({ "partitiontable": t }).to_string()),
                None => Ok(Output {
                    code: Some(1),
                    stdout: String::new(),
                    stderr: "sfdisk: /dev/sda: does not contain a recognized partition table"
                        .into(),
                }),
            },
            ("sfdisk", a) => {
                let new = script_parts(cmd.stdin.as_deref().unwrap_or(""));
                let mut table = lock(&self.table);
                if a.contains(&"--append") {
                    let t = table.as_mut().unwrap();
                    t.partitions.extend(new);
                    if self.move_partition {
                        t.partitions[2].start += 2048;
                    }
                } else {
                    let mut t = Table::parse(SFDISK).unwrap();
                    t.id = Some("NEW-TABLE".into());
                    t.partitions = new;
                    *table = Some(t);
                }
                if !self.stale_kernel {
                    for p in &table.as_ref().unwrap().partitions {
                        let sys = self.root.join("sys/class/block").join(&p.node[5..]);
                        fs::create_dir_all(&sys).unwrap();
                        fs::write(sys.join("start"), format!("{}\n", p.start)).unwrap();
                        fs::write(sys.join("size"), format!("{}\n", p.size)).unwrap();
                    }
                }
                ok("")
            }
            ("bootc", _) => {
                for l in BOOTC_OUT.lines() {
                    on_line(Some(l));
                }
                on_line(None);
                let etc = self
                    .target
                    .join("ostree/deploy/default/deploy/abc123.0/etc");
                fs::create_dir_all(etc.join("selinux/targeted/contexts/files")).unwrap();
                if self.x11_symlink {
                    let host = self.root.join("etc/X11");
                    fs::create_dir_all(host.join("xorg.conf.d")).unwrap();
                    std::os::unix::fs::symlink(&host, etc.join("X11")).unwrap();
                }
                fs::write(
                    etc.join("selinux/targeted/contexts/files/file_contexts"),
                    "",
                )
                .unwrap();
                fs::write(
                    self.target
                        .join("ostree/deploy/default/deploy/abc123.0.origin"),
                    "",
                )
                .unwrap();
                ok(BOOTC_OUT)
            }
            ("efibootmgr", []) => {
                let t = lock(&self.table);
                let esp = t
                    .as_ref()
                    .and_then(|t| {
                        t.partitions
                            .iter()
                            .find(|p| p.kind.eq_ignore_ascii_case(gpt::ESP))
                    })
                    .and_then(|p| p.uuid.clone())
                    .unwrap_or_default()
                    .to_lowercase();
                ok(&format!(
                    "BootCurrent: 0002\nBootOrder: 0005,0002,0004\n\
                     Boot0002* UEFI QEMU DVD-ROM QM00003 \tPciRoot(0x0)/Pci(0x1f,0x2)/Sata(1,65535,0)\n\
                     Boot0004* Windows Boot Manager\tHD(1,GPT,{esp},0x800,0x32000)/\\EFI\\Microsoft\\Boot\\bootmgfw.efi\n\
                     Boot0005* Fedora\tHD(1,GPT,{esp},0x800,0x32000)/\\EFI\\fedora\\shimx64.efi\n"
                ))
            }
            ("mokutil", ["--test-key", _]) => ok(if self.enrolled {
                "x is already enrolled\n"
            } else {
                "x is not enrolled\n"
            }),
            ("mokutil", ["--generate-hash"]) => {
                /* As the real one does with the password on stdin. */
                let pw = cmd.stdin.as_deref().unwrap_or_default();
                let mut twice = pw.lines();
                assert_eq!(twice.next(), twice.next(), "the password, twice");
                ok("input password: \ninput password again: \n$6$salt$hash\n")
            }
            _ => ok(""),
        }
    }
}

impl Fake {
    fn new(env: &Env) -> Fake {
        Fake {
            table: Mutex::new(Some(Table::parse(SFDISK).unwrap())),
            root: env.root.clone(),
            target: env.target.clone(),
            ..Default::default()
        }
    }
}

struct World {
    _dir: tempfile::TempDir,
    env: Env,
    fake: Fake,
}

impl World {
    fn new() -> World {
        let dir = tempfile::tempdir().unwrap();
        let env = Env {
            root: dir.path().join("root"),
            target: dir.path().join("target"),
            run_dir: dir.path().join("run"),
        };
        let put = |rel: &str, text: &[u8]| {
            let p = env.root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        };
        put(
            "proc/self/mounts",
            b"/dev/sr0 /run/initramfs/live iso9660 ro 0 0\n",
        );
        put(
            "usr/share/systemd/kbd-model-map",
            b"de-latin1-nodeadkeys\tde\tpc105\tnodeadkeys\tterminate:ctrl_alt_bksp\n",
        );
        // what the read-only probe mount of Windows' ESP shows
        let boot = env.run_dir.join("esp-probe/EFI/Microsoft/Boot");
        fs::create_dir_all(&boot).unwrap();
        fs::write(boot.join("bootmgfw.efi"), "").unwrap();
        let fake = Fake::new(&env);
        World {
            _dir: dir,
            env,
            fake,
        }
    }

    fn put(&self, rel: &str, text: &[u8]) {
        let p = self.env.root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn install(&self, req: &Request) -> (Result<Outcome, String>, Vec<Progress>) {
        let mut seen = Vec::new();
        let r = install(&self.fake, &self.env, req, &mut |p| seen.push(p.clone()));
        (r, seen)
    }

    /// The calls as command lines, without the read-only probing ones.
    fn writes(&self) -> Vec<String> {
        lock(&self.fake.calls)
            .iter()
            .filter(|c| {
                let a = c.args.join(" ");
                !(c.name() == "lsblk"
                    || c.name() == "sfdisk" && a.starts_with("--json")
                    || c.name() == "mount" && a.starts_with("-o ro,")
                    || c.name() == "umount" && a.ends_with("esp-probe")
                    || c.name() == "mokutil" && a.starts_with("--test-key"))
            })
            .map(|c| {
                format!("{} {}", c.name(), c.args.join(" "))
                    .trim_end()
                    .to_string()
            })
            .map(|s| {
                s.replace(self.env.target.to_str().unwrap(), "T")
                    .replace(self.env.run_dir.to_str().unwrap(), "R")
            })
            .collect()
    }

    fn deploy(&self) -> PathBuf {
        self.env
            .target
            .join("ostree/deploy/default/deploy/abc123.0")
    }
}

fn req(disk: &str, mode: &str) -> Request {
    Request::new(disk, "", mode, "de_DE.UTF-8", "de(nodeadkeys)", "").unwrap()
}

#[test]
fn free_space_beside_windows() {
    let w = World::new();
    let (r, seen) = w.install(&req("sda", "free-space"));
    let out = r.unwrap();
    assert_eq!(
        w.writes(),
        [
            "sfdisk --quiet --wipe-partitions always --append /dev/sda",
            "udevadm settle --timeout=30",
            "udevadm wait --timeout=30 /dev/sda5 /dev/sda6",
            "wipefs --all --quiet /dev/sda5",
            "mkfs.ext4 -q -F -L boot /dev/sda5",
            "wipefs --all --quiet /dev/sda6",
            "mkfs.btrfs -q -f -L atlasos /dev/sda6",
            "mount -o compress=zstd:1 /dev/sda6 T",
            "mount /dev/sda5 T/boot",
            "mount /dev/sda1 T/boot/efi",
            "bootc install to-filesystem --source-imgref containers-storage:ghcr.io/eternalcoder454/atlasos:stable \
             --target-imgref ghcr.io/eternalcoder454/atlasos:stable --skip-fetch-check --karg rootflags=compress=zstd:1 T",
            "mount -o remount,rw T",
            "mount -o remount,rw T/boot",
            "setfiles -F -r T/ostree/deploy/default/deploy/abc123.0 \
             T/ostree/deploy/default/deploy/abc123.0/etc/selinux/targeted/contexts/files/file_contexts \
             T/ostree/deploy/default/deploy/abc123.0/etc/locale.conf \
             T/ostree/deploy/default/deploy/abc123.0/etc/vconsole.conf \
             T/ostree/deploy/default/deploy/abc123.0/etc/X11 \
             T/ostree/deploy/default/deploy/abc123.0/etc/X11/xorg.conf.d \
             T/ostree/deploy/default/deploy/abc123.0/etc/X11/xorg.conf.d/00-keyboard.conf",
            "setfiles -F -r T \
             T/ostree/deploy/default/deploy/abc123.0/etc/selinux/targeted/contexts/files/file_contexts \
             T/boot/grub2 \
             T/boot/grub2/custom.cfg",
            "efibootmgr",
            "efibootmgr --quiet --create --disk /dev/sda --part 1 --loader \\EFI\\fedora\\shimx64.efi --label AtlasOS",
            "efibootmgr --quiet --delete-bootnum --bootnum 0005",
            "umount --recursive T",
        ]
    );
    let script = lock(&w.fake.calls)
        .iter()
        .find(|c| c.name() == "sfdisk" && c.stdin.is_some())
        .unwrap()
        .stdin
        .clone();
    assert_eq!(
        script.unwrap().lines().count(),
        2,
        "only /boot and / are added"
    );

    let etc = w.deploy().join("etc");
    assert_eq!(
        fs::read_to_string(etc.join("locale.conf")).unwrap(),
        "LANG=de_DE.UTF-8\n"
    );
    assert!(
        fs::read_to_string(etc.join("vconsole.conf"))
            .unwrap()
            .starts_with("KEYMAP=de-latin1-nodeadkeys\n")
    );
    assert!(
        fs::read_to_string(etc.join("X11/xorg.conf.d/00-keyboard.conf"))
            .unwrap()
            .contains("\"nodeadkeys\"")
    );
    let cfg = fs::read_to_string(w.env.target.join("boot/grub2/custom.cfg")).unwrap();
    assert!(cfg.contains("--set=root 4A1B-2C3D\n"), "{cfg}");

    assert_eq!(
        out,
        Outcome {
            mok_password: None,
            windows_entry: true,
            boot_media: "cd".into(),
            warnings: vec![],
            log: w.env.log_path().display().to_string(),
        }
    );
    assert_eq!(seen.first().unwrap().stage, Stage::Prepare);
    assert!(
        seen.windows(2).all(|p| p[0].fraction <= p[1].fraction),
        "{seen:?}"
    );
    assert_eq!(seen.last().unwrap().fraction, 1.0);
    assert!(seen.iter().any(|p| p.stage == Stage::Bootloader));

    let log = fs::read_to_string(w.env.log_path()).unwrap();
    assert!(log.contains("$ /usr/bin/bootc install to-filesystem"));
    assert!(log.contains("Installation complete!"));
}

#[test]
fn erase_wipes_everything_and_drops_the_windows_entry() {
    let w = World::new();
    let out = w.install(&req("sda", "erase")).0.unwrap();
    let writes = w.writes();
    assert_eq!(
        writes[..6],
        [
            "wipefs --all --quiet /dev/sda1",
            "wipefs --all --quiet /dev/sda2",
            "wipefs --all --quiet /dev/sda3",
            "wipefs --all --quiet /dev/sda4",
            "wipefs --all --quiet /dev/sda",
            "sfdisk --quiet --wipe-partitions always --wipe always /dev/sda",
        ]
    );
    assert!(writes.contains(&"mkfs.vfat -F 32 -n EFI /dev/sda1".to_string()));
    assert!(writes.contains(&"mount /dev/sda1 T/boot/efi".to_string()));
    assert!(writes.contains(&"efibootmgr --quiet --create --disk /dev/sda --part 1 --loader \\EFI\\fedora\\shimx64.efi --label AtlasOS".to_string()));
    assert!(!out.windows_entry, "Windows was on the erased disk");
    assert!(!w.env.target.join("boot/grub2/custom.cfg").exists());
}

#[test]
fn a_moved_partition_stops_before_anything_is_formatted() {
    let mut w = World::new();
    w.fake.move_partition = true;
    let e = w.install(&req("sda", "free-space")).0.unwrap_err();
    assert!(
        e.starts_with("Stopped before formatting anything: /dev/sda3 moved"),
        "{e}"
    );
    let writes = w.writes();
    assert!(
        writes
            .iter()
            .all(|c| !c.starts_with("mkfs") && !c.starts_with("wipefs") && !c.starts_with("bootc")),
        "{writes:?}"
    );
    assert!(
        fs::read_to_string(w.env.log_path())
            .unwrap()
            .contains("FAILED: Stopped before formatting")
    );
}

#[test]
fn the_boot_media_and_unknown_disks_are_refused_before_any_write() {
    let w = World::new();
    for (disk, want) in [
        ("sr0", "the installer's boot media"),
        ("sdb", "there is no disk"),
    ] {
        let e = w.install(&req(disk, "erase")).0.unwrap_err();
        assert!(e.contains(want), "{e}");
    }
    assert!(w.writes().is_empty(), "{:?}", w.writes());
}

#[test]
fn nvidia_with_secure_boot_queues_the_key() {
    let w = World::new();
    w.put(NVIDIA_KEY, b"der");
    w.put(SECURE_BOOT_VAR, &[6, 0, 0, 0, 1]);
    let out = w.install(&req("sda", "free-space")).0.unwrap();
    let pw = out.mok_password.expect("queued");
    assert!(
        pw.len() == 8 && pw.bytes().all(|b| b.is_ascii_digit()),
        "{pw}"
    );
    let calls = lock(&w.fake.calls);
    let bootc = calls.iter().find(|c| c.name() == "bootc").unwrap();
    assert!(
        bootc
            .args
            .contains(&"ghcr.io/eternalcoder454/atlasos-nvidia:stable".to_string())
    );
    let import = calls
        .iter()
        .find(|c| c.args.first().map(String::as_str) == Some("--import"))
        .unwrap();
    assert!(import.args[1].ends_with(NVIDIA_KEY));
    assert!(
        !w.env.run_dir.join("mok.hash").exists(),
        "the hash file is removed"
    );
    assert!(
        !fs::read_to_string(w.env.log_path()).unwrap().contains(&pw),
        "the password is not logged"
    );
}

#[test]
fn no_key_without_secure_boot_or_when_enrolled() {
    let w = World::new();
    w.put(NVIDIA_KEY, b"der");
    w.put(SECURE_BOOT_VAR, &[6, 0, 0, 0, 0]);
    assert_eq!(
        w.install(&req("sda", "free-space")).0.unwrap().mok_password,
        None
    );

    let mut w = World::new();
    w.fake.enrolled = true;
    w.put(NVIDIA_KEY, b"der");
    w.put(SECURE_BOOT_VAR, &[6, 0, 0, 0, 1]);
    assert_eq!(
        w.install(&req("sda", "free-space")).0.unwrap().mok_password,
        None
    );
}

#[test]
fn wifi_is_carried_over_without_the_live_user() {
    let w = World::new();
    let kf = format!(
        "[connection]\nid=Home\nuuid={UUID}\ntype=wifi\npermissions=user:atlas-installer;\n\n[wifi-security]\npsk=secret\n"
    );
    w.put(
        "etc/NetworkManager/system-connections/Home.nmconnection",
        kf.as_bytes(),
    );
    w.put(
        "etc/NetworkManager/system-connections/Other.nmconnection",
        b"[connection]\nuuid=11111111-2a0c-4d5e-9f1a-3c2b1a0d9e8f\n",
    );
    let r = Request::new("sda", "", "free-space", "en_US.UTF-8", "us", UUID).unwrap();
    w.install(&r).0.unwrap();
    let copy = w
        .deploy()
        .join("etc/NetworkManager/system-connections/Home.nmconnection");
    let text = fs::read_to_string(&copy).unwrap();
    assert!(text.contains("psk=secret") && !text.contains("permissions="));
    assert_eq!(
        fs::metadata(&copy).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        !w.deploy()
            .join("etc/NetworkManager/system-connections/Other.nmconnection")
            .exists()
    );
}

#[test]
fn a_missing_wifi_connection_fails_before_any_write() {
    let w = World::new();
    let r = Request::new("sda", "", "free-space", "en_US.UTF-8", "us", UUID).unwrap();
    assert!(w.install(&r).0.unwrap_err().contains("Wi-Fi"));
    assert!(w.writes().is_empty());
}

#[test]
fn a_failed_bootloader_rename_is_only_a_warning() {
    struct NoEntries(Fake);
    impl Runner for NoEntries {
        fn run(&self, cmd: &Cmd, on: &mut dyn FnMut(Option<&str>)) -> Result<Output, String> {
            if cmd.name() == "efibootmgr" {
                lock(&self.0.calls).push(cmd.clone());
                return ok("BootOrder: 0001\n");
            }
            self.0.run(cmd, on)
        }
    }
    let w = World::new();
    let f = NoEntries(Fake::new(&w.env));
    let out = install(&f, &w.env, &req("sda", "free-space"), &mut |_| {}).unwrap();
    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].contains("still called Fedora"));
    let calls = lock(&f.0.calls);
    assert!(
        !calls
            .iter()
            .any(|c| c.args.contains(&"--delete-bootnum".to_string()))
    );
}

#[test]
fn requests_are_checked() {
    assert!(Request::new("nvme0n1", "", "erase", "en_US.UTF-8", "us", "").is_ok());
    assert!(
        Request::new(
            "nvme0n1",
            "0123456789abcdef",
            "erase",
            "en_US.UTF-8",
            "us",
            ""
        )
        .is_ok()
    );
    for f in [
        "0123",
        "0123456789ABCDEF",
        "0123456789abcdeg",
        "../../../../etc/x",
    ] {
        assert!(
            Request::new("sda", f, "erase", "en_US.UTF-8", "us", "").is_err(),
            "{f}"
        );
    }
    for (d, m, l, k, u) in [
        ("../sda", "erase", "en_US.UTF-8", "us", ""),
        ("/dev/sda", "erase", "en_US.UTF-8", "us", ""),
        ("", "erase", "en_US.UTF-8", "us", ""),
        ("sda", "wipe", "en_US.UTF-8", "us", ""),
        ("sda", "erase", "en_US", "us", ""),
        ("sda", "erase", "en_US.UTF-8", "us\"; x", ""),
        ("sda", "erase", "en_US.UTF-8", "us", "home"),
    ] {
        assert!(
            Request::new(d, "", m, l, k, u).is_err(),
            "{d} {m} {l} {k} {u}"
        );
    }
}

#[test]
fn deployment_must_be_unique() {
    let d = tempfile::tempdir().unwrap();
    assert!(find_deployment(d.path()).is_err());
    fs::create_dir_all(d.path().join("ostree/deploy/default/deploy/a.0")).unwrap();
    assert_eq!(
        find_deployment(d.path()).unwrap(),
        d.path().join("ostree/deploy/default/deploy/a.0")
    );
    fs::create_dir_all(d.path().join("ostree/deploy/default/deploy/b.0")).unwrap();
    assert!(find_deployment(d.path()).is_err());
}

#[test]
fn mok_passwords_are_eight_digits() {
    for _ in 0..50 {
        let p = mok_password().unwrap();
        assert!(p.len() == 8 && p.bytes().all(|b| b.is_ascii_digit()));
    }
    assert_ne!(mok_password().unwrap(), mok_password().unwrap());
}

#[test]
fn mok_hash_is_the_last_line() {
    // mokutil 0.7.2 with the password on stdin, as seen in the live session.
    let out = "input password: \ninput password again: \n$6$7NHcM/2BNnGR$x2o0NVAg\n";
    assert_eq!(mok_hash(out), Some("$6$7NHcM/2BNnGR$x2o0NVAg"));
    assert_eq!(mok_hash("$6$a$b"), Some("$6$a$b"));
    assert_eq!(
        mok_hash("input password: \npassword doesn't match\nAbort\n"),
        None
    );
    assert_eq!(mok_hash(""), None);
}

fn fingerprint_of(w: &World, disk: &str) -> String {
    let l = list_disks(&w.fake, &w.env).unwrap();
    l.disks
        .into_iter()
        .find(|d| d.id == disk)
        .unwrap()
        .fingerprint
}

#[test]
fn the_disk_must_be_the_one_that_was_listed() {
    let w = World::new();
    let f = fingerprint_of(&w, "sda");
    let other = Request::new("sda", "0000000000000000", "erase", "en_US.UTF-8", "us", "").unwrap();
    let e = w.install(&other).0.unwrap_err();
    assert!(e.contains("changed since the disks were listed"), "{e}");
    assert!(w.writes().is_empty(), "{:?}", w.writes());
    let same = Request::new("sda", &f, "erase", "en_US.UTF-8", "us", "").unwrap();
    w.install(&same).0.unwrap();
}

#[test]
fn a_stale_kernel_table_stops_before_anything_is_formatted() {
    let mut w = World::new();
    w.fake.stale_kernel = true;
    let e = w.install(&req("sda", "free-space")).0.unwrap_err();
    assert!(
        e.starts_with("Stopped before formatting anything: the kernel doesn't see /dev/sda5"),
        "{e}"
    );
    assert!(
        !w.writes()
            .iter()
            .any(|c| c.starts_with("mkfs") || c.starts_with("wipefs"))
    );
}

#[test]
fn without_the_boot_media_nothing_is_installed() {
    let w = World::new();
    w.put("proc/self/mounts", b"proc /proc proc rw 0 0\n");
    let e = w.install(&req("sda", "erase")).0.unwrap_err();
    assert!(e.contains("boot media was not found"), "{e}");
    assert!(w.writes().is_empty());
}

#[test]
fn symlinks_in_the_new_system_are_not_followed_and_mounts_are_cleaned_up() {
    let mut w = World::new();
    w.fake.x11_symlink = true;
    let e = w.install(&req("sda", "free-space")).0.unwrap_err();
    assert!(e.contains("is not a directory"), "{e}");
    assert!(
        !w.env
            .root
            .join("etc/X11/xorg.conf.d/00-keyboard.conf")
            .exists()
    );
    assert_eq!(w.writes().last().unwrap(), "umount --recursive T");
    let mounts = fs::read_to_string(w.env.root.join("proc/self/mounts")).unwrap();
    assert!(!mounts.contains(w.env.target.to_str().unwrap()), "{mounts}");
}

#[test]
fn late_failures_are_warnings_on_a_finished_install() {
    let mut w = World::new();
    w.fake.fail = vec![
        "setfiles",
        "umount --recursive",
        "efibootmgr --quiet --delete-bootnum",
    ];
    let out = w.install(&req("sda", "free-space")).0.unwrap();
    let all = out.warnings.join("\n");
    assert_eq!(out.warnings.len(), 4, "{all}");
    assert!(all.contains("SELinux label"), "{all}");
    assert!(all.contains("Boot0005 could not be removed"), "{all}");
    assert!(all.contains("could not be unmounted"), "{all}");
    assert!(w.writes().iter().any(|c| c.ends_with("--label AtlasOS")));
}

#[test]
fn another_fedoras_esp_is_not_shared() {
    let w = World::new();
    fs::create_dir_all(w.env.run_dir.join("esp-probe/EFI/fedora")).unwrap();
    w.install(&req("sda", "free-space")).0.unwrap();
    let writes = w.writes();
    assert!(
        writes.contains(&"mkfs.vfat -F 32 -n EFI /dev/sda5".to_string()),
        "{writes:?}"
    );
    assert!(
        writes.contains(&"mount /dev/sda5 T/boot/efi".to_string()),
        "{writes:?}"
    );
}

#[test]
fn an_esp_that_stays_mounted_stops_the_probe() {
    let mut w = World::new();
    w.fake.fail = vec!["umount /"];
    let e = w.install(&req("sda", "free-space")).0.unwrap_err();
    assert!(e.contains("cannot release /dev/sda1"), "{e}");
    assert!(
        lock(&w.fake.calls)
            .iter()
            .any(|c| c.args.first().map(String::as_str) == Some("--lazy"))
    );
}

#[tokio::test]
async fn status_follows_a_whole_install_and_keeps_the_result() {
    use crate::service::Service;
    let w = World::new();
    w.put(NVIDIA_KEY, b"der");
    w.put(SECURE_BOOT_VAR, &[6, 0, 0, 0, 1]);
    let World { _dir, env, fake } = w;
    let s = Service::new(std::sync::Arc::new(fake), env);
    let status = |secret| -> serde_json::Value {
        serde_json::from_str(&s.do_status(secret).unwrap()).unwrap()
    };
    assert_eq!(status(true)["state"], "idle");
    let mut steps = Vec::new();
    let json = s
        .do_install(req("sda", "free-space"), async { None }, |step, f, _| {
            steps.push((step, f))
        })
        .await
        .unwrap();
    assert!(!steps.is_empty());
    let st = status(true);
    assert_eq!(st["state"], "done");
    assert_eq!(st["fraction"], 1.0);
    assert_eq!(st["step"], steps.last().unwrap().0);
    // the result is what Install returned, MOK password included
    let returned: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(st["result"], returned);
    let pw = returned["mok_password"].as_str().unwrap().to_string();
    assert_eq!(pw.len(), 8);
    // and it stays, unchanged, for later callers; one that may not install
    // gets it without the password
    assert_eq!(status(true)["result"], returned);
    let hidden = s.do_status(false).unwrap();
    assert!(!hidden.contains(&pw));
    assert_eq!(status(false)["state"], "done");
}
