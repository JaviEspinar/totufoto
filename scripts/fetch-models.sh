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
unzip -j -o "$tmp/buffalo_s.zip" det_500m.onnx w600k_mbf.onnx -d models
echo "models ready in ./models"
