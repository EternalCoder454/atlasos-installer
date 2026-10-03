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
- `iso/`, `live/`: the live ISO build. `tests/vm/`: the VM test tools.

## Commands

| Task | Command |
|---|---|
| Tests | `env -u DISPLAY -u WAYLAND_DISPLAY cargo test --workspace` |
| Build | `cargo build --release -p atlas-installer-helper` |
| Lint and format | see below (clippy and rustfmt are not on the host) |

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
