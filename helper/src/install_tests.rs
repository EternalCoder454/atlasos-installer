//! The install sequence against a fake system: a fake runner that keeps a
//! partition table, and a temporary directory for the host and the target.

use installer_core::table::Partition;

use super::*;

const LSBLK: &str = include_str!("../../crates/installer-core/tests/fixtures/lsblk-windows.json");
const SFDISK: &str = include_str!("../../crates/installer-core/tests/fixtures/sfdisk-windows.json");
const RECOVERY_KEY: &str =
    "ulcbjnni-ehtlcfnl-ntenkltt-vjuiicdf-hvdkerji-fjkurjhr-lckjntdb-kvkeeide";
const GRUB_HASH: &str = "grub.pbkdf2.sha512.10000.A81F1971A0042577.12F7519BDEA05552";
const PASSWORD: &str = "correct horse battery";
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
    /// /proc/self/mounts as bootc started.
    mounts_at_bootc: Mutex<String>,
    /// systemd-cryptenroll prints something that is not a recovery key.
    bad_recovery: bool,
    /// The encrypted root that `cryptsetup open` made and nothing closed
    /// yet: (partition, mapper name).
    crypt_open: Mutex<Option<(String, String)>>,
    /// What `restart_now` was asked to unmount, once called (the real one
    /// restarts the computer).
    restarted: Mutex<Option<Vec<String>>>,
    /// Another disk shows up under the same name from the n-th lsblk call on.
    swap_disk_at_lsblk: Option<usize>,
    lsblk_calls: Mutex<usize>,
    /// The partition that held an open map: it shows as LUKS when closed too.
    luks_part: Mutex<Option<String>>,
    /// `cryptsetup close` fails as busy and `--deferred` "succeeds" without
    /// closing the map.
    close_only_deferred: bool,
}

