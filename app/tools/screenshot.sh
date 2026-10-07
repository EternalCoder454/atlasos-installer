#!/bin/bash
# Screenshots of the demo UI, headless. Runs inside the dev container:
#   app/dev.sh app/tools/screenshot.sh OUT.png [STEP] [FLAGS] [light|dark] [WAIT]
# STEP is a step key (welcome, keyboard, wifi, apps, disk, review, progress,
# restart), FLAGS the demo flags (see src/demo.rs), WAIT seconds before the
# shot (default 3). Set SCALE=1.5 to match a 1.5x desktop, and ATLAS=1 for
# the AtlasOS colours and font (IBM Plex Sans) instead of Breeze's. A run
# gives up after SHOT_TIMEOUT seconds (default 300), and after 60 s with no
# installer window. ACTIONS is a bash snippet run before the shot, to reach a
# state a start-up flag can't: click X Y (window pixels at 1x, scaled by
# SCALE), key NAME..., type TEXT, scroll TICKS, size WIDTH HEIGHT (physical pixels) and pause
# SECONDS. XVFB_NUM=<n> picks the X display number (parallel runs: xvfb-run -a
# can give two runs the same one, and one then grabs a black window).
# SCREEN=3840x2160 makes the virtual screen larger than 2560x1600.
set -euo pipefail

out=${1:?usage: screenshot.sh OUT.png [STEP] [FLAGS] [light|dark] [WAIT]}
step=${2:-welcome}
flags=${3:-1}
theme=${4:-light}
wait=${5:-3}
bin=${BIN:-build/app/telamon-installer}
limit=${SHOT_TIMEOUT:-300}
[[ $limit =~ ^[0-9]+$ ]] || {
	echo "screenshot.sh: SHOT_TIMEOUT must be a number of seconds" >&2
	exit 2
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/config" "$tmp/data" "$tmp/cache" "$tmp/runtime"
chmod 700 "$tmp/runtime"
scheme=/usr/share/color-schemes/BreezeLight.colors
[ "$theme" = dark ] && scheme=/usr/share/color-schemes/BreezeDark.colors
if [ "${ATLAS:-0}" = 1 ]; then
	scheme=$(dirname "$0")/schemes/AtlasOSLight.colors
	[ "$theme" = dark ] && scheme=$(dirname "$0")/schemes/AtlasOSDark.colors
fi
cp "$scheme" "$tmp/config/kdeglobals"
# OS_LOGO=file.svg puts a `telamon` icon in the icon theme, as the Telamon OS
# image has, so the brand panel draws that instead of its bundled mark.
if [ -n "${OS_LOGO:-}" ]; then
	mkdir -p "$tmp/data/icons/hicolor/scalable/apps"
	cp "$OS_LOGO" "$tmp/data/icons/hicolor/scalable/apps/telamon.svg"
fi
if [ "${ATLAS:-0}" = 1 ]; then
	printf '\n[General]\nfont=IBM Plex Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\nsmallestReadableFont=IBM Plex Sans,8,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n' >>"$tmp/config/kdeglobals"
	# Without Plasma's platform theme Qt takes fontconfig's default font,
	# not kdeglobals', so make that Plex too.
	mkdir -p "$tmp/config/fontconfig"
	cat >"$tmp/config/fontconfig/fonts.conf" <<'FC'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
  <alias binding="strong"><family>sans-serif</family><prefer><family>IBM Plex Sans</family></prefer></alias>
</fontconfig>
FC
fi
# ICONS names another icon theme, such as Papirus-Dark, Telamon OS's own (the
# container has only Breeze: install papirus-icon-theme in it first).
printf '\n[Icons]\nTheme=%s\n' "${ICONS:-breeze$([ "$theme" = dark ] && echo -dark)}" >>"$tmp/config/kdeglobals"

# 1.7 becomes 170, for shell arithmetic.
SCALE_PCT=$(awk -v s="${SCALE:-1}" 'BEGIN { printf "%d", s * 100 + 0.5 }')
cat >"$tmp/run.sh" <<INNER
#!/bin/bash
set -e
"$bin" &
app=\$!
sleep "$wait"
w=\$(timeout 60 xdotool search --sync --onlyvisible --name "Install Telamon OS" | head -1)
if [ -z "\$w" ]; then
	echo "screenshot.sh: no installer window after 60 s" >&2
	kill \$app 2>/dev/null || true
	exit 1
fi
# No window manager: focus it by hand, or Qt draws the inactive colours.
xdotool windowfocus "\$w"
sleep 0.5
click() { xdotool mousemove --window "\$w" \$(( \$1 * ${SCALE_PCT} / 100 )) \$(( \$2 * ${SCALE_PCT} / 100 )) click 1; sleep 0.4; }
key() { xdotool key --clearmodifiers "\$@"; sleep 0.3; }
type() { xdotool type --delay 40 -- "\$1"; sleep 0.3; }
pause() { sleep "\$1"; }
size() { xdotool windowsize "\$w" "\$1" "\$2"; sleep 1.5; }
scroll() { xdotool mousemove --window "\$w" \$(( 700 * ${SCALE_PCT} / 100 )) \$(( 400 * ${SCALE_PCT} / 100 )) click --repeat "\$1" --delay 40 5; sleep 0.4; }
${ACTIONS:-}
sleep 0.6
# A window that has not painted yet grabs as black (slow on a busy
# machine): grab again until it has.
for _ in \$(seq 1 40); do
	import -window "\$w" "$out"
	[ "\$(identify -format '%[fx:mean>0.02?1:0]' "$out")" = 1 ] && break
	sleep 0.5
done
kill \$app 2>/dev/null || true
wait \$app 2>/dev/null || true
INNER
chmod +x "$tmp/run.sh"

env XDG_CONFIG_HOME="$tmp/config" XDG_DATA_HOME="$tmp/data" \
    XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime" \
    QT_QPA_PLATFORM=xcb QT_SCALE_FACTOR="${SCALE:-1}" \
    TELAMON_INSTALLER_DEMO="$flags" TELAMON_INSTALLER_DEMO_PAGE="$step" \
    timeout -k 10 "$limit" \
    dbus-run-session -- xvfb-run ${XVFB_NUM:+-n "$XVFB_NUM"} -a -s "-screen 0 ${SCREEN:-2560x1600}x24" "$tmp/run.sh"
