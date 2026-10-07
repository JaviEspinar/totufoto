#!/usr/bin/env bash
# Checks that the README's download links point at this version's files:
#
#   scripts/check-download-links.sh 0.3.2
#
# scripts/release.sh updates them; CI and the release workflow run this so a version can't
# be released (or main left) with links to another one.
set -euo pipefail
version=${1:?usage: $0 X.Y.Z}
cd "$(dirname "$0")/.."

links=$(grep -o 'releases/download/v[^/"]*/Imadive-[^"]*' README.md || true)
expected="windows-x64.exe linux-amd64.deb linux-x86_64.AppImage"
status=0
for file in $expected; do
    if ! grep -qx "releases/download/v$version/Imadive-$version-$file" <<< "$links"; then
        echo "README.md has no download link to Imadive-$version-$file" >&2
        status=1
    fi
done
while read -r link; do
    [ -z "$link" ] && continue
    case "$link" in
        "releases/download/v$version/Imadive-$version-"*) ;;
        *) echo "README.md links to another version: $link" >&2; status=1 ;;
    esac
done <<< "$links"
exit $status
