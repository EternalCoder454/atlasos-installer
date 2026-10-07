#!/usr/bin/env bash
# Gives an atlasinst-* VM two simulated Wi-Fi radios (mac80211_hwsim) and a
# WPA2 access point on the second one, wlan1: SSID AtlasTest, password
# atlastest123. Test VMs only.
#
#   tests/vm/wifi-ap.sh <vm>          the live session, through the guest agent
#   tests/vm/wifi-ap.sh <vm> ssh      an installed system, through vm.py ssh
#
# In the live session it also disconnects the wired network, or the UI skips
# its Wi-Fi page. Restart the UI afterwards so it looks again:
# vm.py exec <vm> "pkill -f '^/usr/bin/telamon-installer'".
#
# Fedora ships mac80211_hwsim in kernel-modules-internal, which Telamon OS
# doesn't install, so it comes from that package for the guest's kernel,
# downloaded in a fedora:44 container and kept in build/hwsim/.
set -euo pipefail

usage() {
	echo "usage: $0 <vm> [ssh]" >&2
	exit 2
}
vm=${1:-}
mode=${2:-agent}
[ -n "$vm" ] || usage
case $mode in agent | ssh) ;; *) usage ;; esac
cd "$(dirname "$0")/../.."
V="python3 tests/vm/vm.py"

run() {
	if [ "$mode" = ssh ]; then $V ssh "$vm" "$1"; else $V exec "$vm" "$1" --timeout 120; fi
}

kver=$(run 'uname -r' | tr -d '\r\n')
ko=build/hwsim/$kver/mac80211_hwsim.ko.xz
if [ ! -s "$ko" ]; then
	dir=build/hwsim/$kver
	mkdir -p "$dir"
	if ! podman run --rm --security-opt label=disable -v "$PWD/$dir":/out -w /out \
		-v atlas-dnf:/var/cache/libdnf5 fedora:44 \
		dnf -q download "kernel-modules-internal-$kver"; then
		echo "kernel-modules-internal-$kver is not in the Fedora repos (any more?). Get it from koji.fedoraproject.org into $dir/." >&2
		exit 1
	fi
	(cd "$dir" && rpm2cpio kernel-modules-internal-*.rpm | cpio -id --quiet '*/mac80211_hwsim.ko.xz' &&
		mv "$(find lib -name mac80211_hwsim.ko.xz | head -1)" . &&
		rm -rf lib kernel-modules-internal-*.rpm)
fi

if [ "$mode" = ssh ]; then
	$V ssh "$vm" 'cat >/tmp/mac80211_hwsim.ko.xz' <"$ko"
else
	$V push "$vm" "$ko" /tmp/mac80211_hwsim.ko.xz >/dev/null
fi

run "$(
	cat <<'EOF'
set -e
if ! lsmod | grep -q '^mac80211_hwsim '; then
	modprobe mac80211
	insmod /tmp/mac80211_hwsim.ko.xz radios=2
fi
# wlan1 is listed before NetworkManager can use it ("unavailable"), and an
# activation then fails with "No suitable device found".
ready() { nmcli -t -f DEVICE,STATE dev | grep -Eq '^wlan1:(disconnected|connected)'; }
for _ in $(seq 1 30); do
	ready && break
	sleep 0.5
done
ready || { echo "NetworkManager can't use wlan1 yet" >&2; exit 1; }
nmcli -t -f NAME con show | grep -qx hwsim-ap ||
	nmcli con add type wifi ifname wlan1 con-name hwsim-ap autoconnect no ssid AtlasTest \
		802-11-wireless.mode ap 802-11-wireless.band bg ipv4.method shared ipv6.method disabled \
		wifi-sec.key-mgmt wpa-psk wifi-sec.psk atlastest123 >/dev/null
nmcli con up hwsim-ap >/dev/null
EOF
)"

if [ "$mode" != ssh ]; then
	run "$(
		cat <<'EOF'
for d in $(nmcli -t -f DEVICE,TYPE dev | awk -F: '$2 == "ethernet" {print $1}'); do
	nmcli dev set "$d" autoconnect no
	nmcli dev disconnect "$d" >/dev/null
done
EOF
	)"
fi
run 'nmcli -t -f DEVICE,TYPE,STATE,CONNECTION dev | grep -E "wifi|ethernet" || true'
