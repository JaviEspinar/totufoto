#!/usr/bin/env bash
# Downloads Microsoft's official ONNX Runtime CPU build into ./onnxruntime.
# The official build picks SSE/AVX/AVX2/AVX-512 kernels at runtime, so it also runs on CPUs without AVX2.
set -euo pipefail
VERSION="${ORT_VERSION:-1.28.2}"
cd "$(dirname "$0")/.."

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  pkg="onnxruntime-linux-x64-$VERSION";     lib="libonnxruntime.so" ;;
  Linux-aarch64) pkg="onnxruntime-linux-aarch64-$VERSION"; lib="libonnxruntime.so" ;;
  Darwin-arm64)  pkg="onnxruntime-osx-arm64-$VERSION";     lib="libonnxruntime.dylib" ;;
  Darwin-x86_64) pkg="onnxruntime-osx-x86_64-$VERSION";    lib="libonnxruntime.dylib" ;;
  *) echo "unsupported platform $(uname -s) $(uname -m); on Windows use scripts/fetch-onnxruntime.ps1" >&2; exit 1 ;;
esac

if [[ -f "onnxruntime/$lib" ]]; then
  echo "ONNX Runtime already present in ./onnxruntime"; exit 0
fi
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fL -o "$tmp/ort.tgz" "https://github.com/microsoft/onnxruntime/releases/download/v$VERSION/$pkg.tgz"
tar -xzf "$tmp/ort.tgz" -C "$tmp"
mkdir -p onnxruntime
# Copy the real file under the plain name the app looks for (the archive ships version symlinks).
cp -L "$tmp/$pkg/lib/$lib" "onnxruntime/$lib"
cp "$tmp/$pkg/LICENSE" onnxruntime/LICENSE
echo "ONNX Runtime $VERSION ready in ./onnxruntime"
