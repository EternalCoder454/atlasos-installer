//! installer-core's parsers against what the real cryptsetup and
//! systemd-cryptenroll print. `tests/container/crypt-flow.sh` runs the tools on
//! regular files in a container and points `TELAMON_REAL_TOOLS` at their
//! output; without it these tests only check the fixtures that run left in the
//! repository.

use installer_core::crypt::{Encryption, check_slots, parse_recovery_key};

const PASSWORD: &str = include_str!("fixtures/luksdump-password.json");
const TPM: &str = include_str!("fixtures/luksdump-tpm.json");
const BEFORE: &str = include_str!("fixtures/luksdump-before-remove.json");
const RECOVERY: &str = include_str!("fixtures/cryptenroll-recovery.out");

fn accepted(dump: &str) -> Vec<Encryption> {
    [Encryption::Tpm, Encryption::TpmPin, Encryption::Password]
        .into_iter()
        .filter(|e| check_slots(dump, *e).is_ok())
        .collect()
}

#[test]
fn a_real_password_install_header_passes_the_slot_check_only_as_a_password() {
    assert_eq!(accepted(PASSWORD), [Encryption::Password]);
}

#[test]
fn a_real_tpm_install_header_passes_only_as_a_tpm_one() {
    assert_eq!(accepted(TPM), [Encryption::Tpm, Encryption::TpmPin]);
}

#[test]
fn a_real_header_with_the_temporary_key_still_in_it_fails_every_mode() {
    assert!(accepted(BEFORE).is_empty(), "{BEFORE}");
    let v: serde_json::Value = serde_json::from_str(BEFORE).unwrap();
    assert_eq!(v["keyslots"].as_object().unwrap().len(), 3);
}

#[test]
fn the_recovery_key_systemd_cryptenroll_prints_is_the_format_the_parser_takes() {
    let key = parse_recovery_key(RECOVERY).unwrap();
    assert_eq!(key.len(), 71);
    assert_eq!(key.split('-').count(), 8);
}

/// With the output of a fresh run of the tools (crypt-flow.sh): the same checks.
#[test]
fn a_fresh_run_of_the_real_tools() {
    let Ok(dir) = std::env::var("TELAMON_REAL_TOOLS") else {
        eprintln!("TELAMON_REAL_TOOLS is not set: only the committed fixtures were checked");
        return;
    };
    let read = |f: &str| std::fs::read_to_string(format!("{dir}/{f}")).unwrap();
    assert_eq!(accepted(&read("password.json")), [Encryption::Password]);
    assert_eq!(
        accepted(&read("tpm.json")),
        [Encryption::Tpm, Encryption::TpmPin]
    );
    assert!(accepted(&read("before-remove.json")).is_empty());
    // stdout of `systemd-cryptenroll --recovery-key` on a pipe is the key alone
    let key = read("recovery.out");
    assert!(parse_recovery_key(&key).is_ok(), "{key:?}");
    // its banner is on stderr and is not a key
    assert!(parse_recovery_key(&read("recovery.err")).is_err());
}
