#!/usr/bin/env bash
# Prepares a release: checks the tree and the notes, sets the version, commits and tags.
# Pushing is left to you, so there is a moment to look at the commit first.
#
#   scripts/release.sh 0.1.12
#
# See docs/releasing.md.
set -euo pipefail

version=${1:-}
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "usage: $0 X.Y.Z" >&2
    exit 1
fi
tag="v$version"
cd "$(dirname "$0")/.."

fail() { echo "$*" >&2; exit 1; }

[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || fail "not on main"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "uncommitted changes; commit or stash them first"
git rev-parse -q --verify "refs/tags/$tag" > /dev/null && fail "$tag already exists"

notes="release-notes/$tag.md"
[ -s "$notes" ] || fail "$notes is missing or empty; write it first (see docs/releasing.md)"
grep -q "^## Downloads" "$notes" || fail "$notes has no '## Downloads' section"
grep -q -- "-$version-" "$notes" || fail "$notes doesn't name the $version files"

current=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
[ "$current" != "$version" ] || fail "Cargo.toml already says $version"

# The shared version in [workspace.package]: the first `version = ` line.
CURRENT=$current VERSION=$version perl -0pi -e 's/^version = "\Q$ENV{CURRENT}\E"/version = "$ENV{VERSION}"/m' Cargo.toml
[ "$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)" = "$version" ] || fail "could not set the version in Cargo.toml"
cargo update --workspace --quiet
# The README's download links point at this version's files.
CURRENT=$current VERSION=$version perl -pi -e 's{releases/download/v\Q$ENV{CURRENT}\E/Imadive-\Q$ENV{CURRENT}\E-}{releases/download/v$ENV{VERSION}/Imadive-$ENV{VERSION}-}g' README.md
scripts/check-download-links.sh "$version" || fail "could not update the download links in README.md"

git add Cargo.toml Cargo.lock README.md "$notes"
git commit -q -m "Release $version"
git tag "$tag"

echo "Release $version committed and tagged. Check it with 'git show', then publish it:"
echo "  git push origin main $tag"
