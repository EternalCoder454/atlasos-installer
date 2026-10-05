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

Returns JSON with `disks`, `hidden`, `tpm2` and `secure_boot`. `secure_boot` is
a boolean: Secure Boot is on (the TPM binds the disk to PCR 7, which holds
its state). `tpm2` is a boolean: a
usable TPM 2.0 exists (a `/sys/class/tpm/tpm*` chip whose
`tpm_version_major` is `2`, and `/dev/tpmrm0`). Each disk has these fields:

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

### `Install(disk_id s, fingerprint s, mode s, locale s, keymap s, wifi_uuid s, encryption s, password s) → s`

Polkit action: `net.eterneon.atlas.installer.install`. An active local
session needs admin authentication (`auth_admin_keep`). Other sessions are
refused.

| Argument | Values |
|---|---|
| `disk_id`, `fingerprint` | From ListDisks. Install refuses the disk if anything about it has changed since: a different disk under the same name, or changed partitions. It is checked again just before the first write. |
| `mode` | `erase` or `free-space` |
| `locale` | For example `de_DE.UTF-8`. |
| `keymap` | `de`, or `de(nodeadkeys)` with a variant. |
| `wifi_uuid` | A NetworkManager connection to copy into the new system, or an empty string. |
| `encryption` | `none`, `tpm`, `tpm-pin` or `password`: how the root partition is encrypted (see below). Anything else is refused. `tpm` and `tpm-pin` are refused when ListDisks said `tpm2` is false. |
| `password` | For `password`: the disk password, 8 to 256 characters. For `tpm-pin`: the PIN, 4 to 64 characters. Printable ASCII only (space to `~`): the initramfs prompt can't reliably type anything else (keymaps, dead keys, normalisation). An empty string for `none` and `tpm`; anything else is refused. |

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
{"mok_password": "12345678" | null, "recovery_key": "xxxxxxxx-...(8 groups)" | null,
 "encryption": "tpm",
 "windows_entry": true,
 "boot_media": "cd" | "usb" | "",
 "warnings": ["..."], "log": "/run/atlas-installer/install.log"}
