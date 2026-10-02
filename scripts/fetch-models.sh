#!/usr/bin/env bash
# Downloads the InsightFace "buffalo_s" face models (SCRFD detector + MobileFaceNet ArcFace).
# Note: InsightFace pretrained models are licensed for non-commercial research use.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p models
if [[ -f models/det_500m.onnx && -f models/w600k_mbf.onnx ]]; then
  echo "models already present"; exit 0
fi
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fL -o "$tmp/buffalo_s.zip" https://github.com/deepinsight/insightface/releases/download/v0.7/buffalo_s.zip
# The models end up inside the desktop app, so check they are the expected files.
expected=d85a87f503f691807cd8bb97128bdf7a0660326cd9cd02657127fa978bab8b5e
actual=$( (command -v sha256sum >/dev/null && sha256sum "$tmp/buffalo_s.zip" || shasum -a 256 "$tmp/buffalo_s.zip") | cut -d' ' -f1)
if [[ "$actual" != "$expected" ]]; then
  echo "buffalo_s.zip has SHA-256 $actual, expected $expected; not using it" >&2; exit 1
fi
unzip -j -o "$tmp/buffalo_s.zip" det_500m.onnx w600k_mbf.onnx -d models
echo "models ready in ./models"
