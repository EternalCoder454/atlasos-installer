#!/usr/bin/env bash
# Runs the real cryptsetup and systemd-cryptenroll, on regular files, the way the
# helper does (docs/helper-api.md, "Disk encryption"), and checks that what the
# helper's parsers read is what the tools print:
#
#   - `luksFormat` with the helper's parameters, `systemd-cryptenroll --recovery-key`
#     (its key goes to stdout when that is a pipe), `luksAddKey --new-keyfile -`,
#     `luksRemoveKey`;
#   - `open --test-passphrase --disable-external-tokens` exits 0 for a key that
#     works and 2 for one that doesn't (the removed temporary key, a wrong one);
#   - `luksDump --dump-json-metadata` has two key slots and the recovery token, and
#     installer-core's `crypt::check_slots` accepts it (the TPM modes with a
#     `systemd-tpm2` token that `cryptsetup token import` adds, since a container
#     has no TPM) and refuses the dump taken before the temporary key was removed.
#
# It needs no privilege: no device-mapper, no loop device, no root. Run it in a
# container with cryptsetup, systemd (systemd-cryptenroll), jq, and cargo:
#
#   tests/container/crypt-flow.sh podman [IMAGE]    build the image below and run in it
#   tests/container/crypt-flow.sh                    run here (the tools must be installed)
#   tests/container/crypt-flow.sh --update-fixtures  also rewrite crates/installer-core/tests/fixtures/luksdump-*.json
#
# When a tool is missing or a step can't run here (a container with no memory for
# Argon2, say) it says so and exits 0: it is a test of the tools' behaviour, not
# a gate that fails on a machine that lacks them.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)

if [ "${1:-}" = podman ]; then
	image=${2:-localhost/telamon-installer-crypt-flow}
	if [ -z "${2:-}" ]; then
		podman build -q -t "$image" -f "$here/Containerfile" "$here" >/dev/null
	fi
	# Nothing privileged: a file in the container is all it touches. Never :Z.
	exec podman run --rm --ulimit core=0 --security-opt label=disable \
		--security-opt no-new-privileges -v "$repo":/src -w /src \
		-e CARGO_TARGET_DIR=/src/target/container "$image" tests/container/crypt-flow.sh
fi
update=0
[ "${1:-}" = --update-fixtures ] && update=1

skip() {
	echo "crypt-flow: skipped: $*"
	exit 0
}
for tool in cryptsetup systemd-cryptenroll jq truncate; do
	command -v "$tool" >/dev/null || skip "$tool is not installed"
done

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
out=$work/out
mkdir "$out"
umask 077
cd "$work"

uuid=0b4f6b8e-2a0c-4d5e-9f1a-3c2b1a0d9e8f
password='CANARY-pw-7f3a'
codes=$out/exit-codes.txt
: >"$codes"

# The helper's parameters (helper/src/install.rs: LUKS_*).
format() { # file
	truncate -s 64M "$1"
	head -c 64 /dev/urandom >"$1.key"
	cryptsetup luksFormat --type luks2 --cipher aes-xts-plain64 --key-size 512 \
		--hash sha256 --pbkdf argon2id --batch-mode --uuid "$uuid" --label atlasos \
		--key-file "$1.key" "$1"
}
test_key() { # name file keyfile-or-"-" [stdin]
	local name=$1 file=$2 key=$3 rc=0
	if [ $# -ge 4 ]; then
		printf %s "$4" | cryptsetup open --test-passphrase --disable-external-tokens --key-file "$key" "$file" 2>/dev/null || rc=$?
	else
		cryptsetup open --test-passphrase --disable-external-tokens --key-file "$key" "$file" 2>/dev/null || rc=$?
	fi
	echo "$name $rc" >>"$codes"
}

# --- password mode ---
format password.img || skip "cryptsetup luksFormat can't run here"
systemd-cryptenroll --unlock-key-file=password.img.key --recovery-key password.img >"$out/recovery.out" 2>"$out/recovery.err" ||
	skip "systemd-cryptenroll can't run here: $(tail -1 "$out/recovery.err")"
recovery=$(tr -d '\n' <"$out/recovery.out")
printf %s "$password" | cryptsetup luksAddKey --pbkdf argon2id --key-file password.img.key --new-keyfile - password.img
cryptsetup luksDump --dump-json-metadata password.img >"$out/before-remove.json"
cryptsetup luksRemoveKey --key-file password.img.key password.img
cryptsetup luksDump --dump-json-metadata password.img >"$out/password.json"
test_key temporary-key-after-removal password.img password.img.key
test_key password password.img - "$password"
test_key recovery-key password.img - "$recovery"
test_key wrong-key password.img - "wrong key"
test_key recovery-key-with-newline password.img - "$recovery
"

# --- TPM modes: a second key slot and a systemd-tpm2 token, as the enrolment leaves them ---
format tpm.img
systemd-cryptenroll --unlock-key-file=tpm.img.key --recovery-key tpm.img >/dev/null 2>&1
head -c 32 /dev/urandom >tpm.img.tpmkey
cryptsetup luksAddKey --pbkdf argon2id --key-file tpm.img.key --new-keyfile tpm.img.tpmkey tpm.img
slot=$(cryptsetup luksDump --dump-json-metadata tpm.img | jq -r '.keyslots | keys[] | select(. != "0")' | sort -n | tail -1)
# (the token plugin validates the fields it reads, not the blob: random bytes will do)
blob=$(head -c 64 /dev/urandom | base64 -w0)
policy=$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')
jq -n --arg slot "$slot" --arg blob "$blob" --arg policy "$policy" \
	'{type: "systemd-tpm2", keyslots: [$slot], "tpm2-blob": $blob, "tpm2-pcrs": [], "tpm2-pcr-bank": "sha256", "tpm2-primary-alg": "ecc", "tpm2-policy-hash": $policy, "tpm2-pin": false}' >tpm.token.json
cryptsetup token import --json-file tpm.token.json tpm.img || skip "cryptsetup token import can't run here"
cryptsetup luksRemoveKey --key-file tpm.img.key tpm.img
cryptsetup luksDump --dump-json-metadata tpm.img >"$out/tpm.json"

echo ">> $(cryptsetup --version), $(systemd-cryptenroll --version | head -1)"
cat "$codes"

# The expected codes first, so a tool that changed is named here.
expect() { grep -qx "$1 $2" "$codes" || {
	echo "crypt-flow: FAILED: expected '$1 $2', got: $(grep "^$1 " "$codes")" >&2
	exit 1
}; }
expect temporary-key-after-removal 2
expect password 0
expect recovery-key 0
expect wrong-key 2
expect recovery-key-with-newline 2

if [ "$update" = 1 ]; then
	cp "$out/password.json" "$repo/crates/installer-core/tests/fixtures/luksdump-password.json"
	cp "$out/tpm.json" "$repo/crates/installer-core/tests/fixtures/luksdump-tpm.json"
	cp "$out/before-remove.json" "$repo/crates/installer-core/tests/fixtures/luksdump-before-remove.json"
	printf '%s\n' "$(cat "$out/recovery.out")" >"$repo/crates/installer-core/tests/fixtures/cryptenroll-recovery.out"
	echo ">> fixtures rewritten"
fi

command -v cargo >/dev/null || skip "cargo is not installed: the dumps and the recovery key are in $out, but the parsers were not run"
cd "$repo"
TELAMON_REAL_TOOLS=$out cargo test -p installer-core --test real_tools --locked -- --nocapture
echo "crypt-flow: ok"
