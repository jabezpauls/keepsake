# Keepsake

Local-first, end-to-end encrypted, peer-to-peer personal media management. Ingest messy real-world backup dumps (iPhone folders, Google Takeout, generic directories) into a deduplicated encrypted library with per-album passwords, a hidden vault, on-device ML search, and optional peer-to-peer album sharing — no cloud, no server.

Build instructions, frozen architecture contracts, and phase-by-phase acceptance criteria live in [`plans/`](./plans/README.md). Start there.

## Quickstart

```bash
# 1. Host packages (Ubuntu 24.04). macOS / Windows users: see
#    plans/phase-1-foundation.md §Known unknowns for notes.
sudo apt install -y \
    libsodium-dev libheif-dev \
    libavformat-dev libavcodec-dev libavutil-dev \
    libswscale-dev libswresample-dev libavfilter-dev libavdevice-dev \
    libgtk-3-dev libwebkit2gtk-4.1-dev librsvg2-dev \
    libsoup-3.0-dev libjavascriptcoregtk-4.1-dev \
    pkg-config build-essential

# 2. Toolchains. `rustup` ships the stable channel; corepack ships pnpm;
#    `cargo install tauri-cli@^2` installs the Tauri v2 CLI.
corepack enable && corepack prepare pnpm@10.33.0 --activate
cargo install tauri-cli@^2 --locked

# 3. Install JS deps.
pnpm --filter app install

# 4. Run the desktop shell (dev mode — auto-reloads on changes).
cargo tauri dev
```

The default vault lives at `$XDG_DATA_HOME/media-vault` (falls back to
`~/.local/share/media-vault`). Override with `MV_VAULT_ROOT=/some/path`.

> The on-disk directory and the `MV_*` env vars still carry the
> project's pre-rebrand name. Existing installs keep working
> unchanged; a rename migration is tracked as a follow-up.

## Checks

All commands below run from the repo root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -- --test-threads=2   # Argon2 memlimit
cargo doc --no-deps --workspace

pnpm --filter app lint
pnpm --filter app typecheck
pnpm --filter app test
pnpm --filter app build

pnpm --filter app test:e2e                   # optional; installs browsers
```

## Layout

- `crates/core/` — core library (crypto, CAS, DB, media probe, ingest, analytics, blind-indexed FTS5).
- `crates/sync/` — peer networking (iroh endpoint, blobs bridge, docs CRDT, receive loop).
- `crates/replica/` — headless ciphertext-only replica binary (`mv-replica`).
- `app/src-tauri/` — Tauri v2 shell + IPC command handlers.
- `app/src/` — React UI + generated TS bindings under `src/bindings/`.
- `plans/` — the phase docs; architecture.md is frozen per §2 onward.
- `scripts/codegen_ts.sh` — regenerates TS bindings from Rust via ts-rs.
- `scripts/replica.service` — systemd template for the offsite replica.

## Status

- **Phase 1 foundation — shipped.** Crypto envelope, CAS, SQLite schema,
  generic/iPhone/Google-Takeout ingest adapters, Tauri IPC surface,
  React UI. See [`plans/phase-1-foundation.md`](./plans/phase-1-foundation.md).
- **Phase 2 browsing — shipped.** 64-bit dhash + near-duplicate
  clustering, metadata-filter search (CLIP text rerank gated on
  `ml-models`), Tauri commands for people/search/map/near-dup/ML status,
  React surfaces for Timeline zoom (Year/Month/Day), Search, Map,
  People, Duplicates. See [`plans/phase-2-browsing.md`](./plans/phase-2-browsing.md).
- **Phase 3 peer + smart library — in progress.**
  - Pairing + same-device sharing (3.1–3.2) shipped.
  - Trip detection via DBSCAN on GPS+day clusters (D1).
  - Offline reverse geocoding for trip names (D2).
  - On-this-day memory cards (D3).
  - Smart albums with rule-compiled materialised views (D4, DDL v7).
  - Blind-indexed FTS5 search on per-asset user-authored text (D5).
  - Multi-user on one device + same-device peer listing (D6).
  - Headless offsite replica (`mv-replica`) + systemd template (D8).
  - Public share-links + viewer SPA (D7) and pets classifier (D9) — pending.

### Enabling ML (Phase 2.1+)

Face detection, face clustering, and CLIP natural-language search run
**entirely on-device** via ONNX Runtime. Nothing is uploaded. The
pipeline activates when three things are true: the `ml-models` feature
is on at build time, model weights are present on disk with matching
SHA-256, and libonnxruntime is resolvable at runtime.

```bash
# 1. Point the downloader at the weights you've sourced (OpenCLIP /
#    InsightFace / your own mirror). URLs are intentionally not pinned
#    by this repo — see scripts/download_models.sh for the list.
export MV_MODEL_URL_CLIP_VISUAL=...
export MV_MODEL_URL_CLIP_TEXTUAL=...
export MV_MODEL_URL_CLIP_TOKENIZER=...
export MV_MODEL_URL_SCRFD=...
export MV_MODEL_URL_ARCFACE=...
./scripts/download_models.sh ~/.local/share/media-vault/models

# 2. Tell ort where libonnxruntime lives (installed via apt, brew, or
#    the prebuilt bundles on https://github.com/microsoft/onnxruntime).
export ORT_DYLIB_PATH=/usr/lib/x86_64-linux-gnu/libonnxruntime.so.1

# 3. Optionally point MV_MODELS at a non-default model dir; the app
#    otherwise looks in <vault>/models/.
export MV_MODELS=~/.local/share/media-vault/models

# 4. Build with the feature flag.
cargo build --features ml-models
# or, if you have a CUDA box: --features ml-cuda
# or, if you're on macOS:     --features ml-coreml
```

Without models, Keepsake still runs with everything that doesn't
require inference — metadata search, pHash-based near-dup clustering,
trips, memories, smart albums, blind-FTS text search, peer sharing,
the full Phase-1 pipeline. The People tab renders empty and the CLIP
text search falls back to date-ordered results. The top-nav badge
shows the current state at a glance ("ML off" / "ML — no weights" /
"ML Cpu · idle" / "ML Cuda · 12 queued").

## License

AGPL-3.0-or-later. See [`LICENSE`](./LICENSE).
