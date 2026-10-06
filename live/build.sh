#!/usr/bin/env bash
# Turns the AtlasOS image into the live session (live/Containerfile).
set -euo pipefail

# dmsquash-live boots the squashfs on the ISO as the root filesystem, with an
# overlay in RAM for writes.
echo keepcache=True >>/etc/dnf/dnf.conf
dnf install -y --setopt=install_weak_deps=False dracut-live
sed -i '/^keepcache=True$/d' /etc/dnf/dnf.conf

# /root links to /var/roothome, which doesn't exist at build time, and dracut
# fails on the dangling link. The live root keeps it.
mkdir -p "$(realpath -m /root)"

kernel=$(ls /usr/lib/modules)
[ "$(echo "$kernel" | wc -l)" = 1 ] || { echo "build.sh: expected one kernel, got: $kernel" >&2; exit 1; }
DRACUT_NO_XATTR=1 dracut --force --no-hostonly --zstd --reproducible \
	--add "dmsquash-live dmsquash-live-autooverlay" \
	"/usr/lib/modules/$kernel/initramfs.img" "$kernel"

# The installer, built against this image's Qt and glibc: QML compiled ahead
# of time only runs on the Qt it was built with.
while read -r name built; do
	have=$(rpm -q --qf '%{VERSION}' "$name")
	[ "$have" = "$built" ] || {
		echo "build.sh: the installer was built with $name $built, but the image has $have; iso/make-iso.sh should have pinned its build container to the image's builds" >&2
		exit 1
	}
done </src/installer/built-with
cp -a /src/installer/root/. /

# The live session: autologin as atlas-installer into a bare Plasma session
# that runs the installer full-screen (rootfs/usr/share/atlas-installer-session).
cp -a /src/rootfs/. /
systemd-sysusers /usr/lib/sysusers.d/atlas-installer-session.conf

# Nothing that updates, installs or checks the system runs live: the live
# root is thrown away at shutdown.
systemctl mask \
	atlasos-update-stage.timer \
	atlasos-flatpak-preinstall.timer \
	atlasos-grub-greenboot.service \
	atlas-record-boot.service \
	bootloader-update.service \
	greenboot-healthcheck.service \
	greenboot-set-rollback-trigger.service \
	rpm-ostree-countme.timer \
	fedora-atomic-desktop-appstream-cache-refresh.service \
	fedora-atomic-desktop-mandb-update.service \
	selinux-autorelabel-mark.service \
	systemd-tpm2-clear.service \
	fstrim.timer \
	raid-check.timer
systemctl enable var-tmp.mount var-lib-containers.mount

# The embedded image is bind-mounted here by iso/build-iso.sh. Containers
# storage on the overlay root can't hold overlay layers, hence the tmpfs at
# /var/lib/containers (see rootfs/).
mkdir -p /usr/lib/containers/storage

dnf clean all
rm -rf /var/log/dnf5.log* /var/lib/dnf
