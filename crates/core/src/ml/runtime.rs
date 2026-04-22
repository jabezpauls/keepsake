//! On-device ML runtime + persistent job queue.
//!
//! The runtime itself is **feature-gated** (`ml-models`) because it depends on
//! ONNX Runtime and ~2 GB of model weights. With the flag off,
//! [`MlRuntime::load`] returns `Err(Error::ModelsUnavailable)` and every job
//! kind resolves to a no-op result the worker can still drain from the queue.
//!
//! [`MlJobKind`] and the [`MlWorker`] plumbing are always
//! available so the UI can render "ML work queued: N" status regardless of
//! build configuration.

use std::path::PathBuf;
use std::sync::Arc;

use crate::cas::CasStore;
use crate::{Error, Result};

/// Configuration for loading the ML runtime.
#[derive(Debug, Clone)]
pub struct MlConfig {
    /// Directory containing ONNX model files. Names inside are fixed by
    /// `scripts/download_models.sh`.
    pub model_dir: PathBuf,
    /// Preferred execution provider. `auto` tries CUDA/CoreML then falls back
    /// to CPU (MLAS).
    pub execution_provider: ExecutionProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionProvider {
    Auto,
    Cpu,
    Cuda,
    CoreMl,
}

/// Live runtime: sessions + config. On-flag, `load` walks the manifest and
/// builds four ONNX sessions; off-flag, it always returns `ModelsUnavailable`.
pub struct MlRuntime {
    #[allow(dead_code)]
    config: MlConfig,
    #[cfg(feature = "ml-models")]
    pub(crate) sessions: super::loader::Sessions,
    #[cfg(feature = "ml-models")]
    #[allow(dead_code)] // wired through worker_exec + search in later commits
    pub(crate) tokenizer: super::tokenizer::ClipTokenizer,
}

impl MlRuntime {
    /// Load the runtime. Off-flag this is always `Err(ModelsUnavailable)`.
    /// On-flag: verifies each model's SHA-256 against `ml::manifest`, then
    /// builds sessions with the preferred provider plus CPU fallback.
    #[cfg(feature = "ml-models")]
    pub fn load(config: MlConfig) -> Result<Self> {
        let sessions = super::loader::load_all(&config.model_dir, config.execution_provider)?;
        let tokenizer = super::tokenizer::ClipTokenizer::load(&config.model_dir)?;
        Ok(Self {
            config,
            sessions,
            tokenizer,
        })
    }

    #[cfg(not(feature = "ml-models"))]
    pub fn load(_config: MlConfig) -> Result<Self> {
        Err(Error::ModelsUnavailable)
    }

    /// Human-readable execution-provider label ("Auto" / "Cpu" / ...). Used by
    /// the UI to render where inference is actually running.
    #[cfg(feature = "ml-models")]
    pub fn provider_label(&self) -> &str {
        &self.sessions.provider_label
    }

    #[cfg(not(feature = "ml-models"))]
    pub fn provider_label(&self) -> &'static str {
        "disabled"
    }
}

/// Every kind of work the ML worker pool can run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MlJobKind {
    /// Run CLIP on one asset's thumbnail and upsert `asset_vec`.
    EmbedAsset,
    /// Run SCRFD → ArcFace on one asset and insert face rows.
    DetectFaces,
    /// Pure-Rust dhash + upsert `phash` (runs without ml-models flag).
    Phash,
    /// Run the DBSCAN clustering pass over all face embeddings.
    RebuildPersonClusters,
    /// Near-dup cluster rebuild (runs without ml-models flag).
    RebuildNearDup,
}

impl MlJobKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EmbedAsset => "embed_asset",
            Self::DetectFaces => "detect_faces",
            Self::Phash => "phash",
            Self::RebuildPersonClusters => "rebuild_person_clusters",
            Self::RebuildNearDup => "rebuild_near_dup",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "embed_asset" => Self::EmbedAsset,
            "detect_faces" => Self::DetectFaces,
            "phash" => Self::Phash,
            "rebuild_person_clusters" => Self::RebuildPersonClusters,
            "rebuild_near_dup" => Self::RebuildNearDup,
            _ => return None,
        })
    }

    /// Whether this job kind needs the on-device ML runtime. Off-flag it's
    /// drained from the queue with a `ModelsUnavailable` error rather than
    /// executed.
    pub fn needs_models(&self) -> bool {
        matches!(
            self,
            Self::EmbedAsset | Self::DetectFaces | Self::RebuildPersonClusters
        )
    }
}

/// Worker handle. Methods are no-ops when the feature flag is off — the queue
/// still drains, it just reports `ModelsUnavailable` on jobs that need models.
#[derive(Clone)]
pub struct MlWorker {
    inner: Arc<MlWorkerInner>,
}

