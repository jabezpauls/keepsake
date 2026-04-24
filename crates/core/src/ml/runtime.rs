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

#[cfg(feature = "ml-models")]
use crate::crypto::CollectionKey;

/// Resolver handed to the ML worker so it can fetch the right per-asset
/// collection key at inference time. The app wires this at unlock, returning
/// `None` when the caller's key material isn't unlocked (jobs then fail with
/// `Error::Locked` rather than silently succeeding).
#[cfg(feature = "ml-models")]
pub type KeyResolver = Arc<dyn Fn(i64) -> Option<CollectionKey> + Send + Sync>;

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

    /// Session accessors for external callers (examples, downstream evals).
    /// Returned handles are cheap `SharedSession` clones.
    #[cfg(feature = "ml-models")]
    pub fn clip_visual(&self) -> super::loader::SharedSession {
        self.sessions.clip_visual.clone()
    }
    #[cfg(feature = "ml-models")]
    pub fn clip_textual(&self) -> super::loader::SharedSession {
        self.sessions.clip_textual.clone()
    }
    #[cfg(feature = "ml-models")]
    pub fn scrfd(&self) -> super::loader::SharedSession {
        self.sessions.scrfd.clone()
    }
    #[cfg(feature = "ml-models")]
    pub fn arcface(&self) -> super::loader::SharedSession {
        self.sessions.arcface.clone()
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
    /// Run PaddleOCR (det → cls → rec) and index tokens into
    /// `asset_text` under the owner's blind search key.
    Ocr,
    /// Run the pets classifier (binary dog/cat/bird/none) and
    /// upsert `asset.is_pet` + `pet_species_ct`.
    ClassifyPet,
}

