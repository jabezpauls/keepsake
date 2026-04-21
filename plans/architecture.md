# Architecture — Frozen Contracts

> **Every session: read this document before writing any code.**
> These contracts are binding across all phases. Never break them.
> If you think one is wrong, stop and flag to the human instead of changing it.

---

## 0. Scope of this document

This document locks:

1. Project layout (crate boundaries, directory tree)
2. The **crypto envelope** — primitives, key hierarchy, file format
3. The **CAS store** — on-disk layout, naming, chunking
4. The **database schema split** — plaintext-indexable vs. AEAD-encrypted
5. **Provenance** tables
6. **Peer identity** format (Iroh + libsodium)
7. Code conventions, error handling, logging, security rules
8. Build/test/CI tooling

All phase docs extend this, never override it.

---

## 1. Project layout

**Cargo workspace** rooted at `/home/jabe/Workspace/media-view/Cargo.toml`.

```
media-view/
├── Cargo.toml                     workspace root
├── rust-toolchain.toml            pinned stable Rust
├── plans/                         phase plans (human-readable)
├── crates/
│   ├── core/                      library: all domain logic
│   │   ├── src/
│   │   │   ├── lib.rs             pub re-exports
│   │   │   ├── error.rs           thiserror-based error hierarchy
│   │   │   ├── crypto/            see §2
│   │   │   │   ├── mod.rs
│   │   │   │   ├── envelope.rs    KDF, key chain, secretstream wrappers
│   │   │   │   ├── keystore.rs    user keys, hidden vault, album passphrase
│   │   │   │   └── format.rs      wire format for ciphertext blobs
│   │   │   ├── cas/               see §3
│   │   │   │   ├── mod.rs
│   │   │   │   └── store.rs       CAS read/write
│   │   │   ├── db/                see §4
│   │   │   │   ├── mod.rs
│   │   │   │   ├── schema.rs      DDL (frozen)
│   │   │   │   ├── migrate.rs     only additive changes post-Phase 1
│   │   │   │   └── queries.rs     prepared query helpers
│   │   │   ├── ingest/            see §5
│   │   │   │   ├── mod.rs
│   │   │   │   ├── provenance.rs
│   │   │   │   ├── sidecar.rs     XMP round-trip
│   │   │   │   └── adapters/
│   │   │   │       ├── iphone_folder.rs
│   │   │   │       ├── google_takeout.rs
│   │   │   │       └── generic.rs
│   │   │   ├── media/             EXIF, HEIC, RAW, video probe
│   │   │   │   ├── mod.rs
│   │   │   │   ├── probe.rs
│   │   │   │   ├── derive.rs      thumbs, proxies
│   │   │   │   └── pairs.rs       Live/Motion/RAW+JPEG grouping
│   │   │   ├── ml/                Phase 2+
│   │   │   │   ├── mod.rs
│   │   │   │   ├── clip.rs
│   │   │   │   ├── faces.rs
│   │   │   │   ├── phash.rs
│   │   │   │   └── ocr.rs         Phase 3
│   │   │   ├── search/            Phase 2+
│   │   │   │   └── mod.rs
│   │   │   └── analytics/         Phase 3+
│   │   │       ├── trips.rs
│   │   │       ├── memories.rs
│   │   │       ├── smart_albums.rs
│   │   │       └── pets.rs
│   │   └── Cargo.toml
│   └── sync/                      Phase 3 — Iroh peer node
│       ├── src/
│       │   ├── lib.rs
│       │   ├── peer.rs            Iroh node lifecycle, pairing
│       │   ├── share.rs           X25519-wrap collection keys
│       │   └── blobs.rs           iroh-blobs <-> CAS bridge
│       └── Cargo.toml
├── app/
│   ├── src-tauri/                 Tauri v2 shell (Rust)
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   └── commands/          Tauri IPC surface
│   │   ├── tauri.conf.json
│   │   └── Cargo.toml
│   ├── src/                       React + TypeScript UI
│   │   ├── main.tsx
│   │   ├── App.tsx
│   │   ├── ui/
│   │   │   ├── timeline/
│   │   │   ├── albums/
│   │   │   ├── unlock/
│   │   │   ├── map/               Phase 2
│   │   │   ├── people/            Phase 2
│   │   │   ├── search/            Phase 2
│   │   │   ├── memories/          Phase 3
│   │   │   └── share/             Phase 3
│   │   └── ipc.ts                 typed wrappers around Tauri commands
│   ├── package.json
│   └── vite.config.ts
├── models/                        ONNX weights (Phase 2+), fetched by scripts/download_models.sh
├── tests/
│   ├── fixtures/                  golden dumps, test images
│   └── integration/
│       ├── crypto_roundtrip.rs
│       ├── ingest_golden.rs
│       ├── sidecar_roundtrip.rs
│       └── peer_pairing.rs        Phase 3
└── scripts/
    ├── build.sh
    ├── download_models.sh
    └── codegen_ts.sh              emits TS types from Rust
```

