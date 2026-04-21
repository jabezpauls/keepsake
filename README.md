# Media Vault

Local-first, end-to-end encrypted, peer-to-peer personal media management. Ingest messy real-world backup dumps (iPhone folders, Google Takeout, generic directories) into a deduplicated encrypted library with per-album passwords and a hidden vault.

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

- `crates/core/` — mv-core library (crypto, CAS, DB, media probe, ingest).
- `crates/sync/` — mv-sync stub for Phase 3 peer networking.
- `app/src-tauri/` — Tauri v2 shell + IPC command handlers.
- `app/src/` — React UI + generated TS bindings under `src/bindings/`.
- `plans/` — the sacred phase docs; architecture.md is frozen per §2 onward.
- `scripts/codegen_ts.sh` — regenerates TS bindings from Rust via ts-rs.
- `scripts/make_fixtures.py` — dumps the synthetic iPhone fixture tree.

## Status

Phase 1 foundation — shipped. See [`plans/phase-1-foundation.md`](./plans/phase-1-foundation.md) §9 for the acceptance matrix.
