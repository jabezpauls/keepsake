#!/usr/bin/env bash
# Release build of the Tauri app.
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo tauri build
