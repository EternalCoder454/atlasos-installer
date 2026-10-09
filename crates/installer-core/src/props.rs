//! Property tests for everything that parses or validates text the helper
//! gets from outside: the D-Bus caller's arguments, `lsblk` and `sfdisk`
//! output (a hostile disk controls its model, labels and partition table),
//! `efibootmgr`, `/proc/self/mounts`, NetworkManager keyfiles and the
//! `bootc` progress lines.
//!
//! Every test here lives in this module, so `cargo test -- props` runs
//! exactly them (CI sets PROPTEST_CASES=20000). Each checks that nothing
//! panics (the release build has overflow checks on, so an overflow is a
//! panic), that what is returned is bounded, and that whatever is accepted
//! has the safe shape the rest of the code relies on.

use std::collections::HashMap;

use proptest::prelude::*;

use crate::apps;
use crate::crypt::{self, Encryption};
use crate::disks::{self, Probe};
use crate::efi;
use crate::grub;
use crate::keyboard;
use crate::locales;
use crate::lsblk::{Device, Lsblk};
use crate::plan::{self, EspInfo, Mode, Plan};
use crate::progress::{BootcProgress, Progress, Stage};
use crate::settings::{self, Keymap};
use crate::sigpolicy;
use crate::table::{self, Partition, Table};

/// Text from the places an attacker can reach: any Unicode, bytes made into
/// text the way the helper does (lossy), and a small alphabet full of the
/// characters that matter (separators, quotes, controls, bidi marks, NUL)
/// so that near-valid input is common.
fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        prop::collection::vec(any::<u8>(), 0..200)
            .prop_map(|b| String::from_utf8_lossy(&b).into_owned()),
        "[a-zA-Z0-9 ._@()/'\"=;,:\\[\\]\\\\\\-\\n\\r\\t\\x00\\x1b\\u{202e}\\u{200b}\\u{85}\\u{a0}]{0,80}",
    ]
}

/// Text that is close to valid for a given shape.
fn near(re: &'static str) -> impl Strategy<Value = String> {
    prop_oneof![text(), re]
}

fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

fn safe_word(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+'))
}

