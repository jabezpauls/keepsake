//! SQLite schema + connection setup.
//!
//! DDL is the exact text from `plans/architecture.md` §4.2, checked in as a
//! single string so an external reviewer can diff the two.

use std::path::Path;

use rusqlite::Connection;

use crate::{Error, Result};

/// DDL for schema version 1 (Phase 1). Do not change existing columns — only
/// add new tables/indexes via migrations in `migrate.rs`.
pub const DDL_V1: &str = r"
-- Users of this vault (Phase 1: exactly one row; Phase 3: many)
CREATE TABLE IF NOT EXISTS user (
    id                   INTEGER PRIMARY KEY,
    username_ct          BLOB NOT NULL,           -- display name, encrypted
    main_salt            BLOB NOT NULL,           -- 16 bytes Argon2id salt
    wrapped_master_key   BLOB NOT NULL,           -- secure via KEK derived from password
    hidden_wrapped_master_key BLOB NOT NULL,      -- placeholder or real; always present
    hidden_salt          BLOB NOT NULL,           -- 16 bytes Argon2id salt for hidden vault
    recovery_blob        BLOB,                    -- optional BIP39-wrapped master key
    identity_pub         BLOB NOT NULL,           -- X25519 public key (for sharing)
    wrapped_identity_sk  BLOB NOT NULL,           -- X25519 secret, wrapped by masterKey
    iroh_node_pub        BLOB NOT NULL,           -- Ed25519, Phase 3 uses it
    wrapped_iroh_sk      BLOB NOT NULL,           -- Ed25519 secret, wrapped by masterKey
    created_at           INTEGER NOT NULL
);

-- Sources (backup dumps). Append-only.
CREATE TABLE IF NOT EXISTS source (
    id                INTEGER PRIMARY KEY,
    owner_id          INTEGER NOT NULL REFERENCES user(id),
    name_ct           BLOB NOT NULL,              -- user-facing name
    root_path_ct      BLOB NOT NULL,              -- absolute path, encrypted
    device_hint_ct    BLOB,                       -- 'iPhone 15', 'Canon R5', etc.
    imported_at       INTEGER NOT NULL,
    bytes_total       INTEGER NOT NULL,
    file_count        INTEGER NOT NULL,
    linked_only       INTEGER NOT NULL DEFAULT 0  -- 0 = copied into CAS; 1 = link-don't-copy
);