fn ok(stdout: &str) -> Result<Output, String> {
    Ok(Output {
        code: Some(0),
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

/// lsblk's answer with an open encrypted root on `part`.
fn lsblk_with_crypt(part: &str, mapper: &str) -> String {
    let mut v: serde_json::Value = serde_json::from_str(LSBLK).unwrap();
    let disk = &mut v["blockdevices"][0];
    let children = disk["children"].as_array_mut().unwrap();
    let at = children.iter().position(|c| c["name"] == part).unwrap_or(2); // a leftover from before the new table
    children[at]["fstype"] = "crypto_LUKS".into();
    children[at]["label"] = "atlasos".into();
    children[at]["children"] = serde_json::json!([{
        "name": format!("/dev/mapper/{mapper}"), "kname": "/dev/dm-0", "type": "crypt",
        "size": 1, "mountpoints": [null],
    }]);
    v.to_string()
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
    fn restart_now(&self, unmount: &[String]) -> Result<(), String> {
        *lock(&self.restarted) = Some(unmount.to_vec());
        Ok(())
    }

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
        if args.first() == Some(&"luksFormat") {
            // the temporary key is a private file in a private directory
            use std::os::unix::fs::MetadataExt;
            let i = args.iter().position(|a| *a == "--key-file").unwrap();
            let key = Path::new(args[i + 1]);
            let m = fs::symlink_metadata(key).unwrap();
            assert!(m.file_type().is_file());
            assert_eq!((m.mode() & 0o7777, m.len()), (0o600, 64));
            let d = fs::metadata(key.parent().unwrap()).unwrap();
            assert_eq!(d.mode() & 0o7777, 0o700);
        }
        match (cmd.name(), args.as_slice()) {
            ("mount", [.., dev, dir]) if dir.starts_with(target) && !dev.starts_with("remount") => {
                let mut m = fs::read_to_string(&mounts).unwrap();
                m.push_str(&format!("{dev} {dir} x rw 0 0\n"));
                fs::write(&mounts, m).unwrap();
                ok("")
            }
            ("umount", ["--recursive", dir]) | ("umount", ["--recursive", "--lazy", dir]) => {
                let m: String = fs::read_to_string(&mounts)
                    .unwrap()
                    .lines()
                    .filter(|l| !l.split(' ').nth(1).unwrap().starts_with(dir))
                    .map(|l| format!("{l}\n"))
                    .collect();
                fs::write(&mounts, m).unwrap();
                ok("")
            }
            ("umount", [dir]) | ("umount", ["--lazy", dir]) => {
                let m: String = fs::read_to_string(&mounts)
                    .unwrap()
                    .lines()
                    .filter(|l| l.split(' ').nth(1).unwrap() != *dir)
                    .map(|l| format!("{l}\n"))
                    .collect();
                fs::write(&mounts, m).unwrap();
                ok("")
            }
            ("findmnt", ["--json", "--list", "--output", "TARGET,SOURCE"]) => {
                let fs_list: Vec<_> = fs::read_to_string(&mounts)
                    .unwrap()
                    .lines()
                    .map(|l| {
                        let mut f = l.split(' ');
                        let (src, dir) = (f.next().unwrap(), f.next().unwrap());
                        serde_json::json!({"target": dir, "source": src})
                    })
                    .collect();
                ok(&serde_json::json!({ "filesystems": fs_list }).to_string())
            }
            ("lsblk", _) => {
                let n = {
                    let mut c = lock(&self.lsblk_calls);
                    *c += 1;
                    *c
                };
                if self.swap_disk_at_lsblk.is_some_and(|at| n >= at) {
                    return ok(&LSBLK.replace("QEMU HARDDISK", "SOMEONE ELSES DISK"));
                }
                let open = lock(&self.crypt_open).clone();
                let mut seen = lock(&self.luks_part);
                if let Some((part, _)) = &open {
                    *seen = Some(part.clone());
                }
                ok(&match (open, seen.as_deref()) {
                    (Some((part, mapper)), _) => lsblk_with_crypt(&part, &mapper),
                    (None, Some(part)) => {
                        let mut v: serde_json::Value =
                            serde_json::from_str(&lsblk_with_crypt(part, "x")).unwrap();
                        let at = v["blockdevices"][0]["children"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .position(|c| c["fstype"] == "crypto_LUKS")
                            .unwrap();
                        v["blockdevices"][0]["children"][at]["children"] = serde_json::json!([]);
                        v.to_string()
                    }
                    (None, None) => LSBLK.to_string(),
                })
            }
            ("cryptsetup", ["open", "--allow-discards", "--key-file", _, part, mapper]) => {
                *lock(&self.crypt_open) = Some((part.to_string(), mapper.to_string()));
                ok("")
            }
            ("cryptsetup", ["close", "--deferred", mapper]) => {
                // closed once the last user is gone: here, at once
                if self.close_only_deferred {
                    return ok("");
                }
                let mut open = lock(&self.crypt_open);
                if open.as_ref().is_some_and(|(_, m)| m == mapper) {
                    *open = None;
                }
                ok("")
            }
            ("cryptsetup", ["close", _]) if self.close_only_deferred => Ok(Output {
                code: Some(5),
                stdout: String::new(),
                stderr: "Device is still in use.".into(),
            }),
            ("cryptsetup", ["close", mapper]) => {
                let m = fs::read_to_string(&mounts).unwrap();
                if m.contains(&format!("/dev/mapper/{mapper} ")) {
                    return Ok(Output {
                        code: Some(5),
                        stdout: String::new(),
                        stderr: format!("Device {mapper} is still in use."),
                    });
                }
                let mut open = lock(&self.crypt_open);
                if open.as_ref().is_some_and(|(_, m)| m == mapper) {
                    *open = None;
                }
                ok("")
            }
            ("systemd-cryptenroll", [_, "--recovery-key", _]) => {
                // as on a pipe: the key alone on stdout, the banner on stderr
                Ok(Output {
                    code: Some(0),
                    stdout: if self.bad_recovery {
                        "A secret recovery key has been generated\n".into()
                    } else {
                        format!("{RECOVERY_KEY}\n")
                    },
                    stderr: "A secret recovery key has been generated for this volume:\n".into(),
                })
            }
            ("cryptsetup", ["open", "--test-passphrase", rest @ ..])
                if rest.contains(&"--key-file") =>
            {
                // As cryptsetup 2.8 does: with a TPM token enrolled, an
                // open that may use tokens succeeds whatever key it got.
                let calls = lock(&self.calls);
                let tpm = calls
                    .iter()
                    .any(|c| c.args.iter().any(|a| a == "--tpm2-device=auto"));
                if tpm && !rest.contains(&"--disable-external-tokens") {
                    return ok("");
                }
                let i = rest.iter().position(|a| *a == "--key-file").unwrap();
                let right = if rest[i + 1] == "-" {
                    let given = cmd.stdin.as_deref().unwrap_or_default();
                    given == RECOVERY_KEY || given == PASSWORD
                } else {
                    // the temporary key, which luksRemoveKey made useless
                    !calls
                        .iter()
                        .any(|c| c.args.first().map(String::as_str) == Some("luksRemoveKey"))
                };
                Ok(Output {
                    code: Some(if right { 0 } else { 2 }),
                    ..Default::default()
                })
            }
            ("grub2-mkpasswd-pbkdf2", []) => {
                let given = cmd.stdin.as_deref().unwrap_or_default();
                let mut twice = given.lines();
                let pw = twice.next().unwrap_or_default();
                assert!(pw.len() == 64 && pw.bytes().all(|b| b.is_ascii_hexdigit()));
                assert_eq!(Some(pw), twice.next(), "twice");
                ok(&format!(
                    "Enter password: \nReenter password: \nPBKDF2 hash of your password is {GRUB_HASH}\n"
                ))
            }
            ("cryptsetup", ["luksDump", "--dump-json-metadata", _]) => {
                let calls = lock(&self.calls);
                let had = |f: &dyn Fn(&Cmd) -> bool| calls.iter().any(f);
                let removed = had(&|c| c.args.first().map(String::as_str) == Some("luksRemoveKey"));
                let tpm = had(&|c| c.args.iter().any(|a| a == "--tpm2-device=auto"));
                let slots = if removed { 2 } else { 3 };
                // the temporary key is slot 0
                let keyslots: serde_json::Map<String, serde_json::Value> = (3 - slots..3)
                    .map(|i| (i.to_string(), serde_json::json!({"type": "luks2"})))
                    .collect();
                let mut tokens = serde_json::json!({
                    "1": {"type": "systemd-recovery", "keyslots": ["1"]}
                });
                if tpm {
                    tokens["0"] = serde_json::json!({"type": "systemd-tpm2", "keyslots": ["2"]});
                }
                ok(&serde_json::json!({"keyslots": keyslots, "tokens": tokens}).to_string())
            }
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
                *lock(&self.mounts_at_bootc) = fs::read_to_string(&mounts).unwrap();
                // as the real one: the target stays mounted at its own place
                let root_dev = fs::read_to_string(&mounts)
                    .unwrap()
                    .lines()
                    .find(|l| l.split(' ').nth(1) == Some(target))
                    .and_then(|l| l.split(' ').next().map(str::to_string));
                if let Some(dev) = root_dev {
                    let mut m = fs::read_to_string(&mounts).unwrap();
                    m.push_str(&format!("{dev}[/root] /run/bootc/storage x rw 0 0\n"));
                    fs::write(&mounts, m).unwrap();
                }
                for l in BOOTC_OUT.lines() {
                    on_line(Some(l));
                    if l.starts_with("layers already present") {
                        // time passes during the import
                        on_line(None);
                    }
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
                /* Once the helper has made its entry, as the firmware lists it. */
                let created = lock(&self.calls)
                    .iter()
                    .any(|c| c.name() == "efibootmgr" && c.args.iter().any(|a| a == "--create"));
                let (first, ours) = if created {
                    (
                        "0006,",
                        format!(
                            "Boot0006* AtlasOS\tHD(1,GPT,{esp},0x800,0x32000)/\\EFI\\fedora\\shimx64.efi\n"
                        ),
                    )
                } else {
                    ("", String::new())
                };
                ok(&format!(
                    "BootCurrent: 0002\nBootOrder: {first}0005,0002,0004\n{ours}\
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
        put("dev/sr0", b"");
        for tool in [bin::CRYPTSETUP, bin::CRYPTENROLL, bin::GRUB2_MKPASSWD] {
            let rel = tool.trim_start_matches('/');
            put(rel, b"");
            fs::set_permissions(env.root.join(rel), fs::Permissions::from_mode(0o755)).unwrap();
        }
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
                    || c.name() == "findmnt"
                    || c.name() == "sfdisk" && a.starts_with("--json")
                    || c.name() == "mount" && a.starts_with("-o ro,")
                    || c.name() == "umount" && a.ends_with("esp-probe")
                    || c.name() == "mokutil" && a.starts_with("--test-key")
                    || c.name() == "podman" && a.starts_with("image inspect"))
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
    Request::new(
        disk,
        "",
        mode,
        "de_DE.UTF-8",
        "de(nodeadkeys)",
        "",
        "none",
        "",
    )
    .unwrap()
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
             T/ostree/deploy/default/deploy/abc123.0/etc/X11/xorg.conf.d/00-keyboard.conf \
             T/ostree/deploy/default/deploy/abc123.0/etc/atlasos \
             T/ostree/deploy/default/deploy/abc123.0/etc/atlasos/installer.ini",
            "setfiles -F -r T \
             T/ostree/deploy/default/deploy/abc123.0/etc/selinux/targeted/contexts/files/file_contexts \
             T/boot/grub2 \
             T/boot/grub2/custom.cfg",
            "efibootmgr",
            "efibootmgr --quiet --create --disk /dev/sda --part 1 --loader \\EFI\\fedora\\shimx64.efi --label AtlasOS",
            "efibootmgr",
            "efibootmgr --quiet --bootnext 0006",
            "efibootmgr --quiet --delete-bootnum --bootnum 0005",
            "umount --recursive T",
            "umount --recursive /run/bootc/storage",
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
    let ini = fs::read_to_string(etc.join("atlasos/installer.ini")).unwrap();
    assert!(
        ini.ends_with(
            "Language=de_DE.UTF-8\nKeyboardLayout=de\nKeyboardVariant=nodeadkeys\nNetwork=false\n"
        ),
        "{ini}"
    );
    let cfg = fs::read_to_string(w.env.target.join("boot/grub2/custom.cfg")).unwrap();
    assert!(cfg.contains("--set=root 4A1B-2C3D\n"), "{cfg}");

    assert_eq!(
        out,
        Outcome {
            mok_password: None,
            recovery_key: None,
            encryption: "none".into(),
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
    let pw = out.mok_password.expect("queued").as_str().to_string();
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
    let r = Request::new(
        "sda",
        "",
        "free-space",
        "en_US.UTF-8",
        "us",
        UUID,
        "none",
        "",
    )
    .unwrap();
    w.install(&r).0.unwrap();
    let copy = w
        .deploy()
        .join("etc/NetworkManager/system-connections/Home.nmconnection");
    let text = fs::read_to_string(&copy).unwrap();
    assert!(text.contains("psk=secret") && !text.contains("permissions="));
    let ini = fs::read_to_string(w.deploy().join("etc/atlasos/installer.ini")).unwrap();
    assert!(ini.contains("\nNetwork=true\n"), "{ini}");
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
fn the_wizard_is_told_about_the_network() {
    let network = |w: &World| {
        let ini = fs::read_to_string(w.deploy().join("etc/atlasos/installer.ini")).unwrap();
        ini.lines()
            .find_map(|l| l.strip_prefix("Network="))
            .unwrap()
            .to_string()
    };
    // a cable in a real Ethernet device
    let w = World::new();
    w.put("sys/class/net/enp3s0/type", b"1\n");
    w.put("sys/class/net/enp3s0/carrier", b"1\n");
    w.put("sys/class/net/enp3s0/device/vendor", b"0x8086\n");
    w.install(&req("sda", "free-space")).0.unwrap();
    assert_eq!(network(&w), "true");

    // no cable, a Wi-Fi card, and virtual Ethernet: nothing to carry
    let w = World::new();
    w.put("sys/class/net/enp3s0/type", b"1\n");
    w.put("sys/class/net/enp3s0/carrier", b"0\n");
    w.put("sys/class/net/enp3s0/device/vendor", b"0x8086\n");
    w.put("sys/class/net/wlan0/type", b"1\n");
    w.put("sys/class/net/wlan0/carrier", b"1\n");
    w.put("sys/class/net/wlan0/device/vendor", b"0x8086\n");
    w.put("sys/class/net/wlan0/wireless/x", b"");
    w.put("sys/class/net/veth0/type", b"1\n");
    w.put("sys/class/net/veth0/carrier", b"1\n");
    w.put("sys/class/net/lo/type", b"772\n");
    w.put("sys/class/net/lo/carrier", b"1\n");
    w.install(&req("sda", "free-space")).0.unwrap();
    assert_eq!(network(&w), "false");

    // carrier can't be read (empty, as EINVAL gives): operstate decides
    for (state, want) in [("up\n", "true"), ("down\n", "false")] {
        let w = World::new();
        w.put("sys/class/net/enp3s0/type", b"1\n");
        w.put("sys/class/net/enp3s0/carrier", b"");
        w.put("sys/class/net/enp3s0/operstate", state.as_bytes());
        w.put("sys/class/net/enp3s0/device/vendor", b"0x8086\n");
        w.install(&req("sda", "free-space")).0.unwrap();
        assert_eq!(network(&w), want);
        let log = fs::read_to_string(w.env.log_path()).unwrap();
        assert!(log.contains("network enp3s0"), "{log}");
    }
    // a carrier of 0 is not overruled by operstate
    let w = World::new();
    w.put("sys/class/net/enp3s0/type", b"1\n");
    w.put("sys/class/net/enp3s0/carrier", b"0\n");
    w.put("sys/class/net/enp3s0/operstate", b"up\n");
    w.put("sys/class/net/enp3s0/device/vendor", b"0x8086\n");
    w.install(&req("sda", "free-space")).0.unwrap();
    assert_eq!(network(&w), "false");
}

#[test]
fn a_missing_wifi_connection_fails_before_any_write() {
    let w = World::new();
    let r = Request::new(
        "sda",
        "",
        "free-space",
        "en_US.UTF-8",
        "us",
        UUID,
        "none",
        "",
    )
    .unwrap();
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
    assert!(Request::new("nvme0n1", "", "erase", "en_US.UTF-8", "us", "", "none", "").is_ok());
    assert!(
        Request::new(
            "nvme0n1",
            "0123456789abcdef",
            "erase",
            "en_US.UTF-8",
            "us",
            "",
            "none",
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
            Request::new("sda", f, "erase", "en_US.UTF-8", "us", "", "none", "").is_err(),
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
            Request::new(d, "", m, l, k, u, "none", "").is_err(),
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
        .disks
        .into_iter()
        .find(|d| d.id == disk)
        .unwrap()
        .fingerprint
}

#[test]
fn the_disk_must_be_the_one_that_was_listed() {
    let w = World::new();
    let f = fingerprint_of(&w, "sda");
    let other = Request::new(
        "sda",
        "0000000000000000",
        "erase",
        "en_US.UTF-8",
        "us",
        "",
        "none",
        "",
    )
    .unwrap();
    let e = w.install(&other).0.unwrap_err();
    assert!(e.contains("changed since the disks were listed"), "{e}");
    assert!(w.writes().is_empty(), "{:?}", w.writes());
    let same = Request::new("sda", &f, "erase", "en_US.UTF-8", "us", "", "none", "").unwrap();
    w.install(&same).0.unwrap();
}

#[test]
fn a_disk_swapped_after_prepare_is_caught_before_the_first_write() {
    let mut w = World::new();
    // 1: prepare's probe, 2: the first recheck, 3: unmount_target's
    // listing, 4: the second recheck
    w.fake.swap_disk_at_lsblk = Some(3);
    let f = fingerprint_of(&w, "sda");
    *lock(&w.fake.lsblk_calls) = 0;
    let r = Request::new("sda", &f, "erase", "en_US.UTF-8", "us", "", "none", "").unwrap();
    let e = w.install(&r).0.unwrap_err();
    assert!(e.contains("changed since it was checked"), "{e}");
    assert!(
        !w.writes().iter().any(|c| {
            c.starts_with("wipefs") || c.starts_with("sfdisk") || c.starts_with("mkfs")
        }),
        "{:?}",
        w.writes()
    );
}

#[test]
fn a_disk_swapped_before_the_first_check_is_left_alone() {
    let mut w = World::new();
    // 1: prepare's probe, 2: the first recheck
    w.fake.swap_disk_at_lsblk = Some(2);
    let f = fingerprint_of(&w, "sda");
    *lock(&w.fake.lsblk_calls) = 0;
    let r = Request::new("sda", &f, "erase", "en_US.UTF-8", "us", "", "none", "").unwrap();
    let e = w.install(&r).0.unwrap_err();
    assert!(e.contains("changed since it was checked"), "{e}");
    let w_calls = w.writes();
    assert!(
        !w_calls
            .iter()
            .any(|c| c.starts_with("umount") || c.starts_with("cryptsetup")),
        "{w_calls:?}"
    );
    // nothing, not even the cleanup's disk listing, ran after the recheck
    assert_eq!(*lock(&w.fake.lsblk_calls), 2);
}

#[test]
fn a_partition_table_that_cant_be_read_is_not_called_a_replug() {
    let w = World::new();
    let prepared = prepare(&w.fake, &w.env, &req("sda", "free-space")).unwrap();
    // sfdisk finds no table any more, while lsblk still shows one
    *lock(&w.fake.table) = None;
    let e = recheck_disk(&w.fake, &prepared).unwrap_err();
    assert!(
        e.contains("Could not read the partition table on /dev/sda"),
        "{e}"
    );
    assert!(!e.contains("replugged"), "{e}");
}

#[test]
fn a_table_that_never_could_be_read_can_still_be_erased() {
    let w = World::new();
    // listed that way: sfdisk can't read it, though lsblk sees a table
    *lock(&w.fake.table) = None;
    let prepared = prepare(&w.fake, &w.env, &req("sda", "erase")).unwrap();
    recheck_disk(&w.fake, &prepared).unwrap();
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
    assert_eq!(
        w.writes().last().unwrap(),
        "umount --recursive /run/bootc/storage"
    );
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
        "efibootmgr --quiet --bootnext",
    ];
    let out = w.install(&req("sda", "free-space")).0.unwrap();
    let all = out.warnings.join("\n");
    assert_eq!(out.warnings.len(), 5, "{all}");
    assert!(all.contains("start AtlasOS next"), "{all}");
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

/// How Restart goes for each state of the live medium (World boots from
/// /dev/sr0): `(systemctl reboot was run, restarted directly)`.
fn restarting(w: &World) -> (bool, bool) {
    restart(&w.fake, &w.env).unwrap();
    let systemctl = lock(&w.fake.calls)
        .iter()
        .any(|c| c.name() == "systemctl" && c.args == ["reboot"]);
    (systemctl, lock(&w.fake.restarted).is_some())
}

#[test]
fn restart_goes_through_systemd_while_the_medium_is_there() {
    let w = World::new();
    w.put("sys/class/block/sr0/size", b"7340032\n");
    assert_eq!(restarting(&w), (true, false));
}

#[test]
fn restart_is_direct_once_the_medium_is_gone() {
    // the disc ejected: the drive is still there, empty
    let w = World::new();
    w.put("sys/class/block/sr0/size", b"0\n");
    assert_eq!(restarting(&w), (false, true));
    // the USB stick pulled: its device is gone (and the by-label link with it)
    let w = World::new();
    w.put(
        "proc/self/mounts",
        b"/dev/disk/by-label/AtlasOS /run/initramfs/live iso9660 ro 0 0\n",
    );
    assert_eq!(restarting(&w), (false, true));
}

#[test]
fn restart_is_direct_when_systemctl_fails() {
    let mut w = World::new();
    w.put("sys/class/block/sr0/size", b"7340032\n");
    w.fake.fail = vec!["systemctl reboot"];
    assert_eq!(restarting(&w), (true, true));
}

#[test]
fn restart_without_a_live_medium_goes_through_systemd() {
    let w = World::new();
    w.put("proc/self/mounts", b"/dev/vda3 / btrfs rw 0 0\n");
    assert_eq!(restarting(&w), (true, false));
}

#[test]
fn restart_follows_ventoy_down_to_the_stick() {
    let w = World::new();
    w.put(
        "proc/self/mounts",
        b"/dev/mapper/ventoy /run/initramfs/live iso9660 ro 0 0\n",
    );
    w.put("dev/dm-0", b"");
    fs::create_dir_all(w.env.root.join("dev/mapper")).unwrap();
    std::os::unix::fs::symlink("../dm-0", w.env.root.join("dev/mapper/ventoy")).unwrap();
    w.put("sys/class/block/dm-0/size", b"7340032\n");
    w.put("sys/class/block/dm-0/dm/name", b"ventoy\n");
    w.put("sys/class/block/dm-0/slaves/sdb1", b"");
    w.put("sys/class/block/sdb1/size", b"62914560\n");
    assert_eq!(restarting(&w), (true, false));
    // pulled: the mapping stays, the stick under it doesn't
    fs::remove_dir_all(w.env.root.join("sys/class/block/sdb1")).unwrap();
    lock(&w.fake.calls).clear();
    assert_eq!(restarting(&w), (false, true));
}

#[test]
fn restart_follows_an_iso_file_down_to_its_disk() {
    let w = World::new();
    w.put(
        "proc/self/mounts",
        b"/dev/sdb1 /run/initramfs/isoscan exfat ro 0 0\n\
          /dev/loop0 /run/initramfs/live iso9660 ro 0 0\n",
    );
    w.put("dev/sdb1", b"");
    w.put("dev/loop0", b"");
    w.put("sys/class/block/loop0/size", b"7340032\n");
    w.put(
        "sys/class/block/loop0/loop/backing_file",
        b"/run/initramfs/isoscan/isos/atlasos.iso\n",
    );
    w.put("sys/class/block/sdb1/size", b"62914560\n");
    assert_eq!(restarting(&w), (true, false));
    fs::remove_dir_all(w.env.root.join("sys/class/block/sdb1")).unwrap();
    fs::remove_file(w.env.root.join("dev/sdb1")).unwrap();
    lock(&w.fake.calls).clear();
    assert_eq!(restarting(&w), (false, true));
}

#[test]
fn a_direct_restart_unmounts_what_is_left_of_the_target() {
    let w = World::new();
    let t = w.env.target.display().to_string();
    w.put(
        "proc/self/mounts",
        format!(
            "/dev/sr0 /run/initramfs/live iso9660 ro 0 0\n\
             /dev/sda3 {t} btrfs rw 0 0\n\
             /dev/sda2 {t}/boot ext4 rw 0 0\n\
             /dev/sda1 {t}/boot/efi vfat rw 0 0\n\
             /dev/sda4 {t}-other ext4 rw 0 0\n\
             tmpfs {t}/boot/efi tmpfs rw 0 0\n"
        )
        .as_bytes(),
    );
    w.put("sys/class/block/sr0/size", b"0\n");
    assert_eq!(restarting(&w), (false, true));
    assert_eq!(
        lock(&w.fake.restarted).clone().unwrap(),
        // the tmpfs stacked on the ESP, then the ESP under it
        [
            format!("{t}/boot/efi"),
            format!("{t}/boot/efi"),
            format!("{t}/boot"),
            t.clone()
        ]
    );
}

// ---- disk encryption ----

fn enc_req(disk: &str, mode: &str, enc: &str, password: &str) -> Request {
    Request::new(
        disk,
        "",
        mode,
        "de_DE.UTF-8",
        "de(nodeadkeys)",
        "",
        enc,
        password,
    )
    .unwrap()
}

impl World {
    fn with_tpm(self) -> World {
        self.put("sys/class/tpm/tpm0/tpm_version_major", b"2\n");
        self.put("dev/tpmrm0", b"");
        self
    }

    /// The LUKS UUID of the install, from the luksFormat call.
    fn luks_uuid(&self) -> String {
        let calls = lock(&self.fake.calls);
        let f = calls
            .iter()
            .find(|c| c.args.first().map(String::as_str) == Some("luksFormat"))
            .expect("luksFormat ran");
        let i = f.args.iter().position(|a| a == "--uuid").unwrap();
        f.args[i + 1].clone()
    }

    /// The writes with the LUKS UUID shown as `U`.
    fn writes_u(&self) -> Vec<String> {
        let u = self.luks_uuid();
        self.writes()
            .into_iter()
            .map(|l| l.replace(&u, "U"))
            .collect()
    }

    fn key_path(&self) -> PathBuf {
        self.env.run_dir.join("luks/luks-key")
    }

    fn bootc_args(&self) -> Vec<String> {
        lock(&self.fake.calls)
            .iter()
            .find(|c| c.name() == "bootc")
            .expect("bootc ran")
            .args
            .clone()
    }
}

/// The calls from formatting the root partition to mounting it.
fn root_steps(writes: &[String]) -> Vec<String> {
    let from = writes
        .iter()
        .position(|l| l == "wipefs --all --quiet /dev/sda6")
        .unwrap();
    let to = writes
        .iter()
        .position(|l| l.starts_with("mount -o compress"))
        .unwrap();
    writes[from..=to].to_vec()
}

const KEY: &str = "R/luks/luks-key";

#[test]
fn tpm_encryption_runs_these_commands() {
    let w = World::new().with_tpm();
    let (r, seen) = w.install(&enc_req("sda", "free-space", "tpm", ""));
    let out = r.unwrap();
    let u = w.luks_uuid();
    assert_eq!(u.len(), 36);
    assert_eq!(&u[14..15], "4", "a version 4 UUID");
    let writes = w.writes_u();
    assert_eq!(
        root_steps(&writes),
        [
            "wipefs --all --quiet /dev/sda6".to_string(),
            format!(
                "cryptsetup luksFormat --type luks2 --batch-mode --uuid U --label atlasos --key-file {KEY} /dev/sda6"
            ),
            format!("cryptsetup open --allow-discards --key-file {KEY} /dev/sda6 luks-U"),
            "mkfs.btrfs -q -f -L atlasos /dev/mapper/luks-U".into(),
            format!("systemd-cryptenroll --unlock-key-file={KEY} --recovery-key /dev/sda6"),
            format!(
                "systemd-cryptenroll --unlock-key-file={KEY} --tpm2-device=auto --tpm2-pcrs=7 /dev/sda6"
            ),
            "cryptsetup open --test-passphrase --token-only --token-type systemd-tpm2 /dev/sda6"
                .into(),
            format!("cryptsetup luksRemoveKey --key-file {KEY} /dev/sda6"),
            format!(
                "cryptsetup open --test-passphrase --disable-external-tokens --key-file {KEY} /dev/sda6"
            ),
            "cryptsetup luksDump --dump-json-metadata /dev/sda6".into(),
            "cryptsetup open --test-passphrase --disable-external-tokens --key-file - /dev/sda6"
                .into(),
            "mount -o compress=zstd:1 /dev/mapper/luks-U T".into(),
        ]
    );
    // the ESP and /boot stay plain, and the mapper is closed after the umount
    assert!(writes.contains(&"mkfs.ext4 -q -F -L boot /dev/sda5".to_string()));
    let n = writes.len();
    assert_eq!(writes[n - 3], "umount --recursive T");
    assert_eq!(writes[n - 2], "umount --recursive /run/bootc/storage");
    assert_eq!(writes[n - 1], "cryptsetup close luks-U");
    assert!(lock(&w.fake.crypt_open).is_none(), "closed");

    assert_eq!(
        out.recovery_key.as_ref().map(Secret::as_str),
        Some(RECOVERY_KEY)
    );
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert!(!w.key_path().exists(), "the temporary key is gone");
    assert!(seen.iter().all(|p| p.fraction <= 1.0));
    assert!(seen.iter().any(|p| p.stage == Stage::Format));

    let args = w.bootc_args();
    let kargs: Vec<&String> = args
        .iter()
        .zip(args.iter().skip(1))
        .filter(|(a, _)| *a == "--karg")
        .map(|(_, v)| v)
        .collect();
    assert_eq!(
        kargs,
        [
            "rootflags=compress=zstd:1",
            &format!("rd.luks.uuid={u}"),
            &format!("rd.luks.options={u}=discard,tpm2-device=auto,tries=0"),
            "vconsole.keymap=de-latin1-nodeadkeys",
            "rd.shell=0",
            "rd.emergency=reboot",
        ]
    );
    // the root is mounted from the mapper before bootc runs, and the raw
    // partition is never mounted
    let mounted = w.fake.mounts_at_bootc.lock().unwrap().clone();
    let tgt = w.env.target.display().to_string();
    assert!(
        mounted.contains(&format!("/dev/mapper/luks-{u} {tgt} ")),
        "{mounted}"
    );
    assert!(!mounted.contains("/dev/sda6 "), "{mounted}");
    assert!(
        w.writes()
            .iter()
            .all(|l| !(l.starts_with("mount ") && l.contains("/dev/sda6"))),
        "{:?}",
        w.writes()
    );
}

#[test]
fn password_encryption_puts_the_password_on_stdin_only() {
    let w = World::new();
    let (r, _) = w.install(&enc_req("sda", "erase", "password", PASSWORD));
    let out = r.unwrap();
    let u = w.luks_uuid();
    // erase: the root is /dev/sda3 here; the sequence is the same
    let writes = w.writes_u();
    let root = writes
        .iter()
        .find_map(|l| l.strip_prefix("cryptsetup luksFormat "))
        .and_then(|l| l.rsplit(' ').next())
        .unwrap()
        .to_string();
    let from = writes
        .iter()
        .rposition(|l| *l == format!("wipefs --all --quiet {root}"))
        .unwrap();
    let to = writes
        .iter()
        .position(|l| l.starts_with("mount -o compress"))
        .unwrap();
    assert_eq!(
        writes[from..=to],
        [
            format!("wipefs --all --quiet {root}"),
            format!(
                "cryptsetup luksFormat --type luks2 --batch-mode --uuid U --label atlasos --key-file {KEY} {root}"
            ),
            format!("cryptsetup open --allow-discards --key-file {KEY} {root} luks-U"),
            "mkfs.btrfs -q -f -L atlasos /dev/mapper/luks-U".into(),
            format!("systemd-cryptenroll --unlock-key-file={KEY} --recovery-key {root}"),
            format!("cryptsetup luksAddKey --key-file {KEY} --new-keyfile - {root}"),
            format!("cryptsetup luksRemoveKey --key-file {KEY} {root}"),
            format!(
                "cryptsetup open --test-passphrase --disable-external-tokens --key-file {KEY} {root}"
            ),
            format!("cryptsetup luksDump --dump-json-metadata {root}"),
            format!(
                "cryptsetup open --test-passphrase --disable-external-tokens --key-file - {root}"
            ),
            format!(
                "cryptsetup open --test-passphrase --disable-external-tokens --key-file - {root}"
            ),
            "mount -o compress=zstd:1 /dev/mapper/luks-U T".into(),
        ]
    );
    assert_eq!(
        out.recovery_key.as_ref().map(Secret::as_str),
        Some(RECOVERY_KEY)
    );
    assert!(!w.key_path().exists());

    let calls = lock(&w.fake.calls);
    let add = calls
        .iter()
        .find(|c| c.args.first().map(String::as_str) == Some("luksAddKey"))
        .unwrap();
    assert_eq!(add.stdin.as_deref(), Some(PASSWORD), "no trailing newline");
    assert!(add.secret);
    // the password is on stdin of those two commands and nowhere else
    for c in calls.iter() {
        let holds = |t: &str| {
            c.program.contains(t)
                || c.args.iter().any(|a| a.contains(t))
                || c.stdin.as_deref() == Some(t)
        };
        assert!(!c.args.iter().any(|a| a.contains(PASSWORD)), "{c:?}");
        if holds(PASSWORD) {
            assert!(c.secret && c.args.contains(&"-".to_string()), "{c:?}");
        }
    }
    drop(calls);

    let args = w.bootc_args();
    for k in [
        format!("rd.luks.uuid={u}"),
        format!("rd.luks.options={u}=discard,tries=0"),
        "vconsole.keymap=de-latin1-nodeadkeys".to_string(),
        "rd.shell=0".to_string(),
        "rd.emergency=reboot".to_string(),
    ] {
        assert!(args.contains(&k), "{k}: {args:?}");
    }
    assert!(!args.iter().any(|a| a.contains("tpm2")));
}

#[test]
fn every_encrypted_mode_keeps_asking_for_the_key() {
    for (enc, pw) in [("tpm", ""), ("tpm-pin", PIN), ("password", PASSWORD)] {
        let w = World::new().with_tpm();
        w.install(&enc_req("sda", "free-space", enc, pw)).0.unwrap();
        assert!(
            w.bootc_args()
                .iter()
                .any(|a| a.starts_with("rd.luks.options=") && a.ends_with(",tries=0")),
            "{enc}"
        );
    }
}

#[test]
fn bootcs_own_mount_is_released_before_the_volume_is_closed() {
    for (enc, pw) in [("tpm", ""), ("password", PASSWORD), ("none", "")] {
        let w = World::new().with_tpm();
        let out = w.install(&enc_req("sda", "free-space", enc, pw)).0.unwrap();
        let writes = if enc == "none" {
            w.writes()
        } else {
            w.writes_u()
        };
        let at = |l: &str| writes.iter().position(|w| w == l);
        let storage = at("umount --recursive /run/bootc/storage");
        assert!(storage.is_some(), "{enc}: {writes:?}");
        if enc != "none" {
            let close =
                at("cryptsetup close luks-U").unwrap_or_else(|| panic!("{enc}: {writes:?}"));
            assert!(storage < Some(close), "{enc}: {writes:?}");
            assert!(at("cryptsetup close --deferred luks-U").is_none(), "{enc}");
        }
        assert!(out.warnings.is_empty(), "{enc}: {:?}", out.warnings);
        let log = fs::read_to_string(w.env.log_path()).unwrap();
        assert!(log.contains("unmounted /run/bootc/storage"), "{enc}: {log}");
    }
}

#[test]
fn secrets_stay_out_of_the_log() {
    for (enc, pw) in [("tpm", ""), ("password", PASSWORD)] {
        let w = World::new().with_tpm();
        let out = w.install(&enc_req("sda", "free-space", enc, pw)).0.unwrap();
        assert!(out.recovery_key.is_some());
        assert_eq!(out.encryption, enc);
        let log = fs::read_to_string(w.env.log_path()).unwrap();
        assert!(!log.contains(RECOVERY_KEY), "{enc}");
        assert!(!log.contains(PASSWORD), "{enc}");
        assert!(!log.contains("ulcbjnni"), "{enc}");
        assert!(log.contains("$ /usr/bin/cryptsetup luksFormat"));
        assert!(log.contains("$ /usr/bin/systemd-cryptenroll (arguments hidden)"));
        // the key's name is fine; its contents were never in a command
        let key = w.key_path();
        assert!(!key.exists());
    }
}

#[test]
fn without_encryption_cryptsetup_is_never_run() {
    let w = World::new();
    let out = w.install(&req("sda", "free-space")).0.unwrap();
    assert_eq!(out.recovery_key, None);
    assert_eq!(out.encryption, "none");
    assert!(
        lock(&w.fake.calls)
            .iter()
            .all(|c| !c.program.contains("crypt")),
    );
    assert!(!w.env.run_dir.join("luks").exists());
    assert!(!w.bootc_args().iter().any(|a| a.contains("luks")));
}

#[test]
fn a_tpm_that_does_not_unlock_stops_the_install() {
    let mut w = World::new().with_tpm();
    w.fake.fail = vec!["cryptsetup open --test-passphrase --token-only"];
    let e = w
        .install(&enc_req("sda", "free-space", "tpm", ""))
        .0
        .unwrap_err();
    assert!(e.contains(TPM_FAILED), "{e}");
    let writes = w.writes_u();
    assert!(
        writes
            .iter()
            .all(|l| !l.contains("luksRemoveKey") && !l.starts_with("bootc")),
        "{writes:?}"
    );
    assert!(!w.key_path().exists(), "the temporary key is gone");
    assert!(lock(&w.fake.crypt_open).is_none(), "the mapper is closed");
    assert_eq!(writes.last().unwrap(), "cryptsetup close luks-U");
    let log = fs::read_to_string(w.env.log_path()).unwrap();
    assert!(log.contains("FAILED: This PC's security chip"));
    assert!(log.contains("# exit 1"), "{log}");

    // the enrollment itself failing says the same
    let mut w = World::new().with_tpm();
    let tmp = w.env.run_dir.join("luks/luks-key");
    w.fake.fail = vec![leak(format!(
        "systemd-cryptenroll --unlock-key-file={} --tpm2-device=auto",
        tmp.display()
    ))];
    let e = w
        .install(&enc_req("sda", "free-space", "tpm", ""))
        .0
        .unwrap_err();
    assert!(e.contains(TPM_FAILED), "{e}");
    assert!(!tmp.exists());
}

/// `Fake::fail` wants `&'static str`; a test's few leaks are harmless.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

#[test]
fn the_key_is_removed_whatever_fails() {
    for fail in [
        "cryptsetup luksFormat",
        "cryptsetup open --allow-discards",
        "mkfs.btrfs",
        "systemd-cryptenroll",
        "cryptsetup luksAddKey",
        "cryptsetup luksRemoveKey",
        "cryptsetup luksDump",
        "cryptsetup open --test-passphrase --disable-external-tokens --key-file -",
        "mount -o compress",
        "bootc",
    ] {
        let mut w = World::new();
        w.fake.fail = vec![fail];
        let e = w
            .install(&enc_req("sda", "free-space", "password", PASSWORD))
            .0
            .unwrap_err();
        assert!(!w.key_path().exists(), "{fail}: {e}");
        assert!(
            !e.contains(PASSWORD) && !e.contains(RECOVERY_KEY),
            "{fail}: {e}"
        );
        let log = fs::read_to_string(w.env.log_path()).unwrap();
        assert!(
            !log.contains(PASSWORD) && !log.contains(RECOVERY_KEY),
            "{fail}"
        );
        if !fail.starts_with("cryptsetup luksFormat") {
            let writes = w.writes_u();
            assert!(
                lock(&w.fake.crypt_open).is_none(),
                "{fail}: the mapper is closed: {writes:?}"
            );
        }
        let m = fs::read_to_string(w.env.root.join("proc/self/mounts")).unwrap();
        assert!(!m.contains("/dev/mapper/"), "{fail}: unmounted");
    }
}

#[test]
fn a_late_failure_unmounts_before_it_closes_the_mapper() {
    let mut w = World::new();
    w.fake.fail = vec!["bootc"];
    w.install(&enc_req("sda", "free-space", "password", PASSWORD))
        .0
        .unwrap_err();
    let writes = w.writes_u();
    let n = writes.len();
    assert_eq!(writes[n - 2], "umount --recursive T");
    assert_eq!(writes[n - 1], "cryptsetup close luks-U");
}

#[test]
fn a_recovery_key_that_is_not_one_is_refused_and_not_shown() {
    let mut w = World::new();
    w.fake.bad_recovery = true;
    let e = w
        .install(&enc_req("sda", "free-space", "password", PASSWORD))
        .0
        .unwrap_err();
    assert!(e.contains("while creating the recovery key"), "{e}");
    assert!(!e.contains("generated"), "the tool's output stays out: {e}");
    assert!(!w.key_path().exists());
}

#[test]
fn a_temporary_key_that_still_works_fails_the_install() {
    let mut w = World::new();
    // the probe exits 1 (not 2, "no key matched"): the key is not known gone
    let key = w.env.run_dir.join("luks/luks-key");
    w.fake.fail = vec![leak(format!(
        "cryptsetup open --test-passphrase --disable-external-tokens --key-file {}",
        key.display()
    ))];
    let e = w
        .install(&enc_req("sda", "free-space", "password", PASSWORD))
        .0
        .unwrap_err();
    assert!(e.contains("while removing the temporary key"), "{e}");
    assert!(
        fs::read_to_string(w.env.log_path())
            .unwrap()
            .contains("encryption failed while removing")
    );
}

#[test]
fn tpm_needs_a_tpm_2() {
    // none at all
    let w = World::new();
    let e = w
        .install(&enc_req("sda", "free-space", "tpm", ""))
        .0
        .unwrap_err();
    assert!(e.contains("no usable security chip"), "{e}");
    assert!(w.writes().is_empty(), "{:?}", w.writes());
    // a TPM 1.2, and a 2.0 without the resource manager
    let w = World::new();
    w.put("sys/class/tpm/tpm0/tpm_version_major", b"1\n");
    w.put("dev/tpmrm0", b"");
    assert!(!tpm2_present(&w.env));
    let w = World::new();
    w.put("sys/class/tpm/tpm0/tpm_version_major", b"2\n");
    assert!(!tpm2_present(&w.env));
    // junk names are not chips
    let w = World::new();
    w.put("sys/class/tpm/tpmrm0/tpm_version_major", b"2\n");
    w.put("dev/tpmrm0", b"");
    assert!(!tpm2_present(&w.env));
    // a second chip may be the 2.0 one
    let w = World::new();
    w.put("sys/class/tpm/tpm0/tpm_version_major", b"1\n");
    w.put("sys/class/tpm/tpm1/tpm_version_major", b"2\n");
    w.put("dev/tpmrm0", b"");
    assert!(tpm2_present(&w.env));
    // password and no encryption don't care
    let w = World::new();
    assert!(
        w.install(&enc_req("sda", "free-space", "password", PASSWORD))
            .0
            .is_ok()
    );
}

#[test]
fn the_listing_says_whether_there_is_a_tpm() {
    let w = World::new();
    let l = list_disks(&w.fake, &w.env).unwrap();
    assert!(!l.tpm2);
    assert_eq!(serde_json::to_value(&l).unwrap()["tpm2"], false);
    let w = World::new().with_tpm();
    let v = serde_json::to_value(list_disks(&w.fake, &w.env).unwrap()).unwrap();
    assert_eq!(v["tpm2"], true);
    assert!(v["disks"].is_array() && v["hidden"].is_array());
}

#[test]
fn requests_with_encryption_are_checked() {
    let new =
        |enc: &str, pw: &str| Request::new("sda", "", "erase", "en_US.UTF-8", "us", "", enc, pw);
    assert!(new("none", "").is_ok());
    assert!(new("tpm", "").is_ok());
    assert!(new("password", "12345678").is_ok());
    assert!(new("password", &"x".repeat(256)).is_ok(), "256 characters");
    assert!(new("password", "pass word  ~!").is_ok());
    assert!(new("tpm-pin", "1234").is_ok());
    assert!(new("tpm-pin", &"9".repeat(64)).is_ok());
    for (enc, pw) in [
        ("TPM", ""),
        ("yes", ""),
        ("", ""),
        ("none ", ""),
        ("none", "12345678"),
        ("tpm", "12345678"),
        ("password", ""),
        ("password", "1234567"),
        ("password", &"x".repeat(257)),
        ("password", "pässwörd"),
        ("password", "日本語のパスワードです"),
        ("password", "abcdefgh\u{a0}"),
        ("tpm-pin", ""),
        ("tpm-pin", "123"),
        ("tpm-pin", &"9".repeat(65)),
        ("tpm-pin", "12345é"),
        ("tpm-pin", "12345\n6"),
        ("tpm-pins", "123456"),
        ("password", "abcdefgh\n"),
        ("password", "abcdefgh\r"),
        ("password", "abc\tdefgh"),
        ("password", "abcdefg\0h"),
        ("password", "abcdefgh\u{7f}"),
        ("password", "abcdefgh\u{1b}"),
        ("password", "abcdefgh\u{80}"),
        ("password", "abcdefgh\u{9f}"),
    ] {
        assert!(new(enc, pw).is_err(), "{enc:?} {pw:?}");
    }
    // the password is never in a Debug print
    let r = new("password", PASSWORD).unwrap();
    assert!(!format!("{r:?}").contains(PASSWORD));
}

#[test]
fn the_plan_shows_the_encryption_without_secrets() {
    for (enc, pw, want) in [
        ("tpm", "", "--tpm2-device=auto --tpm2-pcrs=7"),
        ("password", PASSWORD, "luksAddKey"),
    ] {
        let w = World::new().with_tpm();
        let r = enc_req("sda", "free-space", enc, pw);
        let p = prepare(&w.fake, &w.env, &r).unwrap();
        let text = describe(&p, &r, &w.env);
        let u = p.luks_uuid.clone().unwrap();
        assert!(text.contains(want), "{text}");
        assert!(text.contains(&format!("--karg rd.luks.uuid={u}")), "{text}");
        assert!(text.contains("<temporary key>"));
        assert!(!text.contains(PASSWORD) && !text.contains(RECOVERY_KEY));
        assert!(
            !text.contains("mkfs.btrfs -q -f -L atlasos /dev/sda6"),
            "the root is formatted on the mapper: {text}"
        );
        assert!(text.contains(&format!("mkfs.btrfs -q -f -L atlasos /dev/mapper/luks-{u}")));
        assert!(w.writes().is_empty(), "describing writes nothing");
    }
    let w = World::new();
    let r = req("sda", "free-space");
    let p = prepare(&w.fake, &w.env, &r).unwrap();
    assert!(describe(&p, &r, &w.env).contains("encryption: none"));
}

#[test]
fn a_mapper_left_by_a_failed_attempt_is_closed_before_the_disk_is_written() {
    let w = World::new();
    let stale = format!("luks-{UUID}");
    *lock(&w.fake.crypt_open) = Some(("/dev/sda3".into(), stale.clone()));
    let out = w.install(&req("sda", "erase")).0.unwrap();
    assert!(out.warnings.is_empty());
    let writes = w.writes();
    let close = writes
        .iter()
        .position(|l| *l == format!("cryptsetup close {stale}"))
        .expect("closed");
    let first_write = writes
        .iter()
        .position(|l| l.starts_with("wipefs") || l.starts_with("sfdisk"))
        .unwrap();
    assert!(close < first_write, "{writes:?}");
    assert!(lock(&w.fake.crypt_open).is_none());

    // another disk's mapper is left alone: lsblk shows it under sda only, so
    // asking for a different disk closes nothing
    let w = World::new();
    *lock(&w.fake.crypt_open) = Some(("/dev/sda3".into(), stale));
    let l = Lsblk::parse(&lsblk_with_crypt("/dev/sda3", &format!("luks-{UUID}"))).unwrap();
    assert!(l.luks_mappers_on("/dev/sdb").is_empty());
}

#[test]
fn a_mapper_that_will_not_close_stops_the_install_before_any_write() {
    let mut w = World::new();
    *lock(&w.fake.crypt_open) = Some(("/dev/sda3".into(), format!("luks-{UUID}")));
    w.fake.fail = vec!["cryptsetup close"];
    let e = w.install(&req("sda", "erase")).0.unwrap_err();
    assert!(e.contains("couldn't be closed"), "{e}");
    assert!(
        w.writes()
            .iter()
            .all(|l| !l.starts_with("wipefs") && !l.starts_with("sfdisk")),
        "{:?}",
        w.writes()
    );
}

#[test]
fn a_deferred_close_does_not_count_as_closed_before_the_first_write() {
    let mut w = World::new();
    *lock(&w.fake.crypt_open) = Some(("/dev/sda3".into(), format!("luks-{UUID}")));
    w.fake.close_only_deferred = true;
    let e = w.install(&req("sda", "erase")).0.unwrap_err();
    assert!(e.contains("still open"), "{e}");
    assert!(
        w.writes()
            .iter()
            .all(|l| !l.starts_with("wipefs") && !l.starts_with("sfdisk")),
        "{:?}",
        w.writes()
    );
    // after an install, a deferred close is enough: the cleanup carries on
    let w2 = World::new();
    let log = Log::create(&w2.env.log_path());
    *lock(&w2.fake.crypt_open) = Some(("/dev/sda3".into(), format!("luks-{UUID}")));
    unmount_target(&w2.fake, &log, &w2.env, Some("/dev/sda"), &[], false).unwrap();
}

#[test]
fn encryption_needs_its_tools_before_anything_is_written() {
    for broken in [bin::CRYPTSETUP, bin::CRYPTENROLL, bin::GRUB2_MKPASSWD] {
        for remove in [true, false] {
            let w = World::new();
            let f = w.env.host(broken.trim_start_matches('/'));
            if remove {
                fs::remove_file(&f).unwrap();
            } else {
                fs::set_permissions(&f, fs::Permissions::from_mode(0o644)).unwrap();
            }
            let e = w
                .install(&enc_req("sda", "erase", "password", PASSWORD))
                .0
                .unwrap_err();
            let name = broken.rsplit('/').next().unwrap();
            assert!(e.contains(name) && e.contains("Nothing was written"), "{e}");
            assert!(w.writes().is_empty(), "{:?}", w.writes());
            // no encryption, no need for them
            w.install(&req("sda", "erase")).0.unwrap();
        }
    }
}

#[test]
fn the_temporary_key_never_follows_a_link() {
    let w = World::new();
    fs::create_dir_all(w.env.run_dir.join("luks")).unwrap();
    let victim = w.env.run_dir.join("victim");
    fs::write(&victim, "keep").unwrap();
    std::os::unix::fs::symlink(&victim, w.key_path()).unwrap();
    let key = KeyFile::create(&w.env).unwrap();
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
    assert_eq!(fs::read(&key.path).unwrap().len(), 64);
    let p = key.path.clone();
    drop(key);
    assert!(!p.exists());
    assert!(victim.exists());
    // a directory that is a link is refused
    let w = World::new();
    fs::create_dir_all(&w.env.run_dir).unwrap();
    std::os::unix::fs::symlink(&w.env.root, w.env.run_dir.join("luks")).unwrap();
    assert!(KeyFile::create(&w.env).is_err());
}

#[test]
fn uuids_are_version_4_and_differ() {
    let (a, b) = (new_uuid().unwrap(), new_uuid().unwrap());
    assert_ne!(a, b);
    for u in [&a, &b] {
        assert!(settings::validate_uuid(u).is_ok());
        assert_eq!(&u[14..15], "4");
        assert!("89ab".contains(&u[19..20]));
    }
}

const PIN: &str = "482913";

#[test]
fn tpm_with_a_pin_puts_the_pin_in_the_environment_only() {
    let w = World::new().with_tpm();
    let out = w
        .install(&enc_req("sda", "free-space", "tpm-pin", PIN))
        .0
        .unwrap();
    let u = w.luks_uuid();
    let writes = w.writes_u();
    assert_eq!(
        root_steps(&writes),
        [
            "wipefs --all --quiet /dev/sda6".to_string(),
            format!(
                "cryptsetup luksFormat --type luks2 --batch-mode --uuid U --label atlasos --key-file {KEY} /dev/sda6"
            ),
            format!("cryptsetup open --allow-discards --key-file {KEY} /dev/sda6 luks-U"),
            "mkfs.btrfs -q -f -L atlasos /dev/mapper/luks-U".into(),
            format!("systemd-cryptenroll --unlock-key-file={KEY} --recovery-key /dev/sda6"),
            format!(
                "systemd-cryptenroll --unlock-key-file={KEY} --tpm2-device=auto --tpm2-pcrs=7 --tpm2-with-pin=yes /dev/sda6"
            ),
            format!("cryptsetup luksRemoveKey --key-file {KEY} /dev/sda6"),
            format!(
                "cryptsetup open --test-passphrase --disable-external-tokens --key-file {KEY} /dev/sda6"
            ),
            "cryptsetup luksDump --dump-json-metadata /dev/sda6".into(),
            "cryptsetup open --test-passphrase --disable-external-tokens --key-file - /dev/sda6"
                .into(),
            "mount -o compress=zstd:1 /dev/mapper/luks-U T".into(),
        ]
    );
    assert!(out.recovery_key.is_some());
    let calls = lock(&w.fake.calls);
    for c in calls.iter() {
        let pin_env = c.secret_env.iter().any(|(_, v)| v == PIN);
        assert_eq!(
            pin_env,
            c.args.iter().any(|a| a == "--tpm2-with-pin=yes"),
            "{c:?}"
        );
        assert!(!c.args.iter().any(|a| a.contains(PIN)), "{c:?}");
        assert!(c.stdin.as_deref() != Some(PIN), "{c:?}");
        assert!(!format!("{c:?}").contains(PIN), "{c:?}");
    }
    let enroll = calls
        .iter()
        .find(|c| c.args.iter().any(|a| a == "--tpm2-with-pin=yes"))
        .unwrap();
    assert_eq!(enroll.secret_env, [("NEWPIN", PIN.to_string())]);
    drop(calls);
    let log = fs::read_to_string(w.env.log_path()).unwrap();
    assert!(!log.contains(PIN) && !log.contains(RECOVERY_KEY));
    let args = w.bootc_args();
    assert!(args.contains(&format!(
        "rd.luks.options={u}=discard,tpm2-device=auto,tries=0"
    )));
    assert!(args.contains(&"vconsole.keymap=de-latin1-nodeadkeys".to_string()));
}

#[test]
fn tpm_with_a_pin_needs_a_tpm() {
    let w = World::new();
    let e = w
        .install(&enc_req("sda", "free-space", "tpm-pin", PIN))
        .0
        .unwrap_err();
    assert!(e.contains("no usable security chip"), "{e}");
}

#[test]
fn passphrase_tests_ignore_the_tpm_token_and_the_tpm_proof_uses_it() {
    for (enc, pw) in [("tpm", ""), ("tpm-pin", PIN), ("password", PASSWORD)] {
        let w = World::new().with_tpm();
        w.install(&enc_req("sda", "free-space", enc, pw)).0.unwrap();
        let calls = lock(&w.fake.calls);
        let tests: Vec<&Cmd> = calls
            .iter()
            .filter(|c| c.args.iter().any(|a| a == "--test-passphrase"))
            .collect();
        assert!(tests.len() >= 2, "{enc}");
        for c in tests {
            let token_proof = c.args.iter().any(|a| a == "--token-only");
            let no_tokens = c.args.iter().any(|a| a == "--disable-external-tokens");
            assert!(token_proof != no_tokens, "{enc}: {c:?}");
            if token_proof {
                assert!(
                    c.args
                        .windows(2)
                        .any(|w| w == ["--token-type", "systemd-tpm2"])
                );
            } else {
                assert!(c.args.iter().any(|a| a == "--key-file"));
            }
        }
    }
}

#[test]
fn the_boot_menu_is_locked_on_encrypted_installs_only() {
    for (enc, pw) in [("tpm", ""), ("tpm-pin", PIN), ("password", PASSWORD)] {
        let w = World::new().with_tpm();
        w.install(&enc_req("sda", "free-space", enc, pw)).0.unwrap();
        let user = w.env.target.join("boot/grub2/user.cfg");
        assert_eq!(
            fs::read_to_string(&user).unwrap(),
            format!("GRUB2_PASSWORD={GRUB_HASH}\n"),
            "{enc}"
        );
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(&user).unwrap().mode() & 0o7777, 0o600);
        let calls = lock(&w.fake.calls);
        let mk = calls
            .iter()
            .find(|c| c.name() == "grub2-mkpasswd-pbkdf2")
            .unwrap();
        assert!(mk.secret && mk.args.is_empty());
        let pw_used = mk.stdin.clone().unwrap();
        drop(calls);
        // the random password is not kept anywhere
        let log = fs::read_to_string(w.env.log_path()).unwrap();
        let pw64 = pw_used.lines().next().unwrap();
        assert!(!log.contains(pw64) && !fs::read_to_string(&user).unwrap().contains(pw64));
        // labelled with the new system's policy, with the Windows entry
        let relabel = w
            .writes()
            .into_iter()
            .rfind(|l| l.starts_with("setfiles -F -r T "))
            .unwrap();
        assert!(relabel.contains("T/boot/grub2/user.cfg"), "{relabel}");
        assert!(relabel.contains("T/boot/grub2/custom.cfg"), "{relabel}");
        let cfg = fs::read_to_string(w.env.target.join("boot/grub2/custom.cfg")).unwrap();
        assert!(cfg.contains("--unrestricted"));
    }
    let w = World::new();
    w.install(&req("sda", "free-space")).0.unwrap();
    assert!(!w.env.target.join("boot/grub2/user.cfg").exists());
    assert!(
        lock(&w.fake.calls)
            .iter()
            .all(|c| c.name() != "grub2-mkpasswd-pbkdf2")
    );
}

#[test]
fn a_boot_menu_that_cannot_be_locked_fails_the_install() {
    let mut w = World::new().with_tpm();
    w.fake.fail = vec!["grub2-mkpasswd-pbkdf2"];
    let e = w
        .install(&enc_req("sda", "free-space", "tpm", ""))
        .0
        .unwrap_err();
    assert!(e.starts_with("Locking the boot menu failed."), "{e}");
    assert!(!w.env.target.join("boot/grub2/user.cfg").exists());
    assert!(!w.key_path().exists());
    assert!(lock(&w.fake.crypt_open).is_none());
}

#[test]
fn secrets_are_not_in_debug_output() {
    let w = World::new().with_tpm();
    let out = w
        .install(&enc_req("sda", "free-space", "tpm-pin", PIN))
        .0
        .unwrap();
    let shown = format!("{out:?}");
    assert!(
        !shown.contains(RECOVERY_KEY) && shown.contains("<hidden>"),
        "{shown}"
    );
    // but it serializes as itself, for the caller who may see it
    assert_eq!(
        serde_json::to_value(&out).unwrap()["recovery_key"],
        RECOVERY_KEY
    );
    let c = Cmd::new(bin::CRYPTSETUP, ["x"])
        .stdin(PASSWORD)
        .secret_env("NEWPIN", PIN);
    let shown = format!("{c:?}");
    assert!(!shown.contains(PASSWORD) && !shown.contains(PIN), "{shown}");
    let r = enc_req("sda", "erase", "tpm-pin", PIN);
    assert!(!format!("{r:?}").contains(PIN));
}

#[test]
fn a_failing_unmount_still_cleans_up_and_the_next_install_works() {
    // the plain unmount fails: it is retried lazily, and the mapper closed
    let mut w = World::new();
    w.fake.fail = vec!["bootc"];
    let tgt = w.env.target.display().to_string();
    let failing = leak(format!("umount --recursive {tgt}"));
    w.fake.fail.push(failing);
    let e = w
        .install(&enc_req("sda", "erase", "password", PASSWORD))
        .0
        .unwrap_err();
    assert!(e.starts_with("boom") || e.contains("bootc"), "{e}");
    assert!(!w.key_path().exists());
    assert!(lock(&w.fake.crypt_open).is_none(), "the mapper is closed");
    let writes = w.writes_u();
    assert!(
        writes.iter().any(|l| l == "umount --recursive --lazy T"),
        "{writes:?}"
    );
    // everything fine now: a second encrypted install on the same machine
    w.fake.fail.clear();
    let out = w
        .install(&enc_req("sda", "erase", "password", PASSWORD))
        .0
        .unwrap();
    assert!(out.recovery_key.is_some());
    assert!(!w.key_path().exists());

    // even the lazy unmount fails and the mapper is busy: closed deferred
    let mut w = World::new();
    let tgt2 = w.env.target.display().to_string();
    w.fake.fail = vec![
        "bootc",
        leak(format!("umount --recursive {tgt2}")),
        leak(format!("umount --recursive --lazy {tgt2}")),
        // nor can the single mounts be released
        leak(format!("umount {tgt2}")),
        leak(format!("umount --lazy {tgt2}")),
        "umount --recursive /run/bootc/storage",
        "umount --lazy /run/bootc/storage",
    ];
    w.install(&enc_req("sda", "erase", "password", PASSWORD))
        .0
        .unwrap_err();
    assert!(
        w.writes_u()
            .iter()
            .any(|l| l == "cryptsetup close --deferred luks-U"),
        "{:?}",
        w.writes_u()
    );
    assert!(!w.key_path().exists());
    let log = fs::read_to_string(w.env.log_path()).unwrap();
    assert!(log.contains("cleanup:"), "cleanup errors are logged: {log}");
}

#[test]
fn a_failure_can_be_retried_on_the_same_machine() {
    for fail in [
        "bootc",
        "cryptsetup luksRemoveKey",
        "systemd-cryptenroll --unlock-key-file",
    ] {
        for (enc, pw) in [("tpm", ""), ("password", PASSWORD)] {
            let mut w = World::new().with_tpm();
            w.fake.fail = vec![fail];
            w.install(&enc_req("sda", "erase", enc, pw)).0.unwrap_err();
            assert!(!w.key_path().exists(), "{fail} {enc}");
            assert!(lock(&w.fake.crypt_open).is_none(), "{fail} {enc}");
            w.fake.fail.clear();
            w.fake.calls.lock().unwrap().clear();
            let out = w.install(&enc_req("sda", "erase", enc, pw)).0.unwrap();
            assert!(out.recovery_key.is_some(), "{fail} {enc}");
            assert!(!w.key_path().exists());
        }
    }
}

#[test]
fn a_failed_final_unmount_is_a_warning_and_the_volume_is_closed_when_free() {
    let mut w = World::new();
    let tgt = w.env.target.display().to_string();
    w.fake.fail = vec![leak(format!("umount --recursive {tgt}"))];
    let out = w
        .install(&enc_req("sda", "erase", "password", PASSWORD))
        .0
        .unwrap();
    assert!(
        out.warnings
            .iter()
            .any(|m| m.contains("could not be unmounted"))
    );
    let writes = w.writes_u();
    assert!(writes.iter().any(|l| l == "umount --recursive --lazy T"));
    assert!(lock(&w.fake.crypt_open).is_none());
    assert!(!w.key_path().exists());
}

#[test]
fn only_the_installers_own_volume_is_closed_and_a_users_keeps_the_disk_busy() {
    // lsblk where the disk's crypt volume is not ours
    let w = World::new();
    let mut v: serde_json::Value =
        serde_json::from_str(&lsblk_with_crypt("/dev/sda3", &format!("luks-{UUID}"))).unwrap();
    v["blockdevices"][0]["children"][2]["label"] = "mydata".into();
    let l = Lsblk::parse(&v.to_string()).unwrap();
    assert!(l.luks_mappers_on("/dev/sda").is_empty());
    assert!(installer_core::plan::in_use(l.find("/dev/sda").unwrap()));
    let _ = w;
}

#[test]
fn the_listing_says_whether_secure_boot_is_on() {
    let w = World::new();
    let v = serde_json::to_value(list_disks(&w.fake, &w.env).unwrap()).unwrap();
    assert_eq!(v["secure_boot"], false);
    w.put(SECURE_BOOT_VAR, &[6, 0, 0, 0, 1]);
    let v = serde_json::to_value(list_disks(&w.fake, &w.env).unwrap()).unwrap();
    assert_eq!(v["secure_boot"], true);
}

#[test]
fn encryption_errors_are_plain_and_point_to_the_log() {
    for fail in [
        "cryptsetup luksFormat",
        "cryptsetup open --allow-discards",
        "mkfs.btrfs",
        "systemd-cryptenroll --unlock-key-file",
        "cryptsetup luksAddKey",
        "cryptsetup luksRemoveKey",
        "cryptsetup luksDump",
    ] {
        let mut w = World::new();
        w.fake.fail = vec![fail];
        let e = w
            .install(&enc_req("sda", "free-space", "password", PASSWORD))
            .0
            .unwrap_err();
        assert!(
            e.starts_with("Setting up encryption failed while ")
                && e.contains("The install log has the details: ")
                && e.contains("install.log")
                && !e.contains("boom"),
            "{fail}: {e}"
        );
    }
    let mut w = World::new().with_tpm();
    w.fake.fail = vec!["cryptsetup open --test-passphrase --token-only"];
    let e = w
        .install(&enc_req("sda", "free-space", "tpm", ""))
        .0
        .unwrap_err();
    assert!(
        e.starts_with(TPM_FAILED) && e.contains("install.log"),
        "{e}"
    );
}

#[test]
fn mounts_by_another_name_of_the_device_are_released() {
    let w = World::new();
    let mounts = w.env.host("proc/self/mounts");
    fs::write(
        &mounts,
        "/dev/dm-3 /mnt/a x rw 0 0\n\
         /dev/disk/by-uuid/U /mnt/b x rw 0 0\n\
         /dev/dm-3[/root] /mnt/a/c x rw 0 0\n\
         /dev/dm-9 /mnt/other x rw 0 0\n",
    )
    .unwrap();
    let canon = |d: &str| match d {
        "/dev/mapper/luks-U" | "/dev/dm-3" => Some("/dev/dm-3".to_string()),
        "/dev/disk/by-uuid/U" => Some("/dev/sda6".to_string()),
        "/dev/sda6" => Some("/dev/sda6".to_string()),
        "/dev/dm-9" => Some("/dev/dm-9".to_string()),
        _ => None,
    };
    let log = Log::create(&w.env.log_path());
    let devices = vec!["/dev/mapper/luks-U".to_string(), "/dev/sda6".to_string()];
    release_mounts_with(&w.fake, &log, &devices, &canon);
    let left = fs::read_to_string(&mounts).unwrap();
    assert_eq!(left, "/dev/dm-9 /mnt/other x rw 0 0\n");
    let umounts: Vec<String> = lock(&w.fake.calls)
        .iter()
        .filter(|c| c.name() == "umount")
        .map(|c| c.args.join(" "))
        .collect();
    assert_eq!(
        umounts,
        [
            "--recursive /mnt/a/c",
            "--recursive /mnt/b",
            "--recursive /mnt/a"
        ]
    );
    let text = fs::read_to_string(w.env.log_path()).unwrap();
    assert!(!text.contains("still mounted"), "{text}");
    // nothing left: said so
    release_mounts_with(&w.fake, &log, &devices, &canon);
    let text = fs::read_to_string(w.env.log_path()).unwrap();
    assert!(text.contains("no leftover mounts"), "{text}");
}

#[test]
fn a_failed_disk_listing_still_releases_the_known_nodes() {
    let mut w = World::new();
    w.fake.fail = vec!["lsblk"];
    let mounts = w.env.host("proc/self/mounts");
    fs::write(&mounts, "/dev/sda6[/root] /run/bootc/storage x rw 0 0\n").unwrap();
    let log = Log::create(&w.env.log_path());
    let _ = unmount_target(
        &w.fake,
        &log,
        &w.env,
        Some("/dev/sda"),
        &["/dev/sda6".to_string()],
        false,
    );
    assert_eq!(fs::read_to_string(&mounts).unwrap(), "");
    let text = fs::read_to_string(w.env.log_path()).unwrap();
    assert!(text.contains("cannot list the disks"), "{text}");
}

/// The unit file must stay valid systemd syntax: one corrupted line and
/// the helper doesn't start.
#[test]
fn the_service_unit_is_well_formed() {
    let text = include_str!("../data/systemd/atlas-installer-helper.service");
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let section = line.starts_with('[') && line.ends_with(']') && line.len() > 2;
        let key_value = line
            .split_once('=')
            .is_some_and(|(k, _)| !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphabetic()));
        assert!(section || key_value, "malformed unit line: {line:?}");
    }
    assert!(text.lines().any(|l| l == "LimitMEMLOCK=infinity"));
}

/// A fake /proc entry: `comm`, `cmdline` (NUL-separated), `stat` with its
/// parent, process group and start time, and `io` with `wchar`.
fn fake_proc(root: &Path, pid: u32, comm: &str, args: &[&str], ppid: u32, pgrp: u32, wchar: u64) {
    let d = root.join("proc").join(pid.to_string());
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("comm"), format!("{comm}\n")).unwrap();
    let mut cmdline = args.join("\0");
    cmdline.push('\0');
    fs::write(d.join("cmdline"), cmdline).unwrap();
    // state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt
    // utime stime cutime cstime priority nice threads itrealvalue starttime
    fs::write(
        d.join("stat"),
        format!(
            "{pid} ({comm}) S {ppid} {pgrp} {pgrp} 0 -1 4194560 100 0 0 0 5 5 0 0 20 0 1 0 {} 1000\n",
            1000 + pid
        ),
    )
    .unwrap();
    fs::write(
        d.join("io"),
        format!("rchar: 99\nwchar: {wchar}\nsyscr: 1\nsyscw: 1\n"),
    )
    .unwrap();
}

const PROXY: [&str; 4] = ["skopeo", "experimental-image-proxy", "--sockfd", "3"];

#[test]
fn the_copy_meter_counts_only_bootcs_image_proxies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let helper = 77;
    fake_proc(root, 4240, "bootc", &["bootc", "install"], helper, 4240, 0);
    // the proxy writes each byte twice: to its temporary file, then to bootc
    fake_proc(root, 4242, "skopeo", &PROXY, 4240, 4240, 1_000);
    // in bootc's group, but not the proxy
    fake_proc(root, 4243, "skopeo", &["skopeo", "copy"], 4240, 4240, 7_000);
    // a "bootc" the helper didn't start, and its "proxy"
    fake_proc(root, 4300, "bootc", &["bootc"], 1, 4300, 0);
    fake_proc(root, 4301, "skopeo", &PROXY, 4300, 4300, 9_000);
    // a process that names itself after the group without being in it
    fake_proc(root, 4302, "x) S 77 4240", &PROXY, 4300, 4300, 9_000);
    fs::create_dir_all(root.join("proc/self")).unwrap();
    let mut m = CopyMeter::new(root.join("proc"), helper);
    assert_eq!(m.sample(), 500);
    // a second proxy; the first exits, and its count stays
    fake_proc(root, 4250, "skopeo", &PROXY, 4240, 4240, 600);
    fs::remove_dir_all(root.join("proc/4242")).unwrap();
    assert_eq!(m.sample(), 800);
    // a counted proxy's pid used again by a new proxy: both count
    fs::remove_dir_all(root.join("proc/4250")).unwrap();
    fake_proc(root, 4250, "skopeo", &PROXY, 4240, 4240, 100);
    let stat = root.join("proc/4250/stat");
    let s = fs::read_to_string(&stat)
        .unwrap()
        .replace(" 5250 ", " 9999 ");
    fs::write(&stat, s).unwrap();
    assert_eq!(m.sample(), 850);
    // nothing readable: what was counted stays
    let mut empty = CopyMeter::new(root.join("nowhere"), helper);
    assert_eq!(empty.sample(), 0);
}

#[test]
fn the_copy_moves_the_bar_by_the_bytes_copied() {
    struct Sized(Fake);
    impl Runner for Sized {
        fn run(&self, cmd: &Cmd, on: &mut dyn FnMut(Option<&str>)) -> Result<Output, String> {
            if cmd.name() == "podman" {
                lock(&self.0.calls).push(cmd.clone());
                return ok("1000000000\n");
            }
            self.0.run(cmd, on)
        }
    }
    let w = World::new();
    let helper = std::process::id();
    fake_proc(&w.env.root, 4240, "bootc", &["bootc"], helper, 4240, 0);
    // half the image, written twice
    fake_proc(
        &w.env.root,
        4242,
        "skopeo",
        &PROXY,
        4240,
        4240,
        1_000_000_000,
    );
    // and half of bootc's 128 layers
    let refs = w.env.target.join(BLOB_REFS);
    fs::create_dir_all(&refs).unwrap();
    for i in 0..64 {
        fs::write(refs.join(format!("sha256_3A_{i}")), "").unwrap();
    }
    let f = Sized(Fake::new(&w.env));
    let mut seen = Vec::new();
    install(&f, &w.env, &req("sda", "free-space"), &mut |p| {
        seen.push(p.clone())
    })
    .unwrap();
    let half = Progress::at(Stage::Copy, 0.06 + 0.94 * 0.5).fraction;
    assert!(
        seen.iter().any(|p| (p.fraction - half).abs() < 1e-9),
        "{seen:?}"
    );
    assert!(seen.windows(2).all(|p| p[0].fraction <= p[1].fraction));
    let calls = lock(&f.0.calls);
    let podman = calls.iter().find(|c| c.name() == "podman").unwrap();
    assert_eq!(
        podman.args,
        [
            "image",
            "inspect",
            "--format",
            "{{.Size}}",
            "ghcr.io/eternalcoder454/atlasos:stable"
        ]
    );
    let log = fs::read_to_string(w.env.log_path()).unwrap();
    assert!(log.contains("# the image is 1.00 GB uncompressed"), "{log}");
    assert!(log.contains("# the image proxy handed bootc 0.50 GB of the expected 1.00 GB"));
}

#[test]
fn without_the_image_size_the_copy_still_finishes() {
    // the fake's podman prints nothing
    let w = World::new();
    let (out, seen) = w.install(&req("sda", "free-space"));
    out.unwrap();
    assert!(seen.windows(2).all(|p| p[0].fraction <= p[1].fraction));
    let log = fs::read_to_string(w.env.log_path()).unwrap();
    assert!(log.contains("# the image's size is unknown"), "{log}");
}

#[test]
fn the_copy_log_stops_when_the_layers_are_in() {
    let w = World::new();
    let log = Log::create(&w.env.log_path());
    let mut c = CopyLog::default();
    let s = Duration::from_secs;
    const GB: u64 = 1_000_000_000;
    // nothing copied yet: no line
    c.note(&log, Some((0, 4 * GB)), 0, false, s(0));
    // the first count starts the clock; a line every 30 s
    c.note(&log, Some((GB / 2, 4 * GB)), 10, false, s(2));
    c.note(&log, Some((GB, 4 * GB)), 20, false, s(20));
    c.note(&log, Some((2 * GB, 4 * GB)), 60, false, s(32));
    // all in: a last line at once, then none
    c.note(&log, Some((4 * GB, 4 * GB)), 127, true, s(40));
    c.note(&log, Some((5 * GB, 4 * GB)), 127, true, s(80));
    c.note(&log, Some((5 * GB, 4 * GB)), 127, true, s(200));
    let text = fs::read_to_string(w.env.log_path()).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("# copy")).collect();
    assert_eq!(
        lines,
        [
            "# copy: 2.00 GB of 4.00 GB (50 %), 50.0 MB/s, 60 layers imported",
            "# copy: 4.00 GB of 4.00 GB (100 %), 250.0 MB/s, 127 layers imported",
        ],
        "{text}"
    );
}

#[test]
fn a_copy_in_at_the_first_look_still_gets_its_line() {
    let w = World::new();
    let log = Log::create(&w.env.log_path());
    let mut c = CopyLog::default();
    let s = Duration::from_secs;
    c.note(&log, Some((4_000_000_000, 4_000_000_000)), 127, true, s(2));
    c.note(&log, Some((4_000_000_000, 4_000_000_000)), 127, true, s(40));
    // in by bootc's line before any count: no line, and done
    let mut d = CopyLog::default();
    d.note(&log, Some((0, 4_000_000_000)), 0, true, s(2));
    assert!(d.ended);
    let text = fs::read_to_string(w.env.log_path()).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("# copy")).collect();
    assert_eq!(
        lines,
        ["# copy: 4.00 GB of 4.00 GB (100 %), 127 layers imported"],
        "{text}"
    );
}
