//! Timeline + asset detail + binary-read commands.

use std::sync::Arc;

use mv_core::cas::CasStore;
use mv_core::crypto::{open_row, unwrap_file_key, CollectionKey};
use mv_core::db;
use tauri::State;

use crate::dto::{AssetDetailView, GpsView, TimelineCursor, TimelineEntryView, TimelinePage};
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

const DEFAULT_LIMIT: u32 = 120;

#[tauri::command]
pub async fn timeline_page(
    state: State<'_, AppState>,
    cursor: Option<TimelineCursor>,
    limit: Option<u32>,
) -> Result<TimelinePage, String> {
    wire(timeline_page_impl(&state, cursor, limit).await)
}

async fn timeline_page_impl(
    state: &AppState,
    cursor: Option<TimelineCursor>,
    limit: Option<u32>,
) -> AppResult<TimelinePage> {
    let db_handle = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        s.db.clone()
    };
    let cur = cursor.unwrap_or_else(TimelineCursor::start);
    let lim = limit.unwrap_or(DEFAULT_LIMIT).min(500);

    tokio::task::spawn_blocking(move || -> AppResult<TimelinePage> {
        let guard = db_handle.blocking_lock();
        let rows = db::list_timeline_page(&guard, cur.day, cur.id, lim)?;
        let entries: Vec<_> = rows
            .iter()
            .map(|r| TimelineEntryView {
                id: r.id,
                taken_at_utc_day: r.taken_at_utc_day,
                mime: r.mime.clone(),
                is_video: r.is_video,
                is_live: r.is_live,
                is_raw: r.is_raw,
            })
            .collect();
        let next_cursor = rows.last().map(|r| TimelineCursor {
            day: r.taken_at_utc_day.unwrap_or(0),
            id: r.id,
        });
        Ok(TimelinePage {
            entries,
            next_cursor,
        })
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn asset_detail(state: State<'_, AppState>, id: i64) -> Result<AssetDetailView, String> {
    wire(asset_detail_impl(&state, id).await)
}

async fn asset_detail_impl(state: &AppState, id: i64) -> AppResult<AssetDetailView> {
    let (db_handle, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.default_collection_key.clone())
    };

    tokio::task::spawn_blocking(move || -> AppResult<AssetDetailView> {
        let guard = db_handle.blocking_lock();
        let row = db::get_asset(&guard, id)?.ok_or(AppError::NotFound)?;
        let filename = open_row(&row.filename_ct, 0, ck.as_bytes())
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();
        let taken_at_utc = row
            .taken_at_utc_ct
            .as_deref()
            .and_then(|b| open_row(b, 0, ck.as_bytes()).ok())
            .and_then(|b| String::from_utf8(b).ok());
        let gps = row
            .gps_ct
            .as_deref()
            .and_then(|b| open_row(b, 0, ck.as_bytes()).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| {
                let lat = v.get("lat")?.as_f64()?;
                let lon = v.get("lon")?.as_f64()?;
                let alt = v.get("alt").and_then(serde_json::Value::as_f64);
                Some(GpsView { lat, lon, alt })
            });
        let device = row
            .device_ct
            .as_deref()
            .and_then(|b| open_row(b, 0, ck.as_bytes()).ok())
            .and_then(|b| String::from_utf8(b).ok());
        let lens = row
            .lens_ct
            .as_deref()
            .and_then(|b| open_row(b, 0, ck.as_bytes()).ok())
            .and_then(|b| String::from_utf8(b).ok());
        let exif_json = row
            .exif_all_ct
            .as_deref()
            .and_then(|b| open_row(b, 0, ck.as_bytes()).ok())
            .and_then(|b| String::from_utf8(b).ok());

        Ok(AssetDetailView {
            id: row.id,
            mime: row.mime,
            bytes: row.bytes,
            width: row.width,
            height: row.height,
            duration_ms: row.duration_ms,
            taken_at_utc_day: row.taken_at_utc_day,
            is_video: row.is_video,
            is_live: row.is_live,
            is_motion: row.is_motion,
            is_raw: row.is_raw,
            is_screenshot: row.is_screenshot,
            filename,
            taken_at_utc,
            gps,
            device,
            lens,
            exif_json,
        })
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn asset_thumbnail(
    state: State<'_, AppState>,
    id: i64,
    size: u32,
) -> Result<Vec<u8>, String> {
    wire(asset_thumbnail_impl(&state, id, size).await)
}

async fn asset_thumbnail_impl(state: &AppState, id: i64, size: u32) -> AppResult<Vec<u8>> {
    let (db_handle, cas, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            s.cas.clone(),
            s.default_collection_key.clone(),
        )
    };
    let kind = if size >= 1024 {
        "thumb1024"
    } else {
        "thumb256"
    };

    read_derivative_or_original(db_handle, cas, ck, id, Some(kind)).await
}

#[tauri::command]
pub async fn asset_original(state: State<'_, AppState>, id: i64) -> Result<Vec<u8>, String> {
    wire(asset_original_impl(&state, id).await)
}

async fn asset_original_impl(state: &AppState, id: i64) -> AppResult<Vec<u8>> {
    let (db_handle, cas, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            s.cas.clone(),
            s.default_collection_key.clone(),
        )
    };
    read_derivative_or_original(db_handle, cas, ck, id, None).await
}

async fn read_derivative_or_original(
    db_handle: Arc<tokio::sync::Mutex<rusqlite::Connection>>,
    cas: Arc<CasStore>,
    ck: Arc<CollectionKey>,
    id: i64,
    derivative_kind: Option<&'static str>,
) -> AppResult<Vec<u8>> {
    tokio::task::spawn_blocking(move || -> AppResult<Vec<u8>> {
        let (cas_ref, wrapped_fk) = {
            let guard = db_handle.blocking_lock();
            let row = db::get_asset(&guard, id)?.ok_or(AppError::NotFound)?;
            match derivative_kind {
                Some(kind) => {
                    if let Some(deriv) = db::get_derivative(&guard, id, kind)? {
                        // Phase-1 thumb keys are wrapped under the same collection
                        // key as the asset; re-using `wrapped_file_key` works here
                        // because the generic ingest re-seals thumb FileKeys with
                        // the collection key. When thumbs have their own wrap row
                        // (Phase 2) we'll index by kind.
                        (deriv, row.wrapped_file_key.clone())
                    } else {
                        (row.cas_ref, row.wrapped_file_key.clone())
                    }
                }
                None => (row.cas_ref, row.wrapped_file_key.clone()),
            }
        };
        let fk = unwrap_file_key(&wrapped_fk, &ck)?;
        let bytes = cas.get(&cas_ref, &fk)?;
        Ok(bytes)
    })
    .await
    .map_err(AppError::from)?
}
