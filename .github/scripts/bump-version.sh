#!/usr/bin/env bash
#
# Cut a release: set the version everywhere it is recorded, regenerate the
# changelog, commit, and tag — in that order, in one commit.
#
#   .github/scripts/bump-version.sh 0.8.11
#
# It stops before pushing. Review the commit, then send it and its tag with
# `git push --follow-tags`, which is what the release workflow reacts to.
#
# Using this is optional. Tagging by hand still works: the release workflow
# regenerates the changelog from the tag and opens a pull request with it if
# the committed copy has fallen behind. The script is the tidier path because
# the changelog then lands *inside* the release commit, so the tag covers it.
set -euo pipefail

version="${1:-}"
if [ -z "$version" ]; then
    echo "usage: ${0##*/} X.Y.Z" >&2
    exit 1
fi

case "$version" in
    v*)
        echo "Pass the bare version, not the tag name: ${version#v}" >&2
        exit 1
        ;;
    [0-9]*.[0-9]*.[0-9]*) ;;
    *)
        echo "Not a version: $version" >&2
        exit 1
        ;;
esac

cd "$(git rev-parse --show-toplevel)"

tag="v$version"
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "Tag $tag already exists" >&2
    exit 1
fi

# A release commit should contain the release and nothing else. Refuse to
# build one on top of unrelated edits rather than sweeping them in.
if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "Working tree is dirty; commit or stash first" >&2
    exit 1
fi

# The three places the version is written. Anywhere else derives it:
# packaging/archlinux/PKGBUILD computes pkgver() from git describe, and every
# crate takes `version.workspace = true`.
sed -i '/^\[workspace\.package\]/,/^\[/ s/^version = ".*"/version = "'"$version"'"/' Cargo.toml
sed -i 's/^Version:.*/Version:        '"$version"'/' packaging/fedora/lian-li-linux.spec

# Rewrites only the workspace members' own entries in the lock file, which is
# the 14-line diff every previous bump commit shows. A bare `cargo update`
# would re-resolve every dependency and bury the release in unrelated churn.
cargo update --workspace --quiet

# --tag names a version that has no tag yet, so the commits since the last
# release are filed under it instead of under "Unreleased".
git cliff --config .github/cliff.toml --tag "$tag" -o CHANGELOG.md

git add Cargo.toml Cargo.lock packaging/fedora/lian-li-linux.spec CHANGELOG.md
git commit -m "bump version $version"

# Annotated, and made now rather than left for later: git-cliff dates an
# as-yet-untagged release from the clock, so a tag created on a later day
# would disagree with the date already written into the changelog.
git tag -a "$tag" -m "$tag"

echo
echo "Committed and tagged $tag. Review it, then:"
echo "    git push --follow-tags"
