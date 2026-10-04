# atlas-installer-helper D-Bus API

The installer's root helper. It runs only in the live session and is started
by D-Bus activation through systemd (`atlas-installer-helper.service`). It
exits after 60 seconds without calls, unless an install has started or the
computer is restarting: then it stays until systemd stops it, so a result
nobody has read yet, or the busy flag, is not lost. A stop (SIGTERM) waits
for a running install, and for an `umount` of the cleanup that is already
running, to finish. It is modelled on Atlas Updater's
`atlas-system-helper`.

| | |
|---|---|
| Bus | system |
| Name | `net.eterneon.atlas.InstallerHelper` |
| Object | `/net/eterneon/atlas/InstallerHelper` |
| Interface | `net.eterneon.atlas.InstallerHelper1` |
| Binary | `/usr/libexec/atlas-installer-helper` |

Polkit checks every call, using the caller's bus name as the subject.

## Methods

### `ListDisks() → s`

Polkit action: `net.eterneon.atlas.installer.list-disks`. An active local
session is allowed without authentication.

Returns JSON with `disks` and `hidden`. Each disk has these fields:

| Field | Meaning |
|---|---|
| `id` | Kernel name, such as `nvme0n1`. Pass it to Install. |
| `fingerprint` | 16 hex digits. Pass it to Install. |
| `path` | Device path. |
| `name` | Model name. |
| `serial` | Serial number, when the disk has one. Left out otherwise. |
| `size` | Size in bytes. |
| `usb` | USB disk: show a badge. |
| `removable` | Removable or hotplug disk. |
| `contents` | What is on the disk, as a list. |
| `description` | The same in words: "Windows", "Windows and Linux", "Empty". |
| `bitlocker` | Holds a BitLocker volume. |
| `too_small` | Under 40 GiB: list it, but grey it out. |
| `erase` | `{possible, reason?}` |
| `free_space` | `{possible, reason?, bytes, shared_esp?}` |

`hidden` lists devices that are never offered, with the reason. It is for
logs; the UI ignores it.

These devices are always hidden:

- the boot media, found from the device behind `/run/initramfs/live` or
  `/run/initramfs/isoscan`
- anything with the label `ATLASOS` or `ATLASOS-NV`
- Ventoy sticks
- read-only devices, optical drives, loop, zram and RAM devices
- RAID and multipath devices

To read EFI partitions, ListDisks mounts them read-only.

### `Install(disk_id s, fingerprint s, mode s, locale s, keymap s, wifi_uuid s) → s`

Polkit action: `net.eterneon.atlas.installer.install`. An active local
session needs admin authentication (`auth_admin_keep`). Other sessions are
refused.

| Argument | Values |
|---|---|
| `disk_id`, `fingerprint` | From ListDisks. Install refuses the disk if anything about it has changed since: a different disk under the same name, or changed partitions. |
| `mode` | `erase` or `free-space` |
| `locale` | For example `de_DE.UTF-8`. |
| `keymap` | `de`, or `de(nodeadkeys)` with a variant. |
| `wifi_uuid` | A NetworkManager connection to copy into the new system, or an empty string. |

Install probes the disks again and decides everything before it writes
anything. It refuses in these cases:

- the boot media, or any disk that is not offered
- a disk whose fingerprint has changed
- a plan that fails its own check: partitions must stay inside the usable
  sectors, be aligned, overlap nothing and reuse no partition number

After partitioning, every partition that existed before must keep its
start, size, type, UUID and name, and the kernel must see the new
partitions where they were planned. If not, Install stops before
formatting anything.

Install returns this JSON:

```json
{"mok_password": "12345678" | null, "windows_entry": true,
 "boot_media": "cd" | "usb" | "",
 "warnings": ["..."], "log": "/run/atlas-installer/install.log"}
```

`boot_media` is what the installer was started from, so the UI can say
what to take out once the computer restarts: an optical drive, a USB disk,
or neither (an internal disk, or not found). The install sets the
firmware's BootNext to the new AtlasOS entry, so leaving it in is safe.

`warnings` covers things that went wrong without failing the install: the
firmware entry could not be renamed, an SELinux label was not set, the NVIDIA
key was not queued, or the final unmount failed. Show them to the user.

While Install runs:

- the helper holds a logind block inhibitor for shutdown, sleep and idle
- every other disk call returns `Busy`

Install is refused with `Busy` once an install has finished (`Status` says
`done`): a second one would lose the first result's MOK password, and the
system is installed. A UI that gets `Busy` from Install follows `Status`.
The final state is written before the busy flag is released.

The install keeps going if the caller disconnects or crashes: it runs in a
task of its own, and its state is kept (see `Status`). A UI that lost the
call asks `Status` instead of reporting a failure.

### `Status() → s`

Polkit action: `net.eterneon.atlas.installer.list-disks` (active local
session, no authentication). Works at any time, also while installing.

Returns the state of the install this helper has run, kept until the
helper stops:

```json
{"state": "idle" | "installing" | "done" | "failed",
 "step": "copy", "fraction": 0.42, "text": "Copying AtlasOS",
 "result": null | { Install's JSON }, "error": null | "message"}
```

