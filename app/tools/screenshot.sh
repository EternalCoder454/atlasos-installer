#!/bin/bash
# Screenshots of the demo UI, headless. Runs inside the dev container:
#   app/dev.sh app/tools/screenshot.sh OUT.png [STEP] [FLAGS] [light|dark] [WAIT]
# STEP is a step key (welcome, keyboard, wifi, disk, review, progress,
# restart), FLAGS the demo flags (see src/demo.rs), WAIT seconds before the
# shot (default 3). Set SCALE=1.5 to match a 1.5x desktop, and ATLAS=1 for
# the AtlasOS colours and font (IBM Plex Sans) instead of Breeze's.
set -euo pipefail

out=${1:?usage: screenshot.sh OUT.png [STEP] [FLAGS] [light|dark] [WAIT]}
step=${2:-welcome}
flags=${3:-1}
theme=${4:-light}
wait=${5:-3}
bin=${BIN:-build/app/atlas-installer}

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
# ICONS names another icon theme, such as Papirus-Dark, AtlasOS's own (the
# container has only Breeze: install papirus-icon-theme in it first).
printf '\n[Icons]\nTheme=%s\n' "${ICONS:-breeze$([ "$theme" = dark ] && echo -dark)}" >>"$tmp/config/kdeglobals"

cat >"$tmp/run.sh" <<INNER
#!/bin/bash
set -e
"$bin" &
app=\$!
sleep "$wait"
w=\$(xdotool search --sync --onlyvisible --name "Install AtlasOS" | head -1)
# No window manager: focus it by hand, or Qt draws the inactive colours.
xdotool windowfocus "\$w"
sleep 0.5
import -window "\$w" "$out"
kill \$app 2>/dev/null || true
wait \$app 2>/dev/null || true
INNER
chmod +x "$tmp/run.sh"

env XDG_CONFIG_HOME="$tmp/config" XDG_DATA_HOME="$tmp/data" \
    XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime" \
    QT_QPA_PLATFORM=xcb QT_SCALE_FACTOR="${SCALE:-1}" \
    ATLAS_INSTALLER_DEMO="$flags" ATLAS_INSTALLER_DEMO_PAGE="$step" \
    dbus-run-session -- xvfb-run -a -s "-screen 0 2560x1600x24" "$tmp/run.sh"
