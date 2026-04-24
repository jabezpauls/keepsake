//! ML pipeline status + progress commands.

use std::path::Path;
#[cfg(feature = "ml-models")]
use std::sync::Arc;

use mv_core::{db, ml};
#[cfg(feature = "ml-models")]
use tauri::Emitter;
use tauri::{AppHandle, State};

use crate::dto::{MlReindexReport, MlStatus};
use crate::errors::{wire, AppError, AppResult};
use crate::state::{AppState, Session};

#[tauri::command]
pub async fn ml_status(state: State<'_, AppState>) -> Result<MlStatus, String> {
    wire(ml_status_impl(&state).await)
}

async fn ml_status_impl(state: &AppState) -> AppResult<MlStatus> {
    let (db_handle, runtime_loaded, provider) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        let rt = s.ml_runtime.lock().unwrap().clone();
        let provider = rt
            .as_ref()
            .map(|r| r.provider_label().to_string())
            .unwrap_or_else(|| "disabled".to_string());
        (s.db.clone(), rt.is_some(), provider)
    };
    tokio::task::spawn_blocking(move || -> AppResult<MlStatus> {
        let guard = db_handle.blocking_lock();
        let counts = db::count_ml_jobs_by_state(&guard)?;
        let get = |k: &str| {
            counts
                .iter()
                .find_map(|(s, n)| if s == k { Some(*n) } else { None })
                .unwrap_or(0)
        };
        Ok(MlStatus {
            models_available: mv_core::ml::MODELS_ENABLED,
            runtime_loaded,
            execution_provider: provider,
            pending: get("pending"),
            running: get("running"),
            done: get("done"),
            failed: get("failed"),
        })
    })
    .await
    .map_err(AppError::from)?
}

/// Compile-time flag — `true` when the app was built with `--features
/// ml-models`. Cheap command the UI can call before rendering the "models
/// unavailable" banner vs a "download models" affordance.
#[tauri::command]
pub fn ml_models_enabled() -> bool {
    mv_core::ml::MODELS_ENABLED
}

/// First-run wizard: which model files are present + valid.
///
/// Always returns a shape the UI can consume. Off-flag we report a synthetic
/// "feature disabled" snapshot so the wizard's UI gating has a single code
/// path.
#[tauri::command]
pub async fn ml_models_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    wire(ml_models_status_impl(&state).await)
}

async fn ml_models_status_impl(state: &AppState) -> AppResult<serde_json::Value> {
    #[cfg(feature = "ml-models")]
    {
        let model_dir = resolve_model_dir(state).await?;
        let snapshot = tokio::task::spawn_blocking(move || ml::downloader::survey(&model_dir))
            .await
            .map_err(AppError::from)??;
        Ok(serde_json::to_value(snapshot).map_err(|e| AppError::Ingest(e.to_string()))?)
    }
    #[cfg(not(feature = "ml-models"))]
    {
        let _ = state;
        Ok(serde_json::json!({
            "files": [],
            "all_present_valid": false,
        }))
    }
}

/// Start downloading missing / corrupt model files into the resolved models
/// directory. Streams [`ml::downloader::DownloadEvent`]s onto the
/// `ml-download-event` Tauri channel so the wizard can render live progress.
///
/// Returns when the download completes (Ok) or when at least one file failed
/// (Err). Either way the terminal `all_done` event fires — the error is a
/// coarse signal; the detailed per-file outcomes live on the channel.
///
/// After a successful run the caller should invoke [`ml_runtime_reload`] so
/// the badge flips from "no weights" to the actual execution provider.
#[tauri::command]
pub async fn ml_models_download(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    wire(ml_models_download_impl(app, &state).await)
}

#[cfg(feature = "ml-models")]
async fn ml_models_download_impl(app: AppHandle, state: &AppState) -> AppResult<()> {
    let model_dir = resolve_model_dir(state).await?;
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        let app_for_emit = app.clone();
        ml::downloader::download_missing(&model_dir, move |event| {
            // Best-effort emit: if the frontend isn't listening, we still
            // finish the download. Tauri's Emitter returns Err only when
            // the app handle itself is gone (window closed mid-download).
            let _ = app_for_emit.emit("ml-download-event", &event);
        })?;
        Ok(())
    })
    .await
    .map_err(AppError::from)??;
    Ok(())
}

#[cfg(not(feature = "ml-models"))]
async fn ml_models_download_impl(_app: AppHandle, _state: &AppState) -> AppResult<()> {
    Err(AppError::Ingest(
        "ml-models feature not compiled into this build".into(),
    ))
}

/// After a successful download, re-attempt runtime bootstrap so the ORT
/// sessions come alive without a full lock/unlock cycle. Safe to call any
/// time; idempotent when the runtime is already loaded (just re-runs the
/// load, replaces the Arc).
#[tauri::command]
pub async fn ml_runtime_reload(state: State<'_, AppState>) -> Result<MlStatus, String> {
    wire(ml_runtime_reload_impl(&state).await)
}

