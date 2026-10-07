# Changelog

## Unreleased

- UI polish: Telamon.Ui's scroll bar everywhere (the lists and the pages), lists
  and choice cards in the same card colours as the sections, Title Case
  headings ("Web Browser", "Developer Tools", "How to Install on <disk>",
  "Pick Your Apps"), the disk choices named after the disk they are for, and
  the Disk page scrolls to them when a disk is chosen in a short window. The
  Local AI warning now appears under the switches once one is turned on, and
  the page's two lines of introduction are one. The recovery key and the MOK
  password are in the mono font. The Wi-Fi list is only as tall as its rows.
  Pages without a field take the keyboard, so Tab starts at their first
  control. The install bar shows it is working until the helper's first step.
  The brand mark is the image's own `telamon` icon when the icon theme has it,
  else the copy bundled in the app (it was drawn blank when neither loaded).
- Faster and lighter: the brand panel loads each step's drawing when first
  needed instead of all eight at start-up.
- First-start apps: the Flathub apps install in one `flatpak install`
  transaction (one at a time only if that fails, to find the failing app), and
  the `mise` tools and the toolbox install at the same time. The done markers
  go in `~/.local/state/telamon` again (a rename slip had swapped them with the
  old `atlasos` folder, which is still read).
- Tools: `app/tools/screenshot.sh` can click, type and scroll before the
  shot (`ACTIONS`), draws the OS mark (`OS_LOGO`), and regrabs a window that
  has not painted yet; `app/tools/bench.sh` measures start-up, memory and CPU.

## 0.2.0

- Renamed to Telamon Installer: Atlas Installer is now `telamon-installer`
  (helper `telamon-installer-helper`, `net.eterneon.telamon.InstallerHelper`,
  `net.eterneon.telamon.installer.*` polkit actions, `net.eterneon.telamon.installer`
  as the app ID, the live session user and files `telamon-installer-*`). Every
  user-visible "AtlasOS" is "Telamon OS". The UI is on Telamon.Ui 2.0.0
  (`telamon-ui`): the ISO needs an image that ships it.
- The first-start apps step is `telamon-first-boot-apps`
  (`telamon-first-boot-apps.service`, system and user, `/usr/libexec/telamon/`,
  `/usr/share/telamon/`, `/var/lib/telamon/`, `~/.local/state/telamon/`,
  `/etc/profile.d/telamon-mise.sh`). The old names keep working for this
  release: `atlas-first-boot-apps.service` is a link to the new unit, the old
  record, status and done markers are read (a first start that finished under
  the old name does not run again), and the installer writes the record under
  both names.
- The installed system gets `/etc/telamon/installer.ini` and, for the
  first-run wizard of an image built before the rename, `/etc/atlasos/installer.ini`
  too; the TPM seal service is `telamon-tpm-seal.service` (files in
  `/etc/telamon-installer`), the saved install log is in `/var/log/telamon-installer`,
  and the firmware boot entry is called "Telamon OS" (an "AtlasOS" entry of an
  earlier install is still replaced).
- Unchanged until the image and the ISO names move: the ISO file names and
  volume labels (`atlasos.iso`, `ATLASOS`), the image references
  (`ghcr.io/eternalcoder454/atlasos`), the btrfs and LUKS label `atlasos`, and
  the image's own files under `/usr/share/atlasos` and `/usr/libexec/atlasos`.

## 0.1.0

First version.
