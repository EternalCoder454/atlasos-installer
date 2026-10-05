#!/usr/bin/env bash
# Makes build/vm/win11-base.qcow2: Windows 11 installed unattended on a 128 GB
# disk (100 MB EFI, 16 MB MSR, 60 GB C:, about 67 GB left free), for the
# dual-boot tests. Takes about 20 minutes; the VM powers off when Windows
# reaches the desktop. Test VMs use overlays of the base and never change it.
#
#   tests/vm/windows.sh [path to Windows 11 ISO]
set -euo pipefail

# Checked before anything is made. Digits only: $(( )) would evaluate
# anything else as an expression.
limit=${WIN_TIMEOUT:-5400}
[[ $limit =~ ^[0-9]+$ ]] || {
	echo "windows.sh: WIN_TIMEOUT must be a number of seconds" >&2
	exit 2
}

iso=$(realpath "${1:-$HOME/VMs/Win11_25H2_English_x64_v2.iso}")
cd "$(dirname "$0")/../.."
export LIBVIRT_DEFAULT_URI=qemu:///system
dom=atlasinst-win11-setup
base=build/vm/win11-base.qcow2
mkdir -p build/vm/win-answer

[ -s build/vm/win-password ] || (umask 077 && openssl rand -hex 8 >build/vm/win-password)
sed "s/@PASSWORD@/$(cat build/vm/win-password)/" tests/vm/windows/autounattend.xml \
	>build/vm/win-answer/autounattend.xml
xorriso -as mkisofs -quiet -V ANSWER -J -R -o build/vm/win-answer.iso build/vm/win-answer

rm -f "$base"
qemu-img create -q -f qcow2 "$base" 128G
virt-install --name "$dom" --osinfo win11 \
	--memory 8192 --vcpus 4 --cpu host-passthrough \
	--boot uefi,firmware.feature0.name=secure-boot,firmware.feature0.enabled=yes,firmware.feature1.name=enrolled-keys,firmware.feature1.enabled=yes \
	--tpm emulator,model=tpm-crb,version=2.0 \
	--disk "path=$PWD/$base,bus=sata,boot.order=1" \
	--disk "path=$iso,device=cdrom,bus=sata,readonly=on,boot.order=2" \
	--disk "path=$PWD/build/vm/win-answer.iso,device=cdrom,bus=sata,readonly=on" \
	--network none \
	--graphics spice,listen=none --video vga \
	--noautoconsole

# The Windows ISO waits for a key press to boot from the CD.
for _ in $(seq 1 10); do
	sleep 2
	virsh send-key "$dom" KEY_ENTER >/dev/null 2>&1 || true
done

# Without a product key Setup stops at its key page whatever the answer file
# says. Tab, Enter picks "I don't have a product key" (the page was up about
# 40 s after boot, 2026-10-02); Setup then runs on unattended.
sleep 40
virsh send-key "$dom" KEY_TAB >/dev/null
sleep 1
virsh send-key "$dom" KEY_ENTER >/dev/null

echo ">> Installing Windows; waiting for the VM to power off"
# About 20 minutes normally. Past WIN_TIMEOUT seconds Setup has hung: stop
# the VM and remove the half-made base, so no test runs on it.
deadline=$((SECONDS + limit))
while [ "$(virsh domstate "$dom")" != "shut off" ]; do
	if [ "$SECONDS" -ge "$deadline" ]; then
		echo "windows.sh: Setup still running after $limit s; stopping it" >&2
		virsh destroy "$dom" >/dev/null 2>&1 || true
		virsh undefine "$dom" --nvram --tpm >/dev/null 2>&1 || true
		rm -f "$base"
		exit 1
	fi
	sleep 30
done
virsh undefine "$dom" --nvram --tpm >/dev/null
echo ">> Wrote $base"
