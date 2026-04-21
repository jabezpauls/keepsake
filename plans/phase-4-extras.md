# Phase 4 — Extras (Whisper, mobile, edits, extra adapters, LLM chat, RAW develop, prints)

> Prerequisite: Phase 3 ships + acceptance criteria green. The core product is "done"; Phase 4 is polish + ecosystem completeness.

---

## Goal

Add features that make the app delightful rather than foundational. Each task here is **independently shippable** behind a feature flag — reorder freely based on user demand.

1. Video speech-to-text via Whisper.
2. Mobile companion (Tauri v2 iOS/Android) as a read-only peer.
3. Light non-destructive edits (crop, rotate, straighten, basic curves).
4. Import adapters for iMazing/iTunes backups, WhatsApp, Telegram.
5. Local LLM chat surface over the library.
6. RAW develop.
7. Physical-prints scanner (capture with camera → auto-crop/deskew).

Unlike Phases 1-3, Phase 4 has **no required order** and **no single "done" point**. Ship individual features as they stabilise.

---

## Prerequisites

- All Phase 3 acceptance criteria green.
- Stable API surface for `crates/core` (no breaking changes since Phase 3 ship).

---

## Feature flags

Each feature gates behind a Cargo feature on `crates/core` + a TS feature gate in the app:

- `whisper`
- `mobile` (affects `app/src-tauri/tauri.conf.json` — mobile targets)
- `edit`
- `adapter-imazing`
- `adapter-whatsapp`
- `adapter-telegram`
- `llm-chat`
- `raw-develop`
- `prints-scan`

The app's Settings page has a "Labs" section that lists experimental features with a toggle; disabled features are not present in the UI.

---

## Features

### 4.1 Whisper transcripts (`ml/whisper.rs`)

- Dep: `whisper-rs` (FFI to `whisper.cpp`). Model: `ggml-base.en.bin` default, `ggml-small.en.bin` optional.
- Job queue: for every video asset at ingest, enqueue `MlJob::Transcribe(asset_id)`.
- Pipeline:
  1. `ffmpeg-next` extracts audio to 16-kHz mono PCM.
  2. Whisper produces segments `[{start_ms, end_ms, text}]`.
  3. Store segments in `transcript_ct`.
  4. HMAC each segment's tokens into `asset_text` FTS5 (same blind-index scheme as Phase 3 OCR).
- UI: video detail view shows a timeline scrub with a transcript panel; clicking a segment seeks the video.
- Performance: CPU-only whisper.cpp is OK for base model; surface an est. "~5× realtime" to the user.
- Out of scope for Phase 4: translation, speaker diarization.

**Acceptance:** on a fixture of 10 short clips with known spoken words, >= 8 return the correct asset when searching for the word.

### 4.2 Mobile companion (Tauri v2 iOS/Android)

