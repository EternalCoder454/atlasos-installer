#!/usr/bin/env bash
# Spike S2: install AtlasOS by hand from the live session, the way the
# installer's helper will. Runs as root in the live VM.
#
#   s2-install.sh <disk> podman|direct
#
# Erases <disk> (GPT: 600 MiB ESP, 2 GiB /boot, btrfs root), installs the
# embedded image with `bootc install to-filesystem` and points it at ghcr.io.
set -euxo pipefail

disk=$1
how=$2
src=ghcr.io/eternalcoder454/atlasos:latest
target=/run/atlas-target

case $disk in *nvme* | *mmcblk*) p=p ;; *) p= ;; esac

wipefs -a "$disk"
sfdisk -q "$disk" <<EOF
label: gpt
size=600MiB, type=uefi, name="EFI System Partition"
size=2GiB, type=linux, name=boot
type=4f68bce3-e8cd-4db1-96e7-fbcaf984b709, name=root
EOF
udevadm settle
mkfs.vfat -F 32 -n EFI "$disk${p}1"
mkfs.ext4 -q -F -L boot "$disk${p}2"
mkfs.btrfs -q -f -L atlasos "$disk${p}3"

mkdir -p "$target"
mount -o compress=zstd:1 "$disk${p}3" "$target"
mkdir -p "$target/boot"
mount "$disk${p}2" "$target/boot"
mkdir -p "$target/boot/efi"
mount "$disk${p}1" "$target/boot/efi"

args=(install to-filesystem
	--source-imgref "containers-storage:$src"
	--target-imgref "$src"
	--karg rootflags=compress=zstd:1
	"$target")

start=$(date +%s)
case $how in
podman)
	podman run --rm --privileged --pid=host \
		--security-opt label=type:unconfined_t \
		-v /dev:/dev -v /var/lib/containers:/var/lib/containers \
		-v /usr/lib/containers/storage:/usr/lib/containers/storage \
		-v "$target:$target" \
		"$src" bootc "${args[@]}"
	;;
direct)
	bootc "${args[@]}"
	;;
esac
echo "bootc install took $(($(date +%s) - start)) s"
umount -R "$target"
