//! Sources + ingest commands.

use std::path::PathBuf;
use std::sync::Arc;

use mv_core::crypto::{open_row, seal_row};
use mv_core::db;
use mv_core::ingest::{
    GenericAdapter, GoogleTakeoutAdapter, IPhoneFolderAdapter, IngestAdapter, IngestCtx, Progress,
    ProgressEvent,
};
use tauri::State;

use crate::dto::{IngestState, IngestStatus, SourceView};
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

#[tauri::command]
pub async fn add_source(
    state: State<'_, AppState>,
    name: String,
    root: String,
    adapter: String,
    linked_only: bool,
) -> Result<i64, String> {
    wire(add_source_impl(&state, name, root, adapter, linked_only).await)
}

async fn add_source_impl(
    state: &AppState,
    name: String,
    root: String,
    adapter: String,
    linked_only: bool,
) -> AppResult<i64> {
    let root_path = PathBuf::from(&root);
    if !root_path.is_dir() {
        return Err(AppError::BadRequest("root is not a directory".into()));
    }
    if !matches!(
        adapter.as_str(),
        "generic" | "iphone_folder" | "google_takeout"
    ) {
        return Err(AppError::BadRequest(format!("unknown adapter: {adapter}")));
    }

    // Snapshot handles under the state lock, then release before running ingest.
    let (db_handle, cas, user, ck, ingests, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            s.cas.clone(),
            s.user.clone(),
            s.default_collection_key.clone(),
            s.ingests.clone(),
            s.user.user_id,
        )
    };

    // Create the source row synchronously.
    let name_bytes = name.clone();
    let root_bytes = root.clone();
    let adapter_tag = adapter.clone();
    let source_id = {
        let db_owned = db_handle.clone();
        let ck_owned = ck.clone();
        tokio::task::spawn_blocking(move || -> AppResult<i64> {
            let guard = db_owned.blocking_lock();
            let name_ct = seal_row(name_bytes.as_bytes(), 0, ck_owned.as_bytes())?;
            let root_ct = seal_row(root_bytes.as_bytes(), 0, ck_owned.as_bytes())?;
            let hint_ct = seal_row(adapter_tag.as_bytes(), 0, ck_owned.as_bytes())?;
            let now = chrono::Utc::now().timestamp();
            let id = db::insert_source(
                &guard,
                user_id,
                &name_ct,
                &root_ct,
                Some(&hint_ct),
                linked_only,
                now,
            )?;
            Ok(id)
        })
        .await
        .map_err(AppError::from)??
    };

    // Seed ingest status + spawn the ingest task.
    {
        let mut map = ingests.lock().await;
        map.insert(
            source_id,
            IngestStatus {
                source_id,
                state: IngestState::Running {
                    files_processed: 0,
                    files_total: 0,
                    current: None,
                },
            },
        );
    }

    let ingests_task = ingests.clone();
    let ctx = IngestCtx {
        cas,
        db: db_handle,
        user,
        default_collection_key: ck,
    };
    let root_owned = root_path;
    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let progress = Progress(tx);
        // Drive progress updates concurrently with the ingest future.
        let status_task = {
            let ingests = ingests_task.clone();
            tokio::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    let mut map = ingests.lock().await;
                    if let Some(entry) = map.get_mut(&source_id) {
                        match ev {
                            ProgressEvent::FileStarted { path, index, total } => {
                                entry.state = IngestState::Running {
                                    files_processed: index,
                                    files_total: total,
                                    current: Some(path.to_string_lossy().into_owned()),
                                };
                            }
                            ProgressEvent::Done(r) => {
                                entry.state = IngestState::Done {
                                    inserted: r.inserted,
                                    deduped: r.deduped,
                                    skipped: r.skipped,
                                    errors: r.errors,
                                };
                            }
                            _ => {}
                        }
                    }
                }
            })
        };

        let result: Result<_, mv_core::Error> = match adapter.as_str() {
            "iphone_folder" => {
                IPhoneFolderAdapter::new()
                    .ingest(&ctx, &root_owned, source_id, progress)
                    .await
            }
            "google_takeout" => {
                GoogleTakeoutAdapter::new()
                    .ingest(&ctx, &root_owned, source_id, progress)
                    .await
            }
            _ => {
                GenericAdapter::new()
                    .ingest(&ctx, &root_owned, source_id, progress)
                    .await
            }
        };

        let _ = status_task.await;
        if let Err(e) = result {
            let mut map = ingests_task.lock().await;
            if let Some(entry) = map.get_mut(&source_id) {
                entry.state = IngestState::Failed {
                    message: format!("{e}"),
                };
            }
        }
    });

    Ok(source_id)
}

#[tauri::command]
pub async fn list_sources(state: State<'_, AppState>) -> Result<Vec<SourceView>, String> {
    wire(list_sources_impl(&state).await)
}

async fn list_sources_impl(state: &AppState) -> AppResult<Vec<SourceView>> {
    let (db_handle, ck, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            s.default_collection_key.clone(),
            s.user.user_id,
        )
    };

    tokio::task::spawn_blocking(move || -> AppResult<Vec<SourceView>> {
        let guard = db_handle.blocking_lock();
        let rows = db::list_sources(&guard, user_id)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let name = open_row(&r.name_ct, 0, ck.as_bytes())
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_default();
            let root_path = open_row(&r.root_path_ct, 0, ck.as_bytes())
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_default();
            let adapter_kind = r
                .device_hint_ct
                .as_deref()
                .and_then(|b| open_row(b, 0, ck.as_bytes()).ok())
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_else(|| "generic".into());
            out.push(SourceView {
                id: r.id,
                name,
                root_path,
                adapter_kind,
                linked_only: r.linked_only,
                bytes_total: r.bytes_total,
                file_count: r.file_count,
                imported_at: r.imported_at,
            });
        }
        Ok(out)
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn ingest_status(
    state: State<'_, AppState>,
    source_id: i64,
) -> Result<IngestStatus, String> {
    wire(
        async {
            let ingests: Arc<tokio::sync::Mutex<_>> = {
                let guard = state.inner.lock().await;
                let s = guard.session.as_ref().ok_or(AppError::Locked)?;
                s.ingests.clone()
            };
            let map = ingests.lock().await;
            Ok(map.get(&source_id).cloned().unwrap_or(IngestStatus {
                source_id,
                state: IngestState::Idle,
            }))
        }
        .await,
    )
}
