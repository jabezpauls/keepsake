//! Phase-3 D9 pets commands.
//!
//! Ships the manual-flagging + listing surface. A future weight-backed
//! classifier will populate the same columns at ingest; this module
//! works today with zero ML dependencies.

use mv_core::crypto::envelope::{open_row, seal_row};
use mv_core::crypto::MasterKey;
use mv_core::db::queries as q;
use tauri::State;

use crate::dto::PetAssetView;
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

#[tauri::command]
pub async fn set_asset_pet(
    state: State<'_, AppState>,
    asset_id: i64,
    is_pet: bool,
    species: Option<String>,
) -> Result<bool, String> {
    wire(set_asset_pet_impl(&state, asset_id, is_pet, species).await)
}

async fn set_asset_pet_impl(
    state: &AppState,
    asset_id: i64,
    is_pet: bool,
    species: Option<String>,
) -> AppResult<bool> {
    let (db_handle, master_bytes) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), *s.user.master_key.as_bytes())
    };
    tokio::task::spawn_blocking(move || -> AppResult<bool> {
        let guard = db_handle.blocking_lock();
        let species_ct = if is_pet {
            match species.as_deref() {
                Some(s) if !s.is_empty() => {
                    let mk = MasterKey::from_bytes(master_bytes);
                    Some(seal_row(s.as_bytes(), 0, mk.as_bytes())?)
                }
                _ => None,
            }
        } else {
            None
        };
        q::set_asset_pet(&guard, asset_id, is_pet, species_ct.as_deref()).map_err(AppError::from)
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn list_pet_assets(state: State<'_, AppState>) -> Result<Vec<PetAssetView>, String> {
    wire(list_pet_assets_impl(&state).await)
}

async fn list_pet_assets_impl(state: &AppState) -> AppResult<Vec<PetAssetView>> {
    let (db_handle, master_bytes, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), *s.user.master_key.as_bytes(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<PetAssetView>> {
        let guard = db_handle.blocking_lock();
        let rows = q::list_pet_assets_for_user(&guard, user_id)?;
        let mk = MasterKey::from_bytes(master_bytes);
        Ok(rows
            .into_iter()
            .map(|(id, day, species_ct)| PetAssetView {
                id,
                taken_at_utc_day: day,
                species: species_ct.and_then(|ct| {
                    open_row(&ct, 0, mk.as_bytes())
                        .ok()
                        .and_then(|b| String::from_utf8(b).ok())
                }),
            })
            .collect())
    })
    .await
    .map_err(AppError::from)?
}
