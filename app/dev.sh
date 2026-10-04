#!/usr/bin/env bash
# Runs a command in the installer UI's build container, with the repo at /src.
#
#   app/dev.sh                         configure and build app/ (build/app)
#   app/dev.sh <command...>            run a command in /src
#
# Caches live in named podman volumes. The container has no system bus, so
# the app can only run in demo mode there (ATLAS_INSTALLER_DEMO).
#
# The UI builds against the installed Atlas.Ui (atlas-framework). Here it is
# built from the atlas-framework checkout at $ATLAS_FRAMEWORK_SRC (default
# ../Atlas Framework, next to this repository) into the container's /usr.
set -euo pipefail
cd "$(dirname "$0")/.."
# ATLAS_DEV_IMAGE names another image to run in, used as it is: iso/make-iso.sh
# uses one pinned to the Qt, Kirigami and glibc of the image it builds an ISO
# of, with that image's Atlas.Ui.
image=${ATLAS_DEV_IMAGE:-}
if [ -n "$image" ]; then
	podman image exists "$image" || { echo "dev.sh: no image $image" >&2; exit 1; }
else
	framework=${ATLAS_FRAMEWORK_SRC:-../Atlas Framework}
	[ -f "$framework/ui/CMakeLists.txt" ] || {
		echo "dev.sh: no atlas-framework checkout at '$framework' (set ATLAS_FRAMEWORK_SRC)" >&2
		exit 1
	}
	podman image exists localhost/atlas-installer-dev ||
		podman build -q -t localhost/atlas-installer-dev -f app/Containerfile.dev app >/dev/null
	# Cached: rebuilt only when the framework's source changes.
	image=localhost/atlas-installer-dev:atlas-ui
	podman build -q -t "$image" --build-context atlas-framework="$framework" \
		-f app/Containerfile.atlas-ui app >/dev/null
fi
if [ $# -eq 0 ]; then
	set -- bash -c 'cmake -S app -B build/app -G Ninja -DCMAKE_BUILD_TYPE=Release >/dev/null && cmake --build build/app'
fi
# Never :Z, which relabels build/ and breaks libvirt's access to the VM disks.
exec podman run --rm --security-opt label=disable -v "$PWD":/src -w /src \
	-v atlas-cargo:/root/.cargo/registry -v atlas-cargo-git:/root/.cargo/git \
	-e CARGO_TARGET_DIR=/src/target/container "$image" "$@"
