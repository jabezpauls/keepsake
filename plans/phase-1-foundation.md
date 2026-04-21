# Phase 1 — Foundation ("no regret")

> **Read `architecture.md` before touching any code in this phase.** Phase 1 freezes contracts that later phases must respect. Breaking them now = rewriting everything later.

---

## Goal

Ship a thin but real app that:

1. Locks in the **immutable foundations** — crypto envelope, CAS layout, DB schema, provenance format, peer-identity format.
2. Ingests messy real-world backup dumps (iPhone loose folders, Google Takeout, generic folders) into a deduplicated encrypted library.
3. Renders a browsable timeline.
4. Creates albums with per-album passwords and a hidden vault (UI can be minimal; the **key material must be real**).
5. Round-trips XMP sidecars on export so the vault is never a roach motel.

A single local user on a single machine. No peers, no ML search, no map, no faces yet — but the schema columns and key material for those are already in place so Phase 2/3 graft on rather than migrate.

**This phase ends when the app can daily-driver for single-user timeline browsing, and every Acceptance criterion in §9 passes.**

---

## Prerequisites

- Empty repo at `/home/jabe/Workspace/media-view/` (confirmed at plan time).
- Working internet to fetch crates, Node packages, and Tauri templates.
- Dev dependencies on the host: Rust stable, pnpm, Tauri CLI, `libsodium-dev`, `libheif-dev`, `ffmpeg-dev`, `pkg-config`, `build-essential`.

---

## Immutable contracts introduced here (do not change later)

These are pulled straight from `architecture.md` and must be respected from the first commit:

- Crypto primitives, Argon2 params, key hierarchy, CAS blob format (§2).
- CAS directory layout and filename scheme (§3).
- DDL for `user`, `source`, `asset`, `asset_location`, `collection`, `collection_member`, `collection_key`, `face` (shape), `person`, `phash`, `edit`, `derivative`, `asset_vec`, `asset_text` (§4.2). **All ciphertext columns listed there must exist even if Phase 1 doesn't populate them.**
- Provenance append-only semantics (§5).
- Peer-identity format generated at user creation (§6), even though networking doesn't wire up until Phase 3.

---

## Tasks

### 1. Repo scaffolding

1. `Cargo.toml` workspace at root with members: `crates/core`, `crates/sync` (stub — empty `lib.rs`), `app/src-tauri`.
2. `rust-toolchain.toml` pinning stable, with `rustfmt`+`clippy`.
3. `pnpm-workspace.yaml` for the app.
4. `.gitignore`, `.editorconfig`, `.cargo/config.toml` (with `RUSTFLAGS` for dev/release split), `.github/workflows/ci.yml` running the checks listed in `architecture.md` §7.5.
5. `scripts/dev.sh` (one command to start `tauri dev`), `scripts/build.sh` (release), `scripts/codegen_ts.sh` (run `ts-rs` to emit TS types).
6. `README.md` at repo root: one paragraph + link to `plans/README.md`.

Deliverable: `cargo check --workspace` returns success on an empty workspace; `pnpm --filter app install` succeeds.

### 2. `crates/core/src/error.rs`

Single `thiserror` hierarchy for the whole crate:

```rust
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("crypto failure")]              Crypto,
    #[error("invalid blob format")]         BlobFormat,
    #[error("key wrong or data corrupt")]   KeyOrData,
    #[error("i/o: {0}")]                    Io(#[from] std::io::Error),
    #[error("db: {0}")]                     Db(#[from] rusqlite::Error),
    #[error("media: {0}")]                  Media(String),
    #[error("ingest: {0}")]                 Ingest(String),
    #[error("not found")]                   NotFound,
    #[error("locked")]                      Locked,
    #[error("rate limited")]                RateLimited,
}
pub type Result<T> = std::result::Result<T, Error>;
```

Rule: **decryption errors never include the reason** (`Crypto` and `KeyOrData` are deliberately vague). Internal logs can have more detail via `tracing::debug!`.

### 3. `crates/core/src/crypto/` — the one irreversible module

Implement exactly the public API in `architecture.md` §2.6. Details:

