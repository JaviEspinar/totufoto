#!/usr/bin/env bash
# Downloads Microsoft's official ONNX Runtime CPU build into ./onnxruntime.
# The official build picks SSE/AVX/AVX2/AVX-512 kernels at runtime, so it also runs on CPUs without AVX2.
set -euo pipefail
VERSION="${ORT_VERSION:-1.28.2}"
cd "$(dirname "$0")/.."

# SHA-256 of each archive of the default version (as published on the release page).
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  pkg="onnxruntime-linux-x64-$VERSION";     lib="libonnxruntime.so"
                 sum=d7209b8751b27b862b0c76332c2e20e203396edb5dab700ecf4bb485cf147415 ;;
  Linux-aarch64) pkg="onnxruntime-linux-aarch64-$VERSION"; lib="libonnxruntime.so"
                 sum=f020b3d31106cc7db03889b4a5c21e7c38ce4a09ad26119c11d1ad6d3fa0ec04 ;;
  Darwin-arm64)  pkg="onnxruntime-osx-arm64-$VERSION";     lib="libonnxruntime.dylib"
                 sum=c4fceacfc53765d0869dc9180c31ec91054d149017a99d1e80ffe28dc79596de ;;
  Darwin-x86_64)
    echo "ONNX Runtime publishes no build for Intel Macs any more. The gallery works without it," >&2
    echo "just without face recognition; or set ORT_VERSION to an older release that has one." >&2
    [[ -n "${ORT_VERSION:-}" ]] || exit 1
    pkg="onnxruntime-osx-x86_64-$VERSION"; lib="libonnxruntime.dylib" ;;
  *) echo "unsupported platform $(uname -s) $(uname -m); on Windows use scripts/fetch-onnxruntime.ps1" >&2; exit 1 ;;
esac
if [[ -n "${ORT_VERSION:-}" && "$ORT_VERSION" != "1.28.2" ]]; then
  echo "ORT_VERSION=$ORT_VERSION: no checksum known for it, so the download is not verified" >&2
  sum=""
fi

if [[ -f "onnxruntime/$lib" ]]; then
  echo "ONNX Runtime already present in ./onnxruntime"; exit 0
fi
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fL -o "$tmp/ort.tgz" "https://github.com/microsoft/onnxruntime/releases/download/v$VERSION/$pkg.tgz"
if [[ -n "${sum:-}" ]]; then
  actual=$( (command -v sha256sum >/dev/null && sha256sum "$tmp/ort.tgz" || shasum -a 256 "$tmp/ort.tgz") | cut -d' ' -f1)
  if [[ "$actual" != "$sum" ]]; then
    echo "$pkg.tgz has SHA-256 $actual, expected $sum; not using it" >&2; exit 1
  fi
fi
tar -xzf "$tmp/ort.tgz" -C "$tmp"
mkdir -p onnxruntime
# Copy the real file under the plain name the app looks for (the archive ships version symlinks).
cp -L "$tmp/$pkg/lib/$lib" "onnxruntime/$lib"
cp "$tmp/$pkg/LICENSE" onnxruntime/LICENSE
echo "ONNX Runtime $VERSION ready in ./onnxruntime"
