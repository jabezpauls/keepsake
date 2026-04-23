//! Tauri app state.
//!
//! The state has two phases:
//!
//! * **Locked** — only `vault_root` is set; every command other than
//!   `user_exists`, `create_user`, `unlock` returns `AppError::Locked`.
//! * **Unlocked** — a [`Session`] holds the in-memory key material plus the
//!   CAS + DB handles. Dropping the session zeroes the key material.
//!
//! All mutable fields are guarded behind a single async `Mutex`. The Mutex
//! is held only while filling/reading state; long-running operations (ingest,
//! CAS reads) clone `Arc<_>` handles out and release the lock before doing
//! IO.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mv_core::cas::CasStore;
use mv_core::crypto::keystore::{UnlockedUser, UserRecord};
use mv_core::crypto::CollectionKey;
use mv_core::ml::{MlRuntime, MlWorker};
use tokio::sync::Mutex;

use crate::dto::IngestStatus;

pub struct AppState {
    pub vault_root: PathBuf,
    pub inner: Mutex<Inner>,
}

#[derive(Default)]
pub struct Inner {
    pub session: Option<Session>,
}

pub struct Session {
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub cas: Arc<CasStore>,
    pub user: Arc<UnlockedUser>,
    pub user_record: UserRecord,
    pub username: String,
    pub default_collection_id: i64,
    pub default_collection_key: Arc<CollectionKey>,
    /// Non-default collection keys that have been unwrapped via the master key
    /// (no password) or an album password. Keyed by `collection.id`.
    pub unlocked_albums: HashMap<i64, Arc<CollectionKey>>,
    pub hidden_unlocked: bool,
    /// In-memory view of ingest progress, keyed by source_id.
    pub ingests: Arc<Mutex<HashMap<i64, IngestStatus>>>,
    /// Background ML worker — always present so `ml_status` can report queue
    /// counts even with weights missing. Runtime is lazy-loaded.
    pub ml_worker: MlWorker,
    /// Loaded ML runtime. `None` until `try_load_models` succeeds (off-flag
    /// or missing weights keep this `None`; search + worker fall back to
    /// their metadata-only paths).
    pub ml_runtime: std::sync::Mutex<Option<Arc<MlRuntime>>>,
    /// Live Iroh peer for Phase 3+ sync. `None` until the first Peers-tab
    /// operation triggers a bootstrap; held in a tokio Mutex so async
    /// commands can await the slow endpoint bind without blocking the
    /// shared-state lock.
    pub peer: Mutex<Option<Arc<mv_sync::Peer>>>,
}

impl AppState {
    pub fn new(vault_root: PathBuf) -> Self {
        Self {
            vault_root,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn index_db_path(&self) -> PathBuf {
        self.vault_root.join("index.db")
    }
}

/// Determine the default vault root. Prefers `$MV_VAULT_ROOT`, falls back to
/// `$XDG_DATA_HOME/media-vault` / `~/.local/share/media-vault`.
pub fn default_vault_root() -> PathBuf {
    if let Ok(env) = std::env::var("MV_VAULT_ROOT") {
        return PathBuf::from(env);
    }
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("media-vault")
}

/// Ensure the vault root and its required subdirectories exist.
pub fn ensure_vault_dirs(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    std::fs::create_dir_all(root.join("cas"))?;
    Ok(())
}
