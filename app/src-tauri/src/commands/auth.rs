//! Auth commands: user_exists / create_user / unlock / lock / unlock_hidden.

use std::sync::Arc;

use mv_core::crypto::keystore::{self, UserRecord};
use mv_core::crypto::{seal_row, wrap_collection_key, CollectionKey};
use mv_core::{cas::CasStore, db};
use secrecy::SecretString;
use tauri::State;

use crate::dto::SessionHandle;
use crate::errors::{wire, AppError, AppResult};
use crate::state::{ensure_vault_dirs, AppState, Inner, Session};

#[tauri::command]
pub async fn user_exists(state: State<'_, AppState>) -> Result<bool, String> {
    wire(user_exists_impl(&state).await)
}

async fn user_exists_impl(state: &AppState) -> AppResult<bool> {
    let path = state.index_db_path();
    if !path.exists() {
        return Ok(false);
    }
    let path_owned = path.clone();
    let exists = tokio::task::spawn_blocking(move || -> AppResult<bool> {
        let conn = db::schema::open(&path_owned).map_err(AppError::from)?;
        Ok(db::user_exists(&conn)?)
    })
    .await
    .map_err(AppError::from)??;
    Ok(exists)
}

#[tauri::command]
pub async fn create_user(
    state: State<'_, AppState>,
    username: String,
    password: String,
) -> Result<SessionHandle, String> {
    wire(create_user_impl(&state, username, password).await)
}

async fn create_user_impl(
    state: &AppState,
    username: String,
    password: String,
) -> AppResult<SessionHandle> {
    if username.trim().is_empty() {
        return Err(AppError::BadRequest("username required".into()));
    }
    if password.len() < 12 {
        return Err(AppError::BadRequest("password must be ≥ 12 chars".into()));
    }

    ensure_vault_dirs(&state.vault_root)?;
    let root = state.vault_root.clone();
    let index_path = state.index_db_path();
    let pw = SecretString::from(password);
    let uname = username.clone();

    let session = tokio::task::spawn_blocking(move || -> AppResult<Session> {
        if index_path.exists() {
            let conn = db::schema::open(&index_path).map_err(AppError::from)?;
            if db::user_exists(&conn)? {
                return Err(AppError::AlreadyExists);
            }
        }

        let (record, mut unlocked) = keystore::create_user(&uname, &pw)?;
        let conn = db::schema::open(&index_path)?;
        let now = chrono::Utc::now().timestamp();
        let user_id = db::insert_user(&conn, &record, now)?;
        unlocked.user_id = user_id;

        // Seed the "Unsorted" default collection + its master-wrapped key.
        let default_name_ct = seal_row(b"Unsorted", 0, unlocked.master_key.as_bytes())?;
        let cid =
            db::insert_collection(&conn, user_id, "album", &default_name_ct, false, None, now)?;
        let ck = CollectionKey::random()?;
        let wrapped = wrap_collection_key(&ck, &unlocked.master_key)?;
        db::upsert_collection_key(&conn, cid, user_id, "master", &wrapped)?;

        let cas = Arc::new(CasStore::open(&root)?);
        let user = Arc::new(unlocked);
        let db_arc = Arc::new(tokio::sync::Mutex::new(conn));
        let ml_worker = mv_core::ml::MlWorker::new(db_arc.clone(), cas.clone());
        Ok(Session {
            db: db_arc,
            cas,
            user,
            user_record: record,
            username: uname,
            default_collection_id: cid,
            default_collection_key: Arc::new(ck),
            unlocked_albums: std::collections::HashMap::new(),
            hidden_unlocked: false,
            ingests: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            ml_worker,
            ml_runtime: std::sync::Mutex::new(None),
            peer: tokio::sync::Mutex::new(None),
            blobs: tokio::sync::Mutex::new(None),
            docs: tokio::sync::Mutex::new(None),
            gossip: tokio::sync::Mutex::new(None),
            receive_handles: tokio::sync::Mutex::new(Vec::new()),
        })
    })
    .await
    .map_err(AppError::from)??;

    let handle = SessionHandle {
        user_id: session.user.user_id,
        username: session.username.clone(),
        default_collection_id: session.default_collection_id,
        hidden_unlocked: false,
    };
    crate::commands::ml::try_bootstrap_runtime(&session, &state.vault_root);
    *state.inner.lock().await = Inner {
        session: Some(session),
    };
    Ok(handle)
}

#[tauri::command]
pub async fn unlock(
    state: State<'_, AppState>,
    username: String,
    password: String,
) -> Result<SessionHandle, String> {
    wire(unlock_impl(&state, username, password).await)
}

