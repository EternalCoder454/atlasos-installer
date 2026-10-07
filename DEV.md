# Developing Atlas Installer

## How it fits together

| Part | What it is |
|---|---|
| `crates/installer-core` | Pure logic, no I/O: lsblk and `sfdisk --json` parsing, which disks to offer, partition plans and their safety checks, settings files, GRUB `custom.cfg`, efibootmgr parsing, progress. Tested with the fixtures in `tests/fixtures/`. |
| `helper/` | `atlas-installer-helper`, the root helper on the system D-Bus, behind polkit. It lists disks and runs the install. Its API is in [docs/helper-api.md](docs/helper-api.md), and its D-Bus, polkit and systemd files are in `helper/data/`. |
| `app/` | `atlas-installer`, the UI: Rust with CXX-Qt, and QML with Kirigami, built with CMake and Corrosion. `src/backend.rs` is the QObject the pages use, `src/helper.rs` talks to the helper, `src/network.rs` does Wi-Fi through NetworkManager, and `src/view.rs` decides what each page shows. |
| `live/` | The live image: the AtlasOS image plus the installer session. |
| `iso/` | The ISO build. |
| `tests/vm/` | Tools for the VM tests. |

The helper refuses the boot media and any disk it didn't offer, whatever the
UI asks for. Before writing anything it checks that the disk still matches
what was listed and that the plan overlaps no existing partition. After
partitioning, and before formatting anything, it checks that the kernel sees
the partitions as planned and that the old ones are unchanged.

## Building and testing

| Task | Command |
|---|---|
| Tests | `env -u DISPLAY -u WAYLAND_DISPLAY cargo test --workspace` |
| The helper | `cargo build --release -p atlas-installer-helper` |
| The UI | `app/dev.sh` (output in `build/app/atlas-installer`) |
| UI tests, lint, qmllint | `app/dev.sh bash -c 'cd app && cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cd .. && cmake --build build/app --target all_qmllint'` |

The UI is built and tested only in its container (`app/Containerfile.dev`),
through `app/dev.sh`, so its Qt and Kirigami match the live image's.

