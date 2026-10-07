#!/usr/bin/env bash
# Builds the AtlasOS live ISO, without root.
#
#   iso/make-iso.sh [--local] [--verify-only] [-o FILE] [atlasos|atlasos-nvidia] [tag]
#
# By default it embeds the published ghcr.io/eternalcoder454/<name>:<tag>
# (tag: stable), pinned by the digest the tag points to now, so an installed
# system's first update downloads only the layers that changed since.
# --local embeds this machine's localhost/<name>:<tag> (tag: latest) instead,
# for testing a build before it is published. Its layers match nothing on
# ghcr.io, so its first update downloads the whole image.
#
# Either way the image is stored on the ISO, and installed, as
# ghcr.io/eternalcoder454/<name>:stable: the name the helper installs from
# and the installed system tracks.
#
# A published image is only used after its cosign signature checks out against
# AtlasOS's cosign.pub (../AtlasOS/cosign.pub, or ATLASOS_COSIGN_PUB), the same
# policy the installed system updates under. --local images are not signed and
# skip the check. --verify-only stops after that check.
#
# Output: build/<name>.iso, or FILE, and beside it FILE.image naming the
# image it installs.
set -euo pipefail

usage() {
	echo "usage: $0 [--local] [--verify-only] [-o FILE] [atlasos|atlasos-nvidia] [tag]" >&2
	exit 2
}
local=0
verify_only=0
out=
while [ $# -gt 0 ]; do
	case $1 in
	--local) local=1 ;;
	--verify-only) verify_only=1 ;;
	-o)
		[ -n "${2:-}" ] || usage
		out=$(realpath -m -- "$2")
		shift
		;;
	-*) usage ;;
	*) break ;;
	esac
	shift
done
[ $# -le 2 ] || usage
if [ "$verify_only" = 1 ] && [ "$local" = 1 ]; then
	echo "--verify-only checks a published image's signature; it can't be used with --local." >&2
	exit 2
fi
name=${1:-atlasos}
repo=ghcr.io/eternalcoder454/$name
# The same chunkah AtlasOS's Justfile pins.
chunkah=quay.io/coreos/chunkah@sha256:0da1fa543fafe92468ad667d00580aea544a384198f668f1499675c241642e11
case $name in
atlasos) label=ATLASOS ;;
atlasos-nvidia) label=ATLASOS-NV ;;
*) usage ;;
esac

cd "$(dirname "$0")/.."
out=${out:-$PWD/build/$name.iso}
mkdir -p build/cache build/iso-work "$(dirname "$out")"

# Checks the cosign signature of $1@$2 against the public key, then leaves the
# image's blobs in the OCI layout $3. skopeo enforces a throwaway policy
# (sigstoreSigned, matchRepository, everything else rejected) like the
# installed system's, and a layout that already holds the blobs is not
# downloaded again. A failed check writes nothing to the layout.
verify_signature() {
	local repo=$1 digest=$2 oci=$3 pub work rc=0
	pub=${ATLASOS_COSIGN_PUB:-$PWD/../AtlasOS/cosign.pub}
	[ -r "$pub" ] || {
		echo "No cosign public key at $pub: set ATLASOS_COSIGN_PUB, or use --local." >&2
		return 1
	}
	work=$(mktemp -d build/.verify.XXXXXX)
	_verify_signature "$repo" "$digest" "$oci" "$pub" "$work" || rc=$?
	rm -rf "$work"
	return "$rc"
}

_verify_signature() {
	local repo=$1 digest=$2 oci=$3 pub=$4 work=$5 blob
	# skopeo reuses a blob already in the layout by its digest without
	# hashing it again, so a blob cut short by a killed run would pass.
	# Hash them first and drop the bad ones: skopeo then fetches them again.
	if [ -d "$oci/blobs/sha256" ]; then
		for blob in "$oci"/blobs/sha256/*; do
			[ -f "$blob" ] || continue
			[ "$(sha256sum <"$blob" | cut -d' ' -f1)" = "${blob##*/}" ] || rm -f -- "$blob"
		done
	fi
	jq -n --arg repo "$repo" --arg key "$(realpath -- "$pub")" \
		'{default: [{type: "reject"}], transports: {docker: {($repo): [{type: "sigstoreSigned", keyPaths: [$key], signedIdentity: {type: "matchRepository"}}]}}}' \
		>"$work/policy.json"
	mkdir "$work/registries.d"
	printf 'docker:\n  ghcr.io/eternalcoder454:\n    use-sigstore-attachments: true\n' >"$work/registries.d/atlasos.yaml"
	echo ">> Verifying the cosign signature of $repo@$digest"
	skopeo --policy "$work/policy.json" --registries.d "$work/registries.d" \
		copy --preserve-digests --remove-signatures --quiet "docker://$repo@$digest" "oci:$oci:latest" || {
		echo "The signature of $repo@$digest could not be verified against $pub (see above). Not building." >&2
		return 1
	}
	echo ">> Signature verified"
}