| State | Meaning |
|---|---|
| `idle` | No install has started in this helper. |
| `installing` | Running. `step`, `fraction` and `text` are the latest Progress. |
| `done` | Finished. `result` is what Install returned. |
| `failed` | `error` is the text Install's `Failed` error carried. |

`result.mok_password` is filled only for a caller that polkit would let
install (`net.eterneon.atlas.installer.install`) without asking: the check
runs with no interaction. For anyone else it is `null`. The password is
never logged.

A `failed` state stays for a UI that reattaches, until the next
`ListDisks` call (the user going back to choose a disk): that resets it to
`idle`.

A UI that starts, or whose `Install` call dropped, calls Status: `installing`
goes to the Progress page and Status is asked every two seconds (the
Progress signal still works); `done` goes to the Restart page; `failed`
shows the error. After a drop, `idle` means the install did not survive (the
helper restarted) and counts as a failure.

### `Reboot()`

Polkit action: `net.eterneon.atlas.installer.reboot`. An active local
session is allowed without authentication.

Reboot is refused while an install is running. Once the reboot has started,
the helper refuses all disk calls, and no longer exits when idle, so that a
slow shutdown does not lose the flag.

People take the USB stick or disc out before they press Restart, and then
`systemctl reboot` and the shutdown after it can't read the programs they
need. So when the live medium is gone (its device has vanished or has no
size), or `systemctl reboot` fails, the helper syncs the disks and restarts
straight through the kernel. It locks itself in memory at startup so that
this path needs nothing from the medium.

## Signal

### `Progress(step s, fraction d, text s)`

| Field | Meaning |
|---|---|
| `step` | `prepare`, `partition`, `format`, `copy`, `bootloader`, `settings` or `finish` |
| `fraction` | Progress through the whole install, from 0 to 1. It never goes backwards. |
| `text` | Text to show the user. |

The copy stage moves when bootc prints a milestone line, and eases forward
with time while the image layers import.

## Errors

| Error | When |
|---|---|
| `net.eterneon.atlas.Error.InvalidArgument` | An argument is malformed, or the fingerprint is missing. |
| `net.eterneon.atlas.Error.NotAuthorized` | Polkit refused the call. |
| `net.eterneon.atlas.Error.Busy` | An install is running, or the computer is restarting. |
| `net.eterneon.atlas.Error.ShuttingDown` | The helper is exiting. Call again. |
| `net.eterneon.atlas.Error.Failed` | The text says what happened. The install log has the details. |

## What an install does

1. Erase only: wipe the disk's signatures. Erase writes a new GPT with three
   partitions:

   | Partition | Size | Filesystem |
   |---|---|---|
   | ESP | 600 MiB | FAT32 |
   | `/boot` | 2 GiB | ext4 |
   | root | the rest | btrfs, labelled `atlasos` |

   Free space instead appends `/boot` and the root to the largest free
   region with `sfdisk --append`. It shares an existing ESP if:

   - the partition is at least 100 MiB
   - its filesystem has at least 40 MiB free
   - it holds no other Fedora-family system

   If no ESP qualifies, it makes a new 600 MiB one.
2. Run the safety checks, then format and mount at `/run/atlas-target`.
3. Run `bootc install to-filesystem`, from the image embedded in the live
   system:
   - `ghcr.io/eternalcoder454/atlasos:stable`, or
   - `atlasos-nvidia:stable` when the live system carries the NVIDIA
     signing key.
4. Remount read-write, then write these files into the new deployment's
   `/etc`:
   - `locale.conf`
   - `vconsole.conf`
   - `X11/xorg.conf.d/00-keyboard.conf`
   - the Wi-Fi keyfile, with `permissions=` removed and mode 0600

   Label them with the new system's own SELinux policy (`setfiles -r`).
5. With Windows on a surviving ESP: write `/boot/grub2/custom.cfg`, which
   chainloads Windows and sets `timeout=5`.
6. Create the "AtlasOS" firmware boot entry, then delete bootupd's "Fedora"
   entry for that ESP.
7. NVIDIA with Secure Boot on and the key not enrolled: queue
   `mokutil --import` with a random 8-digit password, which Install returns.
8. Unmount.

Every command and its output, except secrets, go to
`/run/atlas-installer/install.log` (mode 0600).

## Command line (root only, for testing)

```
atlas-installer-helper --list
atlas-installer-helper --plan <disk> erase|free-space [locale] [keymap] [wifi-uuid]
```

`--plan` is a dry run: it prints the plan and writes nothing.

## Files

| Path | Source in this repo |
|---|---|
| `/usr/libexec/atlas-installer-helper` | built from `helper/` |
| `/usr/share/dbus-1/system.d/net.eterneon.atlas.InstallerHelper.conf` | `helper/data/dbus-1/system.d/` |
| `/usr/share/dbus-1/system-services/net.eterneon.atlas.InstallerHelper.service` | `helper/data/dbus-1/system-services/` |
| `/usr/share/polkit-1/actions/net.eterneon.atlas.installer.policy` | `helper/data/polkit-1/actions/` |
| `/usr/lib/systemd/system/atlas-installer-helper.service` | `helper/data/systemd/` |
