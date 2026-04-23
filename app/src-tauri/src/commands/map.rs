//! Map-view command — decrypts plaintext lat/lon points for rendering.

use mv_core::crypto::open_row;
use mv_core::db::{self, AssetFilter};
use tauri::State;

use crate::dto::MapPoint;
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

#[tauri::command]
pub async fn map_points(
    state: State<'_, AppState>,
    after_day: Option<i64>,
    before_day: Option<i64>,
    source_id: Option<i64>,
) -> Result<Vec<MapPoint>, String> {
    wire(map_points_impl(&state, after_day, before_day, source_id).await)
}

async fn map_points_impl(
    state: &AppState,
    after_day: Option<i64>,
    before_day: Option<i64>,
    source_id: Option<i64>,
) -> AppResult<Vec<MapPoint>> {
    let (db_handle, ck, hidden_unlocked) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            s.default_collection_key.clone(),
            s.hidden_unlocked,
        )
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<MapPoint>> {
        let guard = db_handle.blocking_lock();
        let filter = AssetFilter {
            after_day,
            before_day,
            source_id,
            hidden_vault_unlocked: hidden_unlocked,
            ..Default::default()
        };
        let rows = db::list_assets_with_gps(&guard, &filter)?;
        let mut out = Vec::with_capacity(rows.len());
        for (id, gps_ct, day) in rows {
            let Ok(plain) = open_row(&gps_ct, 0, ck.as_bytes()) else {
                continue;
            };
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&plain) else {
                continue;
            };
            let Some(lat) = v.get("lat").and_then(serde_json::Value::as_f64) else {
                continue;
            };
            let Some(lon) = v.get("lon").and_then(serde_json::Value::as_f64) else {
                continue;
            };
            out.push(MapPoint {
                asset_id: id,
                lat,
                lon,
                taken_at_utc_day: day,
            });
        }
        Ok(out)
    })
    .await
    .map_err(AppError::from)?
}
