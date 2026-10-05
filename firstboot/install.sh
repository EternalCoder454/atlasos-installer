#!/bin/sh
# Install the first-start apps step into an image tree: firstboot/install.sh DESTDIR
set -eu
[ $# -eq 1 ] || { echo "usage: $0 DESTDIR" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
root=${1%/}
lib=$root/usr/lib/systemd

install -Dm755 "$here/atlas-first-boot-apps" "$root/usr/libexec/atlasos/atlas-first-boot-apps"
install -Dm644 "$here/apps.json" "$root/usr/share/atlasos/first-boot-apps.json"
install -Dm644 "$here/units/atlas-first-boot-apps.service" "$lib/system/atlas-first-boot-apps.service"
install -Dm644 "$here/units/atlas-first-boot-apps.user.service" "$lib/user/atlas-first-boot-apps.service"
install -Dm644 "$here/profile.d/atlas-mise.sh" "$root/etc/profile.d/atlas-mise.sh"

# Enable them as [Install] would (an image has no first boot for presets).
mkdir -p "$lib/system/multi-user.target.wants" "$lib/user/graphical-session.target.wants"
ln -sf ../atlas-first-boot-apps.service "$lib/system/multi-user.target.wants/atlas-first-boot-apps.service"
ln -sf ../atlas-first-boot-apps.service "$lib/user/graphical-session.target.wants/atlas-first-boot-apps.service"
