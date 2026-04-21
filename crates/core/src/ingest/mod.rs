//! Ingest pipeline — scan + encrypt + store + provenance.

pub mod adapters;
pub mod provenance;
pub mod sidecar;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};

use crate::cas::CasStore;
use crate::crypto::keystore::UnlockedUser;
use crate::crypto::CollectionKey;
use crate::Result;

pub use adapters::{generic::GenericAdapter, google_takeout::GoogleTakeoutAdapter, iphone_folder::IPhoneFolderAdapter};
pub use sidecar::{read_xmp_sidecar, write_xmp_sidecar, XmpFields};

/// Trait every adapter implements. `kind()` is returned by
/// `list_sources` so the UI can render the right icon.
#[async_trait::async_trait]
pub trait IngestAdapter: Send + Sync {
    fn kind(&self) -> &'static str;

    /// Scan the source root without writing. Returns the file count and
    /// total plaintext bytes so the UI can size a progress bar.
    async fn scan(&self, root: &Path) -> Result<ScanReport>;

    /// Perform the full ingest. Sends progress updates through `progress`.
    async fn ingest(
        &self,
        ctx: &IngestCtx,
        root: &Path,
        source_id: i64,
        progress: Progress,
    ) -> Result<IngestReport>;
}

#[derive(Debug, Default, Clone)]
pub struct ScanReport {
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Default, Clone)]
pub struct IngestReport {
    pub inserted: u64,
    pub deduped: u64,
    pub skipped: u64,
    pub errors: u64,
}

/// Context passed to adapters: the CAS handle, a unlocked-collection key for
/// the default "unsorted" album, and the DB pool.
pub struct IngestCtx {
    pub cas: Arc<CasStore>,
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub user: Arc<UnlockedUser>,
    pub default_collection_key: Arc<CollectionKey>,
}

/// Progress channel sender. Cloneable; drop all senders when done.
#[derive(Debug, Clone)]
pub struct Progress(pub mpsc::UnboundedSender<ProgressEvent>);

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    ScanStarted(PathBuf),
    FileStarted { path: PathBuf, index: u64, total: u64 },
    FileInserted { asset_id: i64, bytes: u64 },
    FileDeduped { asset_id: i64 },
    FileSkipped { path: PathBuf, reason: String },
    Done(IngestReport),
}

impl Progress {
    pub fn send(&self, ev: ProgressEvent) {
        let _ = self.0.send(ev);
    }
}