async fn ml_runtime_reload_impl(state: &AppState) -> AppResult<MlStatus> {
    #[cfg(feature = "ml-models")]
    {
        let vault_root = state.vault_root.clone();
        let guard = state.inner.lock().await;
        let session = guard.session.as_ref().ok_or(AppError::Locked)?;
        // Intentionally call the same best-effort hook used post-unlock. If
        // weights still don't validate, the runtime remains None and the
        // wizard can surface the failure through the refreshed ml_status.
        try_bootstrap_runtime_no_drain(session, &vault_root);
        drop(guard);
    }
    ml_status_impl(state).await
}

/// Resolve the effective model directory. Honours `MV_MODELS` first so devs
/// can point at an external pool, otherwise uses `<vault>/models/`.
#[cfg(feature = "ml-models")]
async fn resolve_model_dir(state: &AppState) -> AppResult<std::path::PathBuf> {
    if let Some(env) = std::env::var_os("MV_MODELS") {
        return Ok(std::path::PathBuf::from(env));
    }
    Ok(state.vault_root.join("models"))
}

/// Enqueue embed + detect jobs for every asset that hasn't had them yet.
/// Idempotent. Safe to run any time; even off-flag it populates the queue
/// so the work happens as soon as weights arrive.
#[tauri::command]
pub async fn ml_reindex(state: State<'_, AppState>) -> Result<MlReindexReport, String> {
    wire(ml_reindex_impl(&state).await)
}

async fn ml_reindex_impl(state: &AppState) -> AppResult<MlReindexReport> {
    let db_handle = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        s.db.clone()
    };
    tokio::task::spawn_blocking(move || -> AppResult<MlReindexReport> {
        let guard = db_handle.blocking_lock();
        let now = chrono::Utc::now().timestamp();
        let report = ml::reindex::reindex_all(&guard, now)?;
        Ok(MlReindexReport {
            embed_queued: report.embed_queued,
            detect_queued: report.detect_queued,
            assets_touched: report.assets_touched,
        })
    })
    .await
    .map_err(AppError::from)?
}

/// Post-unlock hook: try to bootstrap the ML runtime, install a per-asset key
/// resolver, and spawn the background drain loop.
///
/// All three steps are best-effort — if the runtime can't load (missing
/// weights, checksum mismatch, ORT dylib missing) the worker still drains
/// model-free jobs, and CLIP-backed features simply no-op until weights
/// arrive. Callers never block on this path.
pub fn try_bootstrap_runtime(session: &Session, vault_root: &Path) {
    try_bootstrap_runtime_no_drain(session, vault_root);

    // Drain loop: always spawn, even off-flag, so pure-Rust job kinds get
    // retired from the queue. Separated from runtime load so
    // `ml_runtime_reload` can re-load weights post-download without
    // double-spawning the loop.
    let worker = session.ml_worker.clone();
    tokio::spawn(async move {
        loop {
            match worker.drain_one().await {
                Ok(Some((_kind, _outcome))) => {}
                Ok(None) => {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                Err(err) => {
                    tracing::warn!(?err, "ml drain loop error; backing off");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    });
}

/// Load (or reload) the ML runtime into a session without spawning the drain
/// loop. Used post-download by `ml_runtime_reload` so freshly-fetched weights
/// activate without a lock/unlock cycle.
pub fn try_bootstrap_runtime_no_drain(session: &Session, vault_root: &Path) {
    #[cfg(feature = "ml-models")]
    {
        let model_dir = std::env::var_os("MV_MODELS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| vault_root.join("models"));
        // Always `Auto`. ORT silently ignores providers whose runtime deps
        // don't resolve, and `mv_core::ml::loader::resolve_actual_provider`
        // probes each candidate's dylibs so `ml_status.execution_provider`
        // reports the provider that actually registered — a build with
        // `ml-cuda` on a CPU-only machine just falls through to CPU cleanly.
        let cfg = mv_core::ml::MlConfig {
            model_dir,
            execution_provider: mv_core::ml::ExecutionProvider::Auto,
        };
        match mv_core::ml::MlRuntime::load(cfg) {
            Ok(rt) => {
                tracing::info!(
                    provider = %rt.provider_label(),
                    "ml runtime loaded"
                );
                let arc = Arc::new(rt);
                *session.ml_runtime.lock().unwrap() = Some(arc.clone());
                // Share the same runtime with the drain worker so it can run
                // model-backed jobs instead of failing every one with
                // `ModelsUnavailable`.
                session.ml_worker.set_runtime(arc.clone());
                // Give the worker a resolver that hands back the default
                // collection key for every asset. Phase 3 extends this for
                // password-album assets — for now, non-default-album assets
                // will fail with Error::Locked and skip.
                let ck_default = session.default_collection_key.clone();
                session
                    .ml_worker
                    .set_key_resolver(Arc::new(move |_asset_id: i64| {
                        // CollectionKey is not Clone by design (ZeroizeOnDrop).
                        // Rebuild from the unlocked copy via from_bytes — the
                        // bytes already live in memory, this just gives the
                        // worker an owned, zero-on-drop handle.
                        Some(mv_core::crypto::CollectionKey::from_bytes(
                            *ck_default.as_bytes(),
                        ))
                    }));
            }
            Err(e) => {
                tracing::warn!(
                    ?e,
                    "ml runtime did not load — search + worker keep metadata-only fallbacks"
                );
            }
        }
    }
    #[cfg(not(feature = "ml-models"))]
    {
        let _ = (session, vault_root);
    }
}
