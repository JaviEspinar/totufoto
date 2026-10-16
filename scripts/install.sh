#!/bin/sh
# Installs (or updates) Imadive as a service on a Linux server:
#
#   curl -fsSL https://raw.githubusercontent.com/waiting4timeout/imadive/main/scripts/install.sh | sh
#
# It downloads the latest release's server build for this computer (x86-64 or ARM64), checks
# it against the release's SHA256SUMS, and starts `imadive setup`, a wizard with an answer
# ready for every question. Options after `sh -s --` go to setup, for example
# `| sh -s -- --yes --folder /srv/photos` to install with the defaults and no questions.
# IMADIVE_VERSION=0.3.3 picks a version instead of the latest. See docs/server.md.
set -eu

repo="waiting4timeout/imadive"
fail() { echo "imadive install: $*" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || fail "this installs a Linux service; see the README for the other systems"
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) fail "there is no server build for $(uname -m) (only x86-64 and ARM64); see docs/server.md to build it" ;;
esac
for tool in curl tar sha256sum; do
  command -v "$tool" > /dev/null || fail "$tool is needed (sudo apt install $tool)"
done

if [ -n "${IMADIVE_VERSION:-}" ]; then
  version="${IMADIVE_VERSION#v}"
else
  # The latest release's page address ends in its tag (no API call, so no rate limit).
  latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest") \
    || fail "couldn't reach GitHub"
  version="${latest##*/v}"
fi
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) fail "couldn't tell the latest version ($version)" ;;
esac

name="Imadive-server-$version-linux-$arch.tar.gz"
base="https://github.com/$repo/releases/download/v$version"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "Downloading Imadive $version for $arch..."
curl -fL --progress-bar -o "$tmp/$name" "$base/$name" \
  || fail "$name isn't in release $version (server builds start with 0.3.3)"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" || fail "couldn't download the checksums"
(cd "$tmp" && grep " $name\$" SHA256SUMS | sha256sum -c --quiet -) || fail "$name doesn't match its checksum; not using it"
tar -xzf "$tmp/$name" -C "$tmp"

program="$tmp/imadive-$version/imadive"
[ -x "$program" ] || fail "the archive has no program"
# The wizard asks on the terminal, also when this script itself arrives through a pipe (with
# no terminal at all, as in CI, only `--yes` works).
setup() {
  if (exec < /dev/tty) 2> /dev/null; then "$@" < /dev/tty; else "$@"; fi
}
if [ "$(id -u)" -eq 0 ]; then
  setup "$program" setup "$@"
else
  echo "Setup changes system files, so it runs with sudo."
  setup sudo "$program" setup "$@"
fi
