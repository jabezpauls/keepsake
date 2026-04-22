//! ML pipeline status + progress commands.

use std::path::Path;
#[cfg(feature = "ml-models")]
use std::sync::Arc;

use mv_core::db;
use tauri::State;

use crate::dto::MlStatus;
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

/// Post-unlock hook: try to bootstrap the ML runtime, install a per-asset key
/// resolver, and spawn the background drain loop.
///
/// All three steps are best-effort — if the runtime can't load (missing
/// weights, checksum mismatch, ORT dylib missing) the worker still drains
/// model-free jobs, and CLIP-backed features simply no-op until weights
/// arrive. Callers never block on this path.
pub fn try_bootstrap_runtime(session: &Session, vault_root: &Path) {
    // Scope the ml-models work inside cfg so default builds stay unchanged.
    #[cfg(feature = "ml-models")]
    {
        let model_dir = std::env::var_os("MV_MODELS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| vault_root.join("models"));
        let cfg = mv_core::ml::MlConfig {
            model_dir,
            execution_provider: mv_core::ml::ExecutionProvider::Auto,
        };
        match mv_core::ml::MlRuntime::load(cfg) {
            Ok(rt) => {
                let arc = Arc::new(rt);
                *session.ml_runtime.lock().unwrap() = Some(arc.clone());
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
                tracing::info!(
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

    // Drain loop: always spawn, even off-flag, so pure-Rust job kinds get
    // retired from the queue.
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