- `envelope.rs`: `derive_kek_from_password`, `wrap/unwrap_master_key`, `wrap/unwrap_collection_key`, `wrap/unwrap_file_key`, `seal/open_row`, `seal/open_blob_*`, `seal_for_peer`, `open_from_peer`.
- `keystore.rs`:
  - `create_user(username, password) -> User` — generates masterKey, X25519, Iroh Ed25519, wraps all, inserts into `user`.
  - `unlock(username, password) -> UnlockedUser` — derives KEK, unwraps masterKey, keeps unlocked secrets in-memory only.
  - `set_hidden_vault_password(user, second_password)` — derives a second masterKey via a second Argon2id(salt2), stored *without* the `has_hidden_vault=1` flag flipping (to preserve deniability — the flag is purely local UI hint that resets on lock).
  - `unlock_hidden_vault(user, second_password)` — attempts a second derivation; on success, exposes a second `UnlockedContext`.
  - `create_album_password(collection_id, password)` — generates an `albumKey` via Argon2id(album_salt, password); uses it to wrap the collection key into a `collection_key` row with `wrapping='album_password'`.
  - `unlock_album(collection_id, password)` — symmetric.
- `format.rs`: parsing/writing `MVV1` blob header, row AEAD `(nonce || ct)` layout.

**Tests (unit + integration):**
- Known-vector Argon2id with `sensitive` limits against libsodium reference.
- `seal_row` → `open_row` identity for 1 KB/1 MB inputs; wrong `row_id` fails.
- `seal_blob_writer` → `open_blob_reader` identity for 0 B, 1 B, 4 MiB, 4 MiB + 1 B, 100 MiB streams.
- `seal_for_peer` → `open_from_peer` with correct/incorrect keypair.
- Property test (`proptest`): random plaintext round-trips; any one-byte flip in ciphertext → `Error::KeyOrData`.
- Adversarial test: without the password, a dump of a 50-asset vault's `index.db` + CAS is statistically indistinguishable from random in the ciphertext columns (chi-square on column bytes).

### 4. `crates/core/src/cas/store.rs`

Implement:

```rust
pub struct CasStore { root: PathBuf }

impl CasStore {
    pub fn open(root: &Path) -> Result<Self>;

    /// Encrypts `plaintext` with `file_key`, writes to CAS keyed by BLAKE3(plaintext).
    /// Returns the hex hash. Idempotent: already-present blobs are not rewritten.
    pub fn put(&self, plaintext: &[u8], file_key: &FileKey) -> Result<String>;

    /// Streaming version: consume a `Read`; compute BLAKE3 incrementally; encrypt; write.
    pub fn put_streaming<R: Read>(&self, src: R, file_key: &FileKey) -> Result<(String, u64)>;

    /// Decrypts a blob into memory (use only for small derivatives).
    pub fn get(&self, cas_ref: &str, file_key: &FileKey) -> Result<Vec<u8>>;

    /// Streaming decrypt. Caller drives.
    pub fn open_reader(&self, cas_ref: &str, file_key: &FileKey) -> Result<impl Read>;

    /// Mark-sweep GC. Not called from Phase 1 UI; exposed for tests.
    pub fn gc(&self, live: &HashSet<String>) -> Result<GcReport>;

    /// Integrity check: rehash ciphertext, verify magic + MAC via open-reader-read-to-end.
    pub fn verify(&self, cas_ref: &str, file_key: &FileKey) -> Result<()>;
}
```

Writes go through `<root>/cas/tmp/<uuid>`, fsync, rename to `<root>/cas/<AA>/<HASH>`. Tests: interrupted write cleaned up; double put is a no-op; GC moves orphans to `cas/trash/` with a timestamped manifest.

### 5. `crates/core/src/db/`

- `schema.rs`: exact DDL from `architecture.md` §4.2 as a single `init.sql` embedded string + `fn init(conn: &Connection) -> Result<()>`.
- `migrate.rs`: `user_version` pragma; Phase 1 is version 1; future additions bump and run additive migrations.
- `queries.rs`: typed helpers for the hot-paths used in Phase 1:
  - `insert_source`, `insert_asset_if_new` (returns inserted/existing), `insert_asset_location`, `list_timeline_page`, `insert_collection`, `add_to_collection`, `upsert_collection_key`, `set_album_password`, `set_edit`, `insert_derivative`.
- Connection pool: single-writer via a `Mutex<Connection>`, readers via independent connections opened with `SQLITE_OPEN_READONLY`. WAL mode.
- Enable FTS5 + sqlite-vec loading at connection open.

### 6. `crates/core/src/media/`