Crates depend downward only: `sync → core`; `app/src-tauri → core, sync`. Never the reverse.

---

## 2. Crypto envelope

### 2.1 Primitives — the only ones allowed

| Purpose | Primitive | Crate |
|---|---|---|
| Password KDF | Argon2id, `sensitive` limits | `libsodium-sys-stable` via `crypto_pwhash` |
| Symmetric AEAD (row-level) | XChaCha20-Poly1305-IETF | `crypto_aead_xchacha20poly1305_ietf` |
| Symmetric stream (file chunks) | XChaCha20-Poly1305 secretstream | `crypto_secretstream_xchacha20poly1305` |
| Asymmetric sealing (shares) | X25519 + XSalsa20-Poly1305 sealed box | `crypto_box_seal` |
| Hash | BLAKE3 | `blake3` crate |
| Random | OS CSPRNG | libsodium `randombytes_buf` |

**Forbidden:** AES-GCM, ChaCha20 non-Poly1305, SHA-2 for KDF-like use, PBKDF2, any hand-rolled construction.

### 2.2 Argon2id parameters (FROZEN)

```rust
pub const ARGON2_OPSLIMIT: u64 = 4;               // sodium OPSLIMIT_SENSITIVE
pub const ARGON2_MEMLIMIT: usize = 1_073_741_824; // 1 GiB, MEMLIMIT_SENSITIVE
pub const ARGON2_ALG: i32 = 2;                    // crypto_pwhash_ALG_ARGON2ID13
pub const SALT_LEN: usize = 16;                   // crypto_pwhash_SALTBYTES
```

These must never change post-Phase 1 without a full re-key migration release.

### 2.3 Key hierarchy (Ente-derived, frozen)

```
password ─Argon2id(salt)─▶ keyEncryptionKey (256-bit)
                              │
                              │ decrypts
                              ▼
                          masterKey (256-bit, random at signup)
                              │
                              ├── derives ──▶ searchKey (for sqlite-vec blind index)
                              ├── wraps ────▶ collectionKey[i] (per album/collection)
                              │                    │
                              │                    │ wraps
                              │                    ▼
                              │                fileKey[j] (per file, 256-bit, random)
                              │
                              └── wraps ────▶ hiddenVaultMasterKey   (only if 2nd passphrase entered)

 Per-user identity keys (independent of masterKey, sealed by it):
   libsodium keypair (X25519) ── for sharing
   Iroh Ed25519 node key       ── for peer identity

 Per-album-password albums:
   Album password ─Argon2id(album_salt)─▶ albumKey → wraps collectionKey (in addition to masterKey wrap)
   Result: album stays locked even when main user is unlocked, until album password is entered.
```

### 2.4 File format for CAS blobs (FROZEN)

Every CAS blob is a libsodium secretstream with a 24-byte prepended header:

```
┌──────────────────────────────────────────────┐
│  magic     : 4  bytes  = b"MVV1"              │   (media-view vault v1)
│  header    : 24 bytes  = secretstream header  │   (from crypto_secretstream_init_push)
│  chunks ...: each <= 4 MiB plaintext          │
└──────────────────────────────────────────────┘
```

