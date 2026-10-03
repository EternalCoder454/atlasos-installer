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
- The ISO is `build/atlasos.iso`, built by `iso/make-iso.sh`.
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
  Set `SCALE=1.5` to match the user's desktop.
- **In a VM**, the UI runs in the live Plasma session:
  - Push the helper with `vm.py push`, and copy `build/app/atlas-installer`
    to `/usr/bin/`.
  - Add a user `live` (in wheel) with autologin in
    `/etc/plasmalogin.conf.d/autologin.conf`:
    `[Autologin]`, `User=live`, `Session=plasma`, `Relogin=true`.
  - Add a test polkit rule in `/etc/polkit-1/rules.d/` that allows `live`
    the `net.eterneon.atlas.installer.*` and
    `org.freedesktop.NetworkManager.*` actions. Make it mode 644:
    `vm.py exec` creates files 0600, and polkitd ignores what it can't read.
    The live ISO needs the same rule for real (Phase 3).
  - Start the app with
    `systemd-run --machine=live@ --user --unit=atlasinst-ui /usr/bin/atlas-installer`.
  - Click with `tests/vm/click.sh <vm> X Y`, type with `virsh send-key`,
    and look with `vm.py shot`. If the screen locks, use
    `loginctl unlock-session`.
- **Keyboard:** KWin 6.7 reloads the layout only on KConfig's change
  notice (`org.kde.kconfig.notify` `ConfigChanged` on path `/kxkbrc`).
  The old `org.kde.keyboard` `reloadConfig` signal does nothing.
