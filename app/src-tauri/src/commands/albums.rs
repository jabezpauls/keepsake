//! Album + collection-key commands.

use std::sync::Arc;

use mv_core::crypto::keystore::{self};
use mv_core::crypto::{
    open_row, seal_row, unwrap_collection_key, wrap_collection_key, CollectionKey,
};
use mv_core::db;
use secrecy::SecretString;
use tauri::State;

use crate::dto::{AlbumView, TimelineCursor, TimelineEntryView, TimelinePage};
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

const DEFAULT_LIMIT: u32 = 120;

#[tauri::command]
pub async fn create_album(
    state: State<'_, AppState>,
    name: String,
    password: Option<String>,
) -> Result<i64, String> {
    wire(create_album_impl(&state, name, password).await)
}

async fn create_album_impl(
    state: &AppState,
    name: String,
    password: Option<String>,
) -> AppResult<i64> {
    if name.trim().is_empty() {
        return Err(AppError::BadRequest("name required".into()));
    }
    let (db_handle, master_bytes, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), *s.user.master_key.as_bytes(), s.user.user_id)
    };

    let cid = tokio::task::spawn_blocking(move || -> AppResult<i64> {
        let guard = db_handle.blocking_lock();
        let master_key = mv_core::crypto::MasterKey::from_bytes(master_bytes);
        let name_ct = seal_row(name.as_bytes(), 0, master_key.as_bytes())?;
        let now = chrono::Utc::now().timestamp();

        let ck = CollectionKey::random()?;
        let wrapped_master = wrap_collection_key(&ck, &master_key)?;

        let (has_pw, salt_fixed) = match password {
            Some(pw) if !pw.is_empty() => {
                let pw = SecretString::from(pw);
                let (wrapped_pw, salt) = keystore::wrap_collection_key_for_album(&ck, &pw)?;
                // Insert album row with the salt.
                let cid = db::insert_collection(
                    &guard,
                    user_id,
                    "album",
                    &name_ct,
                    true,
                    Some(&salt),
                    now,
                )?;
                db::upsert_collection_key(&guard, cid, user_id, "master", &wrapped_master)?;
                db::upsert_collection_key(&guard, cid, user_id, "album_password", &wrapped_pw)?;
                return Ok(cid);
            }
            _ => (false, None),
        };
        let cid =
            db::insert_collection(&guard, user_id, "album", &name_ct, has_pw, salt_fixed, now)?;
        db::upsert_collection_key(&guard, cid, user_id, "master", &wrapped_master)?;
        Ok(cid)
    })
    .await
    .map_err(AppError::from)??;

    Ok(cid)
}

#[tauri::command]
pub async fn list_albums(
    state: State<'_, AppState>,
    include_hidden: bool,
) -> Result<Vec<AlbumView>, String> {
    wire(list_albums_impl(&state, include_hidden).await)
}

