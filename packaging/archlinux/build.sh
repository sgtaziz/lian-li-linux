#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
output=$(realpath -m "${1:-$root/tmp/archlinux-artifacts}")
. /etc/os-release
if [ "$ID" != arch ] || [ "$(uname -m)" != x86_64 ]; then
    echo "Build inside x86_64 Arch Linux so the package links against Arch libraries." >&2
    exit 1
fi
if [ "$(id -u)" = 0 ]; then
    echo "makepkg refuses to run as root; run this as a user with sudo." >&2
    exit 1
fi
version=$(sed -n 's/^version = "\([0-9][0-9.]*\)"$/\1/p' "$root/Cargo.toml")
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Cannot determine the workspace release version." >&2
    exit 1
fi
commit=$(git -C "$root" rev-parse HEAD)
stage=$(mktemp -d "${TMPDIR:-/tmp}/lianli-arch.XXXXXX")
mkdir -p "$output"

# The release package reuses the AUR recipe so both install the same files; only
# its identity and source differ: a fixed version built from this exact commit.
sed -e 's/^pkgname=lianli-linux-git$/pkgname=lianli-linux/' \
    -e "s/^pkgver=.*/pkgver=$version/" \
    -e '/^provides=/d' \
    -e "s/^conflicts=.*/conflicts=('lianli-linux-git')/" \
    -e "s|^source=.*|source=(\"lian-li-linux::git+file://$root#commit=$commit\")|" \
    -e '/^pkgver() {$/,/^}$/d' \
    "$root/packaging/archlinux/PKGBUILD" > "$stage/PKGBUILD"
if ! grep -qx 'pkgname=lianli-linux' "$stage/PKGBUILD" ||
    ! grep -qx "pkgver=$version" "$stage/PKGBUILD" ||
    ! grep -q "#commit=$commit\")$" "$stage/PKGBUILD" ||
    grep -q '^pkgver()' "$stage/PKGBUILD"; then
    echo "packaging/archlinux/PKGBUILD no longer matches the release rewrite." >&2
    exit 1
fi

cd "$stage"
PKGDEST="$stage/out" makepkg --syncdeps --noconfirm --cleanbuild
package="lianli-linux-$version-1-x86_64.pkg.tar.zst"
test -f "out/$package"
cp "out/$package" "$output/"
printf 'Build tree retained for inspection: %s\n' "$stage"
