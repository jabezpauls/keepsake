#!/usr/bin/env bash
# Download + SHA-256 verify ONNX model files for the Phase-2 ML pipeline.
#
# Usage: ./scripts/download_models.sh [DEST]
# Default DEST is ./models/ relative to repo root.
#
# ## BYO URLs
#
# The Media Vault repo does not redistribute model weights. Instead, URLs
# come from environment variables, and we pin only the SHA-256 of each
# expected file here + in `crates/core/src/ml/manifest.rs`:
#
#   MV_MODEL_URL_CLIP_VISUAL     OpenCLIP ViT-L/14 visual ONNX export.
#   MV_MODEL_URL_CLIP_TEXTUAL    OpenCLIP ViT-L/14 textual ONNX export.
#   MV_MODEL_URL_CLIP_TOKENIZER  clip_tokenizer.json (HuggingFace format).
#   MV_MODEL_URL_SCRFD           InsightFace scrfd_10g_bnkps.onnx.
#   MV_MODEL_URL_ARCFACE         InsightFace ArcFace R100 ONNX.
#
# Paste the URLs you've sourced (e.g. from OpenCLIP / InsightFace release
# pages) as env exports, then run this script. The SHA-256 below is the
# ground truth; if a URL's file doesn't match, the download aborts.
#
# ## Updating a pin
#
# Replacing a weight requires three synchronised changes:
#   1. Bump SHA-256 here.
#   2. Bump SHA-256 in `crates/core/src/ml/manifest.rs`.
#   3. Re-run Tier-B model-gated tests to confirm the new weight behaves.
#
# The placeholder checksum (64 zeroes) fails closed: any real file trips
# `Error::MlModelChecksum` on load until the pin is updated intentionally.
set -euo pipefail

DEST="${1:-$(cd "$(dirname "$0")/.." && pwd)/models}"
mkdir -p "$DEST"
cd "$DEST"

# Each entry: env_var|filename|sha256
# Keep in lock-step with the MODELS table in crates/core/src/ml/manifest.rs.
MODELS=(
  "MV_MODEL_URL_CLIP_VISUAL|clip_visual.onnx|2b02d572f59c509f4b97b9c54a868453cca1a652cd5d60e1d51d0052f055cb8c"
  "MV_MODEL_URL_CLIP_TEXTUAL|clip_textual.onnx|9fbe72ea8d36c2effaccedcf7249e3729ad0d9b4af6604b433ecdd0105663c9c"
  "MV_MODEL_URL_CLIP_TOKENIZER|clip_tokenizer.json|6d9109cc838977f3ca94a379eec36aecc7c807e1785cd729660ca2fc0171fb35"
  "MV_MODEL_URL_SCRFD|scrfd.onnx|5838f7fe053675b1c7a08b633df49e7af5495cee0493c7dcf6697200b85b5b91"
  "MV_MODEL_URL_ARCFACE|arcface.onnx|4c06341c33c2ca1f86781dab0e829f88ad5b64be9fba56e56bc9ebdefc619e43"
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

missing_urls=()
for entry in "${MODELS[@]}"; do
  IFS='|' read -r env_var name sha <<<"$entry"
  url_value="${!env_var:-}"
  if [[ -f "$name" ]]; then
    if verify_sha "$name" "$sha" >/dev/null 2>&1; then
      echo "ok    $name"
      continue
    fi
    echo "stale $name (checksum mismatch); redownloading"
    rm -f "$name"
  fi
  if [[ -z "$url_value" ]]; then
    missing_urls+=("$env_var -> $name")
    continue
  fi
  echo "get   $name (from \$$env_var)"
  curl -fSL "$url_value" -o "$name.tmp"
  verify_sha "$name.tmp" "$sha"
  mv "$name.tmp" "$name"
done

if [[ "${#missing_urls[@]}" -gt 0 ]]; then
  cat >&2 <<MSG

Missing URLs for the following models:
MSG
  for item in "${missing_urls[@]}"; do
    echo "  - $item" >&2
  done
  cat >&2 <<'MSG'

Set the MV_MODEL_URL_* env vars (e.g. from OpenCLIP / InsightFace release
pages) and re-run. The SHA-256 manifest in the script is the source of
truth — the URL is only a delivery channel.

Until every file is present, the app runs in no-ML mode: metadata search,
pHash, near-dup, and all Phase-1 functionality still work.
MSG
  exit 1
fi

echo "models ready in $DEST"