- Reuse `crates/core` and `crates/sync` — both already compile for iOS/Android targets (Tauri v2's story).
- UI: trimmed-down React code — timeline, search, people, albums, map, memories. No ingest (it's a read-only peer).
- Auth: pair with a desktop via QR on first launch. Mobile gets its own Iroh node identity + user credentials (either a separate user or a second device for an existing user).
- Offline: keeps a local cache of decrypted thumbnails in the device's secure storage (iOS Keychain / Android EncryptedFile) with a small cache budget (5 GB default).
- Live-Photo / Motion-Photo playback works natively.
- Push: when a paired peer writes to a shared namespace, the mobile app gets a silent push via APNs/FCM... **but we don't run a push server.** Phase 4's mobile app polls when foregrounded and uses background BLE/Bonjour on the same LAN. Off-LAN syncing requires the app foreground.

**Acceptance:** iOS and Android builds launch, pair with a desktop, scroll the timeline, view an album, search via CLIP (mobile runs CLIP on-device — ViT-B/32 for battery).

### 4.3 Non-destructive edits (`crates/core/src/edit/`)

- Ops supported Phase 4:
  - Crop (x, y, w, h)
  - Rotate (0, 90, 180, 270)
  - Straighten (arbitrary angle)
  - Exposure ±
  - Contrast ±
  - Saturation ±
  - Highlights / Shadows
  - White balance (temp/tint)
  - Basic curves (R,G,B,L separately)
- Ops stored as a JSON array in `edit.ops_ct` (FROZEN since Phase 1); applied in order at render time.
- Rendering pipeline: `image` crate + custom LUT routines for curves; GPU-path later.
- Export: baked-in pixel output + sidecar XMP containing the same op set (Phase 1 sidecar round-trip extends here).
- UI: edit panel opens on asset detail with a live preview; histogram; reset button; "save" commits to `edit` table.

**Acceptance:** fixture of 10 edits applied → exported → re-ingested → `edit.ops_ct` preserved; visual output matches reference (PSNR > 40 dB vs. golden-render fixture).

### 4.4 iMazing / iTunes backup adapter

- Parses iOS backup format:
  - Encrypted backups (AES-256, key derived from backup password) decrypted at ingest time with a password prompt.
  - `Manifest.db` is a SQLite file that maps actual paths to `<random-hash>` files in the backup tree.
- Walks `CameraRollDomain` + `HomeDomain/Library/Mobile Documents/com~apple~CloudDocs/`.
- Handles Live Photo pairings correctly via the Manifest.
- Optional dep: `libimobiledevice` for directly pulling from a USB-connected device — low priority, flag-gated behind `adapter-imazing-usb`.

**Acceptance:** fixture backup bundle → ingest → expected 30 assets with Live pairings + screenshots flagged.

### 4.5 WhatsApp / Telegram adapters

- **WhatsApp**: reads the "Export chat" ZIP (media + `_chat.txt`). Associates messages with media via timestamps + "attached:" markers. Records the chat name as a tag + optional album.
- **Telegram**: reads the full-JSON export from Telegram Desktop. Similar shape.
- Privacy: both adapters strip sender numbers/usernames by default unless user opts in.

**Acceptance:** one fixture export per app → ingests complete set of media, tagged with correct chat name.

### 4.6 Local LLM chat (`llm-chat` feature)

- Bundled small LLM (llama.cpp with a 3B-7B Q4 model, e.g. Phi-3-mini or Llama-3.2-3B-Instruct).
- UI: chat panel in the sidebar. Queries routed to a small agent that has access to:
  - `search_library(q: SearchQuery) -> Vec<AssetView>`
  - `describe_asset(id: AssetId) -> Description`
  - `list_people() -> Vec<Person>`
  - `timeline_for_year(year: i32) -> Summary`
- The LLM never sees raw photos — only decrypted metadata + CLIP-retrieved captions.
- Example: "Show me Ami's Tokyo food photos" → the agent parses → calls `search_library({people: [ami], text: "food", place: "Tokyo"})` → rendered in the chat as a clickable row of thumbnails.
- All inference local; no external endpoints.

**Acceptance:** on a fixture library with known people/places/things, the LLM answers 5 canned questions with correct retrieval ≥ 4/5.

### 4.7 RAW develop (`raw-develop` feature)

- Reuse `rawloader` / `libraw-sys` to demosaic.
- UI: open RAW → develop panel with exposure / contrast / white balance / lens correction.
- Export: baked JPEG/TIFF output; edit ops go into the `edit` table (shared with §4.3).
- Lens corrections: use Lensfun (bundled DB, ~30 MB).
- Color: apply ICC profile from embedded camera profile if present; otherwise sRGB.

**Acceptance:** a Canon CR3 fixture develops to a reference TIFF (PSNR > 35 dB).

### 4.8 Physical-prints scanner (`prints-scan` feature)

- iOS/Android camera integration (Phase 4 mobile-only).
- Capture flow: point camera at a physical print; four-point auto-detect with edges / document-corner model; deskew; color-correct; de-flash; save as a new asset with `source` named "Scanned prints".
- ML: a small document-detector model (e.g. MLKit document scanner's equivalent, or the open-source `deep-document-scanner` ONNX).
- EXIF: synthesise `DateTimeOriginal` from user input ("when was this taken?"); GPS optional.

**Acceptance:** scan 10 physical photos; all are oriented correctly; corners detected within 5px accuracy on the fixture test.

---

## Shared tasks across Phase 4

1. **Feature-flag plumbing**: ensure every feature is guarded such that `cargo build --workspace --no-default-features` produces an app identical to Phase 3.
2. **Docs**: each feature adds a section to a new `docs/features/<feature>.md` user-facing doc.
3. **Settings > Labs**: list each Phase 4 feature with a toggle.
4. **Model downloads**: extend `scripts/download_models.sh` with optional model fetches; mandatory models stay in Phase 2/3 set.
5. **CI**: each feature flag has its own matrix job running its feature tests.

---

## File-by-file new artifacts (non-exhaustive)

| Path | Feature | Notes |
|---|---|---|
| `crates/core/src/ml/whisper.rs` | whisper | |
| `crates/core/src/edit/mod.rs` | edit | op pipeline |
| `crates/core/src/edit/ops/*.rs` | edit | per-op modules |
| `crates/core/src/ingest/adapters/imazing.rs` | adapter-imazing | |
| `crates/core/src/ingest/adapters/whatsapp.rs` | adapter-whatsapp | |
| `crates/core/src/ingest/adapters/telegram.rs` | adapter-telegram | |
| `crates/llm/` (new crate) | llm-chat | tool-calling agent over llama.cpp |
| `crates/core/src/raw/develop.rs` | raw-develop | |
| `crates/core/src/media/doc_scan.rs` | prints-scan | |
| `app/src-tauri/src/mobile/` | mobile | |
| `app/src/ui/edit/*` | edit | |
| `app/src/ui/chat/*` | llm-chat | |
| `app/src/ui/scan/*` | prints-scan | |
| `docs/features/*.md` | — | user docs |

---

## Execution

- No prescribed order.
- A single agent can take a single feature end-to-end (code + tests + UI + docs + feature flag + CI job).
- Multiple agents can work in parallel across features.

---

## Acceptance criteria (per feature; shipped individually)

Each feature ships when:

1. Feature-flag build green: `cargo build --features <feature>`.
2. Unit + integration tests for the feature pass.
3. Docs live under `docs/features/<feature>.md`.
4. Settings > Labs toggle wired.
5. All Phase 1/2/3 acceptance criteria remain green with the feature enabled (no regressions).
6. No feature leaks plaintext or breaks the crypto invariants from `architecture.md` §2-§6.

---

## Out of scope for Phase 4

- Web version (browser as a peer). Plausible future Phase 5.
- Cloud-hosted public relay beyond per-user Iroh relays.
- Multi-party video editing / timeline editor.
- Facial morph / deepfake defences.
- Encrypted search over substrings / prefixes (would require an ORAM or encrypted-trie scheme — large project on its own).

---

## Notes for maintainers

- If any Phase 4 feature ends up needing a schema change, it must be an **additive** migration bump — follow the rule from `architecture.md` §4.4. Never alter existing columns.
- Whisper and LLM-chat pull in the largest runtime dependencies; keep them strictly feature-gated so users who don't want them get a lean binary.
- Mobile builds increase the test matrix a lot — consider running mobile CI only on tagged release branches.