async fn unlock_impl(
    state: &AppState,
    username: String,
    password: String,
) -> AppResult<SessionHandle> {
    ensure_vault_dirs(&state.vault_root)?;
    let root = state.vault_root.clone();
    let index_path = state.index_db_path();
    if !index_path.exists() {
        return Err(AppError::NotFound);
    }
    let pw = SecretString::from(password);
    let expected = username.clone();

    let session = tokio::task::spawn_blocking(move || -> AppResult<Session> {
        let conn = db::schema::open(&index_path)?;
        // Phase 1: single user → id = lowest id.
        let user_id: i64 = conn
            .query_row("SELECT id FROM user ORDER BY id LIMIT 1", [], |r| r.get(0))
            .map_err(|e| AppError::from(mv_core::Error::from(e)))?;
        let record: UserRecord = db::get_user_record(&conn, user_id)?;
        let unlocked = keystore::unlock(&record, &pw, user_id)?;
        // Verify username matches what was saved (also validates key).
        let uname_bytes =
            mv_core::crypto::open_row(&record.username_ct, 0, unlocked.master_key.as_bytes())?;
        let actual = String::from_utf8(uname_bytes).map_err(|_| AppError::Crypto)?;
        if actual != expected {
            return Err(AppError::Crypto);
        }

        // Find the default "Unsorted" album (lowest-id album owned by this user).
        let default_id: i64 = conn
            .query_row(
                "SELECT id FROM collection WHERE owner_id = ?1 AND kind = 'album'
                 ORDER BY id LIMIT 1",
                rusqlite::params![user_id],
                |r| r.get(0),
            )
            .map_err(|e| AppError::from(mv_core::Error::from(e)))?;
        let wrapped = db::get_collection_key(&conn, default_id, user_id, "master")?
            .ok_or(AppError::Crypto)?;
        let ck = mv_core::crypto::unwrap_collection_key(&wrapped, &unlocked.master_key)?;

        let cas = Arc::new(CasStore::open(&root)?);
        let db_arc = Arc::new(tokio::sync::Mutex::new(conn));
        let ml_worker = mv_core::ml::MlWorker::new(db_arc.clone(), cas.clone());
        Ok(Session {
            db: db_arc,
            cas,
            user: Arc::new(unlocked),
            user_record: record,
            username: actual,
            default_collection_id: default_id,
            default_collection_key: Arc::new(ck),
            unlocked_albums: std::collections::HashMap::new(),
            hidden_unlocked: false,
            ingests: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            ml_worker,
            ml_runtime: std::sync::Mutex::new(None),
            peer: tokio::sync::Mutex::new(None),
            blobs: tokio::sync::Mutex::new(None),
            docs: tokio::sync::Mutex::new(None),
            gossip: tokio::sync::Mutex::new(None),
            receive_handles: tokio::sync::Mutex::new(Vec::new()),
        })
    })
    .await
    .map_err(AppError::from)??;

    let handle = SessionHandle {
        user_id: session.user.user_id,
        username: session.username.clone(),
        default_collection_id: session.default_collection_id,
        hidden_unlocked: false,
    };
    crate::commands::ml::try_bootstrap_runtime(&session, &state.vault_root);
    *state.inner.lock().await = Inner {
        session: Some(session),
    };
    Ok(handle)
}

#[tauri::command]
pub async fn lock(state: State<'_, AppState>) -> Result<(), String> {
    wire(
        async {
            let mut guard = state.inner.lock().await;
            *guard = Inner::default();
            Ok(())
        }
        .await,
    )
}

#[tauri::command]
pub async fn unlock_hidden(state: State<'_, AppState>, password: String) -> Result<bool, String> {
    wire(unlock_hidden_impl(&state, password).await)
}

async fn unlock_hidden_impl(state: &AppState, password: String) -> AppResult<bool> {
    let pw = SecretString::from(password);
    let mut guard = state.inner.lock().await;
    let session = guard.session.as_mut().ok_or(AppError::Locked)?;

    // Clone the record (Arc<UnlockedUser> is not mut-able here), apply in-memory.
    let mut new_user = mv_core::crypto::keystore::UnlockedUser {
        user_id: session.user.user_id,
        master_key: mv_core::crypto::MasterKey::from_bytes(*session.user.master_key.as_bytes()),
        identity: mv_core::crypto::PeerKeypair::from_bytes(
            session.user.identity.public.0,
            *session.user.identity.secret_bytes(),
        ),
        iroh_node: mv_core::crypto::PeerKeypair::from_bytes(
            session.user.iroh_node.public.0,
            *session.user.iroh_node.secret_bytes(),
        ),
        hidden_master_key: None,
    };

    match keystore::unlock_hidden_vault(&session.user_record, &mut new_user, &pw) {
        Ok(()) => {
            session.user = Arc::new(new_user);
            session.hidden_unlocked = true;
            Ok(true)
        }
        Err(_) => Ok(false), // deliberately non-distinguishable from "no hidden vault"
    }
}
