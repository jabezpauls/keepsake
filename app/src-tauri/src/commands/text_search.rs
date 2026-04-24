//! D5 blind-indexed full-text search commands.
//!
//! Exposes two surfaces to the UI:
//! - [`index_asset_text`] — user-authored searchable text per asset
//!   (think "caption" or "tag"). Replaces any prior tokens for the
//!   asset. A future OCR worker will augment the same table.
//! - [`search_text_exact`] — whole-word AND search against the blind
//!   FTS5 index under the current user's search key.
//!
//! Both commands require an unlocked session — the per-user search
//! key is deterministically derived from the master key, so without
//! the master key there's nothing to hash against.

use mv_core::blind_text;
use mv_core::crypto::MasterKey;
use mv_core::db;
use tauri::State;

use crate::dto::{SearchHitView, TimelineEntryView};
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

#[tauri::command]
pub async fn index_asset_text(
    state: State<'_, AppState>,
    asset_id: i64,
    text: String,
) -> Result<u32, String> {
    wire(index_asset_text_impl(&state, asset_id, text).await)
}

async fn index_asset_text_impl(state: &AppState, asset_id: i64, text: String) -> AppResult<u32> {
    let (db_handle, master_bytes) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), *s.user.master_key.as_bytes())
    };
    let n = tokio::task::spawn_blocking(move || -> AppResult<usize> {
        let guard = db_handle.blocking_lock();
        let mk = MasterKey::from_bytes(master_bytes);
        let sk = blind_text::derive_search_key(&mk);
        let tokens = blind_text::tokenize(&text);
        Ok(blind_text::index_asset_tokens(
            &guard, asset_id, &sk, &tokens,
        )?)
    })
    .await
    .map_err(AppError::from)??;
    Ok(n as u32)
}

#[tauri::command]
pub async fn search_text_exact(
    state: State<'_, AppState>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<SearchHitView>, String> {
    wire(search_text_exact_impl(&state, query, limit).await)
}

async fn search_text_exact_impl(
    state: &AppState,
    query: String,
    limit: Option<u32>,
) -> AppResult<Vec<SearchHitView>> {
    let (db_handle, master_bytes) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), *s.user.master_key.as_bytes())
    };
    let lim = limit.unwrap_or(100).clamp(1, 500);
    tokio::task::spawn_blocking(move || -> AppResult<Vec<SearchHitView>> {
        let guard = db_handle.blocking_lock();
        let mk = MasterKey::from_bytes(master_bytes);
        let sk = blind_text::derive_search_key(&mk);
        let ids = blind_text::search_assets(&guard, &sk, &query, lim)?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = db::list_timeline_by_ids(&guard, &ids)?;
        Ok(rows
            .into_iter()
            .map(|r| SearchHitView {
                id: r.id,
                score: None,
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

// Suppress unused-import warnings when the DTO is only used for return
// typing above.
#[allow(dead_code)]
fn _ty_hint(_: TimelineEntryView) {}