The shared controls (`import Telamon.Ui`) come from
[atlas-framework](https://github.com/EternalCoder454/atlas-framework),
installed in Qt's QML directory like Kirigami; the app links nothing from it.
`app/dev.sh` builds it from a checkout beside this one (`../Atlas Framework`,
or `ATLAS_FRAMEWORK_SRC`) into `localhost/atlas-installer-dev:telamon-ui`
(`app/Containerfile.telamon-ui`). The ISO build instead copies it from the
AtlasOS image it embeds (`iso/Containerfile.telamon-ui`), so the installer is
built against the exact Telamon.Ui the live session runs. Telamon.Ui changes go
to atlas-framework, never here. The installer needs Telamon.Ui 1.3.0 or newer
(TelamonTextField, TelamonComboBox, TelamonSpinner): configuring stops with a plain
message against an older one, which for an ISO means an AtlasOS image that
ships `telamon-ui` older than 1.3.0.

Clippy and rustfmt for the workspace also run in a container:

```sh
podman run --rm --security-opt label=disable -v "$PWD":/src -w /src \
  -v atlas-cargo:/root/.cargo/registry -v atlas-dnf:/var/cache/libdnf5 \
  -e CARGO_TARGET_DIR=/src/target/container fedora:44 bash -c \
  'dnf -y -q --setopt=keepcache=1 install cargo clippy rustfmt gcc >/dev/null; cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings'
```

Never mount the repo with `:Z`. It relabels `build/` and breaks libvirt's
access to the VM disks.

## Running the UI without installing anything

Demo mode, `ATLAS_INSTALLER_DEMO=1`, runs the UI with no helper,
NetworkManager or live session. It's the only way the UI runs in the dev
container, which has no system bus. `ATLAS_INSTALLER_DEMO` also takes flags
such as `wired,mok,fail` (the list is in `app/src/demo.rs`), and
`ATLAS_INSTALLER_DEMO_PAGE=<step>` opens at a step.
The app refuses demo mode in the live session.

For screenshots, `app/dev.sh app/tools/screenshot.sh OUT.png STEP FLAGS
light|dark` runs it on Xvfb in the container. Add `SCALE=1.5` for a 1.5x
display, and `ATLAS=1` for the AtlasOS colours and font.

## The live session

`iso/make-iso.sh` builds the UI and helper in the dev container
(`live/stage-installer.sh`). `live/build.sh` then installs them into the live
image, and refuses if the image's Qt, Kirigami or glibc differ from the
container's. They differ whenever Fedora updates one of them after the image
was built, so `make-iso.sh` then builds in a copy of the dev container pinned
to the image's exact builds (`iso/Containerfile.pin`). `iso/pin-builds.sh`
fetches those from Koji, which keeps every build, and installs them only if
they carry Fedora's signature.

In the live image, plasmalogin logs the `atlas-installer` user into a Plasma
session with no panel or screen lock. The session runs
`/usr/libexec/atlas-installer-session`, which starts the installer full
screen and starts it again if it closes. A polkit rule lets that user run the
installer and NetworkManager actions without a password. None of this is in
the installed system.

## Testing in VMs

Never run the helper's install or `--list`, or anything else that partitions
disks, on your own machine. Real installs are tested only in VMs.

`tests/vm/vm.py` manages `atlasinst-*` VMs in `qemu:///system`, with their
disks in `build/vm/`. It never touches any other VM.

```sh
tests/vm/vm.py create t --iso build/atlasos.iso --disk 64 --secure-boot
tests/vm/vm.py wait t
tests/vm/vm.py shot t shot.png           # screenshot
tests/vm/click.sh t 1212 766             # click at screen pixels (1280x800)
tests/vm/vm.py exec t 'lsblk'            # a command as root, in the live session
tests/vm/vm.py destroy t                 # removes the VM and its disks
```

`create` also takes `--usb` (the ISO as a USB stick, not a CD), `--sata`,
`--disk-file`, `--memory`, `--media-first` (boot the ISO before the disks) and
`--tpm` (a TPM 2.0 from the swtpm emulator, which makes the installer offer
encryption unlocked by the TPM; its state is removed by `destroy`).

To try a new build without rebuilding the ISO, push the helper and its data
files with `vm.py push`, then run `systemctl daemon-reload` and D-Bus
`ReloadConfig`. Copy `build/app/atlas-installer` to `/usr/bin/`, then
`pkill -f '^/usr/bin/atlas-installer'`, and the session restarts the UI.

After an install, and before restarting:

1. Run `tests/vm/test-access.sh <root partition> "$(cat build/vm/id_test.pub)"`
   in the live session (push it first). SELinux confines the guest agent on
   the installed system, so this enables SSH with the test key, and
   `vm.py ssh` works from then on.
2. Restart from the UI rather than with `virsh destroy`, or files written to
   the target can be lost.
3. With `--media-first`, the restart boots the ISO again. Run `virsh destroy`,
   remove the ISO, and start the VM again. The ISO's target is in
   `virsh -c qemu:///system domblklist atlasinst-<vm>`. For a CD, use
   `virsh -c qemu:///system change-media atlasinst-<vm> <target> --eject --config`.
   For a USB stick, use `detach-disk atlasinst-<vm> <target> --config`.

### The test matrix

Each release candidate ISO goes through these runs. Check the installed
system each time: `bootc status` shows `:stable` at the digest the ISO
embeds (in `build/atlasos.iso.image`), SELinux is enforcing, there are no
failed units, the firmware boot entry is called AtlasOS, and the
`atlas-installer` user isn't there.

| Run | VM | Check |
|---|---|---|
| Erase, Secure Boot on | `--disk 64 --disk 30 --secure-boot`, ISO as a CD | The 30 GB disk is greyed out and can't be chosen. The CD isn't listed. The Restart page says to take out the disc. |
| Second disk, Secure Boot off | `--disk 64 --disk 64 --usb --media-first` | The USB stick isn't listed, and the Restart page says to remove it. The two disks are told apart (by serial, or by device path when they have none). Write random data to the start and end of the first disk beforehand, and check that it is identical afterwards. |
| Beside Windows | an overlay of the Windows base, `--sata --usb --secure-boot --media-first` | "Install alongside Windows" is the default. MSR, C: and recovery hash the same before and after, and so does `EFI/Microsoft`. The GRUB menu waits 5 s and has a Windows entry that boots Windows. |
| Encryption, TPM | `--disk 64 --media-first --tpm` | Choose "unlock with the TPM". The recovery key is shown. After the restart the system boots without a prompt. On the installed system `cryptsetup luksDump` shows two slots and one `systemd-tpm2` token with `tpm2-hash-pcrs: 7` (the first start sealed it: `atlas-tpm-seal.service` succeeded, `/etc/atlas-installer/` is empty), `/var/log/atlas-installer/` has `install.log` and `tpm-event-log.bin`, `lsblk` shows `luks-<uuid>` under the root partition, and `/proc/cmdline` has `rd.luks.uuid`. To try the recovery key (the GRUB menu can't be edited), run `systemd-cryptenroll --wipe-slot=tpm2 <dev>` over SSH, reboot and type the recovery key at the prompt, then re-enrol the TPM. |
| Encryption, TPM and PIN | `--disk 64 --media-first --tpm` | As for the TPM, but the PIN is asked at every boot (a wrong PIN asks again, it doesn't restart). After the first boot the `systemd-tpm2` token has `tpm2-pin: true` and `tpm2-hash-pcrs: 7`, `/etc/atlas-installer/tpm-pin` is gone, and the second boot unlocks with the PIN. At the GRUB menu, `e` asks for a password. The recovery key works too. |
| Encryption, password | `--disk 64 --media-first` (no `--tpm`: the choice is not offered) | Choose a password with a non-US keyboard layout. The boot prompt takes the password in that layout, and the recovery key also unlocks. |
| Wi-Fi | `--disk 64 --media-first`, then `tests/vm/wifi-ap.sh <vm>` and restart the UI | Connect to AtlasTest (password atlastest123). The installed system has the keyfile (without `interface-name=`), and connects when `wifi-ap.sh <vm> ssh` brings the access point up there. |

The Windows base is `build/vm/win11-base.qcow2`, made once by
`tests/vm/windows.sh` from a Windows 11 ISO. Never write to it. Give each VM
an overlay named so that `vm.py destroy` removes it:

```sh
qemu-img create -f qcow2 -b "$PWD/build/vm/win11-base.qcow2" -F qcow2 build/vm/atlasinst-win-disk.qcow2
tests/vm/vm.py create win --iso build/atlasos.iso --usb --sata --secure-boot --media-first \
  --disk-file build/vm/atlasinst-win-disk.qcow2
```

VMs have no Wi-Fi. `tests/vm/wifi-ap.sh` adds two simulated radios
(`mac80211_hwsim`, from Fedora's kernel-modules-internal) and runs an access
point on the second one. In the live session it also disconnects the cable,
so the UI shows its Wi-Fi page after a restart.