-- Canonical assets. One row per unique plaintext hash.
CREATE TABLE IF NOT EXISTS asset (
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

    filename_ct          BLOB NOT NULL,
    taken_at_utc_ct      BLOB,
    gps_ct               BLOB,
    device_ct            BLOB,
    lens_ct              BLOB,
    exif_all_ct          BLOB,
    clip_embedding_ct    BLOB,                    -- 768-d f32 array, Phase 2
    ocr_text_ct          BLOB,                    -- Phase 3
    transcript_ct        BLOB,                    -- Phase 4
    color_palette_ct     BLOB,                    -- dominant colors, Phase 2
    user_notes_ct        BLOB,
    tags_ct              BLOB,                    -- json array of tags
    wrapped_file_key     BLOB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_asset_taken_at_utc_day ON asset(taken_at_utc_day);
CREATE INDEX IF NOT EXISTS idx_asset_source_id ON asset(source_id);
CREATE INDEX IF NOT EXISTS idx_asset_flags ON asset(is_live, is_motion, is_raw, is_screenshot);

-- Many-to-one physical locations.
CREATE TABLE IF NOT EXISTS asset_location (
    asset_id         INTEGER NOT NULL REFERENCES asset(id),
    source_id        INTEGER NOT NULL REFERENCES source(id),
    original_path_ct BLOB NOT NULL,
    mtime            INTEGER NOT NULL,
    PRIMARY KEY (asset_id, source_id, original_path_ct)
);

-- Collections: albums, hidden vault, smart albums, trips, memories.
CREATE TABLE IF NOT EXISTS collection (
    id                INTEGER PRIMARY KEY,
    owner_id          INTEGER NOT NULL REFERENCES user(id),
    kind              TEXT NOT NULL CHECK (kind IN
                        ('album','hidden_vault','smart_album','trip','memory')),
    name_ct           BLOB NOT NULL,
    cover_asset_id    INTEGER REFERENCES asset(id),
    has_password      INTEGER NOT NULL DEFAULT 0,
    password_salt     BLOB,
    created_at        INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS collection_member (
    collection_id  INTEGER NOT NULL REFERENCES collection(id),
    asset_id       INTEGER NOT NULL REFERENCES asset(id),
    added_at       INTEGER NOT NULL,
    PRIMARY KEY (collection_id, asset_id)
);

CREATE TABLE IF NOT EXISTS collection_key (
    collection_id  INTEGER NOT NULL REFERENCES collection(id),
    user_id        INTEGER NOT NULL REFERENCES user(id),
    wrapping       TEXT NOT NULL,                 -- 'master' | 'album_password' | 'peer_x25519'
    wrapped_key    BLOB NOT NULL,
    PRIMARY KEY (collection_id, user_id, wrapping)
);

-- Faces, people. Phase 2.
CREATE TABLE IF NOT EXISTS person (
    id               INTEGER PRIMARY KEY,
    owner_id         INTEGER NOT NULL REFERENCES user(id),
    name_ct          BLOB,
    relations_ct     BLOB,
    hidden           INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS face (
    id              INTEGER PRIMARY KEY,
    asset_id        INTEGER NOT NULL REFERENCES asset(id),
    person_id       INTEGER REFERENCES person(id),
    quality         REAL,
    bbox_ct         BLOB NOT NULL,
    embedding_ct    BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS phash (
    asset_id  INTEGER PRIMARY KEY REFERENCES asset(id),
    hash      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS edit (
    asset_id  INTEGER PRIMARY KEY REFERENCES asset(id),
    ops_ct    BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS derivative (
    asset_id   INTEGER NOT NULL REFERENCES asset(id),
    kind       TEXT NOT NULL,
    cas_ref    TEXT NOT NULL,
    PRIMARY KEY (asset_id, kind)
);
";

/// DDL additions shipped in schema version 2 (Phase 2). Additive only.
///
/// Adds:
/// - `ml_job` persistent worker queue (plaintext — leaks work-load size only).
/// - `nd_cluster` near-duplicate cluster membership (plaintext — pHash is
///   already leaky so cluster shape is no worse).
/// - `asset_vec` plaintext CLIP embedding cache; similarity structure leaks
///   by design per architecture.md §4.3.
/// - `path_hash` column + partial unique index on `asset_location` so re-ingest
///   of an unchanged path becomes a true no-op.
pub const DDL_V2: &str = r"
CREATE TABLE IF NOT EXISTS ml_job (
    id            INTEGER PRIMARY KEY,
    kind          TEXT NOT NULL,
    asset_id      INTEGER REFERENCES asset(id),
    state         TEXT NOT NULL DEFAULT 'pending',
    attempts      INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ml_job_state ON ml_job(state, kind);
CREATE INDEX IF NOT EXISTS idx_ml_job_asset ON ml_job(asset_id, kind);

CREATE TABLE IF NOT EXISTS nd_cluster (
    cluster_id  INTEGER NOT NULL,
    asset_id    INTEGER NOT NULL REFERENCES asset(id),
    is_best     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (cluster_id, asset_id)
);
CREATE INDEX IF NOT EXISTS idx_nd_cluster_asset ON nd_cluster(asset_id);

CREATE TABLE IF NOT EXISTS asset_vec (
    asset_id   INTEGER PRIMARY KEY REFERENCES asset(id),
    embedding  BLOB NOT NULL
);

ALTER TABLE asset_location ADD COLUMN path_hash BLOB;
CREATE UNIQUE INDEX IF NOT EXISTS idx_asset_location_path_hash
    ON asset_location(asset_id, source_id, path_hash)
    WHERE path_hash IS NOT NULL;
";

/// DDL v3 — Phase 3.1 additive migration. Adds `peer_accept`, the plaintext
/// table of accepted peer identities. `peer_node_id` is plaintext per
/// architecture.md §9 (an attacker with vault access already sees which
/// peers you collaborate with; the bigger leak is in file metadata). The
/// optional note is master-key sealed.
pub const DDL_V3: &str = r"
CREATE TABLE IF NOT EXISTS peer_accept (
    peer_node_id      BLOB PRIMARY KEY,
    peer_identity_pub BLOB NOT NULL,
    owner_user_id     INTEGER NOT NULL REFERENCES user(id),
    relay_url         TEXT,
    added_at          INTEGER NOT NULL,
    note_ct           BLOB
);
CREATE INDEX IF NOT EXISTS idx_peer_accept_owner
    ON peer_accept(owner_user_id, added_at DESC);
";

/// DDL v4 — Phase 3.2. Reshapes `collection_key` to accept peer-wrapped rows
/// (collection key sealed for a remote X25519 identity, not a local user).
/// Adds `ciphertext_blake3` to `asset` so the iroh-blobs bridge can address
/// the on-disk ciphertext by its BLAKE3 without rehashing every request.
///
/// The reshape uses SQLite's 12-step ALTER pattern: create new table, copy
/// existing rows, drop old, rename. `user_id` becomes NULLable so peer
/// wrappings can leave it NULL; `peer_identity_pub` is the new column and
/// is NULLable so local wrappings can leave it NULL. Two partial unique
/// indexes keep the `(cid, recipient, wrapping)` invariant on each side.
pub const DDL_V4: &str = r"
CREATE TABLE collection_key_v4 (
    collection_id     INTEGER NOT NULL REFERENCES collection(id),
    user_id           INTEGER REFERENCES user(id),
    peer_identity_pub BLOB,
    wrapping          TEXT NOT NULL,
    wrapped_key       BLOB NOT NULL
);
INSERT INTO collection_key_v4 (collection_id, user_id, peer_identity_pub, wrapping, wrapped_key)
    SELECT collection_id, user_id, NULL, wrapping, wrapped_key FROM collection_key;
DROP TABLE collection_key;
ALTER TABLE collection_key_v4 RENAME TO collection_key;

CREATE UNIQUE INDEX idx_collection_key_local
    ON collection_key(collection_id, user_id, wrapping)
    WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX idx_collection_key_peer
    ON collection_key(collection_id, peer_identity_pub, wrapping)
    WHERE peer_identity_pub IS NOT NULL;

ALTER TABLE asset ADD COLUMN ciphertext_blake3 BLOB;
CREATE INDEX IF NOT EXISTS idx_asset_ciphertext_hash
    ON asset(ciphertext_blake3) WHERE ciphertext_blake3 IS NOT NULL;
";

/// Set up an open SQLite connection with the Phase-1 pragmas.
pub fn configure_connection(conn: &Connection) -> Result<()> {
    // WAL + foreign keys + synchronous=NORMAL per §4.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

/// Open (creating if needed) the vault's `index.db` for read/write.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    configure_connection(&conn)?;
    // The FTS5 and sqlite-vec virtual tables are declared by super::migrate::apply
    // once the extensions have been registered; §4.3.
    super::migrate::apply(&conn)?;
    Ok(conn)
}

/// Open the vault's `index.db` read-only. Does NOT run migrations.
pub fn open_readonly(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    Ok(conn)
}

/// Run the base DDL on a fresh connection. Exposed for in-memory tests.
pub fn init(conn: &Connection) -> Result<()> {
    configure_connection(conn)?;
    conn.execute_batch(DDL_V1).map_err(Error::from)?;
    // Virtual tables (FTS5 + sqlite-vec) — FTS5 is compiled into rusqlite's
    // bundled SQLite. sqlite-vec is declared but left unpopulated in Phase 1
    // (see phase-1-foundation.md §374). We declare an FTS5 virtual table now
    // and defer the vec table to the first CLIP migration in Phase 2.
    conn.execute_batch(
        r"
        CREATE VIRTUAL TABLE IF NOT EXISTS asset_text USING fts5(
            asset_id UNINDEXED,
            token_blind,
            tokenize = 'unicode61 remove_diacritics 2'
        );
    ",
    )?;
    Ok(())
}