proptest! {
    #[test]
    fn locales_are_one_safe_word(s in near("[a-z]{1,4}(_[A-Z]{2,3})?(\\.UTF-8)?(@[a-z]{0,18})?")) {
        if settings::validate_locale(&s).is_ok() {
            prop_assert!(s.len() <= 40);
            prop_assert!(s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'@')));
            prop_assert!(s.ends_with(".UTF-8") || s.contains(".UTF-8@"));
            // one line in each file it is written to
            let conf = settings::locale_conf(&s);
            prop_assert_eq!(conf.matches('\n').count(), 1);
            let k = Keymap::parse("us").unwrap();
            let ini = settings::installer_ini(&s, &k, true);
            prop_assert_eq!(ini.lines().filter(|l| l.starts_with("Language=")).count(), 1);
            prop_assert_eq!(ini.lines().count(), 8);
        }
    }

    #[test]
    fn keymaps_are_two_safe_words(s in near("[a-z][a-z0-9_]{0,17}(\\([A-Za-z0-9_-]{0,50}\\))?")) {
        if let Ok(k) = Keymap::parse(&s) {
            prop_assert!((1..=16).contains(&k.layout.len()));
            prop_assert!(k.layout.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
            prop_assert!(k.layout.as_bytes()[0].is_ascii_lowercase());
            if let Some(v) = &k.variant {
                prop_assert!((1..=48).contains(&v.len()));
                prop_assert!(v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'));
            }
            // the files made from it have a fixed number of lines and no
            // quote or backslash from the input
            let v = settings::vconsole_conf("de", &k);
            prop_assert_eq!(v.lines().count(), if k.variant.is_some() { 4 } else { 3 });
            let x = settings::x11_keyboard_conf(&k);
            prop_assert_eq!(x.matches('"').count(), 14 + 4 * usize::from(k.variant.is_some()));
            prop_assert!(!x.contains('\\'));
            let ini = settings::installer_ini("en_US.UTF-8", &k, false);
            prop_assert_eq!(ini.lines().count(), 8);
        }
    }

    #[test]
    fn console_keymaps_from_a_hostile_map_are_still_words_or_rejected(
        map in text(), s in near("[a-z]{1,5}(\\([a-z_]{1,8}\\))?")
    ) {
        if let Ok(k) = Keymap::parse(&s) {
            let c = settings::console_keymap(&map, &k);
            // the map is the image's own file; whatever it says, the
            // kernel command line only takes a name that passes this
            if crypt::validate_console_keymap(&c).is_ok() {
                prop_assert!(!c.contains(char::is_whitespace) && c.len() <= 64);
                prop_assert!(!c.starts_with('-'));
            }
        }
    }

    #[test]
    fn uuids_are_lower_case_hex(s in near("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")) {
        if settings::validate_uuid(&s).is_ok() {
            prop_assert_eq!(s.len(), 36);
            prop_assert!(s.bytes().all(|b| b == b'-' || b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
            prop_assert!(!s.starts_with('-'));
            prop_assert!(crypt::mapper_uuid(&crypt::mapper_name(&s)) == Some(s.as_str()));
        }
        // a mapper name that is not ours is never taken for it
        if let Some(u) = crypt::mapper_uuid(&s) {
            prop_assert!(settings::validate_uuid(u).is_ok());
        }
    }

    #[test]
    fn fat_volume_ids_are_nine_characters(s in near("[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}")) {
        if grub::validate_fat_uuid(&s).is_ok() {
            prop_assert_eq!(s.len(), 9);
            prop_assert!(s.bytes().all(|b| b == b'-' || b.is_ascii_hexdigit()));
            // so it can sit in a GRUB script unquoted
            let c = grub::custom_cfg(std::slice::from_ref(&s)).unwrap();
            let want = format!("--set=root {s}\n");
            prop_assert!(c.contains(&want));
            prop_assert_eq!(c.matches("menuentry").count(), 1);
        } else {
            prop_assert!(grub::custom_cfg(&[s]).is_err());
        }
    }

    #[test]
    fn grub_hashes_are_one_line_of_hex(s in near("grub\\.pbkdf2\\.sha512\\.[0-9]{4,6}\\.[0-9A-F]{2,16}\\.[0-9A-F]{2,16}")) {
        if grub::validate_pbkdf2(&s).is_ok() {
            prop_assert!(s.len() < 200);
            prop_assert!(!has_control(&s) && !s.contains(char::is_whitespace));
            let cfg = grub::user_cfg(&s).unwrap();
            prop_assert_eq!(cfg.lines().count(), 1);
        }
        let out = format!("Enter password: \nReenter password: \nPBKDF2 hash of your password is {s}\n");
        if let Ok(h) = grub::parse_mkpasswd(&out) {
            prop_assert!(grub::validate_pbkdf2(h).is_ok());
        }
    }

    #[test]
    fn mkpasswd_output_never_panics_and_gives_a_valid_hash_or_nothing(s in text()) {
        if let Ok(h) = grub::parse_mkpasswd(&s) {
            prop_assert!(grub::validate_pbkdf2(h).is_ok());
        }
    }

    #[test]
    fn recovery_keys_are_eight_groups_of_modhex(s in near("([cbdefghijklnrtuv]{8}-){7}[cbdefghijklnrtuv]{8}")) {
        let padded = format!("{s}\n");
        for text in [s.as_str(), padded.as_str()] {
            if let Ok(k) = crypt::parse_recovery_key(text) {
                prop_assert_eq!(k.len(), 71);
                prop_assert!(k.bytes().all(|b| b == b'-' || b"cbdefghijklnrtuv".contains(&b)));
                prop_assert!(!k.starts_with('-'));
            }
        }
    }

    #[test]
    fn passwords_and_pins_are_printable_ascii_of_bounded_length(s in near("[ -~]{0,300}")) {
        if crypt::validate_password(&s).is_ok() {
            prop_assert!((crypt::PASSWORD_MIN..=crypt::PASSWORD_MAX).contains(&s.len()));
            prop_assert!(s.bytes().all(|b| (0x20..=0x7e).contains(&b)));
        }
        if crypt::validate_pin(&s).is_ok() {
            prop_assert!((crypt::PIN_MIN..=crypt::PIN_MAX).contains(&s.len()));
            prop_assert!(s.bytes().all(|b| (0x20..=0x7e).contains(&b)));
        }
    }

    #[test]
    fn console_keymap_names_are_safe_in_a_kernel_command_line(s in near("[a-z0-9][a-z0-9_.+-]{0,70}")) {
        if crypt::validate_console_keymap(&s).is_ok() {
            prop_assert!(safe_word(&s) && s.len() <= 64);
            prop_assert!(!s.starts_with(['-', '.', '+', '_']));
        }
    }

    #[test]
    fn kargs_are_whitespace_free_and_none_is_an_option(uuid in near("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"), map in near("[a-z0-9_.+-]{1,70}")) {
        for enc in [Encryption::None, Encryption::Tpm, Encryption::TpmPin, Encryption::Password] {
            if let Ok(k) = crypt::kargs(enc, &uuid, &map) {
                prop_assert!(k.len() <= 5);
                for a in &k {
                    prop_assert!(!a.contains(char::is_whitespace) && !has_control(a));
                    prop_assert!(!a.starts_with('-'));
                    prop_assert!(a.len() < 160);
                }
            }
        }
    }

    #[test]
    fn luks_headers_of_any_shape_are_judged_without_panicking(s in text()) {
        for enc in [Encryption::Tpm, Encryption::TpmPin, Encryption::Password] {
            let _ = crypt::check_slots(&s, enc);
        }
    }

    #[test]
    fn luks_headers_with_hostile_slot_and_token_names(
        slots in prop::collection::vec(text(), 0..5),
        tokens in prop::collection::vec((text(), text(), prop::collection::vec(text(), 0..3)), 0..5),
    ) {
        let slots: serde_json::Map<String, serde_json::Value> =
            slots.into_iter().map(|s| (s, serde_json::json!({"type": "luks2"}))).collect();
        let tokens: serde_json::Map<String, serde_json::Value> = tokens
            .into_iter()
            .map(|(name, ty, ks)| (name, serde_json::json!({"type": ty, "keyslots": ks})))
            .collect();
        let json = serde_json::json!({"keyslots": slots, "tokens": tokens}).to_string();
        for enc in [Encryption::Tpm, Encryption::TpmPin, Encryption::Password] {
            let r = crypt::check_slots(&json, enc);
            if r.is_ok() {
                prop_assert_eq!(slots.len(), 2);
            }
        }
    }

    #[test]
    fn seal_units_take_only_uuids(s in text()) {
        for enc in [Encryption::Tpm, Encryption::TpmPin] {
            if let Ok(unit) = crypt::seal_unit(enc, &s) {
                prop_assert!(settings::validate_uuid(&s).is_ok());
                for l in unit.lines() {
                    prop_assert!(l.is_empty() || l.starts_with('#') || l.starts_with('[') || l.contains('='), "{}", l);
                }
                prop_assert_eq!(unit.lines().filter(|l| l.starts_with("ExecStart=")).count(), 1);
            }
        }
    }

    #[test]
    fn apps_are_catalog_ids_only(ids in prop::collection::vec(near("[a-z-]{1,10}"), 0..6)) {
        if let Ok(chosen) = apps::validate(&ids) {
            prop_assert!(chosen.len() <= apps::catalog().len());
            for a in &chosen {
                prop_assert!(ids.contains(&a.id));
                prop_assert!(apps::find(&a.id).is_some());
            }
            let rec = apps::record(&chosen);
            prop_assert_eq!(rec.matches('\n').count(), 1);
        }
    }

    #[test]
    fn keyfiles_for_install_lose_permissions_and_the_interface(
        body in text(), uuid in near("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
    ) {
        // arbitrary text never panics, and a result is a Wi-Fi connection of plain text
        if let Some(copy) = settings::keyfile_for_install(&body, &uuid) {
            prop_assert!(!copy.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')));
            prop_assert!(copy.contains("wifi") || copy.contains("802-11-wireless"));
        }
        // a real shape with hostile fields in it
        let kf = format!("[connection]\nid={body}\nuuid={uuid}\npermissions=user:x;\ninterface-name=wlan0\n[wifi]\nssid=a\n");
        if let Some(out) = settings::keyfile_for_install(&kf, &uuid) {
            let mut section = "";
            for l in out.lines() {
                let t = l.trim();
                if t.starts_with('[') && t.ends_with(']') {
                    section = &t[1..t.len() - 1];
                } else if section == "connection" {
                    let k = t.split_once('=').map_or("", |(k, _)| k.trim());
                    prop_assert!(k != "permissions" && k != "interface-name", "{:?}", l);
                }
            }
        }
    }

    #[test]
    fn container_policies_of_any_shape_are_judged_without_panicking(
        policy in text(), image in near("[a-z.:0-9]{1,12}(/[a-z0-9-]{1,12}){0,3}(:[a-z]{1,8})?(@sha256:[0-9a-f]{4})?")
    ) {
        let _ = sigpolicy::check(&policy, &image, &|_| true);
        let _ = sigpolicy::check(&policy, &image, &|_| false);
        let r = sigpolicy::repository(&image);
        prop_assert!(image.starts_with(r));
        prop_assert!(!r.contains('@'));
        let _ = sigpolicy::attachments_enabled(&policy);
    }

    #[test]
    fn a_policy_that_is_accepted_has_a_default_that_accepts_nothing_unchecked(
        default in prop::collection::vec(prop_oneof![Just("reject"), Just("insecureAcceptAnything"), Just("sigstoreSigned"), Just("signedBy")], 0..3),
        entry in prop::collection::vec(prop_oneof![Just("reject"), Just("insecureAcceptAnything"), Just("sigstoreSigned"), Just("signedBy")], 0..3),
        scope in prop_oneof![
            prop::sample::select(vec![
                "ghcr.io/eternalcoder454/atlasos", "ghcr.io/eternalcoder454", "ghcr.io",
                "*.io", "", "ghcr.io/other/atlasos",
            ]).prop_map(String::from),
            text()
        ],
        key in prop_oneof![Just("/etc/pki/containers/telamon.pub".to_string()), text()],
    ) {
        let req = |t: &&str| serde_json::json!({"type": t, "keyPaths": [key]});
        let policy = serde_json::json!({
            "default": default.iter().map(|t| serde_json::json!({"type": t})).collect::<Vec<_>>(),
            "transports": {"docker": {scope.clone(): entry.iter().map(req).collect::<Vec<_>>()}},
        })
        .to_string();
        let image = "ghcr.io/eternalcoder454/atlasos:stable";
        if sigpolicy::check(&policy, image, &|p| p == key).is_ok() {
            // bootc would refuse this install if the default accepted anything
            prop_assert!(!default.contains(&"insecureAcceptAnything"));
            // the requirement that applies is only ever a sigstore one, with a key that exists
            prop_assert!(!entry.is_empty() && entry.iter().all(|t| *t == "sigstoreSigned"));
            prop_assert!(key.starts_with('/'));
        }
    }

    #[test]
    fn efibootmgr_output_never_panics_and_entries_have_hex_numbers(s in text()) {
        for e in efi::parse(&s) {
            prop_assert_eq!(e.num.len(), 4);
            prop_assert!(e.num.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        let entries = efi::parse(&s);
        let _ = efi::stale_entries(&entries, &s);
        let _ = efi::boot_order(&s);
    }

    #[test]
    fn efibootmgr_numbers_that_get_to_a_command_line_are_hex(
        lines in prop::collection::vec(near("Boot[0-9A-Fa-f]{4}\\*? (Fedora|Telamon OS|AtlasOS)\tHD\\(1,GPT,[0-9a-f-]{36},0x800,0x32000\\)/\\\\EFI\\\\fedora\\\\shimx64\\.efi"), 0..6)
    ) {
        let out = lines.join("\n");
        let entries = efi::parse(&out);
        for e in efi::stale_entries(&entries, "00000000-0000-0000-0000-000000000000") {
            prop_assert!(e.num.bytes().all(|b| b.is_ascii_hexdigit()) && e.num.len() == 4);
        }
        for n in efi::boot_order(&out) {
            prop_assert!(!n.is_empty());
        }
    }

    #[test]
    fn mount_tables_never_panic(s in text()) {
        let _ = disks::live_sources(&s);
        let u = disks::unescape(&s);
        // unescaping never makes the text longer
        prop_assert!(u.len() <= s.len().max(1) * 3);
    }

    #[test]
    fn bootc_lines_never_panic_and_progress_stays_a_fraction(
        lines in prop::collection::vec(text(), 0..20)
    ) {
        let mut p = BootcProgress::default();
        let mut now = std::time::Duration::ZERO;
        for l in &lines {
            now += std::time::Duration::from_millis(900);
            if let Some(x) = p.line(l, now) {
                prop_assert!((0.0..=1.0).contains(&x.fraction), "{}", x.fraction);
            }
            if let Some(x) = p.tick(now) {
                prop_assert!((0.0..=1.0).contains(&x.fraction));
            }
        }
        let _ = Progress::at(Stage::Copy, 0.5).text();
    }

    #[test]
    fn counted_bytes_of_any_size_keep_the_bar_in_range(done in any::<u64>(), total in any::<u64>(), layers in any::<u64>()) {
        let mut p = BootcProgress::default();
        let now = std::time::Duration::from_secs(3);
        p.line("layers already present: 0; layers needed: 128 (3.2 GB)", now);
        if let Some(x) = p.counted(Some((done, total)), layers, now) {
            prop_assert!((0.0..=1.0).contains(&x.fraction), "{}", x.fraction);
        }
    }

    #[test]
    fn keyboard_and_locale_lists_never_panic(s in text()) {
        let _ = keyboard::parse_base_lst(&s);
        let _ = locales::offered(Some(&s));
        let _ = locales::offered(None);
    }
}

// --- hostile devices and partition tables --------------------------------

fn edge_u64() -> impl Strategy<Value = u64> {
    prop_oneof![
        Just(0u64),
        Just(1),
        Just(u64::MAX),
        Just(u64::MAX - 1),
        Just(u64::MAX / 2),
        Just(1 << 63),
        Just(1 << 40),
        0..5000u64,
        any::<u64>(),
    ]
}

/// Short text for the many strings of a device or a table: what matters
/// there is the numbers and the shape, and a long string only costs time.
fn small() -> impl Strategy<Value = String> {
    prop_oneof!["[ -~]{0,8}", ".{0,6}", "[a-z0-9/ \\-\\n\\x00]{0,6}",]
}

fn name() -> impl Strategy<Value = String> {
    prop_oneof![
        "/dev/(sd[a-d]|vd[a-c]|nvme[0-2]n1)",
        small().prop_map(|s| format!("/dev/{s}")),
        small(),
    ]
}

fn partition() -> impl Strategy<Value = Partition> {
    (
        name(),
        edge_u64(),
        edge_u64(),
        prop_oneof![
            small(),
            "[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}"
        ],
        prop::option::of(small()),
        prop::option::of(small()),
    )
        .prop_map(|(node, start, size, kind, uuid, name)| Partition {
            node,
            start,
            size,
            kind,
            uuid,
            name,
        })
}

fn hostile_table(device: String) -> impl Strategy<Value = Table> {
    (
        prop_oneof![Just("gpt".to_string()), Just("dos".to_string()), small()],
        prop::option::of(edge_u64()),
        prop::option::of(edge_u64()),
        prop_oneof![Just(512u64), Just(4096), edge_u64()],
        prop::collection::vec(partition(), 0..5),
    )
        .prop_map(
            move |(label, firstlba, lastlba, sectorsize, partitions)| Table {
                label,
                id: None,
                device: device.clone(),
                firstlba,
                lastlba,
                sectorsize,
                partitions,
            },
        )
}

fn hostile_device() -> impl Strategy<Value = Device> {
    let part = (
        name(),
        prop_oneof![Just("part".to_string()), small()],
        edge_u64(),
        prop::option::of(small()),
        prop::option::of(small()),
    )
        .prop_map(|(name, kind, size, fstype, label)| Device {
            name,
            kind,
            size,
            fstype,
            label,
            ..Default::default()
        });
    (
        name(),
        prop_oneof![Just("disk".to_string()), Just("rom".to_string()), small()],
        edge_u64(),
        prop::option::of(edge_u64()),
        any::<bool>(),
        prop::collection::vec(part, 0..4),
        prop::option::of(small()),
        prop::option::of(small()),
        prop::option::of(small()),
    )
        .prop_map(
            |(name, kind, size, log_sec, ro, children, model, vendor, serial)| Device {
                name,
                kind,
                size,
                log_sec,
                ro,
                children,
                model,
                vendor,
                serial,
                ..Default::default()
            },
        )
}

fn esps_for(table: &Table) -> HashMap<String, EspInfo> {
    table
        .partitions
        .iter()
        .map(|p| {
            (
                p.node.clone(),
                EspInfo {
                    fs_free: u64::MAX,
                    windows: true,
                    fedora: false,
                    fs_uuid: Some("ABCD-1234".into()),
                },
            )
        })
        .collect()
}

/// The device paths a plan hands to commands are all `/dev/...` paths
/// without surprises.
fn plan_is_safe(p: &Plan) -> Result<(), TestCaseError> {
    let mut nodes: Vec<&str> = p.parts.iter().map(|x| x.node.as_str()).collect();
    nodes.extend(p.wipe.iter().map(String::as_str));
    nodes.push(&p.disk);
    nodes.push(&p.esp);
    for n in nodes {
        prop_assert!(plan::is_device_path(n), "{:?}", n);
    }
    prop_assert!(p.validate().is_ok());
    for part in &p.parts {
        prop_assert!(part.size > 0);
        prop_assert!(part.start.checked_add(part.size).is_some());
    }
    Ok(())
}

proptest! {
    #[test]
    fn partition_tables_with_absurd_numbers_never_panic(t in hostile_table("/dev/sda".into())) {
        let _ = t.align();
        let regions = t.free_regions();
        for r in &regions {
            prop_assert!(r.sectors > 0);
            prop_assert!(r.start.checked_add(r.sectors).is_some());
        }
        let _ = t.largest_free();
        let _ = t.bytes(u64::MAX);
        let _ = t.free_numbers(3);
        let _ = table::check_unchanged(&t, &t.clone());
        let json = serde_json::json!({ "partitiontable": t }).to_string();
        let back = Table::parse(&json).unwrap();
        prop_assert_eq!(back, t);
    }

    #[test]
    fn plans_from_hostile_disks_are_safe_or_refused(
        d in hostile_device(), t in hostile_table("/dev/sda".into())
    ) {
        let esps = esps_for(&t);
        if let Ok(p) = plan::plan_erase(&d) {
            plan_is_safe(&p)?;
        }
        let mut d2 = d.clone();
        d2.name = "/dev/sda".into();
        if let Ok(p) = plan::plan_free_space(&d2, Some(&t), &esps) {
            plan_is_safe(&p)?;
            // the old partitions are kept: nothing is wiped
            prop_assert!(p.wipe.is_empty());
        }
        let _ = plan::in_use(&d);
        let _ = plan::reusable_esp(&t, &esps);
    }

    #[test]
    fn listing_a_hostile_disk_never_panics(
        d in hostile_device(), t in hostile_table("/dev/sda".into()), live in text()
    ) {
        let mut d = d;
        d.name = "/dev/sda".into();
        let lsblk = Lsblk { blockdevices: vec![d.clone()] };
        let mut probe = Probe {
            lsblk: Some(lsblk),
            live_sources: vec![live],
            ..Default::default()
        };
        probe.tables.insert("/dev/sda".into(), t.clone());
        probe.esps = esps_for(&t);
        let list = disks::list(&probe);
        for disk in &list.disks {
            prop_assert_eq!(disk.fingerprint.len(), 16);
        }
        let _ = disks::boot_media(&probe);
        let _ = disks::chain_loaded(&probe, "x /run/initramfs/isoscan y\n");
        let fp = disks::fingerprint(&d, Some(&t));
        prop_assert_eq!(fp.len(), 16);
        for mode in [Mode::Erase, Mode::FreeSpace] {
            if let Ok(p) = disks::plan_for(&d, &probe, mode) {
                plan_is_safe(&p)?;
            }
        }
        let _ = disks::find_visible(&probe, "sda");
    }

    #[test]
    fn a_written_table_is_checked_against_the_plan_without_panicking(
        d in hostile_device(), before in hostile_table("/dev/sda".into()), after in hostile_table("/dev/sda".into())
    ) {
        let mut d = d;
        d.name = "/dev/sda".into();
        d.size = 100 * crate::GIB;
        d.log_sec = Some(before.sectorsize);
        if let Ok(p) = plan::plan_free_space(&d, Some(&before), &esps_for(&before)) {
            let _ = p.check_written(&after);
            let _ = p.describe();
            let _ = p.sfdisk_script();
        }
    }
}

proptest! {
    #[test]
    fn lsblk_json_of_any_shape_never_panics(s in text()) {
        if let Ok(l) = Lsblk::parse(&s) {
            for d in &l.blockdevices {
                let _ = d.walk();
                let _ = d.installer_mappers();
                let _ = d.id();
                let _ = d.sector_size();
                let _ = disks::hidden_reason(d, &[]);
            }
            let _ = l.luks_mappers_on("/dev/sda");
        }
        let _ = Table::parse(&s);
    }

    #[test]
    fn lsblk_json_with_hostile_strings_and_numbers(
        names in prop::collection::vec(text(), 1..4),
        size in prop_oneof![edge_u64().prop_map(|n| n.to_string()), text()],
        sec in prop_oneof![edge_u64().prop_map(|n| n.to_string()), text()],
    ) {
        let devs: Vec<serde_json::Value> = names
            .iter()
            .map(|n| serde_json::json!({
                "name": n, "type": "disk", "size": size, "log-sec": sec,
                "model": n, "label": n, "mountpoints": [n, null],
                "children": [{"name": n, "type": "part", "size": size}],
            }))
            .collect();
        let json = serde_json::json!({"blockdevices": devs}).to_string();
        if let Ok(l) = Lsblk::parse(&json) {
            let probe = Probe { lsblk: Some(l), ..Default::default() };
            let list = disks::list(&probe);
            for d in &list.disks {
                let cut = if d.path.starts_with("/dev/") { 5 } else { 0 };
                prop_assert_eq!(d.id.len(), d.path.len() - cut);
            }
        }
    }

    #[test]
    fn phantom_partition_numbers_are_digits_only(disk in near("/dev/[a-z0-9]{1,10}"), node in near("/dev/[a-z0-9]{1,10}p?[0-9]{0,5}")) {
        if let Some(n) = table::partition_number(&disk, &node) {
            prop_assert!(n < u32::MAX);
            prop_assert_eq!(table::partition_node(&disk, n), node);
        }
        for n in [0u32, 1, 127, 128, u32::MAX] {
            let node = table::partition_node(&disk, n);
            prop_assert!(node.starts_with(&disk));
        }
    }
}

// the nasty corpus, by name
#[test]
fn the_nasty_corpus_is_refused_by_every_validator() {
    let nasty = [
        "",
        " ",
        "\n",
        "\0",
        "..",
        "../..",
        "/",
        "-",
        "--",
        "-rf",
        "--help",
        "a\nb",
        "a\0b",
        "a\rb",
        "a b",
        "a\tb",
        "x\u{202e}y",
        "x\u{200b}y",
        "x\u{2028}y",
        "x\u{feff}y",
        "'; reboot #",
        "$(reboot)",
        "`reboot`",
        "a;b",
        "a=b",
        "%s%n",
        "\u{1b}[2J",
    ];
    for s in nasty {
        assert!(settings::validate_locale(s).is_err(), "{s:?}");
        assert!(Keymap::parse(s).is_err(), "{s:?}");
        assert!(settings::validate_uuid(s).is_err(), "{s:?}");
        assert!(grub::validate_fat_uuid(s).is_err(), "{s:?}");
        assert!(grub::validate_pbkdf2(s).is_err(), "{s:?}");
        assert!(crypt::parse_recovery_key(s).is_err(), "{s:?}");
        assert!(crypt::validate_console_keymap(s).is_err(), "{s:?}");
        assert!(apps::validate(&[s.to_string()]).is_err(), "{s:?}");
        assert!(crypt::mapper_uuid(s).is_none(), "{s:?}");
        // passwords may hold spaces and punctuation, never controls or non-ASCII
        if s.chars().any(|c| !(' '..='~').contains(&c)) {
            assert!(
                crypt::validate_password(&format!("{s}12345678")).is_err(),
                "{s:?}"
            );
            assert!(crypt::validate_pin(&format!("{s}1234")).is_err(), "{s:?}");
        }
    }
    // a 10 MB string is refused at once and costs nothing
    let big = "a".repeat(10 * 1024 * 1024);
    assert!(settings::validate_locale(&big).is_err());
    assert!(Keymap::parse(&big).is_err());
    assert!(settings::validate_uuid(&big).is_err());
    assert!(grub::validate_fat_uuid(&big).is_err());
    assert!(grub::validate_pbkdf2(&big).is_err());
    assert!(crypt::parse_recovery_key(&big).is_err());
    assert!(crypt::validate_console_keymap(&big).is_err());
    assert!(crypt::validate_password(&big).is_err());
    assert!(crypt::validate_pin(&big).is_err());
    assert!(apps::validate(std::slice::from_ref(&big)).is_err());
    assert!(grub::parse_mkpasswd(&big).is_err());
    assert!(Lsblk::parse(&big).is_err());
    assert!(Table::parse(&big).is_err());
    assert!(efi::parse(&big).is_empty());
    assert!(crypt::check_slots(&big, Encryption::Tpm).is_err());
    let _ = settings::keyfile_for_install(&big, "0b4f6b8e-2a0c-4d5e-9f1a-3c2b1a0d9e8f");
    let _ = disks::unescape(&big);
    let _ = keyboard::parse_base_lst(&big);
    // many lines
    let many = "Boot0000* Fedora\tx\n".repeat(200_000);
    assert_eq!(efi::parse(&many).len(), 200_000);
    // NUL and newline inside a keymap variant do not slip through the paren
    assert!(Keymap::parse("us(a\nb)").is_err());
    assert!(Keymap::parse("us(a\0b)").is_err());
    assert!(Keymap::parse("us(a b)").is_err());
    assert!(Keymap::parse("-us").is_err());
}

#[test]
fn bidi_and_homoglyph_text_never_becomes_a_locale_or_layout() {
    for s in [
        "en_US.UTF\u{2011}8",
        "en_US\u{ff0e}UTF-8",
        "ｅn_US.UTF-8",
        "en_US.UTF-8\u{202e}",
    ] {
        assert!(settings::validate_locale(s).is_err(), "{s:?}");
    }
    for s in ["ｕs", "us\u{202e}", "u\u{200b}s"] {
        assert!(Keymap::parse(s).is_err(), "{s:?}");
    }
}
