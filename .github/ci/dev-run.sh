#!/usr/bin/env bash
# Runs a command in the UI's dev image for CI (ui.yml), like app/dev.sh does
# locally: the repo at /src, the caches at /cache ($RUNNER_TEMP/cache).
#
#   [DEV_NETWORK=bridge] dev-run.sh <command...>
#
# The command runs as the runner's user with no capabilities, and with no
# network unless DEV_NETWORK says so (only the step that fetches crates).
# .git and .github are read-only in it: the runner later runs code from
# there on the host. ui.yml runs a copy of this script made before any
# container started.
# ccache serves both CMake and the cc crate (cxx-qt's generated C++).
set -euo pipefail
: "${IMAGE:?}" "${GITHUB_WORKSPACE:?}" "${RUNNER_TEMP:?}"
network=${DEV_NETWORK:-none}
offline=false
[ "$network" = none ] && offline=true
mkdir -p "$RUNNER_TEMP/cache"
exec docker run --rm --user "$(id -u):$(id -g)" \
	--cap-drop=ALL --security-opt=no-new-privileges --network="$network" \
	-v "$GITHUB_WORKSPACE:/src" -w /src -v "$RUNNER_TEMP/cache:/cache" \
	-v "$GITHUB_WORKSPACE/.git:/src/.git:ro" -v "$GITHUB_WORKSPACE/.github:/src/.github:ro" \
	-e HOME=/tmp -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target \
	-e CARGO_NET_OFFLINE="$offline" -e CARGO_INCREMENTAL=0 -e CARGO_PROFILE_DEV_DEBUG=0 \
	-e CARGO_TERM_COLOR=always \
	-e CCACHE_DIR=/cache/ccache -e CCACHE_BASEDIR=/src -e CCACHE_MAXSIZE=400M \
	-e CCACHE_COMPILERCHECK=content \
	-e "CC_x86_64_unknown_linux_gnu=ccache gcc" -e "CXX_x86_64_unknown_linux_gnu=ccache g++" \
	"$IMAGE" "$@"
