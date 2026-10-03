#!/usr/bin/env bash
# Test VMs only. Run in the live session after an install: lets the host log
# in to the installed system as root over SSH with the given public key.
# The installed system enforces SELinux, which confines the guest agent
# (virt_qemu_ga_t) so far that `vm.py exec` can't run bootc or systemctl.
#
#   test-access.sh <root partition> '<ssh public key>'
set -euo pipefail

part=$1 key=$2
mnt=/run/atlas-test-access
mkdir -p "$mnt"
mount "$part" "$mnt"
trap 'umount "$mnt"' EXIT

stateroot="$mnt/ostree/deploy/default"
for dep in "$stateroot"/deploy/*.0; do
	mkdir -p "$dep/etc/systemd/system/multi-user.target.wants"
	ln -sf /usr/lib/systemd/system/sshd.service "$dep/etc/systemd/system/multi-user.target.wants/sshd.service"
	chcon -h system_u:object_r:systemd_unit_file_t:s0 "$dep/etc/systemd/system/multi-user.target.wants/sshd.service"
done

# /root is /var/roothome, which first boot creates if it isn't there yet.
home="$stateroot/var/roothome"
mkdir -p "$home/.ssh"
chmod 0700 "$home/.ssh"
echo "$key" >"$home/.ssh/authorized_keys"
chmod 0600 "$home/.ssh/authorized_keys"
chcon system_u:object_r:admin_home_t:s0 "$home"
chcon -R system_u:object_r:ssh_home_t:s0 "$home/.ssh"
echo "sshd enabled and key added in: $(echo "$stateroot"/deploy/*.0)"