```

`recovery_key` is set when `encryption` is `tpm`, `tpm-pin` or `password`: eight
groups of eight characters from `cbdefghijklnrtuv`, joined by `-`. It is the
only way in if the TPM or the password fails, so the UI must show it and ask
the user to keep it. It is shown once and never logged. `encryption` repeats
the request's mode even when `recovery_key` is withheld (see Status), so the UI
can tell the user that a key exists that it couldn't show, and how to make a
new one. Typing it at the
boot prompt works with the user's keyboard layout (see the kargs below).

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

`result.mok_password` and `result.recovery_key` are filled only for a caller
that polkit would let install (`net.eterneon.atlas.installer.install`)
without asking: the check runs with no interaction. For anyone else they are
`null`. Neither is ever logged, and neither is the disk password.

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

The copy stage moves when bootc prints a milestone line. While the image
layers import, it moves by what has been copied, the average of two shares:
the bytes bootc's image proxy (`skopeo experimental-image-proxy`, in bootc's
process group) has handed it, against the image's uncompressed size from
`podman image inspect` (the proxy writes each layer twice, to a temporary
file and then to bootc, so half of what it writes); and the layers imported
(one ref each in the new system), against bootc's "layers needed". Either
share alone does if the other can't be read, and with neither the bar eases
forward with time. Once all is copied, the bar eases on while bootc merges
and deploys the layers, which it does silently (about 3 minutes in the VM).
The install log gets a line about the copy (amount, speed, layers) every 30
seconds. While bootc runs, Progress is sent at least every 5 seconds, even
when unchanged, so a UI can keep a time estimate current.

## Errors

| Error | When |
|---|---|
| `net.eterneon.atlas.Error.InvalidArgument` | An argument is malformed, or the fingerprint is missing. |
| `net.eterneon.atlas.Error.NotAuthorized` | Polkit refused the call. |
| `net.eterneon.atlas.Error.Busy` | An install is running, or the computer is restarting. |
| `net.eterneon.atlas.Error.ShuttingDown` | The helper is exiting. Call again. |
| `net.eterneon.atlas.Error.Failed` | The text says what happened. The install log has the details. |

## Disk encryption

With `encryption` `tpm`, `tpm-pin` or `password`, only the root partition
is encrypted (LUKS2); the ESP and `/boot` stay plain. In the Format stage,
for the root partition `<node>`, with a random UUID `<uuid>` and a random
64-byte temporary key (a file `0600` in a `0700` directory under
`/run/atlas-installer`, deleted whatever happens):

1. `wipefs --all --quiet <node>`
2. `cryptsetup luksFormat --type luks2 --batch-mode --uuid <uuid> --label atlasos --key-file <key> <node>`
3. `cryptsetup open --allow-discards --key-file <key> <node> luks-<uuid>`, then
   `mkfs.btrfs` on `/dev/mapper/luks-<uuid>`, which is mounted at the target
4. `systemd-cryptenroll --unlock-key-file=<key> --recovery-key <node>`: the
   recovery key is read from its stdout, and refused unless it has the
   format above
5. `tpm`: `systemd-cryptenroll --unlock-key-file=<key> --tpm2-device=auto --tpm2-pcrs=7 <node>`,
   then `cryptsetup open --test-passphrase --token-only --token-type systemd-tpm2 <node>`
   proves the TPM unlocks it. If either fails, Install stops: "This PC's
   security chip (TPM) didn't accept the disk key. Go back and turn
   encryption off, or try again."
   `tpm-pin`: the same enrolment with `--tpm2-with-pin=yes` and the PIN in
   the environment variable `NEWPIN` (never in the arguments). cryptsetup
   can't be given the PIN without a prompt, so the TPM unlock is not
   proven here, only that the enrolment succeeded.
   `password`: `cryptsetup luksAddKey --key-file <key> --new-keyfile - <node>`
   with the password on stdin (no newline, never in arguments or the
   environment)
6. `cryptsetup luksRemoveKey --key-file <key> <node>`, then checks that the
   temporary key no longer works, that `cryptsetup luksDump
   --dump-json-metadata <node>` shows exactly two key slots (and, for the TPM
   modes, one `systemd-tpm2` token), and that the recovery key (and the
   password) unlock the volume. These key tests are
   `cryptsetup open --test-passphrase --disable-external-tokens --key-file ...`:
   cryptsetup tries the TPM token before the key it is given, so without that
   flag every key would seem to work.

A failing step stops the install with "Setting up encryption failed while
<step>. The install log has the details: <log path>"; the tools' own
messages are only in the log. Neither the password, the PIN, the recovery
key nor the temporary key is ever in the log.

bootc gets these kernel arguments besides its own:

| Mode | `rd.luks.options=<uuid>=` |
|---|---|
| `tpm` | `discard,tpm2-device=auto,tries=0` |
| `tpm-pin` | `discard,tpm2-device=auto,tries=0` |
| `password` | `discard,tries=0` |

and for every encrypted mode: `rd.luks.uuid=<uuid>`,
`vconsole.keymap=<console keymap>` (the password, PIN and recovery key are
typed in the user's layout), `rd.shell=0` and `rd.emergency=reboot` (no root
shell from the initramfs after the unlock; a failed boot restarts, so
greenboot's boot counter can fall back). `tries=0` (every mode, the TPM
one too) keeps asking, so three mistyped keys don't end in a restart loop
that would use up the boot counter.

`discard` is deliberate: TRIM reaches the disk (SSD speed and wear), at the
price that someone with the disk can see which blocks are in use, and
roughly how much data there is. BitLocker passes TRIM too.

**GRUB is locked** on every encrypted install, or `e` at the GRUB menu and
`init=/bin/sh` would give a root shell on a disk the TPM has unlocked. The
helper makes a random 32-byte password, hashes it with
`grub2-mkpasswd-pbkdf2` (twice on stdin), writes only the hash as
`GRUB2_PASSWORD=<hash>` to `/boot/grub2/user.cfg` (mode 0600, labelled like
`custom.cfg`) and forgets the password. bootupd's static config then makes
`root` the only superuser, so editing entries and the GRUB command line
need a password nobody has; the boot entries and the Windows entry in
`custom.cfg` (`--unrestricted`, always) stay bootable. If this can't be
done the install fails ("Locking the boot menu failed"). The lock does not
stop someone editing `/boot` offline, from another machine; see the README's
"Known limit".

After the final unmount the helper also unmounts every other mount whose
source is the target's mapper or one of its new partitions (by name or by
real path, so `/dev/dm-N` and `/dev/disk/by-*` match too), deepest first
(found with `findmnt --json --list --output TARGET,SOURCE`; `bootc install
to-filesystem` leaves the target mounted at `/run/bootc/storage`, which
would keep the mapping busy), logging each one. Then it runs
`cryptsetup close luks-<uuid>`; a failure is a warning. The same sweep runs
before the mapping is closed when an install fails. When an unmount fails, it is retried with
`umount --recursive --lazy`, and the mapping is closed with
`cryptsetup close --deferred`. When an install fails, the target is
unmounted and then the mapping is closed; cleanup errors go to the log. At
the start of an install, any `luks-<uuid>` mapping on the chosen disk whose
container is the installer's own (LUKS label `atlasos`), found with `lsblk`
and left by an earlier failed attempt, is closed too, or wipefs and sfdisk
would find the disk busy. A volume the user unlocked themselves keeps the
disk "in use". The partitions of a failed install are left as any other
failed install leaves them.

The helper locks its memory (`mlockall`: all now, later mappings as they are touched) so the secrets
it handles don't reach swap. `Debug` output never shows a secret, and
Install's `mok_password` and `recovery_key` are the only places the helper
writes one out.

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
2. Run the safety checks, then format (see "Disk encryption" for an
   encrypted root) and mount at `/run/atlas-target`.
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
   - `atlasos/installer.ini` (0644): `[Installer]` with `Version=1`,
     `Language`, `KeyboardLayout`, `KeyboardVariant` and `Network` (true
     when a Wi-Fi connection is carried over or a real Ethernet device has
     a cable in). AtlasOS's first-run wizard reads it and skips the
     language and keyboard pages, and the Wi-Fi page when it's online.
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
atlas-installer-helper --plan <disk> erase|free-space [locale] [keymap] [wifi-uuid] [none|tpm|tpm-pin|password]
```

`--plan` is a dry run: it prints the plan, including the encryption steps and
kernel arguments, and writes nothing. It never takes a password.

## Files

| Path | Source in this repo |
|---|---|
| `/usr/libexec/atlas-installer-helper` | built from `helper/` |
| `/usr/share/dbus-1/system.d/net.eterneon.atlas.InstallerHelper.conf` | `helper/data/dbus-1/system.d/` |
| `/usr/share/dbus-1/system-services/net.eterneon.atlas.InstallerHelper.service` | `helper/data/dbus-1/system-services/` |
| `/usr/share/polkit-1/actions/net.eterneon.atlas.installer.policy` | `helper/data/polkit-1/actions/` |
| `/usr/lib/systemd/system/atlas-installer-helper.service` | `helper/data/systemd/` |
