#!/usr/bin/env bash
# Moves packages to exact Fedora builds, up or down:
#
#   pin-builds.sh NAME-VERSION-RELEASE...
#
# For each source package named, every installed package built from it
# (qt6-qtbase-gui and qt6-qtbase-devel for qt6-qtbase) goes to that build.
# The RPMs come from Koji, which keeps every build, as signed by the Fedora
# key the system already trusts: the updates repository holds only the newest
# build, and its archive misses some packages.
set -euo pipefail
[ $# -gt 0 ] || { echo "usage: $0 NAME-VERSION-RELEASE..." >&2; exit 2; }

# Koji keeps a signed copy under the key's short ID; this is the one that
# signed the installed packages.
key=$(rpm -q --qf '%{RSAHEADER:pgpsig}' rpm | sed -n 's/.*Key ID \([0-9a-f]*\).*/\1/p')
key=${key: -8}
[ -n "$key" ] || { echo "pin-builds.sh: can't tell which key signed the installed packages" >&2; exit 1; }

dir=$(mktemp -d)
for nvr; do
	src=${nvr%-*-*}
	vr=${nvr#"$src"-}
	ver=${vr%-*}
	rel=${vr#*-}
	pkgs=$(rpm -qa --qf '%{NAME} %{ARCH} %{SOURCERPM}\n' |
		awk -v s="$src" '{ n = $3; sub(/-[^-]+-[^-]+\.src\.rpm$/, "", n) } n == s { print $1, $2 }')
	[ -n "$pkgs" ] || { echo "pin-builds.sh: nothing installed is built from $src" >&2; exit 1; }
	while read -r name arch; do
		f=$name-$ver-$rel.$arch.rpm
		echo "$f"
		curl -fsSL --retry 3 --proto '=https' --tlsv1.2 -o "$dir/$f" \
			"https://kojipkgs.fedoraproject.org/packages/$src/$ver/$rel/data/signed/$key/$arch/$f"
	done <<<"$pkgs"
done
# Nothing goes in unless every package carries a valid Fedora signature
# (rpmkeys passes a package with no signature at all, so read what it says).
files=("$dir"/*.rpm)
checked=$(rpmkeys --checksig "${files[@]}" 2>&1) || true
ok=$(grep -c ': digests signatures OK$' <<<"$checked" || true)
[ "$ok" = "${#files[@]}" ] || {
	echo "pin-builds.sh: not all signed by a trusted key:" >&2
	echo "$checked" >&2
	exit 1
}
dnf -y -q install --allowerasing "${files[@]}"
rm -rf "$dir"