async fn list_albums_impl(state: &AppState, include_hidden: bool) -> AppResult<Vec<AlbumView>> {
    let (db_handle, master_bytes, user_id, unlocked_ids, hidden_unlocked) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            *s.user.master_key.as_bytes(),
            s.user.user_id,
            s.unlocked_albums.keys().copied().collect::<Vec<_>>(),
            s.hidden_unlocked,
        )
    };
    let effective_hidden = include_hidden && hidden_unlocked;

    tokio::task::spawn_blocking(move || -> AppResult<Vec<AlbumView>> {
        let guard = db_handle.blocking_lock();
        let rows = db::list_collections(&guard, user_id, effective_hidden)?;
        let master_key = mv_core::crypto::MasterKey::from_bytes(master_bytes);
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let name = open_row(&r.name_ct, 0, master_key.as_bytes())
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_default();
            let members = db::count_collection_members(&guard, r.id)?;
            let unlocked = !r.has_password || unlocked_ids.contains(&r.id);
            out.push(AlbumView {
                id: r.id,
                name,
                kind: r.kind,
                member_count: members,
                has_password: r.has_password,
                unlocked,
                hidden: false,
            });
        }
        Ok(out)
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn unlock_album(
    state: State<'_, AppState>,
    id: i64,
    password: String,
) -> Result<bool, String> {
    wire(unlock_album_impl(&state, id, password).await)
}

async fn unlock_album_impl(state: &AppState, id: i64, password: String) -> AppResult<bool> {
    let pw = SecretString::from(password);
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };

    // Pull the wrapped key + salt, try to unlock — returns None on failure
    // (plausible-deniability: wrong password is indistinguishable from "no members").
    let ck_opt = tokio::task::spawn_blocking(move || -> AppResult<Option<CollectionKey>> {
        let guard = db_handle.blocking_lock();
        let coll = match db::get_collection(&guard, id)? {
            Some(c) => c,
            None => return Ok(None),
        };
        if !coll.has_password {
            return Ok(None);
        }
        let Some(salt) = coll.password_salt else {
            return Ok(None);
        };
        let wrapped = match db::get_collection_key(&guard, id, user_id, "album_password")? {
            Some(b) => b,
            None => return Ok(None),
        };
        Ok(keystore::unlock_album(&wrapped, &pw, &salt).ok())
    })
    .await
    .map_err(AppError::from)??;

    match ck_opt {
        Some(ck) => {
            let mut guard = state.inner.lock().await;
            let s = guard.session.as_mut().ok_or(AppError::Locked)?;
            s.unlocked_albums.insert(id, Arc::new(ck));
            Ok(true)
        }
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn album_page(
    state: State<'_, AppState>,
    id: i64,
    cursor: Option<TimelineCursor>,
    limit: Option<u32>,
) -> Result<TimelinePage, String> {
    wire(album_page_impl(&state, id, cursor, limit).await)
}

async fn album_page_impl(
    state: &AppState,
    id: i64,
    cursor: Option<TimelineCursor>,
    limit: Option<u32>,
) -> AppResult<TimelinePage> {
    let (db_handle, unlocked) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        let is_default = id == s.default_collection_id;
        let unlocked = is_default || s.unlocked_albums.contains_key(&id);
        (s.db.clone(), unlocked)
    };
    let cur = cursor.unwrap_or_else(TimelineCursor::start);
    let lim = limit.unwrap_or(DEFAULT_LIMIT).min(500);

    tokio::task::spawn_blocking(move || -> AppResult<TimelinePage> {
        let guard = db_handle.blocking_lock();
        let coll = db::get_collection(&guard, id)?.ok_or(AppError::NotFound)?;
        // If the album is password-protected and not unlocked, return empty
        // (no error leak — matches the hidden-vault contract).
        if coll.has_password && !unlocked {
            return Ok(TimelinePage {
                entries: vec![],
                next_cursor: None,
            });
        }
        let rows = db::list_collection_page(&guard, id, cur.day, cur.id, lim)?;
        let entries: Vec<_> = rows
            .iter()
            .map(|r| TimelineEntryView {
                id: r.id,
                taken_at_utc_day: r.taken_at_utc_day,
                mime: r.mime.clone(),
                is_video: r.is_video,
                is_live: r.is_live,
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
pub async fn add_to_album(
    state: State<'_, AppState>,
    id: i64,
    asset_ids: Vec<i64>,
) -> Result<(), String> {
    wire(add_to_album_impl(&state, id, asset_ids).await)
}

async fn add_to_album_impl(state: &AppState, id: i64, asset_ids: Vec<i64>) -> AppResult<()> {
    let db_handle = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        s.db.clone()
    };
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        let guard = db_handle.blocking_lock();
        let now = chrono::Utc::now().timestamp();
        for aid in asset_ids {
            db::add_to_collection(&guard, id, aid, now)?;
        }
        Ok(())
    })
    .await
    .map_err(AppError::from)?
}

/// Helper shared with export: resolve the CollectionKey for a given album id.
pub(crate) fn collection_key_for(
    master_key_bytes: [u8; 32],
    conn: &rusqlite::Connection,
    user_id: i64,
    collection_id: i64,
    default_collection_id: i64,
    default_ck_bytes: [u8; 32],
    unlocked_override: Option<&CollectionKey>,
) -> AppResult<CollectionKey> {
    if collection_id == default_collection_id {
        return Ok(CollectionKey::from_bytes(default_ck_bytes));
    }
    if let Some(ck) = unlocked_override {
        return Ok(CollectionKey::from_bytes(*ck.as_bytes()));
    }
    let wrapped = db::get_collection_key(conn, collection_id, user_id, "master")?
        .ok_or(AppError::NotFound)?;
    let master_key = mv_core::crypto::MasterKey::from_bytes(master_key_bytes);
    let ck = unwrap_collection_key(&wrapped, &master_key)?;
    Ok(ck)
}
