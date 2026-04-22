//! ML pipeline status + progress commands.

use mv_core::db;
use tauri::State;

use crate::dto::MlStatus;
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

#[tauri::command]
pub async fn ml_status(state: State<'_, AppState>) -> Result<MlStatus, String> {
    wire(ml_status_impl(&state).await)
}

async fn ml_status_impl(state: &AppState) -> AppResult<MlStatus> {
    let db_handle = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        s.db.clone()
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
            pending: get("pending"),
            running: get("running"),
            done: get("done"),
            failed: get("failed"),
        })
    })
    .await
    .map_err(AppError::from)?
}
