//! Security tests of the install (the Secure phase): what reaches a command
//! line, where the secrets go, what is written into the new system and how,
//! the image and its signature policy, and the static properties of the
//! D-Bus, polkit and systemd files that the design relies on.
//!
//! They run against the same fake system as the tests above, and add nothing
//! to the real one.

use super::*;
use std::sync::Arc;

const CANARY_PW: &str = "CANARY-pw-7f3a";
const CANARY_PIN: &str = "CANARY-pin-91";
const CANARY_PSK: &str = "CANARY-psk-5d2c";

fn all_calls(w: &World) -> Vec<Cmd> {
    lock(&w.fake.calls).clone()
}

/// Every file and link below `root`, without following links.
fn tree(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let t = fs::symlink_metadata(&p).unwrap().file_type();
            if t.is_dir() {
                todo.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn mode_of(p: &Path) -> u32 {
    fs::symlink_metadata(p).unwrap().permissions().mode() & 0o7777
}

fn put_wifi(w: &World, psk: &str) {
    let kf = format!(
        "[connection]\nid=Home\nuuid={UUID}\ntype=wifi\npermissions=user:telamon-installer;\n\n[wifi]\nssid=Home\n\n[wifi-security]\nkey-mgmt=wpa-psk\npsk={psk}\n"
    );
    w.put(
        "etc/NetworkManager/system-connections/Home.nmconnection",
        kf.as_bytes(),
    );
}

fn request(mode: &str, enc: &str, pw: &str, wifi: bool, apps: &[&str]) -> Request {
    Request::new(
        "sda",
        "",
        mode,
        "de_DE.UTF-8",
        "de(nodeadkeys)",
        if wifi { UUID } else { "" },
        enc,
        pw,
    )
    .unwrap()
    .with_apps(&apps.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    .unwrap()
}

fn stateroot(w: &World) -> PathBuf {
    w.deploy().parent().unwrap().parent().unwrap().to_path_buf()
}

// --- argument and path injection ----------------------------------------

/// Every program the helper may run (run::bin).
const PROGRAMS: [&str; 20] = [
    bin::BOOTC,
    bin::CRYPTENROLL,
    bin::CRYPTSETUP,
    bin::EFIBOOTMGR,
    bin::GRUB2_MKPASSWD,
    bin::FINDMNT,
    bin::LSBLK,
    bin::MKFS_BTRFS,
    bin::MKFS_EXT4,
    bin::MKFS_VFAT,
    bin::MOKUTIL,
    bin::MOUNT,
    bin::PODMAN,
    bin::SETFILES,
    bin::SFDISK,
    bin::SYNC,
    bin::SYSTEMCTL,
    bin::UDEVADM,
    bin::UMOUNT,
    bin::WIPEFS,
];

/// The options the helper passes. An argument that starts with `-` and is
/// not one of these is a failure: a value from outside never starts with `-`.
const OPTIONS: [&str; 61] = [
    "--json",
    "--bytes",
    "--paths",
    "--output",
    "--quiet",
    "--wipe-partitions",
    "--wipe",
    "--append",
    "--all",
    "--recursive",
    "--lazy",
    "--deferred",
    "--timeout=30",
    "--type",
    "--cipher",
    "--key-size",
    "--hash",
    "--pbkdf",
    "--batch-mode",
    "--uuid",
    "--label",
    "--key-file",
    "--new-keyfile",
    "--allow-discards",
    "--test-passphrase",
    "--disable-external-tokens",
    "--token-only",
    "--token-type",
    "--dump-json-metadata",
    "--recovery-key",
    "--tpm2-device=auto",
    "--tpm2-pcrs=",
    "--tpm2-with-pin=yes",
    "--source-imgref",
    "--target-imgref",
    "--skip-fetch-check",
    "--enforce-container-sigpolicy",
    "--karg",
    "--format",
    "--create",
    "--disk",
    "--part",
    "--loader",
    "--bootnext",
    "--delete-bootnum",
    "--bootnum",
    "--generate-hash",
    "--import",
    "--hash-file",
    "--test-key",
    "--list",
    "-o",
    "-t",
    "-F",
    "-n",
    "-L",
    "-q",
    "-f",
    "-r",
    "-",
    "--help",
];

fn check_command(c: &Cmd) {
    assert!(PROGRAMS.contains(&c.program), "{c:?}");
    assert!(c.program.starts_with("/usr/bin/") && !c.program.contains(".."));
    for (i, a) in c.args.iter().enumerate() {
        assert!(!a.contains(['\0', '\n', '\r']), "{c:?}: {a:?}");
        if a.starts_with('-') {
            let known = OPTIONS.contains(&a.as_str())
                || a.starts_with("--unlock-key-file=/")
                || a.starts_with("--unlock-key-file=-");
            assert!(known, "{c:?}: unexpected option {a:?}");
            // "--help" is only ever there to be refused
            assert!(a != "--help", "{c:?}");
            // a lone "-" only as the value of a key file option
            if a == "-" {
                assert!(
                    i > 0 && matches!(c.args[i - 1].as_str(), "--key-file" | "--new-keyfile"),
                    "{c:?}"
                );
            }
        }
        if a.starts_with("/dev/") {
            assert!(installer_core::plan::is_device_path(a), "{c:?}: {a:?}");
        }
        // a path argument is absolute or relative to nothing in particular
        assert!(!a.contains("/../"), "{c:?}: {a:?}");
    }
}

#[tokio::test]
async fn nothing_about_a_service_handle_changes_the_checks() {
    // (the matrix below runs the install itself; this one runs the service
    // wrapper around it with the same fake, so the call path D-Bus takes is
    // covered too)
    let w = World::new();
    let World { _dir, env, fake } = w;
    let fake = Arc::new(fake);
    let svc = crate::service::Service::new(fake.clone(), env);
    let r = request("free-space", "none", "", false, &[]);
    svc.do_install(r, async { None }, |_, _, _| {})
        .await
        .unwrap();
    for c in lock(&fake.calls).iter() {
        check_command(c);
    }
}

#[test]
fn every_command_of_every_install_is_a_known_program_with_safe_arguments() {
    let mut runs = 0;
    for mode in ["erase", "free-space"] {
        for (enc, pw) in [
            ("none", ""),
            ("tpm", ""),
            ("tpm-pin", CANARY_PIN),
            ("password", CANARY_PW),
        ] {
            for nvidia in [false, true] {
                let w = World::new().with_tpm();
                put_wifi(&w, CANARY_PSK);
                if nvidia {
                    w.put(NVIDIA_KEY, b"der");
                }
                let out = w
                    .install(&request(mode, enc, pw, true, &["firefox", "gh", "alpaca"]))
                    .0
                    .unwrap_or_else(|e| panic!("{mode} {enc}: {e}"));
                assert_eq!(out.encryption, enc);
                let calls = all_calls(&w);
                assert!(calls.len() > 10, "{mode} {enc}");
                for c in &calls {
                    check_command(c);
                    // a command that holds a secret is marked, so its
                    // arguments stay out of the log
                    let holds =
                        c.stdin.is_some() && c.name() != "sfdisk" || !c.secret_env.is_empty();
                    if holds {
                        assert!(c.secret, "{c:?}");
                    }
                }
                // the disk comes from the probe: every device is /dev/sda or a part of it,
                // or the mapper of the new volume
                for c in &calls {
                    for a in c.args.iter().filter(|a| a.starts_with("/dev/")) {
                        assert!(
                            a.starts_with("/dev/sda") || a.starts_with("/dev/mapper/luks-"),
                            "{c:?}"
                        );
                    }
                }
                runs += 1;
            }
        }
    }
    assert_eq!(runs, 16);
}

#[test]
fn hostile_text_in_the_probe_never_reaches_a_command() {
    // A disk (USB stick) chooses its model, vendor, serial, labels and
    // filesystem UUID. None of that may end up in an argument.
    let nasty = "--force $(reboot) `x`;\nrm -rf /\u{202e}\0";
    let mut v: serde_json::Value = serde_json::from_str(LSBLK).unwrap();
    let disk = &mut v["blockdevices"][0];
    for k in ["model", "vendor", "serial", "wwn", "label", "ptuuid"] {
        disk[k] = nasty.into();
    }
    for c in disk["children"].as_array_mut().unwrap() {
        for k in ["label", "partlabel", "uuid", "partuuid"] {
            c[k] = nasty.into();
        }
    }
    let mut w = World::new();
    w.fake.lsblk_text = Some(v.to_string());
    let (r, _) = w.install(&request("free-space", "none", "", false, &[]));
    // the Windows ESP's volume ID is not a FAT ID here: no Windows entry, but the install goes on
    let out = r.unwrap();
    assert!(!out.windows_entry);
    for c in all_calls(&w) {
        check_command(&c);
        assert!(
            c.args
                .iter()
                .all(|a| !a.contains("reboot") && !a.contains("rm -rf")),
            "{c:?}"
        );
    }
    let custom = w.env.target.join("boot/grub2/custom.cfg");
    assert!(
        !custom.exists(),
        "no Windows entry from a volume ID that isn't one"
    );
}

#[test]
fn a_disk_that_cannot_be_named_by_the_caller_cannot_be_installed_to() {
    // devices whose names the caller couldn't send, or that would be
    // options if they were: offered by lsblk, refused by the request
    for id in [
        "-x",
        "--help",
        "sda1 --force",
        "sda\n",
        "../sda",
        "mapper/ventoy",
        "SDA",
        "",
        &"a".repeat(33),
    ] {
        assert!(Request::new(id, "", "erase", "en_US.UTF-8", "us", "", "none", "").is_err());
    }
    let mut v: serde_json::Value = serde_json::from_str(LSBLK).unwrap();
    let mut evil = v["blockdevices"][0].clone();
    evil["name"] = "/dev/-x".into();
    evil["children"] = serde_json::json!([]);
    v["blockdevices"].as_array_mut().unwrap().push(evil);
    let mut w = World::new();
    w.fake.lsblk_text = Some(v.to_string());
    // the listing may show it; an install to it can't even be asked for
    assert!(Request::new("-x", "", "erase", "en_US.UTF-8", "us", "", "none", "").is_err());
    let out = w
        .install(&request("free-space", "none", "", false, &[]))
        .0
        .unwrap();
    assert!(out.log.ends_with("install.log"));
    for c in all_calls(&w) {
        check_command(&c);
    }
    // the odd device was looked at (its name is a path under /dev, never an option)
    assert!(
        all_calls(&w)
            .iter()
            .any(|c| c.args.last().map(String::as_str) == Some("/dev/-x"))
    );
    assert!(
        all_calls(&w)
            .iter()
            .all(|c| c.name() != "wipefs" || !c.args.iter().any(|a| a.contains("-x")))
    );
}

// --- the image and its signature policy ---------------------------------

#[test]
fn the_image_comes_from_constants_only() {
    assert_eq!(image_ref(false), "ghcr.io/eternalcoder454/atlasos:stable");
    assert_eq!(
        image_ref(true),
        "ghcr.io/eternalcoder454/atlasos-nvidia:stable"
    );
    // whatever the request says, bootc gets one of those two, from containers-storage
    for (locale, keymap, wifi) in [
        ("en_US.UTF-8", "us", ""),
        ("de_DE.UTF-8", "de(nodeadkeys)", UUID),
        ("sr_RS.UTF-8@latin", "rs(latin)", ""),
    ] {
        for nvidia in [false, true] {
            let w = World::new();
            put_wifi(&w, "x");
            if nvidia {
                w.put(NVIDIA_KEY, b"der");
            }
            let r =
                Request::new("sda", "", "free-space", locale, keymap, wifi, "none", "").unwrap();
            w.install(&r).0.unwrap();
            let args = w.bootc_args();
            let want = image_ref(nvidia);
            let at = |flag: &str| {
                let i = args.iter().position(|a| a == flag).unwrap();
                args[i + 1].clone()
            };
            assert_eq!(at("--source-imgref"), format!("containers-storage:{want}"));
            assert_eq!(at("--target-imgref"), want);
        }
    }
    // the reference is not built from a request: it takes only a flag
    let _: fn(bool) -> String = image_ref;
}

fn bootc_enforces(w: &World) -> bool {
    w.bootc_args()
        .iter()
        .any(|a| a == "--enforce-container-sigpolicy")
}

fn sig_warnings(out: &Outcome) -> Vec<&String> {
    out.warnings
        .iter()
        .filter(|w| w.contains("require Telamon OS's signature"))
        .collect()
}

#[test]
fn a_live_system_whose_policy_proves_it_installs_a_signed_origin() {
    for nvidia in [false, true] {
        let w = World::new();
        if nvidia {
            w.put(NVIDIA_KEY, b"der");
        }
        let out = w
            .install(&request("free-space", "none", "", false, &[]))
            .0
            .unwrap();
        assert!(bootc_enforces(&w), "nvidia={nvidia}");
        assert!(sig_warnings(&out).is_empty(), "{:?}", out.warnings);
        let log = fs::read_to_string(w.env.log_path()).unwrap();
        assert!(log.contains("--enforce-container-sigpolicy"));
    }
}

/// Install on a system whose policy has been changed; return whether bootc
/// was asked to enforce it, and the warning about it.
fn with_policy(change: impl FnOnce(&World)) -> (bool, Vec<String>, String) {
    let w = World::new();
    change(&w);
    let out = w
        .install(&request("free-space", "none", "", false, &[]))
        .0
        .unwrap();
    let log = fs::read_to_string(w.env.log_path()).unwrap();
    (
        bootc_enforces(&w),
        sig_warnings(&out).into_iter().cloned().collect(),
        log,
    )
}

#[test]
fn a_policy_that_does_not_prove_it_installs_as_before_and_says_so() {
    fn policy(w: &World, text: &str) {
        w.put("etc/containers/policy.json", text.as_bytes());
    }
    type Change = Box<dyn Fn(&World)>;
    let cases: Vec<(&str, Change)> = vec![
        (
            "no readable container policy",
            Box::new(|w| {
                fs::remove_file(w.env.root.join("etc/containers/policy.json")).unwrap();
            }),
        ),
        ("can't be read", Box::new(|w| policy(w, "{ not json"))),
        (
            "can't be read",
            Box::new(|w| policy(w, &"[".repeat(100_000))),
        ),
        (
            "default accepts any image",
            Box::new(|w| policy(w, r#"{"default":[{"type":"insecureAcceptAnything"}]}"#)),
        ),
        (
            "doesn't ask for a signature",
            Box::new(|w| policy(w, r#"{"default":[{"type":"reject"}]}"#)),
        ),
        (
            "something other than its sigstore signature",
            // the repository is not covered: the transport-wide entry accepts anything
            Box::new(|w| {
                policy(
                    w,
                    &POLICY
                        .replace("ghcr.io/eternalcoder454/atlasos\"", "ghcr.io/x/atlasos\"")
                        .replace("ghcr.io/eternalcoder454/atlasos-nvidia\"", "ghcr.io/x/n\""),
                )
            }),
        ),
        (
            "key of Telamon OS is missing",
            Box::new(|w| {
                fs::remove_file(w.env.root.join("etc/pki/containers/telamon.pub")).unwrap()
            }),
        ),
        (
            "key of Telamon OS is missing",
            Box::new(|w| w.put("etc/pki/containers/telamon.pub", b"")),
        ),
        (
            "not set to look for Telamon OS's signatures",
            Box::new(|w| {
                fs::remove_file(w.env.root.join("etc/containers/registries.d/telamon.yaml"))
                    .unwrap()
            }),
        ),
        (
            "not set to look for Telamon OS's signatures",
            Box::new(|w| {
                w.put(
                    "etc/containers/registries.d/telamon.yaml",
                    b"docker:\n  ghcr.io/eternalcoder454:\n    use-sigstore-attachments: false\n",
                )
            }),
        ),
        (
            "not set to look for Telamon OS's signatures",
            // a setting for another registry says nothing about this image
            Box::new(|w| {
                w.put(
                    "etc/containers/registries.d/telamon.yaml",
                    b"docker:\n  docker.io:\n    use-sigstore-attachments: true\n",
                )
            }),
        ),
        (
            "not set to look for Telamon OS's signatures",
            // containers/image reads *.yaml only
            Box::new(|w| {
                fs::remove_file(w.env.root.join("etc/containers/registries.d/telamon.yaml"))
                    .unwrap();
                w.put(
                    "etc/containers/registries.d/telamon.yml",
                    b"docker:\n  ghcr.io/eternalcoder454:\n    use-sigstore-attachments: true\n",
                )
            }),
        ),
        (
            "no readable container policy",
            // a policy too big to be one
            Box::new(|w| w.put("etc/containers/policy.json", &vec![b' '; 2 << 20])),
        ),
    ];
    for (why, change) in cases {
        let (enforced, warnings, log) = with_policy(change);
        assert!(!enforced, "{why}");
        assert_eq!(warnings.len(), 1, "{why}: {warnings:?}");
        assert!(warnings[0].contains(why), "{why}: {warnings:?}");
        assert!(
            warnings[0].starts_with("The new system was not told to require"),
            "{warnings:?}"
        );
        assert!(
            log.contains("update signatures: not enforced at install"),
            "{log}"
        );
    }
}

#[test]
fn the_vendors_policy_file_is_used_when_there_is_no_local_one() {
    // bootc reads /etc/containers/policy.json, else /usr/share/containers/policy.json
    let w = World::new();
    fs::remove_file(w.env.root.join("etc/containers/policy.json")).unwrap();
    w.put("usr/share/containers/policy.json", POLICY.as_bytes());
    w.install(&request("free-space", "none", "", false, &[]))
        .0
        .unwrap();
    assert!(bootc_enforces(&w));
    // but the first one that exists decides, even when it is the worse one
    let w = World::new();
    w.put(
        "etc/containers/policy.json",
        br#"{"default":[{"type":"insecureAcceptAnything"}]}"#,
    );
    w.put("usr/share/containers/policy.json", POLICY.as_bytes());
    let out = w
        .install(&request("free-space", "none", "", false, &[]))
        .0
        .unwrap();
    assert!(!bootc_enforces(&w));
    assert_eq!(sig_warnings(&out).len(), 1);
}

#[test]
fn the_plan_says_whether_updates_are_checked() {
    let w = World::new();
    let r = request("free-space", "none", "", false, &[]);
    let p = prepare(&w.fake, &w.env, &r).unwrap();
    let text = describe(&p, &r, &w.env);
    assert!(text.contains("--enforce-container-sigpolicy"), "{text}");
    assert!(text.contains("update signatures: the new system is told"));
    fs::remove_file(w.env.root.join("etc/containers/policy.json")).unwrap();
    let p = prepare(&w.fake, &w.env, &r).unwrap();
    let text = describe(&p, &r, &w.env);
    assert!(!text.contains("--enforce-container-sigpolicy"));
    assert!(text.contains("update signatures: not enforced at install"));
}

// --- the TPM without Secure Boot ----------------------------------------

#[test]
fn a_tpm_disk_on_a_pc_without_secure_boot_comes_with_a_warning() {
    for (enc, pw, word) in [
        ("tpm", "", "opens the disk for any system"),
        ("tpm-pin", CANARY_PIN, "The PIN you chose"),
    ] {
        let w = World::new().with_tpm();
        w.put(SECURE_BOOT_VAR, &[6, 0, 0, 0, 0]);
        let out = w
            .install(&request("free-space", enc, pw, false, &[]))
            .0
            .unwrap();
        let sb: Vec<_> = out
            .warnings
            .iter()
            .filter(|x| x.contains("Secure Boot is off"))
            .collect();
        assert_eq!(sb.len(), 1, "{enc}: {:?}", out.warnings);
        assert!(sb[0].contains(word), "{enc}: {sb:?}");
        assert!(!sb[0].contains(CANARY_PIN));
        // the install itself is unchanged: enrolled, sealed at the first start
        assert!(w.deploy().join("etc/telamon-installer/tpm-seal").exists());
        // no variable at all (a BIOS, or no efivars): off
        let w = World::new().with_tpm();
        fs::remove_file(w.env.root.join(SECURE_BOOT_VAR)).unwrap();
        let out = w
            .install(&request("free-space", enc, pw, false, &[]))
            .0
            .unwrap();
        assert!(
            out.warnings
                .iter()
                .any(|x| x.contains("Secure Boot is off")),
            "{enc}"
        );
    }
    // on, or no TPM involved: nothing to say
    for (enc, pw) in [("tpm", ""), ("tpm-pin", CANARY_PIN)] {
        let w = World::new().with_tpm();
        let out = w
            .install(&request("free-space", enc, pw, false, &[]))
            .0
            .unwrap();
        assert!(out.warnings.is_empty(), "{enc}: {:?}", out.warnings);
    }
    for (enc, pw) in [("none", ""), ("password", CANARY_PW)] {
        let w = World::new().with_tpm();
        w.put(SECURE_BOOT_VAR, &[6, 0, 0, 0, 0]);
        let out = w
            .install(&request("free-space", enc, pw, false, &[]))
            .0
            .unwrap();
        assert!(out.warnings.is_empty(), "{enc}: {:?}", out.warnings);
    }
}

// --- secrets -------------------------------------------------------------

#[test]
fn canaries_reach_only_the_commands_that_need_them() {
    for (enc, pw) in [
        ("password", CANARY_PW),
        ("tpm-pin", CANARY_PIN),
        ("tpm", ""),
        ("none", ""),
    ] {
        let w = World::new().with_tpm();
        put_wifi(&w, CANARY_PSK);
        let r = request("free-space", enc, pw, true, &["firefox", "gh"]);
        let out = w.install(&r).0.unwrap();
        let calls = all_calls(&w);
        let log = fs::read_to_string(w.env.log_path()).unwrap();
        let saved = fs::read_to_string(stateroot(&w).join("var/log/telamon-installer/install.log"))
            .unwrap();
        let all_secrets = [
            CANARY_PW,
            CANARY_PIN,
            CANARY_PSK,
            RECOVERY_KEY,
            "ulcbjnni-ehtlc",
        ];

        for c in &calls {
            let shown = format!("{c:?}");
            for s in all_secrets {
                assert!(!c.program.contains(s), "{enc}: {c:?}");
                assert!(!c.args.iter().any(|a| a.contains(s)), "{enc}: {c:?}");
                assert!(!shown.contains(s), "{enc}: Debug of {c:?}");
                assert!(!c.display().contains(s), "{enc}: {}", c.display());
            }
            let on_stdin = |s: &str| c.stdin.as_deref().is_some_and(|i| i.contains(s));
            let testing = c.args.iter().any(|a| a == "--test-passphrase");
            // the password: when it is added, and when it is proven to unlock
            if on_stdin(CANARY_PW) {
                assert!(
                    c.args.first().map(String::as_str) == Some("luksAddKey") || testing,
                    "{enc}: {c:?}"
                );
                assert!(c.secret);
            }
            // the recovery key: only to prove it unlocks
            if on_stdin(RECOVERY_KEY) {
                assert!(testing && c.secret, "{enc}: {c:?}");
            }
            assert!(
                !on_stdin(CANARY_PIN) && !on_stdin(CANARY_PSK),
                "{enc}: {c:?}"
            );
            // the PIN: in the environment of the one enrolment, as NEWPIN
            for (k, v) in &c.secret_env {
                assert_eq!(*k, "NEWPIN", "{enc}");
                assert_eq!(v, CANARY_PIN, "{enc}");
                assert!(
                    c.name() == "systemd-cryptenroll" && c.secret,
                    "{enc}: {c:?}"
                );
                assert!(c.args.iter().any(|a| a == "--tpm2-with-pin=yes"));
            }
        }
        assert_eq!(
            calls.iter().filter(|c| !c.secret_env.is_empty()).count(),
            usize::from(enc == "tpm-pin"),
            "{enc}"
        );

        // the logs
        for s in all_secrets {
            assert!(!log.contains(s), "{enc}: install.log has {s}");
            assert!(!saved.contains(s), "{enc}: the saved log has {s}");
        }
        // the saved copy is the log up to the point it was kept
        assert!(log.starts_with(saved.trim_end_matches('\n')), "{enc}");
        // Debug of everything a panic message or a log line could print
        let outcome = format!("{out:?}");
        let request_debug = format!("{r:?}");
        let prepared = format!("{:?}", prepare(&w.fake, &w.env, &r).ok());
        for s in all_secrets {
            assert!(!outcome.contains(s), "{enc}: {outcome}");
            assert!(!request_debug.contains(s), "{enc}: {request_debug}");
            assert!(!prepared.contains(s), "{enc}: {prepared}");
        }

        // where the secrets are on disk afterwards: only where they belong
        let mut holders: Vec<(String, Vec<&str>)> = Vec::new();
        for root in [&w.env.target, &w.env.run_dir] {
            for f in tree(root) {
                let Ok(bytes) = fs::read(&f) else { continue };
                let text = String::from_utf8_lossy(&bytes);
                let hit: Vec<&str> = all_secrets
                    .iter()
                    .copied()
                    .filter(|s| text.contains(s))
                    .collect();
                if !hit.is_empty() {
                    let rel = f
                        .strip_prefix(&w.env.target)
                        .unwrap_or(&f)
                        .display()
                        .to_string();
                    holders.push((rel, hit));
                }
            }
        }
        let mut want: Vec<(String, Vec<&str>)> = vec![(
            "ostree/deploy/default/deploy/abc123.0/etc/NetworkManager/system-connections/Home.nmconnection"
                .into(),
            vec![CANARY_PSK],
        )];
        if enc == "tpm-pin" {
            want.push((
                "ostree/deploy/default/deploy/abc123.0/etc/telamon-installer/tpm-pin".into(),
                vec![CANARY_PIN],
            ));
        }
        holders.sort();
        want.sort();
        assert_eq!(holders, want, "{enc}");
        if enc == "tpm-pin" {
            let pin = w.deploy().join("etc/telamon-installer/tpm-pin");
            assert_eq!(mode_of(&pin), 0o600);
            assert_eq!(mode_of(pin.parent().unwrap()), 0o700);
            assert_eq!(fs::read_to_string(&pin).unwrap(), CANARY_PIN, "no newline");
        }
        let keyfile = w
            .deploy()
            .join("etc/NetworkManager/system-connections/Home.nmconnection");
        assert_eq!(mode_of(&keyfile), 0o600);
        // the log copy in the new system is private
        let saved_path = stateroot(&w).join("var/log/telamon-installer/install.log");
        assert_eq!(mode_of(&saved_path), 0o600);
        assert_eq!(mode_of(saved_path.parent().unwrap()), 0o700);
        assert_eq!(mode_of(&w.env.log_path()), 0o600);
        // and the helper's run directory is root's alone (it was 0755 once)
        assert_eq!(mode_of(&w.env.run_dir), 0o700);
    }
}

#[test]
fn the_run_directory_is_private_whether_it_existed_or_not() {
    let w = World::new();
    fs::create_dir_all(&w.env.run_dir).unwrap();
    fs::set_permissions(&w.env.run_dir, fs::Permissions::from_mode(0o755)).unwrap();
    w.install(&request("free-space", "none", "", false, &[]))
        .0
        .unwrap();
    assert_eq!(mode_of(&w.env.run_dir), 0o700);
    // ListDisks mounts an ESP below it to look: that happens inside the private directory too
    let w = World::new();
    fs::set_permissions(&w.env.run_dir, fs::Permissions::from_mode(0o755)).unwrap();
    list_disks(&w.fake, &w.env).unwrap();
    assert_eq!(mode_of(&w.env.run_dir), 0o700);
}

#[tokio::test]
async fn status_never_hands_secrets_to_a_caller_who_may_not_install() {
    for (enc, pw) in [("password", CANARY_PW), ("tpm-pin", CANARY_PIN)] {
        let w = World::new().with_tpm();
        let World { _dir, env, fake } = w;
        let svc = crate::service::Service::new(Arc::new(fake), env);
        let r = request("free-space", enc, pw, false, &[]);
        let json = svc
            .do_install(r, async { None }, |_, _, _| {})
            .await
            .unwrap();
        // the caller who installed gets the recovery key in the reply, and in Status
        assert!(json.contains(RECOVERY_KEY));
        let granted = svc.do_status(true).unwrap();
        assert!(granted.contains(RECOVERY_KEY));
        // anyone else gets neither it nor anything else secret
        let shown = svc.do_status(false).unwrap();
        for s in [RECOVERY_KEY, "ulcbjnni", CANARY_PW, CANARY_PIN] {
            assert!(!shown.contains(s), "{enc}: {shown}");
        }
        let v: serde_json::Value = serde_json::from_str(&shown).unwrap();
        assert_eq!(v["state"], "done");
        assert!(v["result"]["recovery_key"].is_null());
        assert!(v["result"]["mok_password"].is_null());
        assert_eq!(v["result"]["encryption"], enc);
        // the log path is public; its contents are 0600
        assert!(
            v["result"]["log"]
                .as_str()
                .unwrap()
                .ends_with("install.log")
        );
    }
}

#[test]
fn a_failed_secret_command_never_puts_its_output_in_the_error() {
    // a tool that prints a secret and then fails
    struct Chatty;
    impl Runner for Chatty {
        fn run(&self, _: &Cmd, _: &mut dyn FnMut(Option<&str>)) -> Result<Output, String> {
            Ok(Output {
                code: Some(1),
                stdout: format!("{RECOVERY_KEY}\n{CANARY_PW}\n"),
                stderr: "cryptsetup said no".into(),
            })
        }
    }
    let c = Cmd::new(bin::CRYPTENROLL, ["--recovery-key"]).secret();
    let e = run_secret(&Chatty, c).unwrap_err();
    assert!(!e.contains("ulcbjnni") && !e.contains(CANARY_PW), "{e}");
    assert!(e.contains("cryptsetup said no"), "{e}");
    // and the plain `run` of a non-secret command keeps its stdout for the log only
    struct Quiet;
    impl Runner for Quiet {
        fn run(&self, _: &Cmd, _: &mut dyn FnMut(Option<&str>)) -> Result<Output, String> {
            Ok(Output {
                code: Some(1),
                stdout: RECOVERY_KEY.to_string(),
                stderr: String::new(),
            })
        }
    }
    // (this is why every command that can print a secret is marked `secret`)
    let plain = run(&Quiet, Cmd::new(bin::CRYPTENROLL, ["--recovery-key"])).unwrap_err();
    assert!(
        plain.contains("ulcbjnni"),
        "documents why the marking matters: {plain}"
    );
}

#[test]
fn the_pin_directory_is_private_even_when_the_image_ships_it_open() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("deploy");
    let d = base.join("etc/telamon-installer");
    fs::create_dir_all(&d).unwrap();
    fs::set_permissions(&d, fs::Permissions::from_mode(0o777)).unwrap();
    fs::write(d.join("tpm-pin"), "old").unwrap();
    fs::set_permissions(d.join("tpm-pin"), fs::Permissions::from_mode(0o666)).unwrap();
    write_private(
        &base,
        "etc/telamon-installer/tpm-pin",
        CANARY_PIN.as_bytes(),
    )
    .unwrap();
    assert_eq!(mode_of(&d), 0o700);
    assert_eq!(mode_of(&d.join("tpm-pin")), 0o600);
    assert_eq!(fs::read_to_string(d.join("tpm-pin")).unwrap(), CANARY_PIN);
    // a directory that was only passed through keeps its mode
    let etc = base.join("etc");
    assert_eq!(mode_of(&etc), 0o755 & !0o022 | (mode_of(&etc) & 0o755));
    let public = base.join("etc/other");
    fs::create_dir(&public).unwrap();
    fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();
    write_file(&base, "etc/other/f", "x", 0o644).unwrap();
    assert_eq!(mode_of(&public), 0o755);
}

// --- the files written into the new system --------------------------------

/// Every place the install writes below the target, relative to it (for the
/// fake deployment `abc123.0`).
const WRITES: [&str; 17] = [
    "ostree/deploy/default/deploy/abc123.0/etc/locale.conf",
    "ostree/deploy/default/deploy/abc123.0/etc/vconsole.conf",
    "ostree/deploy/default/deploy/abc123.0/etc/X11/xorg.conf.d/00-keyboard.conf",
    "ostree/deploy/default/deploy/abc123.0/etc/telamon/installer.ini",
    "ostree/deploy/default/deploy/abc123.0/etc/atlasos/installer.ini",
    "ostree/deploy/default/deploy/abc123.0/etc/NetworkManager/system-connections/Home.nmconnection",
    "ostree/deploy/default/deploy/abc123.0/etc/systemd/system/telamon-tpm-seal.service",
    "ostree/deploy/default/deploy/abc123.0/etc/systemd/system/multi-user.target.wants/telamon-tpm-seal.service",
    "ostree/deploy/default/deploy/abc123.0/etc/telamon-installer/tpm-seal",
    "ostree/deploy/default/deploy/abc123.0/etc/telamon-installer/tpm-pin",
    "ostree/deploy/default/var/lib/telamon/first-boot-apps.json",
    "ostree/deploy/default/var/lib/atlasos/first-boot-apps.json",
    "ostree/deploy/default/var/log/telamon-installer/install.log",
    "ostree/deploy/default/var/log/telamon-installer/tpm-event-log.bin",
    "boot/grub2/user.cfg",
    "boot/grub2/custom.cfg",
    "ostree/deploy/default/deploy/abc123.0/etc/selinux/targeted/contexts/files/file_contexts",
];

/// A maximal install: encrypted with a PIN, Wi-Fi, apps, Windows beside it.
fn maximal(w: &World) -> Request {
    put_wifi(w, CANARY_PSK);
    w.put(
        "sys/kernel/security/tpm0/binary_bios_measurements",
        b"\x00\x01event log",
    );
    request(
        "free-space",
        "tpm-pin",
        CANARY_PIN,
        true,
        &["firefox", "gh"],
    )
}

#[test]
fn the_install_writes_exactly_these_places_and_nothing_of_the_live_session() {
    let w = World::new().with_tpm();
    let r = maximal(&w);
    w.install(&r).0.unwrap();
    let files: Vec<String> = tree(&w.env.target)
        .into_iter()
        .map(|p| p.strip_prefix(&w.env.target).unwrap().display().to_string())
        .collect();
    for rel in WRITES {
        // each one is a regular file (or the enable link), made by the install
        assert!(
            files.iter().any(|f| f == rel),
            "{rel} missing in {files:#?}"
        );
    }
    // the fake bootc made the origin and the SELinux file; everything else is ours
    let ours: Vec<&String> = files
        .iter()
        .filter(|f| !WRITES.contains(&f.as_str()) && !f.ends_with(".origin"))
        .collect();
    assert!(
        ours.is_empty(),
        "unexpected files in the new system: {ours:?}"
    );
    // none of the live session's files, accounts or rules
    for f in &files {
        for live in [
            "telamon-installer-session",
            "50-telamon",
            "plasmalogin",
            "sysusers",
            "polkit",
        ] {
            assert!(!f.contains(live), "{f}");
        }
    }
    // modes
    let d = w.deploy();
    for (rel, mode) in [
        ("etc/locale.conf", 0o644),
        ("etc/vconsole.conf", 0o644),
        ("etc/X11/xorg.conf.d/00-keyboard.conf", 0o644),
        ("etc/telamon/installer.ini", 0o644),
        ("etc/atlasos/installer.ini", 0o644),
        (
            "etc/NetworkManager/system-connections/Home.nmconnection",
            0o600,
        ),
        ("etc/systemd/system/telamon-tpm-seal.service", 0o644),
        ("etc/telamon-installer/tpm-seal", 0o600),
        ("etc/telamon-installer/tpm-pin", 0o600),
    ] {
        assert_eq!(mode_of(&d.join(rel)), mode, "{rel}");
    }
    assert_eq!(mode_of(&d.join("etc/telamon-installer")), 0o700);
    let root = stateroot(&w);
    for rel in [
        "var/log/telamon-installer/install.log",
        "var/log/telamon-installer/tpm-event-log.bin",
    ] {
        assert_eq!(mode_of(&root.join(rel)), 0o600, "{rel}");
    }
    assert_eq!(mode_of(&root.join("var/log/telamon-installer")), 0o700);
    for rel in [
        "var/lib/telamon/first-boot-apps.json",
        "var/lib/atlasos/first-boot-apps.json",
    ] {
        assert_eq!(mode_of(&root.join(rel)), 0o644, "{rel}");
    }
    assert_eq!(mode_of(&w.env.target.join("boot/grub2/user.cfg")), 0o600);
    assert_eq!(mode_of(&w.env.target.join("boot/grub2/custom.cfg")), 0o644);
    // ownership: all root's (the tests run as whoever runs them; the helper as root)
    use std::os::unix::fs::MetadataExt;
    let me = rustix::process::geteuid().as_raw();
    for f in tree(&w.env.target) {
        assert_eq!(fs::symlink_metadata(&f).unwrap().uid(), me, "{f:?}");
    }
}

#[test]
fn a_hostile_deployment_tree_is_never_written_through() {
    // Symlinks at every place the install writes, and at each directory on
    // the way: they point at a directory outside the new system, which must
    // stay empty, and at a file there, which must stay as it is.
    let rels: Vec<&str> = WRITES
        .iter()
        .copied()
        .filter(|r| !r.ends_with("file_contexts"))
        .collect();
    let mut places: Vec<String> = Vec::new();
    for rel in &rels {
        // the file itself, then each directory above it (below the deployment, the state
        // root's var, and /boot)
        places.push((*rel).to_string());
        let mut p = Path::new(rel).parent();
        while let Some(dir) = p {
            let s = dir.to_str().unwrap();
            // the deployment and the state root exist already and hold the other files
            let deployment = "ostree/deploy/default/deploy/abc123.0/etc";
            if !s.is_empty()
                && !deployment.starts_with(s)
                && s != "ostree/deploy/default/var"
                && s != "ostree/deploy/default/var/lib"
                && s != "ostree/deploy/default/var/log"
                && s != "boot"
                && !s.ends_with("deploy/abc123.0/etc/selinux")
            {
                places.push(s.to_string());
            }
            p = dir.parent();
        }
    }
    places.sort();
    places.dedup();
    assert!(places.len() > 25, "{places:?}");
    for place in places {
        let w = World::new().with_tpm();
        let r = maximal(&w);
        let victim = w.env.root.join("victim");
        fs::create_dir_all(&victim).unwrap();
        fs::write(victim.join("file"), "keep").unwrap();
        let mut w = w;
        w.fake.plant = vec![(place.clone(), victim.clone())];
        // a link where a file goes is replaced by the file; a link where a
        // directory goes fails the write (the install, or only a warning for
        // the logs and apps): either way nothing is written through it
        let (res, _) = w.install(&r);
        match &res {
            Ok(out) => {
                // refusing is not required of a link to a file
                let _ = out;
            }
            Err(e) => {
                assert!(
                    e.contains("is not a directory")
                        || e.contains("cannot write")
                        || e.contains("Locking the boot menu failed"),
                    "{place}: {e}"
                );
            }
        }
        let after: Vec<_> = fs::read_dir(&victim)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            after,
            ["file"],
            "{place}: something was written through the link"
        );
        assert_eq!(
            fs::read_to_string(victim.join("file")).unwrap(),
            "keep",
            "{place}"
        );
        assert!(!w.key_path().exists());
    }
}

#[test]
fn files_planted_at_the_temporary_names_are_not_followed() {
    // place() writes `.<name>.telamon-tmp-<pid>` first: a link left there is
    // unlinked, not written through
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("deploy");
    fs::create_dir_all(base.join("etc")).unwrap();
    let victim = dir.path().join("victim");
    fs::write(&victim, "keep").unwrap();
    let tmp = base.join(format!("etc/.f.telamon-tmp-{}", std::process::id()));
    std::os::unix::fs::symlink(&victim, &tmp).unwrap();
    write_file(&base, "etc/f", "new", 0o600).unwrap();
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
    assert_eq!(fs::read_to_string(base.join("etc/f")).unwrap(), "new");
    // a hard link to the victim at the final name: the rename replaces the name, not the file
    fs::hard_link(&victim, base.join("etc/g")).unwrap();
    write_file(&base, "etc/g", "new", 0o600).unwrap();
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
}

#[test]
fn paths_with_dots_empty_parts_or_nul_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("deploy");
    fs::create_dir_all(&base).unwrap();
    for bad in [
        "", "/", "a//b", "../x", "a/../x", "a/./b", "a/", "./x", "a/..", "a\0b",
    ] {
        let r = write_file(&base, bad, "x", 0o644);
        // (a NUL in a name is refused by the system call)
        assert!(r.is_err(), "{bad:?}");
    }
    assert_eq!(tree(dir.path()).len(), 0);
    let _ = &Env::system;
}

// --- the Wi-Fi keyfile ---------------------------------------------------

#[test]
fn only_the_chosen_wifi_connection_is_copied_and_only_if_it_is_wifi() {
    let w = World::new();
    put_wifi(&w, CANARY_PSK);
    w.put(
        "etc/NetworkManager/system-connections/Other.nmconnection",
        format!("[connection]\nid=Other\nuuid=11111111-2a0c-4d5e-9f1a-3c2b1a0d9e8f\ntype=wifi\n[wifi-security]\npsk=OTHER-{CANARY_PSK}\n").as_bytes(),
    );
    w.put(
        "etc/NetworkManager/system-connections/vpn.nmconnection",
        format!("[connection]\nid=Work\nuuid=22222222-2a0c-4d5e-9f1a-3c2b1a0d9e8f\ntype=vpn\n[vpn]\nsecret={CANARY_PSK}\n").as_bytes(),
    );
    w.install(&request("free-space", "none", "", true, &[]))
        .0
        .unwrap();
    let conns = w.deploy().join("etc/NetworkManager/system-connections");
    let names: Vec<_> = fs::read_dir(&conns)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(names, ["Home.nmconnection"]);
    let copy = fs::read_to_string(conns.join("Home.nmconnection")).unwrap();
    assert!(
        copy.contains(CANARY_PSK) && !copy.contains("OTHER-") && !copy.contains("permissions=")
    );

    // a VPN profile (or anything that is not Wi-Fi) is not found, so the install stops before it writes
    let w = World::new();
    w.put(
        "etc/NetworkManager/system-connections/vpn.nmconnection",
        format!("[connection]\nid=Work\nuuid={UUID}\ntype=vpn\n[vpn]\nsecret={CANARY_PSK}\n")
            .as_bytes(),
    );
    let e = w
        .install(&request("free-space", "none", "", true, &[]))
        .0
        .unwrap_err();
    assert!(e.contains("not found"), "{e}");
    assert!(w.writes().is_empty());
    // a profile with a NUL or an escape in it, or a huge one, is not copied either
    for body in [
        format!("[connection]\nuuid={UUID}\ntype=wifi\n\0\n"),
        format!("[connection]\nuuid={UUID}\ntype=wifi\n\u{1b}[2J\n"),
        format!(
            "[connection]\nuuid={UUID}\ntype=wifi\n#{}\n",
            "x".repeat(70_000)
        ),
    ] {
        let w = World::new();
        w.put(
            "etc/NetworkManager/system-connections/Home.nmconnection",
            body.as_bytes(),
        );
        assert!(
            w.install(&request("free-space", "none", "", true, &[]))
                .0
                .is_err()
        );
        assert!(w.writes().is_empty());
    }
    // a link in the connections directory (a file that is not one) is not read
    let w = World::new();
    let real = w.env.root.join("elsewhere.nmconnection");
    fs::write(&real, format!("[connection]\nuuid={UUID}\ntype=wifi\n")).unwrap();
    fs::create_dir_all(w.env.root.join("etc/NetworkManager/system-connections")).unwrap();
    std::os::unix::fs::symlink(
        &real,
        w.env
            .root
            .join("etc/NetworkManager/system-connections/Home.nmconnection"),
    )
    .unwrap();
    assert!(
        w.install(&request("free-space", "none", "", true, &[]))
            .0
            .is_err()
    );
}

#[test]
fn the_prepared_install_shows_the_wifi_file_name_and_not_its_password() {
    let w = World::new();
    put_wifi(&w, CANARY_PSK);
    let r = request("free-space", "none", "", true, &[]);
    let p = prepare(&w.fake, &w.env, &r).unwrap();
    assert!(p.wifi.as_ref().unwrap().1.contains(CANARY_PSK));
    let shown = format!("{p:?}");
    assert!(
        shown.contains("Home.nmconnection") && !shown.contains(CANARY_PSK),
        "{shown}"
    );
    assert!(!describe(&p, &r, &w.env).contains(CANARY_PSK));
}

// --- LUKS parameters ------------------------------------------------------

#[test]
fn the_luks_parameters_are_named_and_not_left_to_the_build_of_cryptsetup() {
    for (enc, pw) in [("tpm", ""), ("password", CANARY_PW)] {
        let w = World::new().with_tpm();
        w.install(&request("free-space", enc, pw, false, &[]))
            .0
            .unwrap();
        let calls = all_calls(&w);
        let fmt = calls
            .iter()
            .find(|c| c.args.first().map(String::as_str) == Some("luksFormat"))
            .unwrap();
        let line = fmt.args.join(" ");
        for want in [
            "--type luks2",
            "--cipher aes-xts-plain64",
            "--key-size 512",
            "--hash sha256",
            "--pbkdf argon2id",
            "--batch-mode",
        ] {
            assert!(line.contains(want), "{line}");
        }
        if enc == "password" {
            let add = calls
                .iter()
                .find(|c| c.args.first().map(String::as_str) == Some("luksAddKey"))
                .unwrap();
            assert!(
                add.args
                    .join(" ")
                    .starts_with("luksAddKey --pbkdf argon2id "),
                "{add:?}"
            );
        }
        // nothing weakens it: no option that lowers the key derivation's cost
        for c in &calls {
            for a in &c.args {
                assert!(
                    !a.starts_with("--pbkdf-force-iterations")
                        && !a.starts_with("--iter-time")
                        && !a.starts_with("--pbkdf-memory")
                        && !a.starts_with("--pbkdf-parallel")
                        && a != "pbkdf2",
                    "{a}"
                );
            }
        }
    }
}

// --- static properties of the files the design relies on ------------------

fn unit_keys(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('['))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn has(keys: &[(String, String)], k: &str, v: &str) -> bool {
    keys.iter().any(|(a, b)| a == k && b == v)
}

fn lacks(keys: &[(String, String)], k: &str) -> bool {
    keys.iter().all(|(a, _)| a != k)
}

#[test]
fn the_polkit_actions_default_to_no_for_everyone_but_an_active_local_session() {
    let text = include_str!("../data/polkit-1/actions/net.eterneon.telamon.installer.policy");
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("<action id=\"") {
        rest = &rest[i + "<action id=\"".len()..];
        let id = &rest[..rest.find('"').unwrap()];
        let block = &rest[..rest.find("</action>").unwrap()];
        let default = |name: &str| {
            let open = format!("<{name}>");
            let a = block.find(&open).unwrap() + open.len();
            block[a..a + block[a..].find('<').unwrap()].to_string()
        };
        assert_eq!(default("allow_any"), "no", "{id}");
        assert_eq!(default("allow_inactive"), "no", "{id}");
        let active = default("allow_active");
        // only Install asks an administrator; the other two are for the session
        match id {
            "net.eterneon.telamon.installer.install" => assert_eq!(active, "auth_admin_keep"),
            _ => assert_eq!(active, "yes", "{id}"),
        }
        assert!(!block.contains("org.freedesktop.policykit.imply"), "{id}");
        found.push(id.to_string());
    }
    found.sort();
    let mut want = vec![
        crate::service::action::LIST_DISKS.to_string(),
        crate::service::action::INSTALL.to_string(),
        crate::service::action::REBOOT.to_string(),
    ];
    want.sort();
    assert_eq!(
        found, want,
        "the policy file and the helper name the same actions"
    );
}

#[test]
fn only_root_owns_the_name_and_everything_else_goes_through_polkit() {
    let conf = include_str!("../data/dbus-1/system.d/net.eterneon.telamon.InstallerHelper.conf");
    let owners: Vec<&str> = conf.lines().filter(|l| l.contains("allow own=")).collect();
    assert_eq!(owners.len(), 1, "{owners:?}");
    // the one owner line is in root's policy, before the default policy
    let own_at = conf.find("allow own=").unwrap();
    assert!(conf[..own_at].contains("<policy user=\"root\">"));
    assert!(!conf[conf.find("<policy context=\"default\">").unwrap()..].contains("allow own"));
    // the default policy only lets callers send to the helper, and only these interfaces
    let sends: Vec<&str> = conf
        .lines()
        .filter(|l| l.contains("send_interface="))
        .collect();
    assert_eq!(sends.len(), 4);
    for l in conf.lines().filter(|l| l.contains("<allow")) {
        assert!(
            l.contains("allow own=\"net.eterneon.telamon.InstallerHelper\"")
                || l.contains("send_destination=\"net.eterneon.telamon.InstallerHelper\""),
            "{l}"
        );
    }
    for bad in [
        "send_type",
        "receive_",
        "user=\"*\"",
        "<allow own_prefix",
        "group=",
    ] {
        assert!(!conf.contains(bad), "{bad}");
    }
    // activation is through systemd, as root, running the unit we have
    let act =
        include_str!("../data/dbus-1/system-services/net.eterneon.telamon.InstallerHelper.service");
    let k = unit_keys(act);
    assert!(
        has(&k, "User", "root") && has(&k, "SystemdService", "telamon-installer-helper.service")
    );
    assert!(
        has(&k, "Exec", "/bin/false"),
        "never started by dbus-daemon itself"
    );
    let unit = unit_keys(include_str!(
        "../data/systemd/telamon-installer-helper.service"
    ));
    assert!(
        has(&unit, "BusName", "net.eterneon.telamon.InstallerHelper") && has(&unit, "Type", "dbus")
    );
    assert!(has(
        &unit,
        "ExecStart",
        "/usr/libexec/telamon-installer-helper"
    ));
}

#[test]
fn the_helper_unit_has_what_it_claims_and_lacks_what_it_says_it_leaves_off() {
    let text = include_str!("../data/systemd/telamon-installer-helper.service");
    let keys = unit_keys(text);
    for (k, v) in [
        ("LimitCORE", "0"),
        ("LimitMEMLOCK", "infinity"),
        ("LockPersonality", "yes"),
        ("RestrictRealtime", "yes"),
        ("SystemCallArchitectures", "native"),
        ("KillMode", "mixed"),
    ] {
        assert!(has(&keys, k, v), "{k}={v}");
    }
    // The comments name what is left off on purpose; a key that is set must
    // go from that list, so the unit and what it says about itself agree.
    let left_off: &[&str] = &[
        "NoNewPrivileges",
        "RestrictSUIDSGID",
        "ProtectClock",
        "ProtectKernelTunables",
        "PrivateMounts",
        "ProtectHome",
        "RestrictNamespaces",
        "ProtectSystem",
        "PrivateTmp",
    ];
    let comments: String = text
        .lines()
        .filter(|l| l.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ");
    for k in left_off {
        assert!(
            lacks(&keys, k),
            "{k} is set, but the unit says it is left off"
        );
        assert!(
            comments.contains(k),
            "the unit's comments no longer mention {k}"
        );
    }
    // no accounts or capability limits that would change what root can do here
    for k in [
        "User",
        "DynamicUser",
        "CapabilityBoundingSet",
        "AmbientCapabilities",
        "PrivateDevices",
        "DevicePolicy",
        "PrivateNetwork",
        "ProtectKernelModules",
        "MemoryDenyWriteExecute",
    ] {
        assert!(lacks(&keys, k), "{k}");
    }
}

#[test]
fn the_live_sessions_polkit_rule_grants_only_the_installers_actions_to_its_own_user() {
    let rules = include_str!(
        "../../live/rootfs/usr/share/polkit-1/rules.d/50-telamon-installer-session.rules"
    );
    // the subject test comes first and returns NOT_HANDLED for anyone else
    let guard = rules
        .find("subject.user !== \"telamon-installer\"")
        .unwrap();
    let grant = rules.find("polkit.Result.YES").unwrap();
    assert!(guard < grant);
    assert!(rules.contains("!subject.local") && rules.contains("!subject.active"));
    assert!(rules.contains("return polkit.Result.NOT_HANDLED;"));
    assert_eq!(rules.matches("polkit.Result.YES").count(), 1);
    // (NOT_HANDLED is the one other result there is)
    assert!(rules.matches("polkit.Result.NOT_HANDLED").count() >= 1);
    assert!(
        !rules
            .replace("polkit.Result.NOT_HANDLED", "")
            .contains("polkit.Result.NO")
    );
    for bad in [
        "AUTH_",
        "action.id.indexOf",
        "startsWith",
        "RegExp",
        "match(",
        "subject.isInGroup",
        "return true",
    ] {
        assert!(!rules.contains(bad), "{bad}");
    }
    // the list: our three actions and the NetworkManager ones the Wi-Fi page needs
    let list = &rules[rules.find("var allowed = [").unwrap()..];
    let list = &list[..list.find("];").unwrap()];
    let ids: Vec<&str> = list
        .lines()
        .filter_map(|l| l.trim().strip_prefix('"'))
        .map(|l| l.trim_end_matches(',').trim_end_matches('"'))
        .collect();
    assert!(ids.len() >= 8, "{ids:?}");
    for want in [
        crate::service::action::LIST_DISKS,
        crate::service::action::INSTALL,
        crate::service::action::REBOOT,
    ] {
        assert!(ids.contains(&want), "{want}");
    }
    for id in &ids {
        assert!(
            id.starts_with("net.eterneon.telamon.installer.")
                || (id.starts_with("org.freedesktop.NetworkManager.") && !id.ends_with('*')),
            "{id}"
        );
        // nothing that changes the system beyond networking and this installer
        for forbidden in [
            "systemd1",
            "login1",
            "policykit",
            "packagekit",
            "udisks",
            "rpm-ostree",
            "flatpak",
        ] {
            assert!(!id.to_lowercase().contains(forbidden), "{id}");
        }
    }
    // the installer's actions that exist in the policy file are all granted: a new action is added on purpose
    let policy = include_str!("../data/polkit-1/actions/net.eterneon.telamon.installer.policy");
    for id in policy
        .split("<action id=\"")
        .skip(1)
        .map(|s| &s[..s.find('"').unwrap()])
    {
        assert!(ids.contains(&id), "{id} has no grant in the live rule");
    }
}

#[test]
fn the_live_user_has_no_password_no_shell_login_and_no_groups() {
    let sysusers =
        include_str!("../../live/rootfs/usr/lib/sysusers.d/telamon-installer-session.conf");
    let lines: Vec<&str> = sysusers
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let f: Vec<&str> = lines[0].split_whitespace().collect();
    // type, name, id (`-`: automatic), then the home and shell
    assert_eq!(f[0], "u");
    assert_eq!(f[1], "telamon-installer");
    assert_eq!(
        f[2], "-",
        "a system account with an automatic id (below 1000)"
    );
    assert!(
        !sysusers
            .lines()
            .any(|l| l.starts_with("m ") || l.starts_with("g "))
    );
    // no password is set: sysusers locks the account
    assert!(!sysusers.contains("$6$") && !sysusers.contains("$y$"));
    let tmp = include_str!("../../live/rootfs/usr/lib/tmpfiles.d/telamon-installer-session.conf");
    assert!(tmp.contains("d /run/telamon-installer-session 0750"));
    // the session's home is on /run: nothing of it can be installed
    assert!(!tmp.contains("/var/") && !tmp.contains("/etc/"));
    let login = include_str!("../../live/rootfs/etc/plasmalogin.conf.d/telamon-installer.conf");
    assert!(login.contains("User=telamon-installer"));
}

// --- properties -----------------------------------------------------------

mod props {
    use super::*;
    use proptest::prelude::*;

    fn text() -> impl Strategy<Value = String> {
        prop_oneof![
            any::<String>(),
            prop::collection::vec(any::<u8>(), 0..120)
                .prop_map(|b| String::from_utf8_lossy(&b).into_owned()),
            "[a-z0-9 ._()@/'\"\\-\\n\\x00\\u{202e}]{0,40}",
        ]
    }

    fn near(re: &'static str) -> impl Strategy<Value = String> {
        prop_oneof![text(), re]
    }

    proptest! {
        #[test]
        fn requests_that_are_accepted_have_the_safe_shape(
            disk in near("[a-z0-9]{1,34}"),
            fp in near("[0-9a-f]{16}"),
            mode in near("(erase|free-space)"),
            locale in near("[a-z]{2}_[A-Z]{2}\\.UTF-8"),
            keymap in near("[a-z]{2}(\\([a-z]{3,10}\\))?"),
            wifi in near("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"),
            enc in near("(none|tpm|tpm-pin|password)"),
            pw in near("[ -~]{0,70}"),
        ) {
            if let Ok(r) = Request::new(&disk, &fp, &mode, &locale, &keymap, &wifi, &enc, &pw) {
                prop_assert!((1..=32).contains(&r.disk_id.len()));
                prop_assert!(r.disk_id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
                if let Some(f) = &r.fingerprint {
                    prop_assert_eq!(f.len(), 16);
                    prop_assert!(f.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
                }
                prop_assert!(matches!(r.mode.as_str(), "erase" | "free-space"));
                prop_assert!(r.locale.ends_with(".UTF-8") || r.locale.contains(".UTF-8@"));
                prop_assert!(!r.locale.contains(char::is_control));
                if let Some(u) = &r.wifi_uuid {
                    prop_assert!(installer_core::settings::validate_uuid(u).is_ok());
                }
                prop_assert_eq!(r.password.as_str().is_empty(), !r.encryption.has_secret());
                prop_assert!(r.password.as_str().bytes().all(|b| (0x20..=0x7e).contains(&b)));
            }
        }

        #[test]
        fn a_request_never_shows_its_password(secret in "S3CR3T-[a-z0-9]{8,40}", enc in prop_oneof![Just("password"), Just("tpm-pin")]) {
            let r = Request::new("sda", "", "erase", "en_US.UTF-8", "us", "", enc, &secret).unwrap();
            let plain = format!("{:?}", r);
            let some = format!("{:?}", Some(r.clone()));
            let pretty = format!("{:#?}", r);
            prop_assert!(!plain.contains(&secret) && !some.contains(&secret) && !pretty.contains(&secret));
        }

        #[test]
        fn a_command_never_shows_its_secrets(secret in "S3CR3T-[a-z0-9]{12}", args in prop::collection::vec("[a-z/=.-]{0,20}", 0..6)) {
            let c = Cmd::new(bin::CRYPTSETUP, args)
                .stdin(secret.clone())
                .secret_env("NEWPIN", secret.clone())
                .secret();
            let shown = format!("{:?}", c);
            prop_assert!(!shown.contains(&secret));
            prop_assert!(!c.display().contains(&secret));
            prop_assert!(c.display().contains("arguments hidden"));
            let s = Secret::from(secret.clone());
            prop_assert_eq!(format!("{s:?}"), "<hidden>");
        }

        #[test]
        fn error_text_from_a_tool_is_bounded_and_keeps_the_end(
            out in text(), err in text(), code in prop::option::of(-5i32..300)
        ) {
            let c = Cmd::new(bin::LSBLK, ["--json"]);
            let o = Output { code, stdout: out.clone(), stderr: err.clone() };
            if let Err(e) = o.check(&c) {
                prop_assert!(e.len() < 2048 + 80, "{}", e.len());
            }
            let t = run::tail(&out, 100);
            prop_assert!(t.len() <= 100);
            prop_assert!(out.trim().ends_with(&t));
        }

        #[test]
        fn mok_hashes_and_passwords(s in text()) {
            if let Some(h) = mok_hash(&s) {
                prop_assert!(h.starts_with('$') && !h.contains('\n'));
            }
            let p = mok_password().unwrap();
            prop_assert_eq!(p.len(), 8);
            prop_assert!(p.bytes().all(|b| b.is_ascii_digit()));
        }

        #[test]
        fn proc_stat_lines_never_panic(s in text()) {
            let _ = stat_fields(&s);
        }


        #[test]
        fn unit_paths_and_ids_in_the_seal_unit(uuid in text()) {
            if let Ok(u) = installer_core::crypt::seal_unit(Encryption::TpmPin, &uuid) {
                prop_assert!(installer_core::settings::validate_uuid(&uuid).is_ok());
                let tail = format!("{}\n[", uuid);
                prop_assert!(!u.contains(&tail));
            }
        }
    }

    // (a directory tree per case: fewer cases than the others)
    proptest! {
        #![proptest_config(ProptestConfig { cases: ProptestConfig::default().cases.min(2000), ..ProptestConfig::default() })]
        #[test]
        fn a_path_is_written_inside_its_base_or_not_at_all(rel in near("([a-z]{1,5}/){0,4}[a-z.]{1,5}")) {
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path().join("deploy");
            fs::create_dir_all(&base).unwrap();
            fs::create_dir_all(dir.path().join("outside")).unwrap();
            let r = write_file(&base, &rel, "x", 0o600);
            for f in tree(dir.path()) {
                prop_assert!(f.starts_with(&base), "{:?} escaped via {:?}", f, rel);
            }
            if r.is_ok() {
                prop_assert!(!rel.split('/').any(|c| c.is_empty() || c == "." || c == ".."));
                prop_assert!(base.join(&rel).is_file());
            }
        }
    }

    #[test]
    fn the_fingerprint_of_any_probe_text_is_sixteen_hex_digits() {
        let l = Lsblk::parse(LSBLK).unwrap();
        let d = &l.blockdevices[0];
        let f = disks::fingerprint(d, None);
        assert_eq!(f.len(), 16);
        assert!(f.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
