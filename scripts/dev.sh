#!/usr/bin/env bash
# Launch the Tauri app in dev mode.
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo tauri dev
