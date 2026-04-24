//! Phase 3 analytics Tauri commands — trip detection for now; memories
//! and smart albums extend this module in D3/D4.

use std::sync::Arc;

use mv_core::analytics::trips::{detect_trips, GeoPoint, TripParams};
use mv_core::crypto::envelope::{open_row, seal_row};
use mv_core::crypto::CollectionKey;
use mv_core::db::queries as q;
use tauri::State;

use crate::dto::TripView;
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

#[tauri::command]
pub async fn detect_trips_run(state: State<'_, AppState>) -> Result<u32, String> {
    wire(detect_trips_run_impl(&state).await)
}

async fn detect_trips_run_impl(state: &AppState) -> AppResult<u32> {
    let (db_handle, user_id, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            s.user.user_id,
            s.default_collection_key.clone(),
        )
    };

    let created = tokio::task::spawn_blocking(move || -> AppResult<u32> {
        let guard = db_handle.blocking_lock();
        let rows = q::list_geo_tagged_assets_for_user(&guard, user_id)?;
        let ck: Arc<CollectionKey> = ck;

        // Decrypt `{lat, lon, alt}` JSON per row.
        let mut pts = Vec::with_capacity(rows.len());
        for (asset_id, gps_ct, day) in rows {
            let Ok(plain) = open_row(&gps_ct, 0, ck.as_bytes()) else {
                continue;
            };
            let Ok(v): serde_json::Result<serde_json::Value> = serde_json::from_slice(&plain)
            else {
                continue;
            };
            let (Some(lat), Some(lon)) = (
                v.get("lat").and_then(|x| x.as_f64()),
                v.get("lon").and_then(|x| x.as_f64()),
            ) else {
                continue;
            };
            pts.push(GeoPoint {
                asset_id,
                lat,
                lon,
                day,
            });
        }

        // Rebuild: drop previous `kind='trip'` collections + re-insert.
        q::delete_trips_for_user(&guard, user_id)?;
        let trips = detect_trips(&pts, TripParams::default());

        let now = chrono::Utc::now().timestamp();
        for t in &trips {
            // Placeholder name: "Trip · N photos, <start_day>..<end_day>".
            // D2 (reverse geocoding) swaps this for "<place>, <date>".
            let label = format!(
                "Trip · {} photos · day {}..{}",
                t.member_asset_ids.len(),
                t.start_day,
                t.end_day,
            );
            let name_ct = seal_row(label.as_bytes(), 0, ck.as_bytes())?;
            let cid = q::insert_collection(&guard, user_id, "trip", &name_ct, false, None, now)?;
            for aid in &t.member_asset_ids {
                q::add_to_collection(&guard, cid, *aid, now)?;
            }
        }
        Ok(trips.len() as u32)
    })
    .await
    .map_err(AppError::from)??;

    Ok(created)
}

#[tauri::command]
pub async fn list_trips(state: State<'_, AppState>) -> Result<Vec<TripView>, String> {
    wire(list_trips_impl(&state).await)
}

async fn list_trips_impl(state: &AppState) -> AppResult<Vec<TripView>> {
    let (db_handle, user_id, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            s.user.user_id,
            s.default_collection_key.clone(),
        )
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<TripView>> {
        let guard = db_handle.blocking_lock();
        let rows = q::list_trip_collections(&guard, user_id)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let name = match open_row(&row.name_ct, 0, ck.as_bytes()) {
                Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                Err(_) => "(locked)".into(),
            };
            let member_count = q::count_collection_members(&guard, row.id)?;
            out.push(TripView {
                id: row.id,
                name,
                member_count,
                created_at: row.created_at,
            });
        }
        Ok(out)
    })
    .await
    .map_err(AppError::from)?
}