- `probe.rs`: Given a file path, return a `MediaProbe` struct:
  ```rust
  pub struct MediaProbe {
      pub mime: String,
      pub width: Option<u32>,
      pub height: Option<u32>,
      pub duration_ms: Option<u64>,
      pub taken_at_utc: Option<DateTime<Utc>>,
      pub gps: Option<(f64, f64, Option<f64>)>,
      pub device: Option<String>,
      pub lens: Option<String>,
      pub is_raw: bool,
      pub is_screenshot: bool,
      pub is_live: bool,
      pub is_motion: bool,
      pub exif_all_json: serde_json::Value,
  }
  ```
  Implementation:
  - HEIC/HEIF via `libheif-rs`.
  - EXIF/XMP via `nom-exif` for broad format coverage + `kamadak-exif` fallback for JPEG where `nom-exif` is weaker.
  - Video via `ffmpeg-next`'s `format` + `stream` + metadata.
  - RAW (CR2/CR3/NEF/ARW/DNG/RAF/ORF): probe at least MIME+dimensions+EXIF via `rawloader` or `libraw`-bindings if a Rust binding exists; otherwise Phase 1 can ship partial RAW support (`is_raw=true`, MIME set, other fields None) and Phase 2 fills it in via ONNX preprocessing. **Acceptance for Phase 1 requires the `is_raw` flag to be set correctly**, even if other fields are absent.
  - `is_screenshot`: device-specific markers (iOS file naming, MakerNote key `UserComment=="Screenshot"`, Android `Screenshots/` folder).
  - `is_live` / `is_motion`: set by `pairs.rs` after the whole source is scanned.
- `pairs.rs`: Given all file paths in a Source, group:
  - **Live Photos**: same basename `.HEIC` + `.MOV`, matching Apple's `AssetIdentifier` XMP UUID if present.
  - **Motion Photos**: JPEG files with embedded MP4 at specific offsets (Samsung, Pixel); detect by scanning for the trailing `MotionPhoto_Data` magic.
  - **RAW+JPEG**: same basename different extension, both with camera EXIF.
  - **Bursts**: sequential files from iPhone (IMG_E prefix + matching IMG_ sibling, or EXIF `BurstUUID`).
  Emit `is_live` / `is_motion` flags, pair metadata into the `exif_all_json`.
- `derive.rs`: thumbnail (256px, 1024px) via `image` crate (JPEG → resize → WebP), proxy MP4 (720p H.264) via `ffmpeg-next` for videos. Derivatives stored via `CasStore::put` with a fresh `FileKey`; `derivative` row written with kind=`thumb256`/`thumb1024`/`proxy_mp4`.

### 7. `crates/core/src/ingest/`

- `provenance.rs`: `fn register_source(...) -> Source`, `fn record_location(asset_id, source_id, path, mtime) -> Result<()>`. Both atomic (single transaction).
- `sidecar.rs`:
  - `read_xmp_sidecar(path) -> Option<XmpFields>` — looks for `<path>.xmp` adjacent to the original; parses Adobe XMP (subset: `dc:*`, `xmp:*`, `exif:GPS*`, `photoshop:*`, custom `mediavault:*`).
  - `write_xmp_sidecar(asset, target_dir) -> Result<()>` — produces a standards-compliant XMP next to the exported original. User edits in the app (date correction, tags, people assigned, notes) appear here so the data is portable back out.
