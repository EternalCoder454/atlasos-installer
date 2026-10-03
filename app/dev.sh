#!/usr/bin/env bash
# Runs a command in the installer UI's build container, with the repo at /src.
#
#   app/dev.sh                         configure and build app/ (build/app)
#   app/dev.sh <command...>            run a command in /src
#
# Caches live in named podman volumes. The container has no system bus, so
# the app can only run in demo mode there (ATLAS_INSTALLER_DEMO).
set -euo pipefail
cd "$(dirname "$0")/.."
# ATLAS_DEV_IMAGE names another image to run in: iso/make-iso.sh uses one
# pinned to the Qt, Kirigami and glibc of the image it builds an ISO of.
image=${ATLAS_DEV_IMAGE:-localhost/atlas-installer-dev}
if ! podman image exists "$image"; then
	[ -z "${ATLAS_DEV_IMAGE:-}" ] || { echo "dev.sh: no image $image" >&2; exit 1; }
	podman build -q -t "$image" -f app/Containerfile.dev app >/dev/null
fi
if [ $# -eq 0 ]; then
	set -- bash -c 'cmake -S app -B build/app -G Ninja -DCMAKE_BUILD_TYPE=Release >/dev/null && cmake --build build/app'
fi
# Never :Z, which relabels build/ and breaks libvirt's access to the VM disks.
exec podman run --rm --security-opt label=disable -v "$PWD":/src -w /src \
	-v atlas-cargo:/root/.cargo/registry -v atlas-cargo-git:/root/.cargo/git \
	-e CARGO_TARGET_DIR=/src/target/container "$image" "$@"
