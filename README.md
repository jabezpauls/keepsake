# Media Vault

Local-first, end-to-end encrypted, peer-to-peer personal media management. Ingest messy real-world backup dumps (iPhone folders, Google Takeout, generic directories) into a deduplicated encrypted library with per-album passwords and a hidden vault.

Build instructions, frozen architecture contracts, and phase-by-phase acceptance criteria live in [`plans/`](./plans/README.md). Start there.

## Dev setup

Host packages (Ubuntu 24.04):

```
sudo apt install -y \
    libsodium-dev libheif-dev \
    libavformat-dev libavcodec-dev libavutil-dev libswscale-dev libswresample-dev \
    pkg-config build-essential

corepack enable && corepack prepare pnpm@latest --activate
cargo install tauri-cli@^2 --locked
```

## Status

Phase 1 foundation — in progress. See [`plans/phase-1-foundation.md`](./plans/phase-1-foundation.md) §9 for the acceptance criteria this phase must satisfy.
