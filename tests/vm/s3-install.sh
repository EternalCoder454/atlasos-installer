#!/usr/bin/env bash
# Spike S3: install AtlasOS into the free space beside Windows, by hand from
# the live session, the way the installer's "Use free space" will. Runs as
# root in the live VM.
#
#   s3-install.sh <disk>
#
# Reuses the disk's ESP, adds a 2 GiB /boot and a btrfs root in the largest
# free region, installs the embedded image, and adds a GRUB entry that
# chainloads Windows. Then checks that every partition that was there before,
# other than the ESP, is byte-identical, and lists what changed on the ESP.
set -euo pipefail

disk=$1
src=ghcr.io/eternalcoder454/atlasos:stable
target=/run/atlas-target
state=/run/atlas-s3
esp_type=c12a7328-f81f-11d2-ba4b-00a0c93ec93b
mkdir -p "$state" "$target"

echo ">> Before"
sfdisk -d "$disk" | tee "$state/table-before"
esp=$(lsblk -lnpo NAME,PARTTYPE "$disk" | awk -v t=$esp_type '$2 == t {print $1; exit}')
[ -n "$esp" ] || { echo "no ESP on $disk" >&2; exit 1; }
mapfile -t others < <(lsblk -lnpo NAME,TYPE "$disk" | awk -v esp="$esp" '$2 == "part" && $1 != esp {print $1}')
echo "ESP $esp; other partitions: ${others[*]}"

for p in "${others[@]}"; do md5sum "$p"; done >"$state/sums-before"
mount -o ro "$esp" "$target"
(cd "$target" && find . -type f -exec md5sum {} + | sort -k2) >"$state/esp-before"
avail=$(df -B1M --output=avail "$target" | tail -1)
umount "$target"
# The partition's size, not the filesystem's: Windows' 100 MiB ESP holds a 96 MiB FAT.
size=$(($(lsblk -bdno SIZE "$esp") / 1048576))
echo "ESP partition ${size} MiB, free $((avail)) MiB"
[ "$size" -ge 100 ] && [ "$avail" -ge 40 ] || { echo "ESP too small to share" >&2; exit 1; }

# The largest free region, in sectors.
read -r start end sectors < <(sfdisk -qF "$disk" | awk 'NR > 1 {print $1, $2, $3}' | sort -k3 -n | tail -1)
echo "Free region: sectors $start-$end ($((sectors / 2048)) MiB)"
start=$(((start + 2047) / 2048 * 2048))
boot_sectors=$((2 * 1024 * 2048))
root_start=$((start + boot_sectors))
before=$(lsblk -lno NAME,TYPE "$disk" | awk '$2 == "part"' | wc -l)
sfdisk -q --append "$disk" <<EOF
start=$start, size=$boot_sectors, type=linux, name=boot
start=$root_start, size=$((end - root_start + 1)), type=4f68bce3-e8cd-4db1-96e7-fbcaf984b709, name=root
EOF
udevadm settle
case $disk in *nvme* | *mmcblk*) p=p ;; *) p= ;; esac
boot=$disk$p$((before + 1))
root=$disk$p$((before + 2))
mkfs.ext4 -q -F -L boot "$boot"
mkfs.btrfs -q -f -L atlasos "$root"

mount -o compress=zstd:1 "$root" "$target"
mkdir -p "$target/boot"
mount "$boot" "$target/boot"
mkdir -p "$target/boot/efi"
mount "$esp" "$target/boot/efi"

start_s=$(date +%s)
podman run --rm --privileged --pid=host \
	--security-opt label=type:unconfined_t \
	-v /dev:/dev -v /var/lib/containers:/var/lib/containers \
	-v /usr/lib/containers/storage:/usr/lib/containers/storage \
	-v "$target:$target" \
	"$src" bootc install to-filesystem \
	--source-imgref "containers-storage:$src" \
	--target-imgref "$src" \
	--karg rootflags=compress=zstd:1 \
	"$target"
echo "bootc install took $(($(date +%s) - start_s)) s"

# bootupd's static grub.cfg sources $prefix/custom.cfg (configs.d/41_custom.cfg).
# bootc's finalize step leaves the target read-only.
mount -o remount,rw "$target/boot"
esp_uuid=$(blkid -s UUID -o value "$esp")
cat >"$target/boot/grub2/custom.cfg" <<EOF
menuentry 'Windows' --class windows {
	insmod part_gpt
	insmod fat
	search --no-floppy --fs-uuid --set=root $esp_uuid
	chainloader /EFI/Microsoft/Boot/bootmgfw.efi
}
EOF
umount -R "$target"

echo ">> After"
sfdisk -d "$disk" | tee "$state/table-after"
for p in "${others[@]}"; do md5sum "$p"; done >"$state/sums-after"
if cmp -s "$state/sums-before" "$state/sums-after"; then
	echo "PASS: ${#others[@]} existing partitions byte-identical"
else
	echo "FAIL: existing partitions changed" >&2
	diff "$state/sums-before" "$state/sums-after" >&2 || true
fi
mount -o ro "$esp" "$target"
(cd "$target" && find . -type f -exec md5sum {} + | sort -k2) >"$state/esp-after"
umount "$target"
echo "ESP changes (< before, > after):"
diff "$state/esp-before" "$state/esp-after" || true
