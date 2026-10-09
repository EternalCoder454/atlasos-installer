# Installer: security

The threat model of Telamon Installer: what it protects, who it defends
against, the rule each entry point follows, and the test that keeps the rule
true. `docs/helper-api.md` is the contract of the root helper; this file says
why it is built the way it is, and what is left. Change it together with the
code it describes: some of the tests named below read the unit, policy and QML
files and fail when a rule here is skipped.

Report a vulnerability privately to the maintainer through GitHub's "Report a
vulnerability" on the repository (Security tab), not in a public issue.

## What the installer is, security-wise

- It runs from the live ISO, as **root** (`telamon-installer-helper`): it
  partitions a disk, sets up LUKS2 and the TPM, runs `bootc install`, writes
  files into the new system and the firmware's boot entries. It is the most
  privileged program Telamon ships, and a mistake in it can destroy a disk.
- Its UI (`telamon-installer-app`) runs as the live session's user
  `telamon-installer` (autologin, no password, no groups, home on `/run`),
  never as root. The UI reaches the helper only over the system bus; polkit
  decides every call by the caller's bus name.
- The install is **offline**: the image is embedded in the ISO (cosign-checked
  when the ISO was built, `iso/make-iso.sh`) and installed from containers
  storage. Wi-Fi is only used to carry a connection over to the new system.
- First start of the installed system: `telamon-first-boot-apps` (root, for
  Flathub apps; the user, for developer tools and the Local AI apps) and the
  one-shot unit that seals the TPM key (`telamon-tpm-seal.service`).
- It handles secrets: the disk password or TPM PIN, the recovery key, a
  temporary LUKS key, the MOK password, a Wi-Fi password.

## Who we defend against

| Attacker | What they can reach | Defended? |
|---|---|---|
| **A hostile disk or USB stick** that is plugged in during the install | Its model, serial, labels, partition table, an ESP's files, all of which the helper reads as root | Yes: every number is range-checked, device paths are plain `/dev` paths, names are shown as plain text (see 2, 7) |
| **Another process in the live session** | The system bus, the live user's files | Partly: it can call the helper as the live user can (the live user may install by design); it cannot read secrets from the helper, nor reach more than the granted actions (see 1) |
| **A caller that is not the installer's user** (a remote or inactive session, a service) | The system bus | Yes: polkit refuses, secrets in `Status` are withheld |
| **Hostile input from the UI** (a compromised UI process, a typo) | The `Install` arguments | Yes: the helper validates everything again (see 2) |
| **Someone with the finished disk and no keys** | The installed disk | Yes, within the limits under "Left": LUKS2 with a recovery key and the TPM or a password; GRUB is locked |
| **The network** (Flathub, GitHub, the Fedora registry at the first start) | The apps the first start installs | Partly: TLS 1.2 or later, Flathub's GPG, mise pinned by SHA-256, attestations for `gh` (see 7) |
| **The supply chain** | crates.io, the CI actions, the ISO build's inputs | Partly: locked builds, `cargo-deny`, `cargo-audit`, actions pinned by commit, the image checked against its cosign key (see 8) |

