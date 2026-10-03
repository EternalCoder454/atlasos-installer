#!/usr/bin/env bash
# Builds the AtlasOS live ISO from a published image, without root.
#
#   iso/make-iso.sh [atlasos|atlasos-nvidia] [tag]
#
# Output: build/<name>.iso. The image is pinned by the digest its tag points
# to now; the ISO embeds that exact image, so an installed system's first
# update downloads only the layers that changed since.
set -euo pipefail

name=${1:-atlasos}
tag=${2:-stable}
repo=ghcr.io/eternalcoder454/$name
case $name in
atlasos) label=ATLASOS ;;
atlasos-nvidia) label=ATLASOS-NV ;;
*) echo "usage: $0 [atlasos|atlasos-nvidia] [tag]" >&2; exit 2 ;;
esac

cd "$(dirname "$0")/.."
mkdir -p build/cache build/iso-work

digest=$(skopeo inspect --format '{{.Digest}}' "docker://$repo:$tag")
short=${digest#sha256:}
short=${short:0:8}
echo ">> $repo:$tag is $digest"

podman pull -q "$repo@$digest" >/dev/null
oci=build/cache/$name-$short
[ -f "$oci/index.json" ] || {
	echo ">> Saving the published blobs to $oci"
	skopeo copy --preserve-digests --quiet "docker://$repo@$digest" "oci:$oci:latest"
}

echo ">> Building the live image"
podman build -q --build-arg BASE_IMAGE="$repo@$digest" \
	-f live/Containerfile -t "localhost/$name-live:$short" . >/dev/null

podman build -q -f iso/Containerfile.builder -t localhost/atlas-iso-builder iso >/dev/null

podman run --rm --privileged --security-opt label=disable \
	--mount type=image,src="localhost/$name-live:$short",dst=/rootfs \
	-v "$PWD/$oci:/payload-oci:ro" \
	-v "$PWD/build/iso-work:/work" \
	-v "$PWD/build:/out" \
	-v "$PWD/iso/build-iso.sh:/build-iso.sh:ro" \
	-e ISO_LABEL="$label" -e ISO_NAME="$name.iso" -e PAYLOAD_REF="$repo:$tag" \
	localhost/atlas-iso-builder /build-iso.sh
echo "$repo@$digest" >"build/$name.iso.image"
echo ">> Wrote build/$name.iso (installs $repo@$digest)"
