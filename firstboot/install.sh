#!/bin/sh
# Install the first-start apps step into an image tree: firstboot/install.sh DESTDIR
set -eu
[ $# -eq 1 ] || { echo "usage: $0 DESTDIR" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
root=${1%/}
lib=$root/usr/lib/systemd

install -Dm755 "$here/telamon-first-boot-apps" "$root/usr/libexec/telamon/telamon-first-boot-apps"
install -Dm644 "$here/apps.json" "$root/usr/share/telamon/first-boot-apps.json"
install -Dm644 "$here/units/telamon-first-boot-apps.service" "$lib/system/telamon-first-boot-apps.service"
install -Dm644 "$here/units/telamon-first-boot-apps.user.service" "$lib/user/telamon-first-boot-apps.service"
install -Dm644 "$here/profile.d/telamon-mise.sh" "$root/etc/profile.d/telamon-mise.sh"

# Enable them as [Install] would (an image has no first boot for presets).
mkdir -p "$lib/system/multi-user.target.wants" "$lib/user/graphical-session.target.wants"
ln -sf ../telamon-first-boot-apps.service "$lib/system/multi-user.target.wants/telamon-first-boot-apps.service"
ln -sf ../telamon-first-boot-apps.service "$lib/user/graphical-session.target.wants/telamon-first-boot-apps.service"

# What it was called until the rename (atlas-first-boot-apps), for one release.
# The image's presets and checks, and the enable links of installed systems,
# name the old units: each old name is a link to the new unit file, which is how
# systemd spells an alias, so `systemctl enable|start atlas-first-boot-apps` and
# a wants link under the old name reach the new unit (there is one unit, never
# two copies of it). The old script and catalog paths are links as well.
ln -sf telamon-first-boot-apps.service "$lib/system/atlas-first-boot-apps.service"
ln -sf telamon-first-boot-apps.service "$lib/user/atlas-first-boot-apps.service"
ln -sf ../atlas-first-boot-apps.service "$lib/system/multi-user.target.wants/atlas-first-boot-apps.service"
ln -sf ../atlas-first-boot-apps.service "$lib/user/graphical-session.target.wants/atlas-first-boot-apps.service"
mkdir -p "$root/usr/libexec/atlasos" "$root/usr/share/atlasos"
ln -sf ../telamon/telamon-first-boot-apps "$root/usr/libexec/atlasos/atlas-first-boot-apps"
ln -sf ../telamon/first-boot-apps.json "$root/usr/share/atlasos/first-boot-apps.json"
# Not a link: both profile.d files would be sourced, and mise activated twice.
printf '%s\n' '# shellcheck shell=bash' '# Moved to telamon-mise.sh (this file was atlas-mise.sh).' \
    >"$root/etc/profile.d/atlas-mise.sh"
chmod 0644 "$root/etc/profile.d/atlas-mise.sh"
