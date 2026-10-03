# AtlasOS Installer

The installer for AtlasOS (bootc, Fedora 44 based). The roadmap and the
Phase 0 results are in Atlas Notes, under `AtlasOS/Atlas Installer/Roadmap`.

## Layout

- `crates/installer-core`: pure logic, with no I/O. It covers:
  - lsblk and `sfdisk --json` parsing
  - disk listing and hiding
  - partition plans and their safety checks
  - settings files, grub `custom.cfg`, efibootmgr parsing
  - progress

  Its tests use the fixtures in `crates/installer-core/tests/fixtures/`.
- `helper`: `atlas-installer-helper`, the root D-Bus/polkit helper. Its API
  is in `docs/helper-api.md`. `install_tests.rs` runs the whole install
  against a fake runner and a temporary directory. The data files (D-Bus,
  polkit, systemd) are in `helper/data/`.
- `app/`: `atlas-installer`, the UI (Rust with CXX-Qt, QML with Kirigami,
  built with CMake and Corrosion). It is outside the workspace and is built
  and tested only in its container, through `app/dev.sh`.
  - `src/backend.rs`: the QObject that QML uses. `src/helper.rs`: the helper
    client. `src/network.rs`: Wi-Fi through NetworkManager.
  - `src/view.rs`: what the pages show, unit-tested.
  - `src/demo.rs`: demo mode (see below). `qml/`: the pages.
- `ui/`: a copy of atlasos-updater's shared QML components (Atlas.Ui). Keep
  the files identical to upstream, apart from the ones `ui/README.md` lists
  as added here.
- `iso/`, `live/`: the live ISO build. `tests/vm/`: the VM test tools.

## Commands

| Task | Command |
|---|---|
| Tests | `env -u DISPLAY -u WAYLAND_DISPLAY cargo test --workspace` |
| Build | `cargo build --release -p atlas-installer-helper` |
| Lint and format | see below (clippy and rustfmt are not on the host) |
| Build the UI | `app/dev.sh` (output in `build/app/atlas-installer`) |
| UI tests, lint, qmllint | `app/dev.sh bash -c 'cd app && cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cd .. && cmake --build build/app --target all_qmllint'` |
| UI screenshot | `app/dev.sh app/tools/screenshot.sh OUT.png STEP FLAGS light\|dark [WAIT]` |

Lint and format run in a container:

```sh
podman run --rm --security-opt label=disable -v "$PWD":/src -w /src \
  -v atlas-cargo:/root/.cargo/registry -v atlas-dnf:/var/cache/libdnf5 \
  -e CARGO_TARGET_DIR=/src/target/container fedora:44 bash -c \
  'dnf -y -q --setopt=keepcache=1 install cargo clippy rustfmt gcc >/dev/null; cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings'
```

Never mount the repo with `:Z`. It relabels `build/` and breaks libvirt's
access to the VM disks. Use `--security-opt label=disable` instead.

## Testing

- **Never run the helper's install, `--list`, or anything that partitions
  disks on the host.** Real tests happen only in VMs. Tick a roadmap box only
  after the feature has been built and tested in a VM.
- `tests/vm/vm.py` manages the `atlasinst-*` VMs in `qemu:///system`.
  Leave every other VM alone, including the user's `atlasos-daily`.
- The ISO is `build/atlasos.iso`, built by `iso/make-iso.sh` from the
  published `:stable` (`--local` for this machine's `localhost/atlasos:latest`).
  AtlasOS's `just iso` and `just iso-local` run it with `-o` into AtlasOS's
  own `build/`.
- The test matrix (which VMs, what to check) is in `DEV.md`. VMs have no
  Wi-Fi: `tests/vm/wifi-ap.sh <vm> [ssh]` adds mac80211_hwsim radios and a
  test access point.
- To use the Windows base image, create a qcow2 overlay named
  `build/vm/atlasinst-<vm>-*.qcow2`, which `vm.py destroy` removes.
  Never write to `win11-base.qcow2` itself.
- Use `--media-first` when a disk would otherwise boot first.
- To put the helper into a live VM, use `vm.py push`:
  - the binary goes to `/usr/libexec/`
  - the data files go to `/usr/share/dbus-1/...`,
    `/usr/share/polkit-1/actions/` and `/usr/lib/systemd/system/`
  - then run `systemctl daemon-reload` and the D-Bus `ReloadConfig`
- On an installed system, `vm.py exec` is confined by SELinux. Run
  `tests/vm/test-access.sh` from the live session before rebooting, then
  use `vm.py ssh`.
- With `--media-first`, a reboot after the install starts the ISO again.
  To boot the installed disk, run `virsh destroy`, then
  `virsh change-media atlasinst-<vm> sda --eject --config` (`sdb` with
  `--sata`), then start it.

## The UI

- **Demo mode** runs the UI without the helper, NetworkManager or the
  session: `ATLAS_INSTALLER_DEMO=1`, or flags such as `wired,mok,fail`
  (the list is in `app/src/demo.rs`). `ATLAS_INSTALLER_DEMO_PAGE=<step>`
  opens at a step. The app refuses demo mode in the live session.
- **Screenshots** come from `app/tools/screenshot.sh`. It runs inside the
  container on Xvfb with a private bus and a Breeze colour scheme. It
  focuses the window, because an inactive window draws washed-out colours.
  Set `SCALE=1.5` to match the user's desktop, and `ATLAS=1` for the
  AtlasOS colours and IBM Plex Sans (copies of the schemes are in
  `app/tools/schemes/`).
- **The live session** (`live/`): `iso/make-iso.sh` builds the UI and
  helper in the dev container (`live/stage-installer.sh`, staged in
  `build/live-installer/`), and `live/build.sh` installs them into the live
  image. It refuses if the image's Qt, Kirigami or glibc version differs from the
  dev container's. In that case, rebuild the container with
  `podman rmi localhost/atlas-installer-dev`.
  - plasmalogin logs the `atlas-installer` user (from sysusers, with its
    home in `/run/atlas-installer-session`) into a Plasma session. It has
    no panel and no screen lock, and allows only plasma-setup's shortcuts.
  - The session autostarts `/usr/libexec/atlas-installer-session`. It runs
    the installer with `--fullscreen` and starts it again if it closes.
  - The polkit rule `50-atlas-installer-session.rules` allows that user the
    installer and NetworkManager actions without a password.
- **In a VM**, boot the ISO and the installer comes up by itself:
  - To try a new build without rebuilding the ISO, push the helper with
    `vm.py push` and copy `build/app/atlas-installer` to `/usr/bin/`. Then
    `pkill -f '^/usr/bin/atlas-installer'`, and the session restarts it.
  - Click with `tests/vm/click.sh <vm> X Y`, type with `virsh send-key`,
    and look with `vm.py shot`.
  - Shut the live session down cleanly (the UI's Restart, or `systemctl
    reboot`), not with `virsh destroy`: files `test-access.sh` writes to
    the target can be lost otherwise.
- **Keyboard:** KWin 6.7 reloads the layout only on KConfig's change
  notice (`org.kde.kconfig.notify` `ConfigChanged` on path `/kxkbrc`).
  The old `org.kde.keyboard` `reloadConfig` signal does nothing.