struct MlWorkerInner {
    db: Arc<tokio::sync::Mutex<rusqlite::Connection>>,
    cas: Arc<CasStore>,
    runtime: std::sync::Mutex<Option<Arc<MlRuntime>>>,
}

impl MlWorker {
    pub fn new(db: Arc<tokio::sync::Mutex<rusqlite::Connection>>, cas: Arc<CasStore>) -> Self {
        Self {
            inner: Arc::new(MlWorkerInner {
                db,
                cas,
                runtime: std::sync::Mutex::new(None),
            }),
        }
    }

    /// Attempt to load the runtime. Idempotent; silent on `ModelsUnavailable`.
    pub fn try_load_runtime(&self, config: MlConfig) {
        if let Ok(rt) = MlRuntime::load(config) {
            *self.inner.runtime.lock().unwrap() = Some(Arc::new(rt));
        }
    }

    /// Drain one job off the queue. Returns the kind that was run (if any) and
    /// whether it completed successfully.
    pub async fn drain_one(&self) -> Result<Option<(MlJobKind, Result<()>)>> {
        let db = self.inner.db.clone();
        let cas = self.inner.cas.clone();
        let has_rt = self.inner.runtime.lock().unwrap().is_some();

        tokio::task::spawn_blocking(move || drain_blocking(&db, &cas, has_rt))
            .await
            .map_err(|_| Error::Ingest("worker join".into()))?
    }

    /// Snapshot counts by state for the UI.
    pub async fn state_counts(&self) -> Result<Vec<(String, i64)>> {
        let db = self.inner.db.clone();
        tokio::task::spawn_blocking(move || {
            let conn = db.blocking_lock();
            crate::db::count_ml_jobs_by_state(&conn)
        })
        .await
        .map_err(|_| Error::Ingest("worker join".into()))?
    }
}

fn drain_blocking(
    db: &tokio::sync::Mutex<rusqlite::Connection>,
    _cas: &CasStore,
    has_rt: bool,
) -> Result<Option<(MlJobKind, Result<()>)>> {
    let conn = db.blocking_lock();
    let now = chrono::Utc::now().timestamp();
    let Some(job) = crate::db::claim_next_ml_job(&conn, now)? else {
        return Ok(None);
    };
    let Some(kind) = MlJobKind::from_str(&job.kind) else {
        crate::db::fail_ml_job(&conn, job.id, "unknown kind", now)?;
        return Ok(Some((
            MlJobKind::Phash,
            Err(Error::Ingest("unknown kind".into())),
        )));
    };

    let outcome = if kind.needs_models() && !has_rt {
        Err(Error::ModelsUnavailable)
    } else {
        // The pure-Rust kinds (Phash, RebuildNearDup) would run here; for
        // Phase 2 the pHash pass is done inline during ingest, so Phash
        // jobs are purely a "mark this asset processed" signal and we
        // succeed unconditionally. RebuildNearDup is handled by an
        // explicit command path rather than the queue today.
        Ok(())
    };
    match &outcome {
        Ok(()) => crate::db::finish_ml_job(&conn, job.id, now)?,
        Err(e) => crate::db::fail_ml_job(&conn, job.id, &e.to_string(), now)?,
    }
    Ok(Some((kind, outcome)))
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_load_errors_without_feature_flag() {
        let cfg = MlConfig {
            model_dir: PathBuf::from("/nonexistent"),
            execution_provider: ExecutionProvider::Auto,
        };
        let r = MlRuntime::load(cfg);
        assert!(matches!(r, Err(Error::ModelsUnavailable)));
    }

    #[test]
    fn job_kind_string_round_trip() {
        for kind in [
            MlJobKind::EmbedAsset,
            MlJobKind::DetectFaces,
            MlJobKind::Phash,
            MlJobKind::RebuildPersonClusters,
            MlJobKind::RebuildNearDup,
        ] {
            let s = kind.as_str();
            assert_eq!(MlJobKind::from_str(s), Some(kind));
        }
        assert_eq!(MlJobKind::from_str("garbage"), None);
    }

    #[tokio::test]
    async fn worker_drains_off_flag_with_models_unavailable() {
        use crate::cas::CasStore;
        use crate::db;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let cas = Arc::new(CasStore::open(tmp.path()).unwrap());
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate::apply(&conn).unwrap();
        let db_ref = Arc::new(tokio::sync::Mutex::new(conn));

        db::enqueue_ml_job(&*db_ref.lock().await, "embed_asset", None, 0).unwrap();
        let worker = MlWorker::new(db_ref.clone(), cas);
        let got = worker.drain_one().await.unwrap().unwrap();
        assert_eq!(got.0, MlJobKind::EmbedAsset);
        assert!(matches!(got.1, Err(Error::ModelsUnavailable)));

        let counts = worker.state_counts().await.unwrap();
        assert!(counts.iter().any(|(s, n)| s == "failed" && *n == 1));
    }
}