- Chunk size: **exactly 4 MiB** of plaintext per chunk, final chunk smaller. FROZEN.
- Last chunk is tagged with `TAG_FINAL`.
- The `fileKey` is 32 bytes, random, AEAD-wrapped in the `asset` row's ciphertext columns by the owning `collectionKey`.
- Reader rejects any blob not starting with `MVV1`.

### 2.5 Row-level AEAD (FROZEN)

Encrypted columns use XChaCha20-Poly1305-IETF with:

```
┌────────────────────────────────────────────────┐
│  nonce : 24 bytes (random per row per column) │
│  ad    : 8  bytes = row_id (little-endian)    │
│  ct    : len(plaintext) + 16 bytes (MAC tag)  │
└────────────────────────────────────────────────┘
```

The column type in SQLite is `BLOB`; all values are `nonce || ciphertext_with_tag`.

### 2.6 envelope.rs public API (stable from Phase 1)

```rust
pub struct MasterKey([u8; 32]);        impl Zeroize, Drop
pub struct CollectionKey([u8; 32]);    impl Zeroize, Drop
pub struct FileKey([u8; 32]);          impl Zeroize, Drop
pub struct SearchKey([u8; 32]);        impl Zeroize, Drop

pub fn derive_kek_from_password(pw: &SecretString, salt: &[u8; 16]) -> KeyEncryptionKey;
pub fn wrap_master_key(mk: &MasterKey, kek: &KeyEncryptionKey) -> Vec<u8>;   // nonce || ct
pub fn unwrap_master_key(wrapped: &[u8], kek: &KeyEncryptionKey) -> Result<MasterKey>;

pub fn wrap_collection_key(ck: &CollectionKey, mk: &MasterKey) -> Vec<u8>;
pub fn unwrap_collection_key(wrapped: &[u8], mk: &MasterKey) -> Result<CollectionKey>;

pub fn wrap_file_key(fk: &FileKey, ck: &CollectionKey) -> Vec<u8>;
pub fn unwrap_file_key(wrapped: &[u8], ck: &CollectionKey) -> Result<FileKey>;

pub fn seal_row(plaintext: &[u8], row_id: u64, key: &[u8; 32]) -> Vec<u8>;
pub fn open_row(ciphertext: &[u8], row_id: u64, key: &[u8; 32]) -> Result<Vec<u8>>;

pub fn seal_blob_writer<W: Write>(fk: &FileKey, w: W) -> Result<BlobWriter<W>>;
pub fn open_blob_reader<R: Read>(fk: &FileKey, r: R) -> Result<BlobReader<R>>;

pub fn seal_for_peer(plaintext: &[u8], recipient_pk: &PublicKey) -> Vec<u8>; // sealed box
pub fn open_from_peer(ct: &[u8], kp: &Keypair) -> Result<Vec<u8>>;
```

Callers never touch libsodium directly. New crypto needs? Add it here.

---

## 3. CAS store

### 3.1 Layout (FROZEN)

```
<vault_dir>/cas/<AA>/<HEX_BLAKE3_OF_PLAINTEXT>
```

- `<AA>` = first two hex characters of the BLAKE3 hash.
- Filename = lowercase full hex BLAKE3 (64 chars).
- File contents = ciphertext per §2.4 (magic + secretstream).
- Temp writes go to `<vault>/cas/tmp/<random>` and are `fsync`'d + atomically renamed into place.
- Duplicate writes are no-ops: if `<AA>/<HASH>` already exists, the writer verifies file size and returns.

### 3.2 Garbage collection

Never automatic. A `cas::gc()` call marks unreferenced blobs and moves them to `<vault>/cas/trash/` for N days, then deletes. Controlled by user action only.

### 3.3 Derivatives

Thumbnails, proxies (transcoded video), and representative video frames are **separate CAS entries** with their own BLAKE3 (of the derived bytes, encrypted with a fresh `fileKey`). The `derivative` table maps `(asset_id, kind)` → CAS ref.

---

## 4. Database schema (FROZEN)

SQLite file at `<vault>/index.db`. `sqlite-vec` and `FTS5` extensions loaded at open.

