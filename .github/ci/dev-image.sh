#!/usr/bin/env bash
# The UI's dev image for CI (ui.yml): app/Containerfile.dev plus
# app/Containerfile.telamon-ui, with Telamon.Ui built from atlas-framework at
# FRAMEWORK_SHA. The tag hashes everything that goes into the image, this
# script included, so a change to any of it is a new tag.
#
#   dev-image.sh name     print the image's name on GHCR
#   dev-image.sh build    build the image under that name (it never pushes)
#
# Docker builds it, not podman: the runner's podman (4.9) predates
# COPY --exclude.
set -euo pipefail
cd "$(dirname "$0")/../.."
: "${FRAMEWORK_SHA:?}" "${GITHUB_REPOSITORY_OWNER:?}" "${GITHUB_REPOSITORY:?}"
# The Dockerfile frontend, by digest: it runs inside BuildKit.
syntax=docker/dockerfile:1.27.1@sha256:4edf897a3ffa55b89f906fc8cc78afdb3f1834cc9c7083565e611a8a7d5fe99e
hash=$({
	cat app/Containerfile.dev app/Containerfile.telamon-ui .github/ci/dev-image.sh
	echo "$FRAMEWORK_SHA"
} | sha256sum | cut -c1-16)
image=ghcr.io/${GITHUB_REPOSITORY_OWNER,,}/telamon-installer-dev:$hash

case ${1:-} in
name)
	echo "$image"
	;;
build)
	framework=${RUNNER_TEMP:?}/atlas-framework
	rm -rf "$framework"
	git init -q "$framework"
	git -C "$framework" fetch -q --depth 1 https://github.com/EternalCoder454/atlas-framework "$FRAMEWORK_SHA"
	git -C "$framework" checkout -q FETCH_HEAD
	if [ "$(git -C "$framework" rev-parse HEAD)" != "$FRAMEWORK_SHA" ]; then
		echo "dev-image.sh: atlas-framework is not at $FRAMEWORK_SHA" >&2
		exit 1
	fi
	docker build --progress=plain --build-arg BUILDKIT_SYNTAX="$syntax" \
		-t localhost/telamon-installer-dev -f app/Containerfile.dev app
	docker build --progress=plain --build-arg BUILDKIT_SYNTAX="$syntax" \
		--build-context atlas-framework="$framework" \
		--label org.opencontainers.image.source="https://github.com/$GITHUB_REPOSITORY" \
		-t "$image" -f app/Containerfile.telamon-ui app
	;;
*)
	echo "usage: dev-image.sh name|build" >&2
	exit 2
	;;
esac