if [ "$local" = 1 ]; then
	tag=${2:-latest}
	base=localhost/$name:$tag
	id=$(podman image inspect --format '{{.Id}}' "$base") || {
		echo "No local image $base. Run 'just build' in AtlasOS first." >&2
		exit 1
	}
	short=local-${id:0:8}
	source="$base ($id)"
	echo ">> $base is $id"
	echo ">> --local: the image is not signed, signature not checked"
	# From here on the image is named by its ID, in case the tag moves.
	base=$id
	# Rechunked the way CI does before publishing (AtlasOS's `just rechunk`,
	# with the same chunkah): a local build's own layers hold overlay
	# whiteouts, which skopeo can't unpack in the rootless ISO builder, and
	# the chunked layers have none. One copy, replaced when the local image
	# changes: local builds come often, and each copy is several GB.
	oci=build/cache/$name-local
	if [ "$(cat "$oci.id" 2>/dev/null)" != "$id" ]; then
		echo ">> Rechunking $source into $oci"
		rm -rf "$oci" "$oci.id"
		# The image's config (labels, command) carries over, without the base
		# image's OSTree labels, which describe a commit this image doesn't have.
		podman run --rm --pull=missing --security-opt label=disable \
			--mount type=image,src="$id",target=/chunkah \
			-v "$PWD/build/cache:/out" \
			-e CHUNKAH_CONFIG_STR="$(podman image inspect "$id" | jq -c '.[0].Config')" \
			-e SOURCE_DATE_EPOCH="$(podman image inspect --format '{{.Created.Unix}}' "$id")" \
			"$chunkah" build --rootfs /chunkah --prune /sysroot/ \
			--max-layers 127 --compressed \
			--label ostree.commit- --label ostree.final-diffid- \
			--tag latest --output "oci:/out/$name-local"
		echo "$id" >"$oci.id"
	fi
else
	tag=${2:-stable}
	digest=$(skopeo inspect --format '{{.Digest}}' "docker://$repo:$tag")
	short=${digest#sha256:}
	short=${short:0:8}
	base=$repo@$digest
	source=$base
	echo ">> $repo:$tag is $digest"
	oci=build/cache/$name-$short
	verify_signature "$repo" "$digest" "$oci"
	[ "$verify_only" = 1 ] && exit 0
	podman pull -q "$base" >/dev/null
fi

# The installer must be built against the image's own Qt, Kirigami and glibc
# builds (live/build.sh checks). The dev container has Fedora's newest, which
# differ whenever Fedora updated one of them after the image was built, or
# the container is older than the image. Then a copy of the dev container
# pinned to the image's builds (iso/pin-builds.sh) builds it instead.
# One line per build, so a multilib (i686) copy of one doesn't count twice.
query=(rpm -q --qf '%{NAME}-%{VERSION}-%{RELEASE}\n' qt6-qtbase qt6-qtdeclarative kf6-kirigami glibc)
builds() { sort -u | tr '\n' ' '; }
want=$(podman run --rm --pull=never --entrypoint rpm "$base" "${query[@]:1}" | builds)
podman image exists localhost/atlas-installer-dev ||
	podman build -q -t localhost/atlas-installer-dev -f app/Containerfile.dev app >/dev/null
have=$(ATLAS_DEV_IMAGE=localhost/atlas-installer-dev app/dev.sh "${query[@]}" | builds)
dev=localhost/atlas-installer-dev
if [ "$have" != "$want" ]; then
	echo ">> Pinning the build container to the image's builds: $want"
	# Named by the builds and the dev container it starts from, so a rebuilt
	# dev container gets a new copy.
	from=$(podman image inspect --format '{{.Id}}' localhost/atlas-installer-dev)
	dev=localhost/atlas-installer-dev:pinned-$(echo "$from $want" | sha256sum | cut -c1-12)
	podman image exists "$dev" ||
		podman build -q -t "$dev" --build-arg PINS="$want" \
			-f iso/Containerfile.pin iso >/dev/null
	# dnf --allowerasing may have removed something rather than pin it.
	have=$(ATLAS_DEV_IMAGE=$dev app/dev.sh "${query[@]}" | builds)
	[ "$have" = "$want" ] || {
		echo "The pinned build container has $have, not $want." >&2
		exit 1
	}
fi

# The installer builds against the installed Telamon.Ui: the image's own copy
# (iso/Containerfile.telamon-ui), the one the live system runs it with.
podman run --rm --pull=never --entrypoint test "$base" -f /usr/lib64/qt6/qml/Telamon/Ui/qmldir || {
	echo "$base has no Telamon.Ui (telamon-ui): build it from an AtlasOS with atlas-framework" >&2
	exit 1
}
from=$(podman image inspect --format '{{.Id}}' "$dev")
devui=localhost/atlas-installer-dev:iso-$(echo "$from $base" | sha256sum | cut -c1-12)
podman image exists "$devui" ||
	podman build -q --pull=never -t "$devui" --build-arg DEV_IMAGE="$dev" --build-arg BASE_IMAGE="$base" \
		-f iso/Containerfile.telamon-ui iso >/dev/null

echo ">> Building the installer"
ATLAS_DEV_IMAGE=$devui app/dev.sh live/stage-installer.sh >build/stage-installer.log 2>&1 || {
	tail -30 build/stage-installer.log >&2
	exit 1
}

echo ">> Building the live image"
podman build -q --pull=never --build-arg BASE_IMAGE="$base" \
	-f live/Containerfile -t "localhost/$name-live:$short" . >/dev/null

podman build -q -f iso/Containerfile.builder -t localhost/atlas-iso-builder iso >/dev/null

podman run --rm --privileged --security-opt label=disable \
	--mount type=image,src="localhost/$name-live:$short",dst=/rootfs \
	-v "$PWD/$oci:/payload-oci:ro" \
	-v "$PWD/build/iso-work:/work" \
	-v "$(dirname "$out"):/out" \
	-v "$PWD/iso/build-iso.sh:/build-iso.sh:ro" \
	-e ISO_LABEL="$label" -e ISO_NAME="$(basename "$out")" -e PAYLOAD_REF="$repo:stable" \
	localhost/atlas-iso-builder /build-iso.sh
echo "$source" >"$out.image"
echo ">> Wrote $out (installs $source)"
