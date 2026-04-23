//! Search commands — map SearchRequest DTO → mv_core::search executor.

use chrono::{TimeZone, Utc};
use mv_core::db;
use mv_core::search::{self, SearchQuery};
use tauri::State;

use crate::dto::{SearchHitView, SearchRequest};
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

#[tauri::command]
pub async fn search_assets(
    state: State<'_, AppState>,
    request: SearchRequest,
) -> Result<Vec<SearchHitView>, String> {
    wire(search_assets_impl(&state, request).await)
}

async fn search_assets_impl(state: &AppState, req: SearchRequest) -> AppResult<Vec<SearchHitView>> {
    let (db_handle, ck, runtime) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        let rt = s.ml_runtime.lock().unwrap().clone();
        (s.db.clone(), s.default_collection_key.clone(), rt)
    };

    tokio::task::spawn_blocking(move || -> AppResult<Vec<SearchHitView>> {
        let guard = db_handle.blocking_lock();
        let date_range = match (req.after_day, req.before_day) {
            (Some(a), Some(b)) => Some((
                Utc.timestamp_opt(a * 86_400, 0)
                    .single()
                    .unwrap_or_else(Utc::now),
                Utc.timestamp_opt(b * 86_400, 0)
                    .single()
                    .unwrap_or_else(Utc::now),
            )),
            _ => None,
        };
        let q = SearchQuery {
            text: req.text.filter(|s| !s.is_empty()),
            person_ids: req.person_ids,
            date_range,
            source_id: req.source_id,
            has_faces: req.has_faces,
            is_video: req.is_video,
            is_raw: req.is_raw,
            is_screenshot: req.is_screenshot,
            is_live: req.is_live,
            camera_make: req.camera_make,
            lens: req.lens,
            limit: req.limit.clamp(1, 500),
            // Threading the session's hidden-unlock flag lands in the next
            // commit (C5); hold the safe default here per architecture.md §9.
            hidden_vault_unlocked: false,
        };
        // Runtime may be `None` (off-flag build or models not loaded yet).
        // `search()` handles that case and falls back to date-ordered hits.
        let hits = search::search(&guard, &q, Some(&ck), runtime.as_deref())?;
        let ids: Vec<i64> = hits.iter().map(|h| h.asset_id).collect();
        let rows = db::list_timeline_by_ids(&guard, &ids)?;
        let scores: std::collections::HashMap<i64, Option<f32>> =
            hits.iter().map(|h| (h.asset_id, h.score)).collect();
        Ok(rows
            .into_iter()
            .map(|r| SearchHitView {
                id: r.id,
                score: scores.get(&r.id).copied().unwrap_or(None),
                taken_at_utc_day: r.taken_at_utc_day,
                mime: r.mime,
                is_video: r.is_video,
                is_live: r.is_live,
            })
            .collect())
    })
    .await
    .map_err(AppError::from)?
}
