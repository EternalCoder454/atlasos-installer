#!/usr/bin/env bash
# Builds the AtlasOS live ISO. Runs inside the builder container
# (iso/Containerfile.builder), started by iso/make-iso.sh, with:
#
#   /rootfs        the live image (live/Containerfile), mounted read-only
#   /payload-oci   the image to install, as an OCI layout (for a published
#                  image, the registry's exact blobs, so its digests match ghcr.io)
#   /out           where the ISO is written
#
# and the environment: ISO_LABEL, ISO_NAME, PAYLOAD_REF (the image name the
# embedded copy is stored under, e.g. ghcr.io/eternalcoder454/atlasos:stable).
#
# Adapted from Universal Blue's titanoboa build_iso.sh (Apache-2.0,
# https://github.com/ublue-os/titanoboa).
set -euo pipefail

: "${ISO_LABEL:?}" "${ISO_NAME:?}" "${PAYLOAD_REF:?}"
work=/work # a host folder: containers storage can't keep overlay layers on the container's own overlay root
find "$work" -mindepth 1 -delete
mkdir -p "$work"/iso/LiveOS "$work"/iso/images/pxeboot "$work"/efi/EFI/BOOT "$work"/efi/EFI/fedora

# The image to install, in containers storage, so the live system's podman
# and bootc read it as an extra image store (/usr/lib/containers/storage is
# in Fedora's storage.conf). mksquashfs stores files it has seen once, so
# this costs little beside the live root, which has the same files.
echo ">> Embedding $PAYLOAD_REF"
# Once in a while skopeo fails with "layer not known" partway through (seen
# 1 of 2 runs, 2026-10-02); a second try from an empty store has worked.
embed() {
	skopeo copy --preserve-digests --quiet oci:/payload-oci:latest \
		"containers-storage:[overlay@$work/payload+$work/payload-run]$PAYLOAD_REF"
}
embed || {
	echo ">> Embedding failed; trying once more from an empty store"
	# A failed copy can leave layers mounted, deepest last in the list.
	awk -v p="$work/payload" 'index($2, p) == 1 { print $2 }' /proc/mounts |
		sort -r | xargs -r umount -l
	rm -rf "$work/payload" "$work/payload-run"
	embed
}
rm -rf "$work/payload-run" "$work/payload/overlay-containers"
mount --bind "$work/payload" /rootfs/usr/lib/containers/storage

echo ">> Making the squashfs"
mksquashfs /rootfs "$work/iso/LiveOS/squashfs.img" -noappend -quiet \
	-comp zstd -Xcompression-level 19 -e sysroot ostree
umount /rootfs/usr/lib/containers/storage

kernel_dir=$(echo /rootfs/usr/lib/modules/*)
cp "$kernel_dir/vmlinuz" "$work/iso/images/pxeboot/vmlinuz"
cp "$kernel_dir/initramfs.img" "$work/iso/images/pxeboot/initrd.img"

# Boot chain: firmware -> shim (EFI/BOOT/BOOTX64.EFI) -> grubx64.efi beside
# it -> grub.cfg in EFI/fedora (grub's built-in prefix). fbx64.efi is left
# out: shim would run it from removable media and add a boot entry for the
# stick to the firmware.
shim=$(echo /rootfs/usr/lib/efi/shim/*/EFI)
grub=$(echo /rootfs/usr/lib/efi/grub2/*/EFI)
cp "$shim/BOOT/BOOTX64.EFI" "$shim/fedora/mmx64.efi" "$grub/fedora/grubx64.efi" "$work/efi/EFI/BOOT/"
cp "$shim/fedora/shimx64.efi" "$shim/fedora/mmx64.efi" "$grub/fedora/grubx64.efi" "$work/efi/EFI/fedora/"

args="root=live:CDLABEL=$ISO_LABEL rd.live.image enforcing=0 quiet rhgb"
cat >"$work/grub.cfg" <<EOF
set default=0
set timeout=5
insmod all_video
set gfxpayload=keep
insmod part_gpt
insmod iso9660
search --no-floppy --set=root --label '$ISO_LABEL'

menuentry 'Install AtlasOS' {
	linux /images/pxeboot/vmlinuz $args
	initrd /images/pxeboot/initrd.img
}
menuentry 'Install AtlasOS (basic graphics)' {
	linux /images/pxeboot/vmlinuz $args nomodeset
	initrd /images/pxeboot/initrd.img
}
menuentry 'UEFI Firmware Settings' {
	fwsetup
}
EOF
cp "$work/grub.cfg" "$work/efi/EFI/BOOT/grub.cfg"
cp "$work/grub.cfg" "$work/efi/EFI/fedora/grub.cfg"
cp -r "$work/efi/EFI" "$work/iso/EFI"

# The EFI image, twice: inside the ISO9660 tree for booting as a CD, and
# appended as a GPT partition for booting as a USB stick. With only the
# appended copy, OVMF finds nothing on a CD: the boot record points outside
# the ISO9660 volume with a load size of 0. FAT16, because 32 MiB is too few
# clusters for a valid FAT32, which OVMF also rejects.
truncate -s 32M "$work/efi.img"
mkfs.fat -n EFIBOOT "$work/efi.img" >/dev/null
mcopy -s -i "$work/efi.img" "$work/efi/EFI" ::
cp "$work/efi.img" "$work/iso/images/efiboot.img"

echo ">> Writing $ISO_NAME"
xorriso -as mkisofs -quiet \
	-R -J \
	-V "$ISO_LABEL" \
	-partition_offset 16 \
	-appended_part_as_gpt \
	-append_partition 2 C12A7328-F81F-11D2-BA4B-00A0C93EC93B "$work/efi.img" \
	-iso_mbr_part_type EBD0A0A2-B9E5-4433-87C0-68B6B72699C7 \
	-e images/efiboot.img \
	-no-emul-boot \
	-iso-level 3 \
	-o "/out/$ISO_NAME" \
	"$work/iso"
implantisomd5 "/out/$ISO_NAME" >/dev/null
# Containers storage leaves its overlay folder bind-mounted on itself.
umount "$work/payload/overlay" 2>/dev/null || true
find "$work" -mindepth 1 -delete
ls -la "/out/$ISO_NAME"
