//! Phase 3.1 peer commands — Iroh pairing ticket issue / accept / list / forget.
//!
//! Peer boot is lazy: we only call `mv_sync::Peer::start` on the first
//! `peer_my_ticket` call. This keeps the unlock hot path free of UDP binds
//! for users who never open the Peers tab.

use std::sync::Arc;

use mv_sync::{Peer, PeerConfig};
use tauri::State;

use crate::dto::{PairingTicketView, PeerAcceptedView};
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

/// Ensure `session.peer` is populated, binding the endpoint if needed.
/// Returns the live handle. Idempotent.
async fn ensure_peer(state: &AppState) -> AppResult<Arc<Peer>> {
    // Collect what Peer::start needs while holding the outer session lock
    // briefly; then drop it before the awaitable bind.
    let (user, existing) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        let existing = s.peer.lock().await.clone();
        if existing.is_some() {
            return Ok(existing.unwrap());
        }
        (s.user.clone(), existing)
    };
    let _ = existing; // silence unused-binding lint in the early-return path

    let peer = Peer::start(&user, PeerConfig::default())
        .await
        .map_err(AppError::from)?;
    let arc = Arc::new(peer);

    // Store. Another concurrent caller may have raced us — the last writer
    // wins; the loser just drops its Arc.
    let guard = state.inner.lock().await;
    let s = guard.session.as_ref().ok_or(AppError::Locked)?;
    let mut slot = s.peer.lock().await;
    if slot.is_none() {
        *slot = Some(arc.clone());
        Ok(arc)
    } else {
        Ok(slot.as_ref().unwrap().clone())
    }
}

#[tauri::command]
pub async fn peer_my_ticket(state: State<'_, AppState>) -> Result<PairingTicketView, String> {
    wire(peer_my_ticket_impl(&state).await)
}

async fn peer_my_ticket_impl(state: &AppState) -> AppResult<PairingTicketView> {
    let peer = ensure_peer(state).await?;
    // Grab the signing seed — held in UnlockedUser, not in Peer (which only
    // knows its public bits). Copy out under a short critical section and
    // let the Arc<UnlockedUser> drop with the guard.
    let seed: [u8; 32] = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        *s.user.iroh_node.secret_bytes()
    };
    let ticket = peer.ticket(&seed).map_err(AppError::from)?;
    Ok(PairingTicketView {
        base32: ticket.to_base32(),
        my_node_id_hex: hex::encode(ticket.iroh_node_pub),
        created_at: ticket.timestamp,
    })
}

#[tauri::command]
pub async fn peer_accept_ticket(
    state: State<'_, AppState>,
    base32: String,
) -> Result<PeerAcceptedView, String> {
    wire(peer_accept_ticket_impl(&state, base32).await)
}

async fn peer_accept_ticket_impl(state: &AppState, base32: String) -> AppResult<PeerAcceptedView> {
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<PeerAcceptedView> {
        let guard = db_handle.blocking_lock();
        let row = mv_sync::accept_base32(&guard, user_id, &base32)?;
        Ok(PeerAcceptedView {
            node_id_hex: hex::encode(&row.peer_node_id),
            identity_pub_hex: hex::encode(&row.peer_identity_pub),
            relay_url: row.relay_url,
            added_at: row.added_at,
        })
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn peer_list(state: State<'_, AppState>) -> Result<Vec<PeerAcceptedView>, String> {
    wire(peer_list_impl(&state).await)
}

async fn peer_list_impl(state: &AppState) -> AppResult<Vec<PeerAcceptedView>> {
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<PeerAcceptedView>> {
        let guard = db_handle.blocking_lock();
        let rows = mv_sync::list_for_user(&guard, user_id)?;
        Ok(rows
            .into_iter()
            .map(|r| PeerAcceptedView {
                node_id_hex: hex::encode(&r.peer_node_id),
                identity_pub_hex: hex::encode(&r.peer_identity_pub),
                relay_url: r.relay_url,
                added_at: r.added_at,
            })
            .collect())
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn peer_forget(state: State<'_, AppState>, node_id_hex: String) -> Result<bool, String> {
    wire(peer_forget_impl(&state, node_id_hex).await)
}

async fn peer_forget_impl(state: &AppState, node_id_hex: String) -> AppResult<bool> {
    let node_id = hex::decode(&node_id_hex)
        .map_err(|_| AppError::BadRequest("node_id_hex must be hex".into()))?;
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<bool> {
        let guard = db_handle.blocking_lock();
        Ok(mv_sync::forget(&guard, user_id, &node_id)?)
    })
    .await
    .map_err(AppError::from)?
}