**Journal mode:** WAL. **Foreign keys:** ON. **Synchronous:** NORMAL.

### 4.1 Two-tier rule

Each table row contains:
- **Plaintext** columns: structurally necessary for filtering / joins. Leak only coarse facts.
- **Ciphertext (`_ct`) columns**: BLOB, AEAD-sealed per §2.5. Hold everything that could identify subject matter.

### 4.2 DDL (frozen at Phase 1; additions only later)

```sql
-- Users of this vault (Phase 1: exactly one row; Phase 3: many)
CREATE TABLE user (
    id                   INTEGER PRIMARY KEY,
    username_ct          BLOB NOT NULL,           -- display name, encrypted
    argon2_salt          BLOB NOT NULL,           -- 16 bytes
    wrapped_master_key   BLOB NOT NULL,           -- secure via KEK derived from password
    recovery_blob        BLOB,                    -- optional BIP39-wrapped master key
    identity_pub         BLOB NOT NULL,           -- X25519 public key (for sharing)
    wrapped_identity_sk  BLOB NOT NULL,           -- X25519 secret, wrapped by masterKey
    iroh_node_pub        BLOB NOT NULL,           -- Ed25519, Phase 3 uses it
    wrapped_iroh_sk      BLOB NOT NULL,           -- Ed25519 secret, wrapped by masterKey
    has_hidden_vault     INTEGER NOT NULL DEFAULT 0,   -- bit (plausible-deniability note below)
    created_at           INTEGER NOT NULL
);

-- Sources (backup dumps). Append-only.
CREATE TABLE source (
    id                INTEGER PRIMARY KEY,
    owner_id          INTEGER NOT NULL REFERENCES user(id),
    name_ct           BLOB NOT NULL,              -- user-facing name
    root_path_ct      BLOB NOT NULL,              -- absolute path, encrypted
    device_hint_ct    BLOB,                       -- "iPhone 15", "Canon R5" etc.
    imported_at       INTEGER NOT NULL,
    bytes_total       INTEGER NOT NULL,
    file_count        INTEGER NOT NULL,
    linked_only       INTEGER NOT NULL DEFAULT 0  -- 0 = copied into CAS; 1 = link-don't-copy mode
);

-- Canonical assets. One row per unique plaintext hash.
CREATE TABLE asset (
    id                   INTEGER PRIMARY KEY,
    blake3_plaintext     BLOB NOT NULL UNIQUE,    -- 32 bytes, plaintext — dedupe key
    mime                 TEXT NOT NULL,
    bytes                INTEGER NOT NULL,
    width                INTEGER,
    height               INTEGER,
    duration_ms          INTEGER,
    taken_at_utc_day     INTEGER,                 -- day epoch (leaky but usable)
    is_video             INTEGER NOT NULL DEFAULT 0,
    is_raw               INTEGER NOT NULL DEFAULT 0,
    is_screenshot        INTEGER NOT NULL DEFAULT 0,
    is_live              INTEGER NOT NULL DEFAULT 0,
    is_motion            INTEGER NOT NULL DEFAULT 0,
    source_id            INTEGER NOT NULL REFERENCES source(id),
    cas_ref              TEXT NOT NULL,           -- hex BLAKE3 of ciphertext blob
    imported_at          INTEGER NOT NULL,

    -- Encrypted columns (all BLOB, sealed per §2.5):
    filename_ct          BLOB NOT NULL,
    taken_at_utc_ct      BLOB,                    -- exact timestamp
    gps_ct               BLOB,                    -- lat+lon+alt serialised
    device_ct            BLOB,
    lens_ct              BLOB,
    exif_all_ct          BLOB,                    -- full EXIF/XMP/MakerNotes JSON
    clip_embedding_ct    BLOB,                    -- 768-d f32 array, Phase 2
    ocr_text_ct          BLOB,                    -- Phase 3
    transcript_ct        BLOB,                    -- Phase 4
    color_palette_ct     BLOB,                    -- dominant colors, Phase 2
    user_notes_ct        BLOB,
    tags_ct              BLOB,                    -- json array of tags
    wrapped_file_key     BLOB NOT NULL            -- fileKey sealed by the canonical collectionKey
);

-- Many-to-one physical locations (each Source contributes rows).
CREATE TABLE asset_location (
    asset_id      INTEGER NOT NULL REFERENCES asset(id),
    source_id     INTEGER NOT NULL REFERENCES source(id),
    original_path_ct BLOB NOT NULL,
    mtime         INTEGER NOT NULL,
    PRIMARY KEY (asset_id, source_id, original_path_ct)
);

-- Collections: albums, hidden vault, smart albums, trips, memories.
CREATE TABLE collection (
    id                INTEGER PRIMARY KEY,
    owner_id          INTEGER NOT NULL REFERENCES user(id),
    kind              TEXT NOT NULL CHECK (kind IN
                        ('album','hidden_vault','smart_album','trip','memory')),
    name_ct           BLOB NOT NULL,
    cover_asset_id    INTEGER REFERENCES asset(id),
    has_password      INTEGER NOT NULL DEFAULT 0,
    password_salt     BLOB,                       -- set iff has_password = 1
    created_at        INTEGER NOT NULL
);

-- Members. asset_id is plaintext (joinable); collection_id is plaintext.
CREATE TABLE collection_member (
    collection_id  INTEGER NOT NULL REFERENCES collection(id),
    asset_id       INTEGER NOT NULL REFERENCES asset(id),
    added_at       INTEGER NOT NULL,
    PRIMARY KEY (collection_id, asset_id)
);

-- One row per (collection, reader). Allows sharing + per-album password.
CREATE TABLE collection_key (
    collection_id  INTEGER NOT NULL REFERENCES collection(id),
    user_id        INTEGER NOT NULL REFERENCES user(id),
    wrapping       TEXT NOT NULL,                 -- 'master' | 'album_password' | 'peer_x25519'
    wrapped_key    BLOB NOT NULL,
    PRIMARY KEY (collection_id, user_id, wrapping)
);

-- Faces, people. Phase 2.
CREATE TABLE face (
    id              INTEGER PRIMARY KEY,
    asset_id        INTEGER NOT NULL REFERENCES asset(id),
    person_id       INTEGER REFERENCES person(id),
    quality         REAL,                         -- plaintext: used to pick cluster exemplars
    bbox_ct         BLOB NOT NULL,                -- xywh floats, encrypted
    embedding_ct    BLOB NOT NULL                 -- 512-d f32 ArcFace, encrypted
);

CREATE TABLE person (
    id               INTEGER PRIMARY KEY,
    owner_id         INTEGER NOT NULL REFERENCES user(id),
    name_ct          BLOB,
    relations_ct     BLOB,
    hidden           INTEGER NOT NULL DEFAULT 0
);

-- Perceptual hash, for near-dup clustering. pHash is leaky-ish (it reveals visual similarity),
-- but an attacker without the decrypted thumbnail can't act on it usefully. Accepted trade-off.
CREATE TABLE phash (
    asset_id  INTEGER PRIMARY KEY REFERENCES asset(id),
    hash      INTEGER NOT NULL                   -- 64-bit dhash/phash
);

-- Non-destructive edits.
CREATE TABLE edit (
    asset_id  INTEGER PRIMARY KEY REFERENCES asset(id),
    ops_ct    BLOB NOT NULL                      -- JSON of ops
);

-- Derivatives (thumbs, proxies, keyframes).
CREATE TABLE derivative (
    asset_id   INTEGER NOT NULL REFERENCES asset(id),
    kind       TEXT NOT NULL,                     -- 'thumb256','thumb1024','proxy_mp4','keyframe'
    cas_ref    TEXT NOT NULL,
    PRIMARY KEY (asset_id, kind)
);

-- sqlite-vec virtual table. Vector stored AS plaintext — vec is useful only
-- with the decrypted query vector. If you disagree with this trade, read §4.3.
CREATE VIRTUAL TABLE asset_vec USING vec0(
    asset_id INTEGER PRIMARY KEY,
    embedding FLOAT[768]
);

-- FTS5 for OCR / transcripts. Index contents re-keyed with HMAC-blind-index
-- so stored tokens don't leak readable text. See §4.3.
CREATE VIRTUAL TABLE asset_text USING fts5(
    asset_id UNINDEXED,
    token_blind,                   -- HMAC(searchKey, token)
    tokenize = 'unicode61 remove_diacritics 2'
);
```

