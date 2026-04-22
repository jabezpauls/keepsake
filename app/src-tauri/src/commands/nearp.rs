//! Near-duplicate cluster commands — trigger rebuild + list for the review UI.

use mv_core::crypto::CollectionKey;
use mv_core::ml::nearp::{self, DEFAULT_THRESHOLD};
use tauri::State;

use crate::dto::{NearDupCluster, NearDupMember};
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

#[tauri::command]
pub async fn near_dup_rebuild(state: State<'_, AppState>) -> Result<usize, String> {
    wire(near_dup_rebuild_impl(&state).await)
}

async fn near_dup_rebuild_impl(state: &AppState) -> AppResult<usize> {
    let (db_handle, cas, ck) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (
            s.db.clone(),
            s.cas.clone(),
            s.default_collection_key.clone(),
        )
    };
    let ck_bytes = *ck.as_bytes();
    tokio::task::spawn_blocking(move || -> AppResult<usize> {
        let guard = db_handle.blocking_lock();
        let n = nearp::rebuild(
            &guard,
            &cas,
            move |_id| Some(CollectionKey::from_bytes(ck_bytes)),
            DEFAULT_THRESHOLD,
        )?;
        Ok(n)
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn near_dup_list(state: State<'_, AppState>) -> Result<Vec<NearDupCluster>, String> {
    wire(near_dup_list_impl(&state).await)
}

async fn near_dup_list_impl(state: &AppState) -> AppResult<Vec<NearDupCluster>> {
    let db_handle = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        s.db.clone()
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<NearDupCluster>> {
        let guard = db_handle.blocking_lock();
        let by_id = nearp::clusters_by_id(&guard)?;
        let mut clusters: Vec<NearDupCluster> = by_id
            .into_iter()
            .map(|(cluster_id, members)| NearDupCluster {
                cluster_id,
                members: members
                    .into_iter()
                    .map(|m| NearDupMember {
                        asset_id: m.asset_id,
                        is_best: m.is_best,
                    })
                    .collect(),
            })
            .collect();
        clusters.sort_by_key(|c| c.cluster_id);
        Ok(clusters)
    })
    .await
    .map_err(AppError::from)?
}
