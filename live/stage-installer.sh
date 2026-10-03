#!/usr/bin/env bash
# Builds the installer (UI and helper) and stages its files for the live image
# in build/live-installer/root, plus the Qt, Kirigami and glibc versions it
# was built against, which live/build.sh checks against the image. Runs in
# the UI's build container, started by iso/make-iso.sh:
#
#   app/dev.sh live/stage-installer.sh
set -euo pipefail
cd /src

versions=$(rpm -q --qf '%{NAME} %{VERSION}\n' qt6-qtbase qt6-qtdeclarative kf6-kirigami glibc)

# Its own build folder, started afresh when the container's versions change:
# an incremental build can keep objects compiled against the old Qt.
build=build/app-live
[ "$(cat "$build/built-with" 2>/dev/null)" = "$versions" ] || rm -rf "$build"
cmake -S app -B "$build" -G Ninja -DCMAKE_BUILD_TYPE=Release >/dev/null
echo "$versions" >"$build/built-with"
cmake --build "$build"
cargo build --release --locked -p atlas-installer-helper

out=build/live-installer
rm -rf "$out"
DESTDIR="$PWD/$out/root" cmake --install "$build" >/dev/null
root=$out/root/usr
install -Dm755 "$CARGO_TARGET_DIR/release/atlas-installer-helper" "$root/libexec/atlas-installer-helper"
install -Dm644 -t "$root/share/dbus-1/system.d" helper/data/dbus-1/system.d/*
install -Dm644 -t "$root/share/dbus-1/system-services" helper/data/dbus-1/system-services/*
install -Dm644 -t "$root/share/polkit-1/actions" helper/data/polkit-1/actions/*
install -Dm644 -t "$root/lib/systemd/system" helper/data/systemd/*

echo "$versions" >"$out/built-with"
