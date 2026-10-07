#!/bin/bash
# Start-up, memory and CPU of the demo UI, headless. Runs inside the dev
# container, on Xvfb with a private bus, like screenshot.sh:
#   OUT=build/bench.json app/dev.sh app/tools/bench.sh [RUNS]
# (BIN=path measures another binary. Set OUT: xvfb-run loses a pipe's stdout.)
# Prints one JSON object (medians of RUNS runs, default 5):
#   startup_ms      from exec to the window being mapped
#   rss_kb, pss_kb  after 3 s on the Welcome page
#   idle_cpu_pct    CPU over 10 s at rest (from 11 s on) on the Welcome page, as a
#                   share of one core
#   nav_cpu_ms      CPU for four page changes (Welcome to Disk), one second each
#   peak_rss_kb     the high-water mark after those
set -euo pipefail
runs=${1:-5}
bin=${BIN:-build/app/telamon-installer}

if [ "${2:-}" != --inner ]; then
	tmp=$(mktemp -d)
	trap 'rm -rf "$tmp"' EXIT
	mkdir -p "$tmp/config" "$tmp/data" "$tmp/cache" "$tmp/runtime"
	chmod 700 "$tmp/runtime"
	cp "$(dirname "$0")/schemes/AtlasOSLight.colors" "$tmp/config/kdeglobals"
	exec env XDG_CONFIG_HOME="$tmp/config" XDG_DATA_HOME="$tmp/data" \
		XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime" \
		QT_QPA_PLATFORM=xcb BIN="$bin" TELAMON_INSTALLER_DEMO=1 \
		timeout -k 10 600 dbus-run-session -- xvfb-run -a -s "-screen 0 1600x1000x24" \
		"$0" "$runs" --inner
fi

cpu_ns() { cat /proc/"$1"/task/*/schedstat | awk '{n += $1} END {printf "%.0f", n}'; }
median() { sort -n | awk '{a[NR] = $1} END {print a[int((NR + 1) / 2)]}'; }

starts=() rss=() pss=() idle=() nav=() peak=()
for _ in $(seq 1 "$runs"); do
	t0=$(date +%s%N)
	"$bin" >/dev/null 2>&1 &
	pid=$!
	win=
	while [ -z "$win" ]; do
		win=$(xdotool search --onlyvisible --pid "$pid" 2>/dev/null | head -n1 || true)
		[ -n "$win" ] && break
		kill -0 "$pid" 2>/dev/null || { echo "bench: the app exited" >&2; exit 1; }
		sleep 0.005
	done
	t1=$(date +%s%N)
	starts+=($(((t1 - t0) / 1000000)))
	xdotool windowfocus "$win"
	sleep 3
	v=$(awk '/^Rss:/ {print $2}' /proc/"$pid"/smaps_rollup); rss+=("$v")
	v=$(awk '/^Pss:/ {print $2}' /proc/"$pid"/smaps_rollup); pss+=("$v")
	# The first 10 s count a blinking cursor; measure from 11 s on, at rest.
	sleep 8
	c0=$(cpu_ns "$pid"); s0=$(date +%s%N)
	sleep 10
	c1=$(cpu_ns "$pid"); s1=$(date +%s%N)
	v=$(awk -v a="$c0" -v b="$c1" -v s="$s0" -v e="$s1" 'BEGIN {printf "%.2f", (b - a) / (e - s) * 100}'); idle+=("$v")
	# Four times Continue (Welcome, Keyboard, Wi-Fi, Apps), at the window's
	# default size (1152 x 756 here at 1x).
	geo=$(xdotool getwindowgeometry --shell "$win")
	W=$(sed -n 's/^WIDTH=//p' <<<"$geo"); H=$(sed -n 's/^HEIGHT=//p' <<<"$geo")
	c0=$(cpu_ns "$pid")
	for _ in 1 2 3 4; do
		xdotool mousemove --window "$win" $((W - 67)) $((H - 32)) click 1
		sleep 1
	done
	c1=$(cpu_ns "$pid")
	nav+=($(((c1 - c0) / 1000000)))
	v=$(awk '/^VmHWM:/ {print $2}' /proc/"$pid"/status); peak+=("$v")
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
done
{ printf '{"startup_ms": %s, "rss_kb": %s, "pss_kb": %s, "idle_cpu_pct": %s, "nav_cpu_ms": %s, "peak_rss_kb": %s, "startups_ms": [%s]}\n' \
	"$(printf '%s\n' "${starts[@]}" | median)" "$(printf '%s\n' "${rss[@]}" | median)" \
	"$(printf '%s\n' "${pss[@]}" | median)" "$(printf '%s\n' "${idle[@]}" | median)" \
	"$(printf '%s\n' "${nav[@]}" | median)" "$(printf '%s\n' "${peak[@]}" | median)" \
	"$(IFS=,; echo "${starts[*]}")"; } >"${OUT:-/dev/stdout}"
