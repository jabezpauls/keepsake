//! Phase 3 analytics Tauri commands — trip detection, memories, and
//! smart albums. Each surface keeps its own command path so the UI can
//! address them independently.

use std::sync::Arc;

use mv_core::analytics::memories::{
    on_this_day, person_year_memories, year_in_photos, MemoryGroup, PersonYearMemory, YearInPhotos,
};
use mv_core::analytics::smart_albums::{materialize as materialize_smart, SmartRule};
use mv_core::analytics::trips::{detect_trips, GeoPoint, TripParams};
use mv_core::crypto::envelope::{open_row, seal_row};
use mv_core::crypto::CollectionKey;
use mv_core::db::queries as q;
use mv_core::geocode::Geocoder;
use tauri::State;

use crate::dto::{
    MemoryGroupView, PersonYearMemoryView, PlaceView, SmartAlbumView, SmartRuleView,
    TimelineCursor, TimelineEntryView, TimelinePage, TripView, YearInPhotosView,
};
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

const DEFAULT_SMART_PAGE_LIMIT: u32 = 120;

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

        let geocoder = Geocoder::new();
        let now = chrono::Utc::now().timestamp();
        for t in &trips {
            // Name "{City}, {Country} · {start_date}..{end_date}" when
            // the centroid resolves to a known place; otherwise fall
            // back to the coordinate-only label.
            let place = geocoder.reverse(t.centroid_lat, t.centroid_lon);
            let date_range = format_day_range(t.start_day, t.end_day);
            let label = match place {
                Some(p) if p.city == p.region || p.region.is_empty() => {
                    format!("{}, {} · {}", p.city, p.country, date_range)
                }
                Some(p) => format!("{}, {} · {}", p.city, p.country, date_range),
                None => format!(
                    "Trip · {} photos · {}",
                    t.member_asset_ids.len(),
                    date_range
                ),
            };
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

#[tauri::command]
pub async fn memories_on_this_day(
    state: State<'_, AppState>,
) -> Result<Vec<MemoryGroupView>, String> {
    wire(memories_on_this_day_impl(&state).await)
}

async fn memories_on_this_day_impl(state: &AppState) -> AppResult<Vec<MemoryGroupView>> {
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<MemoryGroupView>> {
        let guard = db_handle.blocking_lock();
        let assets = q::list_dated_assets_for_user(&guard, user_id)?;
        let today = chrono::Utc::now().date_naive();
        let groups: Vec<MemoryGroup> = on_this_day(today, &assets);
        Ok(groups
            .into_iter()
            .map(|g| MemoryGroupView {
                year: g.year,
                years_ago: g.years_ago,
                asset_ids: g.asset_ids,
            })
            .collect())
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn memories_year_in_photos(
    state: State<'_, AppState>,
) -> Result<Vec<YearInPhotosView>, String> {
    wire(memories_year_in_photos_impl(&state).await)
}

async fn memories_year_in_photos_impl(state: &AppState) -> AppResult<Vec<YearInPhotosView>> {
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<YearInPhotosView>> {
        let guard = db_handle.blocking_lock();
        let assets = q::list_dated_assets_for_user(&guard, user_id)?;
        let today = chrono::Utc::now().date_naive();
        let cards: Vec<YearInPhotos> = year_in_photos(today, &assets);
        Ok(cards
            .into_iter()
            .map(|c| YearInPhotosView {
                year: c.year,
                asset_count: c.asset_count as u32,
                highlights: c.highlights,
            })
            .collect())
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn memories_person_year(
    state: State<'_, AppState>,
    min_assets: Option<u32>,
) -> Result<Vec<PersonYearMemoryView>, String> {
    wire(memories_person_year_impl(&state, min_assets).await)
}

async fn memories_person_year_impl(
    state: &AppState,
    min_assets: Option<u32>,
) -> AppResult<Vec<PersonYearMemoryView>> {
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };
    let floor = min_assets.unwrap_or(3).max(1) as usize;
    tokio::task::spawn_blocking(move || -> AppResult<Vec<PersonYearMemoryView>> {
        let guard = db_handle.blocking_lock();
        let rows = q::list_person_asset_days_for_user(&guard, user_id)?;
        let today = chrono::Utc::now().date_naive();
        let cards: Vec<PersonYearMemory> = person_year_memories(today, &rows, floor);
        Ok(cards
            .into_iter()
            .map(|c| PersonYearMemoryView {
                person_id: c.person_id,
                year: c.year,
                asset_ids: c.asset_ids,
            })
            .collect())
    })
    .await
    .map_err(AppError::from)?
}

// =========== Smart albums (D4) ================================================

#[tauri::command]
pub async fn create_smart_album(
    state: State<'_, AppState>,
    name: String,
    rule: SmartRuleView,
) -> Result<i64, String> {
    wire(create_smart_album_impl(&state, name, rule).await)
}

async fn create_smart_album_impl(
    state: &AppState,
    name: String,
    rule: SmartRuleView,
) -> AppResult<i64> {
    if name.trim().is_empty() {
        return Err(AppError::BadRequest("name required".into()));
    }
    let core_rule = from_view(rule);
    if core_rule.is_empty() {
        return Err(AppError::BadRequest(
            "rule needs at least one clause".into(),
        ));
    }
    let (db_handle, master_bytes, user_id, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            *s.user.master_key.as_bytes(),
            s.user.user_id,
            s.default_collection_key.clone(),
        )
    };

    let cid = tokio::task::spawn_blocking(move || -> AppResult<i64> {
        let guard = db_handle.blocking_lock();
        let master_key = mv_core::crypto::MasterKey::from_bytes(master_bytes);
        let name_ct = seal_row(name.as_bytes(), 0, master_key.as_bytes())?;
        let spec_json = serde_json::to_vec(&core_rule).map_err(|e| {
            tracing::error!(error = %e, "serialize smart rule");
            AppError::Internal
        })?;
        let spec_ct = seal_row(&spec_json, 0, master_key.as_bytes())?;
        let now = chrono::Utc::now().timestamp();
        let cid = q::insert_collection(&guard, user_id, "smart_album", &name_ct, false, None, now)?;
        if !q::set_smart_album_spec(&guard, cid, user_id, &spec_ct)? {
            return Err(AppError::Internal);
        }
        let ck: Arc<CollectionKey> = ck;
        materialize_smart(&guard, cid, &core_rule, Some(&ck), now)?;
        Ok(cid)
    })
    .await
    .map_err(AppError::from)??;

    Ok(cid)
}

#[tauri::command]
pub async fn list_smart_albums(state: State<'_, AppState>) -> Result<Vec<SmartAlbumView>, String> {
    wire(list_smart_albums_impl(&state).await)
}

async fn list_smart_albums_impl(state: &AppState) -> AppResult<Vec<SmartAlbumView>> {
    let (db_handle, master_bytes, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), *s.user.master_key.as_bytes(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<SmartAlbumView>> {
        let guard = db_handle.blocking_lock();
        let rows = q::list_smart_album_collections(&guard, user_id)?;
        let master_key = mv_core::crypto::MasterKey::from_bytes(master_bytes);
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let name = open_row(&r.name_ct, 0, master_key.as_bytes())
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_default();
            let rule: SmartRule = match open_row(&r.smart_spec_ct, 0, master_key.as_bytes())
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
            {
                Some(r) => r,
                None => continue,
            };
            let member_count = q::count_smart_album_members(&guard, r.id)?;
            let snapshot_at = q::smart_album_snapshot_at(&guard, r.id)?;
            out.push(SmartAlbumView {
                id: r.id,
                name,
                rule: to_view(rule),
                member_count,
                snapshot_at,
                created_at: r.created_at,
            });
        }
        Ok(out)
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn refresh_smart_album(state: State<'_, AppState>, id: i64) -> Result<u32, String> {
    wire(refresh_smart_album_impl(&state, id).await)
}

async fn refresh_smart_album_impl(state: &AppState, id: i64) -> AppResult<u32> {
    let (db_handle, master_bytes, user_id, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            *s.user.master_key.as_bytes(),
            s.user.user_id,
            s.default_collection_key.clone(),
        )
    };

    let n = tokio::task::spawn_blocking(move || -> AppResult<u32> {
        let guard = db_handle.blocking_lock();
        let spec_ct = q::get_smart_album_spec(&guard, id, user_id)?.ok_or(AppError::NotFound)?;
        let master_key = mv_core::crypto::MasterKey::from_bytes(master_bytes);
        let plain = open_row(&spec_ct, 0, master_key.as_bytes())?;
        let rule: SmartRule = serde_json::from_slice(&plain).map_err(|e| {
            tracing::error!(error = %e, "decode smart rule");
            AppError::Internal
        })?;
        let ck: Arc<CollectionKey> = ck;
        let now = chrono::Utc::now().timestamp();
        let n = materialize_smart(&guard, id, &rule, Some(&ck), now)?;
        Ok(n as u32)
    })
    .await
    .map_err(AppError::from)??;

    Ok(n)
}

#[tauri::command]
pub async fn delete_smart_album(state: State<'_, AppState>, id: i64) -> Result<bool, String> {
    wire(delete_smart_album_impl(&state, id).await)
}

async fn delete_smart_album_impl(state: &AppState, id: i64) -> AppResult<bool> {
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<bool> {
        let guard = db_handle.blocking_lock();
        q::delete_smart_album(&guard, id, user_id).map_err(AppError::from)
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn smart_album_page(
    state: State<'_, AppState>,
    id: i64,
    cursor: Option<TimelineCursor>,
    limit: Option<u32>,
) -> Result<TimelinePage, String> {
    wire(smart_album_page_impl(&state, id, cursor, limit).await)
}

async fn smart_album_page_impl(
    state: &AppState,
    id: i64,
    cursor: Option<TimelineCursor>,
    limit: Option<u32>,
) -> AppResult<TimelinePage> {
    let db_handle = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        s.db.clone()
    };
    let cur = cursor.unwrap_or_else(TimelineCursor::start);
    let lim = limit.unwrap_or(DEFAULT_SMART_PAGE_LIMIT).min(500);

    tokio::task::spawn_blocking(move || -> AppResult<TimelinePage> {
        let guard = db_handle.blocking_lock();
        let rows = q::list_smart_album_page(&guard, id, cur.day, cur.id, lim)?;
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

fn from_view(v: SmartRuleView) -> SmartRule {
    SmartRule {
        is_raw: v.is_raw,
        is_video: v.is_video,
        is_screenshot: v.is_screenshot,
        is_live: v.is_live,
        has_faces: v.has_faces,
        camera_make: v.camera_make,
        lens: v.lens,
        source_id: v.source_id,
        person_ids: v.person_ids,
        after_day: v.after_day,
        before_day: v.before_day,
    }
}

fn to_view(r: SmartRule) -> SmartRuleView {
    SmartRuleView {
        is_raw: r.is_raw,
        is_video: r.is_video,
        is_screenshot: r.is_screenshot,
        is_live: r.is_live,
        has_faces: r.has_faces,
        camera_make: r.camera_make,
        lens: r.lens,
        source_id: r.source_id,
        person_ids: r.person_ids,
        after_day: r.after_day,
        before_day: r.before_day,
    }
}

#[tauri::command]
pub async fn list_places(
    state: State<'_, AppState>,
    after_day: Option<i64>,
    before_day: Option<i64>,
) -> Result<Vec<PlaceView>, String> {
    wire(list_places_impl(&state, after_day, before_day).await)
}

/// Reverse-geocode every GPS-tagged asset in the user's library and
/// group by `{country, city}`. The encrypted GPS column is decrypted in
/// the same way as trip detection (`detect_trips_run`), then the in-
/// memory `Geocoder` (~80 cities) names each point. Output is sorted
/// by `asset_count desc` so callers can paginate top places trivially.
async fn list_places_impl(
    state: &AppState,
    after_day: Option<i64>,
    before_day: Option<i64>,
) -> AppResult<Vec<PlaceView>> {
    let (db_handle, user_id, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            s.user.user_id,
            s.default_collection_key.clone(),
        )
    };

    tokio::task::spawn_blocking(move || -> AppResult<Vec<PlaceView>> {
        let guard = db_handle.blocking_lock();
        let rows = q::list_geo_tagged_assets_for_user(&guard, user_id)?;
        let ck: Arc<CollectionKey> = ck;
        let geocoder = Geocoder::new();

        // Bucket: place_id -> (PlaceView accumulator).
        struct Bucket {
            city: String,
            region: String,
            country: String,
            asset_ids: Vec<i64>,
            sum_lat: f64,
            sum_lon: f64,
        }
        let mut buckets: std::collections::HashMap<String, Bucket> =
            std::collections::HashMap::new();

        for (asset_id, gps_ct, day) in rows {
            // Apply optional day filter — cheap pre-filter before the
            // expensive AEAD open. The query already excludes NULL day,
            // so `day` is always a real number here.
            if let Some(after) = after_day {
                if day < after {
                    continue;
                }
            }
            if let Some(before) = before_day {
                if day > before {
                    continue;
                }
            }
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
            let Some(named) = geocoder.reverse(lat, lon) else {
                // Skip points outside the bundled-cities dataset rather
                // than fabricating a name. The user can still see them
                // on the Map.
                continue;
            };
            let place_id = format!("{}:{}", named.country, named.city);
            let bucket = buckets.entry(place_id).or_insert_with(|| Bucket {
                city: named.city.clone(),
                region: named.region.clone(),
                country: named.country.clone(),
                asset_ids: Vec::new(),
                sum_lat: 0.0,
                sum_lon: 0.0,
            });
            bucket.asset_ids.push(asset_id);
            bucket.sum_lat += lat;
            bucket.sum_lon += lon;
        }

        let mut out: Vec<PlaceView> = buckets
            .into_iter()
            .map(|(place_id, b)| {
                let n = b.asset_ids.len() as f64;
                // Sample ids spread across the asset list so the cover
                // strip doesn't degenerate to one capture session.
                let stride = (b.asset_ids.len().max(6) / 6).max(1);
                let sample: Vec<i64> = b
                    .asset_ids
                    .iter()
                    .step_by(stride)
                    .take(6)
                    .copied()
                    .collect();
                PlaceView {
                    place_id,
                    city: b.city,
                    region: b.region,
                    country: b.country,
                    asset_count: b.asset_ids.len() as u32,
                    sample_asset_ids: sample,
                    centroid_lat: b.sum_lat / n,
                    centroid_lon: b.sum_lon / n,
                }
            })
            .collect();
        out.sort_by(|a, b| b.asset_count.cmp(&a.asset_count));
        Ok(out)
    })
    .await
    .map_err(AppError::from)?
}

/// Render a `days-since-epoch` pair as "Jan 3 → Jan 10, 2024". Falls
/// back to raw day numbers if the conversion overflows.
fn format_day_range(start_day: i64, end_day: i64) -> String {
    use chrono::{Datelike, Days, NaiveDate};
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    let Some(s) = epoch.checked_add_days(Days::new(start_day.max(0) as u64)) else {
        return format!("day {start_day}..{end_day}");
    };
    let Some(e) = epoch.checked_add_days(Days::new(end_day.max(0) as u64)) else {
        return format!("day {start_day}..{end_day}");
    };
    if s == e {
        s.format("%b %-d, %Y").to_string()
    } else if s.year() == e.year() {
        format!("{} → {}", s.format("%b %-d"), e.format("%b %-d, %Y"))
    } else {
        format!("{} → {}", s.format("%b %-d, %Y"), e.format("%b %-d, %Y"))
    }
}
