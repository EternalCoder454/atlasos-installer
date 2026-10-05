#!/usr/bin/env bash
# Times one install in the live VM, phase by phase. Runs as root in the live
# session (vm.py push it, then vm.py exec it). Erases <disk>.
#
#   profile-install.sh <disk id, such as vda> [out dir]
#
# Calls the helper's Install over D-Bus (erase, no encryption) and samples
# once a second until it returns: every disk's read and write counters and
# cache flushes (count, ms), CPU time per command name (kworkers summed),
# btrfs's transaction commit time, I/O pressure and dirty memory, and the
# install log's length, so each of its lines gets a time (in install.log,
# written at the end). Then copy the out dir off with vm.py exec.
set -euo pipefail

disk=${1:?usage: profile-install.sh <disk id> [out dir]}
out=${2:-/run/atlas-profile}
[[ $disk =~ ^[a-z0-9]+$ ]] || {
	echo "profile-install.sh: bad disk id: $disk" >&2
	exit 2
}
rm -rf "$out"
mkdir -p "$out"
log=/run/atlas-installer/install.log
helper=(net.eterneon.atlas.InstallerHelper /net/eterneon/atlas/InstallerHelper
	net.eterneon.atlas.InstallerHelper1)
up() { cut -d' ' -f1 /proc/uptime; }

disks=$(busctl --json=short call "${helper[@]}" ListDisks | python3 -c \
	'import json,sys; print(json.loads(sys.stdin.read())["data"][0])')
fp=$(DISK=$disk python3 -c '
import json, os, sys
for d in json.loads(sys.stdin.read())["disks"]:
    if d["id"] == os.environ["DISK"]:
        print(d["fingerprint"])' <<<"$disks")
[ -n "$fp" ] || {
	echo "profile-install.sh: $disk is not offered" >&2
	exit 1
}

sample() {
	local t m last=
	set +e # a missed sample is no reason to stop sampling
	while :; do
		t=$(up)
		awk -v t="$t" '$3 !~ /^(ram|zram)/ && ($6 || $10) { print t, $3, $6, $10, $18, $19 }' \
			/proc/diskstats >>"$out/disk.tsv"
		# comm can hold spaces: take it between the first ( and the last ).
		for f in /proc/[0-9]*/stat; do
			cat "$f" 2>/dev/null
			echo
		done | awk -v t="$t" '
			NF {
				c = substr($0, index($0, "(") + 1); c = substr(c, 1, match(c, /\) [A-Za-z] /) - 1)
				r = substr($0, match($0, /\) [A-Za-z] /) + 2); split(r, f, " ")
				if (c ~ /^kworker/) c = "kworker*"
				cpu[c] += f[12] + f[13]
			}
			END { for (c in cpu) if (cpu[c]) print t "\t" c "\t" cpu[c] }' >>"$out/cpu.tsv"
		for s in /sys/fs/btrfs/*/commit_stats; do
			[ -r "$s" ] && echo "$t ${s#/sys/fs/btrfs/} $(tr '\n' ' ' <"$s")"
		done >>"$out/btrfs.txt"
		echo "$t io $(head -1 /proc/pressure/io) cpu $(head -1 /proc/pressure/cpu)" >>"$out/pressure.txt"
		echo "$t $(awk '/^(Dirty|Writeback):/ { printf "%s %s ", $1, $2 }' /proc/meminfo)" >>"$out/mem.txt"
		echo "$t $(wc -l 2>/dev/null <"$log" || echo 0)" >>"$out/loglines.txt"
		# the target root's mount options, each time they change
		m=$(awk '$2 == "/run/atlas-target" { print $4 }' /proc/mounts)
		[ "$m" = "$last" ] || echo "$t ${m:-unmounted}" >>"$out/mounts.txt"
		last=$m
		sleep 1
	done
}

sample &
sampler=$!
trap 'kill $sampler 2>/dev/null || true' EXIT

echo "$(up) start $disk" >"$out/phases.txt"
set +e
# Newer helpers take the apps to add at the first start (none here).
sig=$(busctl introspect "${helper[@]::2}" "${helper[2]}" | awk '$1 == ".Install" { print $3 }')
apps=()
[ "$sig" = ssssssssas ] && apps=(0)
busctl --timeout=7200 call "${helper[@]}" Install "$sig" \
	"$disk" "$fp" erase en_US.UTF-8 us "" none "" "${apps[@]}" >"$out/result.txt" 2>&1
code=$?
set -e
echo "$(up) end exit $code" >>"$out/phases.txt"
sleep 2
kill $sampler 2>/dev/null || true
# Each log line gets the time of the first sample that counted it.
awk 'NR == FNR { while (n < $2) t[++n] = $1; next } { print (FNR in t ? t[FNR] : "end"), $0 }' \
	"$out/loglines.txt" "$log" >"$out/install.log"
cat "$out/result.txt"
echo "profile in $out (exit $code)"
exit "$code"