### 4.3 Important trade-offs to KEEP AS IS

- **`asset.blake3_plaintext` is stored plaintext** so dedupe works across sessions and peers without unlocking. It reveals that two users share a photo, nothing more.
- **`taken_at_utc_day` is day-truncated** to make "all photos from 2024" filters fast without revealing exact timestamps. Exact time is in `taken_at_utc_ct`.
- **`phash` is stored plaintext (64-bit).** Leaks visual similarity between your assets but not content. Required for cheap near-dup clustering without decrypting thumbnails.
- **`asset_vec.embedding` stored plaintext** but the vector is only useful when queried with text embedded by the client (which requires the CLIP model, freely available, BUT: the attacker also needs your photos' embeddings to do anything — and those leak anyway since similarity is computable). If this becomes unacceptable later, switch to an encrypted-vector scheme like Wand/CryptVec; not in scope for Phase 1.
- **`asset_text.token_blind = HMAC(searchKey, token)`** means the FTS5 index never contains readable text. Searches happen by HMACing the query token and looking up. Queries for `"boarding"` become `HMAC(searchKey, "boarding")`. Substring search is impossible under this scheme; Phase 3 ships exact-word match only.

### 4.4 What is allowed to change later

- New **indexes** on plaintext columns: fine.
- New tables: fine (additive).
- Schema changes to existing columns: **forbidden** without a migration release.
- Modifying AEAD format, Argon2 params, CAS layout: **forbidden**, period.

---

## 5. Provenance

Every physical copy of every file is recorded. Append-only.

- `source` = one row per backup dump the user added.
- `asset_location` = one row per (canonical asset, physical path found in a source). Multiple paths for the same asset are allowed.
- Removing a Source removes its `asset_location` rows but **never** its `asset` rows (other Sources or the CAS may still reference them). Orphaned assets are candidates for `cas::gc()`.

---

## 6. Peer identity (Iroh + libsodium)

Each user has **two** key pairs:

1. **libsodium X25519** — used for sealing/opening album share envelopes. Public key goes in `user.identity_pub`. Secret key is wrapped by the user's masterKey and stored in `user.wrapped_identity_sk`.
2. **Iroh Ed25519 node key** — used as the network identity for QUIC transport. Public key goes in `user.iroh_node_pub`; secret is likewise wrapped and stored.

Pairing envelope format (Phase 3, frozen here so Phase 1 doesn't generate incompatible keys):

```
PairingTicket = base32(
    version (1 byte = 0x01)
 || iroh_node_pub (32 bytes Ed25519)
 || identity_pub  (32 bytes X25519)
 || relay_url_len (2 bytes LE) || relay_url (UTF-8)
 || timestamp (8 bytes LE, UNIX seconds)
 || signature (64 bytes Ed25519 over the preceding bytes, using iroh_node_sk)
)
```

Phase 1 generates both keys at user creation, stores them, but does not wire Iroh networking.

---

## 7. Code conventions

### 7.1 Rust

- **Edition 2021**, pinned stable via `rust-toolchain.toml`.
- `#![forbid(unsafe_code)]` at every crate root **except** `crates/core/src/crypto/` (FFI to libsodium) and `crates/core/src/media/probe.rs` (FFI to libheif/ffmpeg). Those crates use `#![deny(unsafe_op_in_unsafe_fn)]` and wrap all FFI calls.
- Error type: `thiserror::Error`. No `anyhow` in library crates. App layer may use `anyhow::Result` at Tauri command boundaries.
- All `Secret*` / key types implement `Zeroize + Drop`. Use the `secrecy` crate for password handling.
- Public functions documented with `///`. `cargo doc` must build cleanly.
- Logging: `tracing`. No `println!`/`eprintln!` in library code.
- Async: `tokio`. Blocking work on `rayon` or `tokio::task::spawn_blocking`.
- Lints: `cargo clippy --all-targets --all-features -- -D warnings` is green in CI.

### 7.2 TypeScript / React

- Strict mode on. No `any` without `// TS-EXPECT: <reason>`.
- Types for IPC are generated from Rust via `ts-rs` — never hand-written.
- State management: React Query for server state, Zustand for UI state. No Redux.
- UI virtualization: `@tanstack/react-virtual`.
- Styling: CSS modules. No inline styles except dynamic positioning.

### 7.3 Testing

- Unit tests co-located (`mod tests` inside each module).
- Integration tests under `tests/integration/`, one file per area.
- Golden fixtures in `tests/fixtures/` — generated (deterministically) by scripts; never checked in as giant binaries. Real-looking EXIF comes from a small set of public-domain images + scripted manipulations.
- Crypto round-trip tests use known vectors from the libsodium test suite where possible; otherwise use `proptest` for "encrypt → decrypt = identity" and "wrong key → fails".

### 7.4 Dependency policy

- **No transitive pull-in of alternative crypto libraries.** `cargo deny` in CI with an explicit allow-list for crypto-adjacent crates.
- Major version bumps require a justification note in the commit message.
- No `openssl-sys` anywhere.

### 7.5 CI tooling (shipping with Phase 1)

- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo fmt --check`
- `cargo audit` (warnings tolerated, errors fail)
- `cargo deny check`
- `pnpm --filter app lint && pnpm --filter app typecheck && pnpm --filter app test`

---

## 8. Build & dev tooling

- **Rust toolchain**: `rust-toolchain.toml` pins stable (whatever is current at project start).
- **Node**: `pnpm` via `packageManager` field in `package.json`.
- **Tauri v2** for the app shell.
- **SQLite extensions**: `sqlite-vec` and `FTS5` loaded via `rusqlite` feature flags.
- **libheif / ffmpeg / libsodium / onnxruntime** resolved via system packages on dev machines; for release builds, statically linked where feasible (`libsodium` yes, `ffmpeg` via `ffmpeg-next`'s static build feature, `onnxruntime` shipped as a dylib in the app bundle).

```toml
# rust-toolchain.toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

---

## 9. Security rules (short version)

- **Never log secrets.** `tracing` formatters for key types must redact.
- **Never concatenate user input into SQL strings.** All queries parameterized.
- **Timing:** constant-time comparisons for any secret comparison (use `subtle`).
- **Errors:** decryption failures never reveal *why* (key wrong vs. data tampered vs. truncated).
- **Rate limits:** password unlock attempts throttled (exponential backoff). Lock-out on 10 consecutive failures until the app is killed.
- **Memory:** sensitive buffers zeroized on drop. Don't `.to_vec()` a `Secret` unless you zeroize.
- **FFI:** every FFI call checked for return code; no assumed success.
- **Hidden vault:** its existence never appears in error messages, UI state, or logs unless it is unlocked. Plausible-deniability is a hard requirement.

---

## 10. Glossary

- **Vault**: the on-disk encrypted library (`<vault>/cas/` + `<vault>/index.db`).
- **Source**: a user-added backup dump, recorded in the `source` table.
- **Asset**: a canonical, deduplicated piece of media (one row in `asset`). Many `asset_location` rows may point at it.
- **Collection**: album, smart album, hidden-vault, trip, or memory — anything with members and a key.
- **Peer**: another install of this app, paired with you via an Iroh ticket.
- **Derivative**: a generated thumb/proxy/keyframe of an asset, stored in the CAS under its own hash.

---

## 11. What this document doesn't cover

- UX copy, color palette, icon set — phase docs lay these out per-screen.
- Model weights' licenses — see each phase's notes.
- Performance targets — phase docs state specific SLOs; this doc ensures the architecture *can* meet them.

If a phase doc conflicts with this document, **this document wins**. Open a conversation with the human instead of diverging.