- `adapters/`:
  - `generic.rs` — recursive scan, MIME sniff, probe, insert asset (dedupe by BLAKE3), encrypt-and-store into CAS, record location, read+apply sidecar if present, generate thumbnails. Exposes a progress channel (`tokio::sync::mpsc`).
  - `iphone_folder.rs` — wraps `generic` with:
    - Live/Motion/Burst pairing across the whole folder before inserts.
    - Recognition of DCIM structures (`100APPLE`, `101APPLE`...), heuristics to infer source device from embedded MakerNote.
  - `google_takeout.rs` — walks unzipped Takeout dir; for each media file looks for a sibling `<name>.json` (Google's sidecar format) and **uses it to reconstruct EXIF fields Google strips** (particularly `photoTakenTime.timestamp` and `geoData.latitude/longitude`). Produces `exif_all_json` that's a merge of on-file EXIF + Takeout JSON (with a tag indicating Takeout-reconstructed fields).

All three adapters share the `trait IngestAdapter` in `ingest/mod.rs`:
```rust
#[async_trait]
pub trait IngestAdapter {
    fn kind(&self) -> &'static str;
    async fn scan(&self, root: &Path) -> Result<ScanReport>;
    async fn ingest(&self, ctx: &IngestCtx, progress: Progress) -> Result<IngestReport>;
}
```
`IngestCtx` carries the unlocked user's collectionKey (for default "unsorted" album), the CAS handle, and the DB pool.

### 8. Tauri IPC surface (`app/src-tauri/src/commands/`)

Minimal set for Phase 1, all generated-typed to the TS side via `ts-rs`:

```rust
// unlock
#[tauri::command] async fn user_exists() -> bool;
#[tauri::command] async fn create_user(username: String, password: SecretString) -> Result<()>;
#[tauri::command] async fn unlock(username: String, password: SecretString) -> Result<SessionHandle>;
#[tauri::command] async fn lock() -> Result<()>;
#[tauri::command] async fn unlock_hidden(password: SecretString) -> Result<()>;

// sources
#[tauri::command] async fn add_source(name: String, root: PathBuf, adapter: String, linked_only: bool) -> Result<SourceId>;
#[tauri::command] async fn list_sources() -> Result<Vec<SourceView>>;
#[tauri::command] async fn ingest_status(source_id: SourceId) -> Result<IngestStatus>;

// timeline
#[tauri::command] async fn timeline_page(cursor: Option<TimelineCursor>, limit: u32) -> Result<TimelinePage>;
#[tauri::command] async fn asset_detail(id: AssetId) -> Result<AssetDetailView>;
#[tauri::command] async fn asset_thumbnail(id: AssetId, size: u32) -> Result<Vec<u8>>; // decrypted bytes
#[tauri::command] async fn asset_original(id: AssetId) -> Result<Vec<u8>>;

// albums
#[tauri::command] async fn create_album(name: String, password: Option<SecretString>) -> Result<CollectionId>;
#[tauri::command] async fn list_albums(include_hidden: bool) -> Result<Vec<AlbumView>>;
#[tauri::command] async fn unlock_album(id: CollectionId, password: SecretString) -> Result<()>;
#[tauri::command] async fn album_page(id: CollectionId, cursor: Option<TimelineCursor>, limit: u32) -> Result<TimelinePage>;
#[tauri::command] async fn add_to_album(id: CollectionId, asset_ids: Vec<AssetId>) -> Result<()>;

// export
#[tauri::command] async fn export_album(id: CollectionId, dest: PathBuf, options: ExportOptions) -> Result<ExportReport>;
```

All commands return `Result<T, String>` to TS (errors already normalised; never include sensitive internals).

### 9. React UI (`app/src/`)

Minimum screens, styled with CSS modules:

1. **Unlock** (`ui/unlock/`) — single-field password prompt; on first run, double-field create-user. Hidden-vault unlock is a separate gesture: long-press the app logo for 2 seconds, brings up a password field that looks identical to the main unlock. On wrong password it shakes *exactly the same* as the main unlock — no tell.
2. **Sources manager** (`ui/sources/`) — "Add a source" dialog: pick folder, pick adapter (iPhone / Takeout / Generic), toggle "link only." Progress bars for ingests.
3. **Timeline** (`ui/timeline/`) — virtualized grid. 3-columns on small screens, 5 on large. Sticky month headers. Tap → asset detail.
4. **Asset detail** (`ui/timeline/AssetDetail.tsx`) — large view with EXIF panel (camera/lens/exposure/GPS/device), original/download button, add-to-album, move-to-hidden-vault, edit-date, delete.
5. **Albums** (`ui/albums/`) — list of albums (regular first, hidden-vault albums only visible when hidden is unlocked). Create album (optional password). Album detail = same as timeline scoped to an album.
6. **Export** — button on album detail. Writes decrypted originals + XMP sidecars into a destination folder.

State:
- Zustand store for session + unlocked-album set + hidden-unlocked boolean.
- React Query for timeline/album pages (keyed by cursor).
- Virtualizer: `@tanstack/react-virtual`.

### 10. Golden test fixtures

Under `tests/fixtures/`:

- `iphone_dump/` — synthesised DCIM tree (10 JPEGs, 3 HEICs with paired MOVs, 2 bursts, 1 screenshot, 1 video). Built by `scripts/make_fixtures.py` (small Python-in-CI, not app code) from public-domain source images + `exiftool` to stamp EXIF/UUID correctly.
- `takeout_dump/` — same 10 assets but with Google Takeout JSON sidecars, EXIF partially stripped as Google actually does it.
- `near_dup_set/` — 5 photos + 3 near-duplicates (rotated / re-encoded / cropped) + 2 exact duplicates.

Check in fixtures as small as possible; regenerate in CI if missing.

### 11. Integration tests (`tests/integration/`)

Each test creates a fresh temp vault, runs a flow, asserts:

- `crypto_roundtrip.rs` — exercises every envelope API. Property test.
- `ingest_golden.rs` — ingests `iphone_dump/`, asserts 16 assets, 3 Live pairs, 2 bursts flagged, 1 screenshot flagged. Repeats ingest of the *same* dir and asserts zero new assets (dedupe). Ingests `takeout_dump/` and asserts the same 16 assets are already there (content dedupe across sources), with Takeout JSON sidecar data merged into `exif_all_ct`.
- `sidecar_roundtrip.rs` — ingest → edit tags/date → export → re-ingest into a fresh vault → assert edits preserved via XMP.
- `album_password.rs` — create album with password, lock vault, unlock user, listing the album shows the album exists but no members; unlock album → members appear; wrong password → no members and no error leak.
- `hidden_vault.rs` — without hidden password, `list_albums(include_hidden=true)` returns the same as without. Set hidden password, move 2 albums there, lock, re-unlock main: hidden albums still invisible. Unlock hidden: they appear. Wrong hidden password: unchanged (no error).
- `adversarial_dump.rs` — ingest fixtures, snapshot the vault dir, zero out the password, verify that a listing of ciphertext columns is indistinguishable from random (chi-square).

### 12. CI

`.github/workflows/ci.yml` running everything in `architecture.md` §7.5 on Ubuntu. Mac + Windows runners can be added later; Phase 1 targets Linux-only builds.

---

## File-by-file checklist

| Path | Owner (module) | Lines (rough) |
|---|---|---|
| `Cargo.toml` | root workspace | 40 |
| `rust-toolchain.toml` | — | 5 |
| `pnpm-workspace.yaml` | — | 3 |
| `.github/workflows/ci.yml` | — | 80 |
| `.cargo/config.toml` | — | 15 |
| `crates/core/Cargo.toml` | — | 50 |
| `crates/core/src/lib.rs` | exports | 20 |
| `crates/core/src/error.rs` | errors | 60 |
| `crates/core/src/crypto/mod.rs` | pub use | 10 |
| `crates/core/src/crypto/envelope.rs` | envelope | 400 |
| `crates/core/src/crypto/keystore.rs` | keystore | 300 |
| `crates/core/src/crypto/format.rs` | wire format | 150 |
| `crates/core/src/cas/mod.rs` | pub use | 5 |
| `crates/core/src/cas/store.rs` | CAS | 400 |
| `crates/core/src/db/schema.rs` | DDL + init | 200 |
| `crates/core/src/db/migrate.rs` | | 60 |
| `crates/core/src/db/queries.rs` | hot-path helpers | 500 |
| `crates/core/src/media/probe.rs` | EXIF/HEIC/video | 400 |
| `crates/core/src/media/derive.rs` | thumbs/proxies | 200 |
| `crates/core/src/media/pairs.rs` | Live/Motion/Burst | 250 |
| `crates/core/src/ingest/mod.rs` | trait | 80 |
| `crates/core/src/ingest/provenance.rs` | | 80 |
| `crates/core/src/ingest/sidecar.rs` | XMP r/w | 300 |
| `crates/core/src/ingest/adapters/generic.rs` | | 350 |
| `crates/core/src/ingest/adapters/iphone_folder.rs` | | 250 |
| `crates/core/src/ingest/adapters/google_takeout.rs` | | 300 |
| `crates/sync/src/lib.rs` | stub | 10 |
| `app/src-tauri/Cargo.toml` | | 40 |
| `app/src-tauri/src/main.rs` | | 120 |
| `app/src-tauri/src/commands/*.rs` | cmd surface | 500 (split by area) |
| `app/package.json` | | 40 |
| `app/vite.config.ts` | | 40 |
| `app/src/main.tsx` | | 20 |
| `app/src/App.tsx` | routing | 80 |
| `app/src/ipc.ts` | generated | 200 |
| `app/src/ui/unlock/*` | | 250 |
| `app/src/ui/sources/*` | | 300 |
| `app/src/ui/timeline/*` | | 500 |
| `app/src/ui/albums/*` | | 400 |
| `tests/integration/*.rs` | | 1000 total |
| `tests/fixtures/*` | data | — |
| `scripts/make_fixtures.py` | | 150 |

Total: ~7.5k LOC Rust + ~2k LOC TS/React, roughly.

---

## Execution order (suggested)

A single agent can do this sequentially; multiple agents can split on module boundaries.

1. Scaffolding + CI (Task 1).
2. `error.rs` (Task 2).
3. `crypto/` complete — **stop and verify round-trip + adversarial tests green before moving on.** (Task 3.)
4. `cas/store.rs` (Task 4).
5. `db/schema.rs` + `migrate.rs` + `queries.rs` for the Phase 1 subset (Task 5).
6. `media/probe.rs` + `derive.rs` (Task 6).
7. `ingest/sidecar.rs` (Task 7.a).
8. `ingest/adapters/generic.rs` (Task 7.b).
9. `media/pairs.rs` (Task 6 continued).
10. `ingest/adapters/iphone_folder.rs`, `google_takeout.rs` (Task 7.c/d).
11. Tauri commands (Task 8).
12. React UI (Task 9).
13. Golden fixtures + integration tests (Tasks 10 + 11) — these can run in parallel with UI.
14. CI green across the full test suite.

---

## Acceptance criteria (Phase 1 is DONE when all pass)

1. **`cargo test --workspace` green.** Includes all unit + integration + property tests. No ignored tests.
2. **CI green on Ubuntu latest.** `cargo clippy -D warnings`, `cargo fmt --check`, `cargo audit`, `cargo deny`, pnpm lint/typecheck/test all pass.
3. **Adversarial dump test passes.** A vault's ciphertext columns are statistically indistinguishable from random without the password (chi-square p-value within expected bounds over 100 runs).
4. **Crypto round-trips**: every API in `envelope.rs` round-trips on inputs spanning 0 B → 100 MiB. Wrong key → `Error::KeyOrData`. Tamper → `Error::KeyOrData`.
5. **Ingest golden flow**: `tests/fixtures/iphone_dump` yields 16 assets; re-ingest 0 new assets; ingest of `takeout_dump` adds 0 new assets (pure dedupe); `asset.is_live` set on 3 assets, `is_screenshot` on 1, `is_raw` on 0, `is_motion` on 0 (fixture doesn't include motion photos), burst grouping on 2 bursts correct.
6. **XMP round-trip**: assign a person, change a date, add a tag → export an album → re-ingest into a fresh vault → all edits preserved.
7. **Album password flow**: create album with password, lock, re-unlock user → album visible as an entity, 0 members leaked; wrong password → 0 members leaked, no error; right password → members appear.
8. **Hidden vault flow**: plausible-deniability holds — without entering the hidden password, `list_albums(include_hidden=true)` returns the same set as with it off. Wrong hidden password → same. Right hidden password → hidden collections appear and can be browsed.
9. **UI smoke**: Playwright run launches the app, creates a user, adds the `iphone_dump` as a source, waits for ingest, opens the timeline, scrolls, opens an asset detail, creates a password-protected album, unlocks it, exports it. All assertions pass.
10. **Docs**: `cargo doc --no-deps` builds clean with no private-item warnings in the public API.

---

## Known unknowns / things to escalate to the human

- **RAW probe depth.** If no pure-Rust RAW library is mature enough at build time, Phase 1 may ship RAW as "flagged, dimensions only" and defer proper probing to Phase 2 (same model used for CLIP can decode RAW preview images). State this in commit messages if it happens.
- **libheif packaging.** Linux ships it; macOS needs Homebrew; Windows needs vcpkg. If static linking on macOS is too painful, note the dynamic dependency in the app bundle and move on.
- **FFI crate choice for ffmpeg.** `ffmpeg-next` is canonical but breaks on some distros. Fallback: `ez-ffmpeg` or `rsmpeg`. Stick with `ffmpeg-next` unless it fights you for >1 hour — then pivot.
- **`sqlite-vec` availability.** If the `sqlite-vec` extension isn't shipping a prebuilt for the target triple, Phase 1 can leave `asset_vec` table declared but not populated (Phase 2 owns the CLIP pipeline anyway). Don't remove the virtual-table DDL.

---

## What Phase 1 explicitly does NOT ship

- CLIP, faces, OCR, Whisper, or any ML.
- Map view.
- Peer-to-peer or multi-user.
- Memories / trips / smart albums.
- Mobile app.
- Non-destructive edit UI (the `edit` table exists; no UI path writes to it).
- RAW develop.
- Public share links.
- iMazing/iTunes/WhatsApp adapters.

All of those are covered by later phases. **Do not smuggle them in.** Phase 1's job is foundations you won't regret — keep it boring.