impl MlJobKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EmbedAsset => "embed_asset",
            Self::DetectFaces => "detect_faces",
            Self::Phash => "phash",
            Self::RebuildPersonClusters => "rebuild_person_clusters",
            Self::RebuildNearDup => "rebuild_near_dup",
            Self::Ocr => "ocr",
            Self::ClassifyPet => "classify_pet",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "embed_asset" => Self::EmbedAsset,
            "detect_faces" => Self::DetectFaces,
            "phash" => Self::Phash,
            "rebuild_person_clusters" => Self::RebuildPersonClusters,
            "rebuild_near_dup" => Self::RebuildNearDup,
            "ocr" => Self::Ocr,
            "classify_pet" => Self::ClassifyPet,
            _ => return None,
        })
    }

    /// Whether this job kind needs the on-device ML runtime. Off-flag it's
    /// drained from the queue with a `ModelsUnavailable` error rather than
    /// executed.
    pub fn needs_models(&self) -> bool {
        matches!(
            self,
            Self::EmbedAsset
                | Self::DetectFaces
                | Self::RebuildPersonClusters
                | Self::Ocr
                | Self::ClassifyPet
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
    #[cfg(feature = "ml-models")]
    key_resolver: std::sync::Mutex<Option<KeyResolver>>,
}

impl MlWorker {
    pub fn new(db: Arc<tokio::sync::Mutex<rusqlite::Connection>>, cas: Arc<CasStore>) -> Self {
        Self {
            inner: Arc::new(MlWorkerInner {
                db,
                cas,
                runtime: std::sync::Mutex::new(None),
                #[cfg(feature = "ml-models")]
                key_resolver: std::sync::Mutex::new(None),
            }),
        }
    }

    /// Install / replace the per-asset key resolver. Called by the app at
    /// unlock time. Absent a resolver, any job requiring key material fails
    /// with `Error::Locked`.
    #[cfg(feature = "ml-models")]
    pub fn set_key_resolver(&self, resolver: KeyResolver) {
        *self.inner.key_resolver.lock().unwrap() = Some(resolver);
    }

    /// Attempt to load the runtime. Idempotent; silent on `ModelsUnavailable`.
    pub fn try_load_runtime(&self, config: MlConfig) {
        if let Ok(rt) = MlRuntime::load(config) {
            *self.inner.runtime.lock().unwrap() = Some(Arc::new(rt));
        }
    }

    /// Install an already-built runtime (e.g. one the app also keeps in its
    /// session for search re-rank). Both the worker and the session observe
    /// the same `Arc<MlRuntime>` so draining and search stay consistent.
    pub fn set_runtime(&self, runtime: Arc<MlRuntime>) {
        *self.inner.runtime.lock().unwrap() = Some(runtime);
    }

    /// Drain one job off the queue. Returns the kind that was run (if any) and
    /// whether it completed successfully.
    pub async fn drain_one(&self) -> Result<Option<(MlJobKind, Result<()>)>> {
        let db = self.inner.db.clone();
        let cas = self.inner.cas.clone();
        let runtime = self.inner.runtime.lock().unwrap().clone();
        #[cfg(feature = "ml-models")]
        let resolver = self.inner.key_resolver.lock().unwrap().clone();

        tokio::task::spawn_blocking(move || {
            #[cfg(feature = "ml-models")]
            {
                drain_blocking(&db, &cas, runtime.as_deref(), resolver.as_ref())
            }
            #[cfg(not(feature = "ml-models"))]
            {
                let _ = (&cas, &runtime);
                drain_blocking(&db)
            }
        })
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

#[cfg(feature = "ml-models")]
fn drain_blocking(
    db: &tokio::sync::Mutex<rusqlite::Connection>,
    cas: &CasStore,
    runtime: Option<&MlRuntime>,
    resolver: Option<&KeyResolver>,
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

    let outcome: Result<()> = if kind.needs_models() {
        match (runtime, resolver) {
            (None, _) => Err(Error::ModelsUnavailable),
            (_, None) => Err(Error::Locked),
            (Some(rt), Some(resolver)) => {
                let ck = |asset_id: i64| resolver(asset_id);
                match kind {
                    MlJobKind::EmbedAsset => match job.asset_id {
                        Some(asset_id) => {
                            super::worker_exec::run_embed_asset(&conn, cas, rt, asset_id, &ck)
                        }
                        None => Err(Error::Ingest("embed_asset: missing asset_id".into())),
                    },
                    MlJobKind::DetectFaces => match job.asset_id {
                        Some(asset_id) => {
                            super::worker_exec::run_detect_faces(&conn, cas, rt, asset_id, &ck)
                        }
                        None => Err(Error::Ingest("detect_faces: missing asset_id".into())),
                    },
                    MlJobKind::RebuildPersonClusters => {
                        super::worker_exec::run_rebuild_person_clusters(&conn, &ck)
                    }
                    // D5/D9 follow-ups — a future `worker_exec::run_ocr` /
                    // `run_classify_pet` drops in here. Until then the
                    // queue drains these with ModelsUnavailable so the
                    // ingest path doesn't wedge on pending rows.
                    MlJobKind::Ocr | MlJobKind::ClassifyPet => Err(Error::ModelsUnavailable),
                    _ => unreachable!("needs_models covered above"),
                }
            }
        }
    } else {
        // Pure-Rust kinds (Phash, RebuildNearDup) — Phase 2 runs pHash inline
        // during ingest, and near-dup rebuild is an explicit command, so
        // these jobs are purely a "mark processed" signal.
        Ok(())
    };

    match &outcome {
        Ok(()) => crate::db::finish_ml_job(&conn, job.id, now)?,
        Err(e) => crate::db::fail_ml_job(&conn, job.id, &e.to_string(), now)?,
    }
    Ok(Some((kind, outcome)))
}

#[cfg(not(feature = "ml-models"))]
fn drain_blocking(
    db: &tokio::sync::Mutex<rusqlite::Connection>,
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
    let outcome: Result<()> = if kind.needs_models() {
        Err(Error::ModelsUnavailable)
    } else {
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
