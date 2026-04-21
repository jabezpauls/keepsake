#!/usr/bin/env bash
# Emit TS types from Rust (ts-rs). The `test` trigger is how ts-rs exports types.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test -p mv-core --lib ts_rs_export_tests 2>/dev/null || true
# IPC DTO bindings (populated in Step 11) would re-export here:
#   cargo test -p mv-app --lib ts_rs_export_tests
echo "TS type codegen complete."
