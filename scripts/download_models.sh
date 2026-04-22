#!/usr/bin/env bash
# Download + SHA-256 verify ONNX model files for the Phase-2 ML pipeline.
#
# Usage: ./scripts/download_models.sh [DEST]
# Default DEST is ./models/ relative to repo root.
#
# This script is intentionally idempotent and offline-safe: it re-runs without
# re-downloading files that already match their expected checksum. Mismatched
# files fail loudly.
#
# After running, build with `--features ml-models` to enable CLIP + face
# pipelines. Without this step or the feature flag, the app degrades to a
# metadata-only search and skips face detection.
set -euo pipefail

DEST="${1:-$(cd "$(dirname "$0")/.." && pwd)/models}"
mkdir -p "$DEST"
cd "$DEST"

# Each entry: url|filename|sha256
# Checksums are placeholders — replace when finalising sources. They are
# required, not optional: CI refuses to download without them.
MODELS=(
  # CLIP ViT-L/14 — OpenCLIP ONNX export. See https://github.com/mlfoundations/open_clip
  # "https://example.invalid/clip-vit-l14-visual.onnx|clip_visual.onnx|PLACEHOLDER_SHA256"
  # "https://example.invalid/clip-vit-l14-textual.onnx|clip_textual.onnx|PLACEHOLDER_SHA256"
  # SCRFD 10G BNKPS — InsightFace detection model.
  # "https://example.invalid/scrfd_10g_bnkps.onnx|scrfd.onnx|PLACEHOLDER_SHA256"
  # ArcFace R100.
  # "https://example.invalid/arcface_r100.onnx|arcface.onnx|PLACEHOLDER_SHA256"
)

verify_sha() {
  local file="$1" expected="$2"
  local actual
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$file" | awk '{print $1}')
  else
    actual=$(shasum -a 256 "$file" | awk '{print $1}')
  fi
  if [[ "$actual" != "$expected" ]]; then
    echo "CHECKSUM MISMATCH: $file" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual"   >&2
    return 1
  fi
}

if [[ ${#MODELS[@]} -eq 0 ]]; then
  cat >&2 <<'MSG'
No model entries configured. To enable on-device ML:

  1. Populate the MODELS array in scripts/download_models.sh with the
     production URLs + SHA-256 checksums for each ONNX file.
  2. Re-run this script.
  3. Rebuild with `cargo build --features ml-models`.

Until then, the app runs in "no-ML" mode: metadata search, pHash, near-dup,
and all Phase-1 functionality still work.
MSG
  exit 0
fi

for entry in "${MODELS[@]}"; do
  IFS='|' read -r url name sha <<<"$entry"
  if [[ -f "$name" ]]; then
    if verify_sha "$name" "$sha" >/dev/null 2>&1; then
      echo "ok    $name"
      continue
    fi
    echo "stale $name (checksum mismatch); redownloading"
    rm -f "$name"
  fi
  echo "get   $name"
  curl -fSL "$url" -o "$name.tmp"
  verify_sha "$name.tmp" "$sha"
  mv "$name.tmp" "$name"
done

echo "models ready in $DEST"
