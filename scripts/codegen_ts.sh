#!/usr/bin/env bash
# Regenerate TypeScript bindings from Rust.
#
# ts-rs emits a test per `#[derive(TS)]` struct; those tests write the files
# in `app/src/bindings/` when run. `cargo test -p mv-app --lib` is the single
# trigger.

set -euo pipefail
cd "$(dirname "$0")/.."
cargo test -p mv-app --lib -- --skip _ >/dev/null 2>&1 || cargo test -p mv-app --lib
echo "TS bindings written to app/src/bindings/"
