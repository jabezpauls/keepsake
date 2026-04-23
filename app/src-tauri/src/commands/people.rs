//! Person/face CRUD commands.
//!
//! Phase 2 scope: list/rename/hide/merge/split on persons, and a
//! rebuild-clusters trigger. Detection + embedding run off the `ml-models`
//! feature flag (Step 4) — this module only manages the schema side.

use mv_core::crypto::{open_row, seal_row, unwrap_file_key};
use mv_core::db;
use mv_core::media::crop_face_webp;
use tauri::State;

use crate::dto::PersonView;
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

/// Upper bound on requested face-thumb size — clamp defensively even though
/// the UI only ever asks for 96 / 256.
const FACE_THUMB_MAX: u32 = 512;
/// Per-side padding fraction (30% total = 15% each side). Sits between
/// Immich's 20% total and Apple Photos's more generous framing — tight
/// enough to read as "just the face" at avatar sizes.
const FACE_THUMB_PADDING: f32 = 0.15;

#[tauri::command]
pub async fn list_people(
    state: State<'_, AppState>,
    include_hidden: Option<bool>,
) -> Result<Vec<PersonView>, String> {
    wire(list_people_impl(&state, include_hidden.unwrap_or(false)).await)
}

async fn list_people_impl(state: &AppState, include_hidden: bool) -> AppResult<Vec<PersonView>> {
    let (db_handle, user_id, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            s.user.user_id,
            s.default_collection_key.clone(),
        )
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<PersonView>> {
        let guard = db_handle.blocking_lock();
        let rows = db::list_persons(&guard, user_id, include_hidden)?;
        let face_counts = db::count_faces_by_person(&guard)?;
        let count_map: std::collections::HashMap<Option<i64>, i64> =
            face_counts.into_iter().collect();
        let mut out = Vec::new();
        for p in rows {
            let face_count = count_map.get(&Some(p.id)).copied().unwrap_or(0);
            let name = p
                .name_ct
                .as_deref()
                .and_then(|b| open_row(b, 0, ck.as_bytes()).ok())
                .and_then(|b| String::from_utf8(b).ok());
            // Cover = highest-quality face's asset. Cheap single query.
            let cover: Option<i64> = guard
                .query_row(
                    r"SELECT asset_id FROM face WHERE person_id = ?1 ORDER BY quality DESC LIMIT 1",
                    [p.id],
                    |r| r.get(0),
                )
                .ok();
            out.push(PersonView {
                id: p.id,
                name,
                hidden: p.hidden,
                face_count,
                cover_asset_id: cover,
            });
        }
        // Hide zero-member singletons from the main people grid.
        out.retain(|p| p.face_count > 0 || p.hidden);
        Ok(out)
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn rename_person(
    state: State<'_, AppState>,
    person_id: i64,
    name: String,
) -> Result<(), String> {
    wire(rename_person_impl(&state, person_id, name).await)
}

async fn rename_person_impl(state: &AppState, person_id: i64, name: String) -> AppResult<()> {
    let (db_handle, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.default_collection_key.clone())
    };
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        let guard = db_handle.blocking_lock();
        let name_ct = seal_row(name.as_bytes(), 0, ck.as_bytes())?;
        db::set_person_name(&guard, person_id, &name_ct)?;
        Ok(())
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn hide_person(
    state: State<'_, AppState>,
    person_id: i64,
    hidden: bool,
) -> Result<(), String> {
    wire(hide_person_impl(&state, person_id, hidden).await)
}

async fn hide_person_impl(state: &AppState, person_id: i64, hidden: bool) -> AppResult<()> {
    let db_handle = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        s.db.clone()
    };
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        let guard = db_handle.blocking_lock();
        db::set_person_hidden(&guard, person_id, hidden)?;
        Ok(())
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn merge_people(state: State<'_, AppState>, src: i64, dst: i64) -> Result<(), String> {
    wire(merge_people_impl(&state, src, dst).await)
}

async fn merge_people_impl(state: &AppState, src: i64, dst: i64) -> AppResult<()> {
    if src == dst {
        return Err(AppError::BadRequest("src == dst".into()));
    }
    let db_handle = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        s.db.clone()
    };
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        let guard = db_handle.blocking_lock();
        db::merge_persons(&guard, src, dst)?;
        Ok(())
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn person_face_thumbnail(
    state: State<'_, AppState>,
    person_id: i64,
    size: u32,
) -> Result<Vec<u8>, String> {
    wire(person_face_thumbnail_impl(&state, person_id, size).await)
}

async fn person_face_thumbnail_impl(
    state: &AppState,
    person_id: i64,
    size: u32,
) -> AppResult<Vec<u8>> {
    let (db_handle, cas, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            s.cas.clone(),
            s.default_collection_key.clone(),
        )
    };
    let size = size.min(FACE_THUMB_MAX).max(32);

    tokio::task::spawn_blocking(move || -> AppResult<Vec<u8>> {
        // Resolve the representative face + the cas-ref/key to its source
        // thumb in one DB guard scope.
        let (bbox, cas_ref, wrapped_fk) = {
            let guard = db_handle.blocking_lock();
            let row: Result<(i64, Vec<u8>), rusqlite::Error> = guard.query_row(
                r"SELECT asset_id, bbox_ct
                    FROM face
                    WHERE person_id = ?1
                    ORDER BY quality DESC NULLS LAST, id ASC
                    LIMIT 1",
                [person_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            );
            let (asset_id, bbox_ct) = match row {
                Ok(t) => t,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Err(AppError::NotFound),
                Err(e) => return Err(AppError::from(mv_core::Error::from(e))),
            };
            let bbox_plain = open_row(&bbox_ct, asset_id as u64, ck.as_bytes())?;
            if bbox_plain.len() != 16 {
                return Err(AppError::BadRequest("invalid face bbox payload".into()));
            }
            let bbox: [f32; 4] = [
                f32::from_le_bytes(bbox_plain[0..4].try_into().unwrap()),
                f32::from_le_bytes(bbox_plain[4..8].try_into().unwrap()),
                f32::from_le_bytes(bbox_plain[8..12].try_into().unwrap()),
                f32::from_le_bytes(bbox_plain[12..16].try_into().unwrap()),
            ];

            let asset = db::get_asset(&guard, asset_id)?.ok_or(AppError::NotFound)?;
            let (cas_ref, wrapped_fk) = if let Some(deriv) =
                db::get_derivative(&guard, asset_id, "thumb1024")?
            {
                (deriv, asset.wrapped_file_key.clone())
            } else if let Some(deriv) = db::get_derivative(&guard, asset_id, "thumb256")? {
                (deriv, asset.wrapped_file_key.clone())
            } else {
                (asset.cas_ref, asset.wrapped_file_key.clone())
            };
            (bbox, cas_ref, wrapped_fk)
        };

        let fk = unwrap_file_key(&wrapped_fk, &ck)?;
        let thumb_bytes = cas.get(&cas_ref, &fk)?;

        // Best-effort crop. Any decode/encode failure falls back to the raw
        // thumb so the UI never shows a broken image.
        match crop_face_webp(&thumb_bytes, bbox, size, FACE_THUMB_PADDING) {
            Ok(bytes) => Ok(bytes),
            Err(_) => Ok(thumb_bytes),
        }
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn split_person(
    state: State<'_, AppState>,
    source_person: i64,
    face_ids: Vec<i64>,
) -> Result<i64, String> {
    wire(split_person_impl(&state, source_person, face_ids).await)
}

async fn split_person_impl(
    state: &AppState,
    _source_person: i64,
    face_ids: Vec<i64>,
) -> AppResult<i64> {
    if face_ids.is_empty() {
        return Err(AppError::BadRequest("no faces".into()));
    }
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<i64> {
        let guard = db_handle.blocking_lock();
        let new_id = db::insert_person(&guard, user_id, None)?;
        db::reassign_faces_to_person(&guard, &face_ids, Some(new_id))?;
        Ok(new_id)
    })
    .await
    .map_err(AppError::from)?
}
