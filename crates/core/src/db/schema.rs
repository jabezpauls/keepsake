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