Out of scope, because it is not the installer's to stop: someone at the
keyboard of the live session (they booted the ISO and own the PC); physical
access to a running or suspended installed system; a malicious or compromised
Telamon OS image or ISO; bugs in the tools the helper calls (`cryptsetup`,
`bootc`, `sfdisk`, the kernel's file systems).

## 1. D-Bus and polkit

| Piece | Rule | Held by |
|---|---|---|
| Bus policy (`helper/data/dbus-1`) | Only root may own the name; everyone may call, polkit decides | `only_root_owns_the_name_*` |
| Polkit actions (`helper/data/polkit-1`) | `allow_any` and `allow_inactive` are `no`; `install` is `auth_admin_keep`; `list-disks` and `reboot` are `yes` for an active local session only | `the_polkit_actions_default_*` |
| The live session's rule (`live/rootfs/.../50-telamon-installer-session.rules`) | Only the user `telamon-installer`, local and active, only the three installer actions and six NetworkManager ones | `the_live_sessions_polkit_rule_*` |
| Order of checks | `ListDisks`, `Install`, `Status` and `Reboot` authorize (subject: the caller's bus name) before any side effect | `helper/src/secure_tests.rs` |
| Secrets in `Status` | The recovery key and the MOK password are withheld from a caller that polkit would make authenticate (checked without interaction) | `status_never_hands_secrets_to_a_caller_who_may_not_install` |
| The live user | Has no password, no groups; nothing of the live session (rules, users, autologin) reaches the installed system | `the_live_user_has_no_password_*`, `the_install_writes_exactly_these_places_and_nothing_of_the_live_session` |
| The helper's unit | `LimitCORE=0`, `LockPersonality`, `RestrictRealtime`, `SystemCallArchitectures`, `LimitMEMLOCK`; the keys it leaves out (`NoNewPrivileges`, `ProtectHome`, ...) are the ones `bootc` and the mounts need, and a test asserts the unit matches its own comment | `secure_tests.rs` |
| The process | `PR_SET_DUMPABLE` 0 and `mlockall` at start: no core dump, no ptrace by others, secrets not in swap | `main.rs` |

## 2. Input to the root helper

Entry points: the `Install` arguments, the output of `lsblk`, `sfdisk`,
`efibootmgr` and `findmnt`, a disk's own contents (model, labels, serial,
partition table, an ESP), NetworkManager keyfiles.

- **The helper never runs a shell** and never takes a device path from the
  caller. Every program is a `run::bin::*` absolute path, run with an empty
  environment (`PATH=/usr/bin`; the one exception is `NEWPIN` for one process,
  see 4), a timeout, a process group, and capped output (4 MiB per stream).
- **Arguments** are checked by `installer-core` before anything else: a disk
  id is `[a-z0-9]{1,32}` and must be one the probe offers, a fingerprint 16
  hex digits, locale and keymap have a fixed shape, UUIDs and FAT volume IDs
  fixed lengths of hex, app ids come from the catalog, a password is 8 to 256
  and a PIN 4 to 64 printable ASCII characters.
  *Tests:* the `props` modules (property tests at 20000 cases in CI: nothing
  panics, and what is accepted has the safe shape) and the nasty-corpus
  tests next to them.
- **Device paths** come from `lsblk`/`sfdisk`, never from the caller, and must
  be plain `/dev` paths (`plan::is_device_path`, enforced by `Plan::validate`);
  the wipe list may only hold the chosen disk and its own partitions. A
  disk that is not offered (the boot media, anything read-only, Ventoy) is
  refused, and the disk's fingerprint is checked again just before the first
  write.
  *Tests:* `device_paths_are_plain`, `a_plan_with_a_device_that_is_not_a_plain_dev_path_is_refused`.
- **Argument injection:** the matrix test installs in all 16 combinations
  (erase and free space, each encryption mode, NVIDIA on and off, Wi-Fi, apps)
  against the recording fake runner and asserts for every command that the
  program is known, no argument holds NUL or a newline, every argument that
  starts with `-` is on a fixed option list, and every `/dev/` argument
  matches `^/dev/[A-Za-z0-9/_.-]+$`; hostile model, label and serial strings
  in the `lsblk` fixture never reach an argument.
  *Test:* `every_command_of_every_install_*`.
- **The image reference** is built from constants (`ghcr.io/eternalcoder454/atlasos[-nvidia]:stable`)
  and a flag; nothing from the caller reaches it. *Test:* `the_image_comes_from_constants_only`.
- **A hostile partition table** (`lastlba = u64::MAX`, huge sector sizes) no
  longer overflows (the release build panics on overflow: `overflow-checks`
  is on). *Tests:* `a_table_naming_the_last_sector_of_the_number_range_does_not_overflow`,
  `describing_a_plan_from_absurd_numbers_does_not_overflow`.
- **A hostile ESP** is mounted read-only with `nosuid,nodev,noexec`, as `vfat`,
  only to look for Windows' boot manager and free space.

## 3. The image and the signature policy

- The ISO embeds the image pinned by digest; `iso/make-iso.sh` checks its
  cosign signature against `cosign.pub` with a throwaway policy (default
  `reject`, `sigstoreSigned`, `matchRepository`) before anything is built
  (`--local` skips this on purpose and says so).
- At install, `bootc install to-filesystem` gets `--enforce-container-sigpolicy`
  when the live system's own policy proves it will work: `policy.json` has a
  `default` that does not accept everything, the most specific `docker` scope
  of the image holds only `sigstoreSigned` requirements whose key files exist,
  and a `registries.d` file reads sigstore attachments (`sigpolicy::check`).
  The installed system's origin is then `ostree-image-signed`, so its first
  update is already signature-checked. When the policy does not prove it, the
  install goes ahead as before and `Outcome.warnings` says so (the image's
  update stager converts the origin at the first update check).
  *Tests:* `a_live_system_whose_policy_proves_it_installs_a_signed_origin`,
  `a_policy_that_does_not_prove_it_installs_as_before_and_says_so`.
  **Not tested end to end** without a VM: one install, then `bootc status`.
- The install uses no network.

## 4. Passwords, PINs and keys

- The disk password goes to `cryptsetup luksAddKey --new-keyfile -` on stdin;
  the TPM PIN goes in `NEWPIN` of the one `systemd-cryptenroll` process that
  enrols it; neither is ever in an argument. The recovery key is read from
  `systemd-cryptenroll`'s stdout (a pipe) and shown once. The temporary LUKS
  key is a random 64-byte file, `0600`, in a `0700` directory, deleted
  however the install ends, and checked to no longer work.
- Copies are wiped when dropped (`Secret`, `Cmd`, volatile writes), the helper
  is `mlock`ed and non-dumpable, `Debug` output never shows a secret, and a
  failed secret command's stdout never goes into an error.
  *Tests:* `canaries_reach_only_the_commands_that_need_them` (a canary
  password, PIN and Wi-Fi password through a full install in every mode: they
  appear only where they must, and not in argv, `Debug`, the log, the saved
  log, or `Status` for an unprivileged caller), `the_prepared_install_shows_the_wifi_file_name_and_not_its_password`.
- LUKS2 with named parameters (`aes-xts-plain64`, 512-bit key, `sha256`,
  `argon2id`), not the defaults of the cryptsetup build. *Test:*
  `the_luks_parameters_are_named_and_not_left_to_the_build_of_cryptsetup`.
- The UI wipes its own copies of the disk password, PIN and Wi-Fi password.
  *Test:* `wiping_zeroes_the_whole_buffer`.
- Real tools: `tests/container/crypt-flow.sh podman` runs the real `cryptsetup`
  and `systemd-cryptenroll` on regular files in an unprivileged container and
  checks that what `installer-core` parses is what they print (exit code 2 for
  the removed key, the dump of two key slots, the recovery key format).

## 5. TPM2, PIN and the boot menu

- The TPM key is enrolled with no PCR policy at the install (the live
  system's PCR 7 can hold another boot loader's key) and bound to PCR 7 by a
  sandboxed one-shot unit at the first start (`telamon-tpm-seal.service`:
  `ProtectSystem=strict`, no network, a system-call filter, `LoadCredential=`
  for the PIN, no secret in `ExecStart`; it removes the PIN file and the
  marker only after it succeeded). *Test:* `the_seal_unit_is_sandboxed_and_holds_no_secret`.
- With **Secure Boot off** PCR 7 does not identify the OS: the TPM then opens
  the disk for any system started on that PC. The install returns a warning
  for `tpm` and `tpm-pin` in that case. *Test:* `a_tpm_disk_on_a_pc_without_secure_boot_comes_with_a_warning`.
- Even with Secure Boot on, TPM-only unlocks for any system with the same
  PCR 7 (a Fedora-signed live USB). The PIN mode is the answer; the README's
  "Known limit" says so.
- GRUB is locked on every encrypted install (a random password nobody knows,
  only its PBKDF2 hash in `user.cfg`, `0600`); `rd.shell=0 rd.emergency=reboot`
  and `rd.luks.options=...,tries=0` are set. `/boot` stays editable offline.

## 6. Files written into the new system

- Every write goes through `place()`: each path component opened with
  `openat(O_NOFOLLOW)`, a temporary name created exclusively, then renamed,
  then the directory synced. A hostile deployment tree with a symlink at every
  file and directory on every path is never written through. *Test:*
  `a_hostile_deployment_tree_is_never_written_through` (more than 25 places;
  fails when `O_NOFOLLOW` is removed).
- The set of files written is asserted exactly, with modes: `0644`
  configuration, `0600` for the PIN, the seal marker, the Wi-Fi keyfile,
  `user.cfg` and the logs; the directory of a private file is forced to
  `0700` even if the image ships it open. *Tests:* `the_install_writes_exactly_these_places_and_nothing_of_the_live_session`,
  `the_pin_directory_is_private_even_when_the_image_ships_it_open`.
- The Wi-Fi keyfile is the **one** chosen connection, only if it is a Wi-Fi
  connection (`type=wifi`), with no control characters; `permissions=` and
  `interface-name=` are dropped. *Test:* `only_the_chosen_wifi_connection_is_copied_and_only_if_it_is_wifi`.
- Text interpolated into files (locale, keymap, FAT volume ID in `custom.cfg`)
  is validated to a charset with no newline, quote or space (`props`).
- `/run/telamon-installer` is `0700`. The install log (`0600`, and its copy in
  the new system in a `0700` directory) holds the commands and their output
  with secrets omitted; it does hold disk serials, partition GUIDs and the
  Wi-Fi file name, which are not secret but personal.

## 7. The UI and the first start

- **UI:** demo mode is refused in the live session and fails closed (the
  checks read `/proc/cmdline`, the user, the session directory); every QML
  label shows plain text; disk and network names are sanitized (control and
  bidi characters replaced) because a USB stick chooses its model and serial;
  no page opens links, loads code or runs a program. *Tests:*
  `every_label_shows_plain_text`, `no_page_opens_links_loads_code_or_runs_a_program`,
  `a_disk_chooses_its_name_but_not_how_it_is_shown`.
- **First-start apps** (`firstboot/`). What is installed is chosen from the
  catalog `/usr/share/telamon/first-boot-apps.json`; the record the installer
  wrote (`/var/lib/telamon/first-boot-apps.json`, root-owned, checked in both
  the system and user parts) only holds ids. Nothing in it is executed or used
  as a URL; flatpak ids, tool names and package names have strict patterns
  that cannot start with `-`. *Tests:* `test_ids_and_values_that_could_be_options_or_urls_are_never_run`,
  `test_user_mode_ignores_a_record_that_is_not_the_systems`.
- **Local AI** is two catalog entries on the apps page, under a graphics
  warning, and installs no model:
  - `ollama`: `mise use -g ollama`, the upstream release from GitHub through
    mise's `aqua` backend. Upstream publishes no signature or attestation for
    it: the check is the digest GitHub lists for the file. Unpinned (latest).
  - `alpaca`: the Flatpak `com.jeffser.Alpaca` and its plugin add-ons
    (`...Plugins.Ollama`, `...Plugins.AMD` on AMD GPUs) from Flathub system-wide;
    Flathub's GPG signs the repository.
  - `mise` itself is pinned (`MISE_VERSION`) and its SHA-256 is checked before
    it is made executable (`curl --proto =https --tlsv1.2 --max-filesize`).
    mise runs with only the backends that download and check (`MISE_ENV`:
    `asdf`, `vfox`, `cargo`, `go`, `npm`, `pypi`, `gem`, `dotnet`, `spm`,
    `conda` disabled, since an `asdf` plugin is a third-party git repository
    whose scripts run as the user) and with cosign, SLSA, minisign and GitHub
    attestation verification forced on. *Test:* `test_mise_runs_with_only_the_backends_that_download_and_check`.
- The root part (`flatpak install --system`) runs in a sandboxed unit and adds
  Flathub over https with GPG on (the default).

## 8. Build hardening and supply chain

- **Programs:** the helper is built with full RELRO, `BIND_NOW` and a
  non-executable stack asked for by name, and `overflow-checks` on. `scripts/check-hardening.sh`
  reads the finished programs back with `readelf` (position-independent,
  `GNU_RELRO` and `BIND_NOW`, no executable stack, no `RPATH`, no text
  relocations, stack protectors in the C++ UI) and `live/stage-installer.sh`
  refuses to stage a program that fails; CI builds the release helper and runs
  the same check, and tests the check itself with programs built with and
  without each flag (`scripts/test-check-hardening.sh`). Intel CET `IBT` is a
  note, not a requirement: rustc has no stable switch for it.
- **Dependencies:** `Cargo.lock` is committed and every build is `--locked`.
  `deny.toml` (advisories with `unmaintained = "all"` and `yanked = "deny"`,
  licences, banned crates, sources: only crates.io) and `cargo-audit` run on
  every change to the dependencies and every Monday
  (`.github/workflows/security.yml`), for the helper's workspace and for the
  UI's own (`app/`).
- **CI:** every action is pinned by commit; the default token is
  `contents: read`; `packages: write` is held by the one job that publishes the
  dev image, on `main` only; `persist-credentials: false`; the UI's jobs run
  in the dev image with no capabilities and no network, and `.git` and `.github`
  read-only; `atlas-framework` is pinned by commit.
- **ISO:** the image is verified against `cosign.pub` and pinned by digest;
  Fedora RPMs pinned for the Qt build come from Koji, signed, and every RPM's
  signature is checked before `dnf` installs it.

## Tests

| What | How |
|---|---|
| Unit and fake-runner tests, property tests | `cargo test --workspace --locked`; `PROPTEST_CASES=20000 cargo test --workspace --locked -- props` |
| Real `cryptsetup` / `systemd-cryptenroll` | `tests/container/crypt-flow.sh podman` (unprivileged container, regular files) |
| First-start script | `env -u DISPLAY -u WAYLAND_DISPLAY python3 -m unittest discover -s firstboot/tests -v` |
| UI | `app/dev.sh` (see `CLAUDE.md`) |
| Hardening | `scripts/test-check-hardening.sh`, `scripts/check-hardening.sh <elf>` |

## Left

| Item | Why it stays | What would close it |
|---|---|---|
| The TPM PIN is a file on the encrypted root until the first-start seal succeeds | The seal unit re-enrols without a prompt; if an update is staged before it succeeds the PIN is also copied into the new deployment | Ask for the PIN at the first start (a product change) |
| The TPM key has no PCR policy between the install and the first start | PCR 7 of the live system is not the installed system's (Ventoy, ISO files) | none known |
| TPM-only opens for any system with the same PCR 7 | The design of PCR 7 | The PIN mode |
| `/boot` and the EFI partition can be edited offline | Not encrypted by design | A UKI with a measured boot (later) |
| `ollama` has no upstream signature; `mise use -g` is unpinned | Latest is the point; verification is the digest GitHub lists | Pin versions in `apps.json` and bump them |
| The toolbox image comes from the Fedora registry under the image's catch-all `insecureAcceptAnything` policy | That policy is the image repository's | Sign-check it there |
| The helper's and first-start units have no more sandbox keys | Not testable without a VM, and a wrong key breaks every install | A VM run: `ProtectKernelLogs`, `ProtectHostname`, `RestrictAddressFamilies` |
| `iso/make-iso.sh` reads the cosign key from the sibling `../AtlasOS/cosign.pub` | Key rotation is a file change | Pin the key's hash in the script |
| `bootc install --enforce-container-sigpolicy` is not tested end to end | Needs a VM | One install, then `bootc status` |
| The disk fingerprint is FNV, not a cryptographic hash | It detects an accidental swap; whoever can swap a disk mid-install has the machine | none wanted |
