//! Phase 3.2 album-sharing Tauri commands — wraps `mv_sync::share` +
//! `mv_sync::receive` for the React UI.
//!
//! Lazy-init mirrors the existing `commands::peer::ensure_peer`
//! pattern: iroh-blobs / iroh-docs / gossip / receive loop all come up
//! on the first share command, so peers who never open the Share
//! surface don't pay the cost.

use std::sync::Arc;

use iroh_gossip::net::Gossip;
use mv_core::db::queries as db;
use mv_sync::{
    accept_namespace_ticket, generate_share_ticket, publish_album_to_peer, revoke_peer,
    BlobsBridge, DocsBridge, ReceiveContext, ShareContext,
};
use tauri::State;

use super::albums::collection_key_for;
use crate::dto::{IncomingShareView, ShareInviteView, ShareRecipientView};
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

// --- Lazy-init helpers ---------------------------------------------------

async fn ensure_blobs(state: &AppState) -> AppResult<Arc<BlobsBridge>> {
    let (cas, db_arc) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        if let Some(b) = s.blobs.lock().await.clone() {
            return Ok(b);
        }
        (s.cas.clone(), s.db.clone())
    };
    let bridge = Arc::new(
        BlobsBridge::start(&state.vault_root, cas, db_arc)
            .await
            .map_err(AppError::from)?,
    );
    let guard = state.inner.lock().await;
    let s = guard.session.as_ref().ok_or(AppError::Locked)?;
    let mut slot = s.blobs.lock().await;
    if let Some(existing) = slot.clone() {
        Ok(existing)
    } else {
        *slot = Some(bridge.clone());
        Ok(bridge)
    }
}

async fn ensure_gossip(state: &AppState) -> AppResult<Gossip> {
    let peer = super::peer::ensure_peer_public(state).await?;
    let guard = state.inner.lock().await;
    let s = guard.session.as_ref().ok_or(AppError::Locked)?;
    let mut slot = s.gossip.lock().await;
    if let Some(g) = slot.clone() {
        return Ok(g);
    }
    let g = Gossip::builder().spawn(peer.endpoint().clone());
    *slot = Some(g.clone());
    Ok(g)
}

async fn ensure_docs(state: &AppState) -> AppResult<Arc<DocsBridge>> {
    // Order matters: blobs + gossip have to be live before Docs::spawn,
    // because Docs borrows both.
    let blobs = ensure_blobs(state).await?;
    let gossip = ensure_gossip(state).await?;
    let peer = super::peer::ensure_peer_public(state).await?;
    {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        let existing = s.docs.lock().await.clone();
        if let Some(d) = existing {
            return Ok(d);
        }
    }
    let blobs_store: iroh_blobs::api::Store = blobs.store().clone().into();
    let docs_bridge = DocsBridge::start(
        &state.vault_root,
        peer.endpoint().clone(),
        blobs_store,
        gossip.clone(),
    )
    .await
    .map_err(AppError::from)?;
    let docs_arc = Arc::new(docs_bridge);

    // Tell the peer to mount the full router (blobs + gossip + docs).
    peer.mount_router(blobs.clone(), Some(gossip), Some(docs_arc.clone()))
        .map_err(AppError::from)?;

    let guard = state.inner.lock().await;
    let s = guard.session.as_ref().ok_or(AppError::Locked)?;
    let mut slot = s.docs.lock().await;
    if let Some(d) = slot.clone() {
        Ok(d)
    } else {
        *slot = Some(docs_arc.clone());
        Ok(docs_arc)
    }
}

async fn ensure_receive_loop(state: &AppState) -> AppResult<()> {
    let docs = ensure_docs(state).await?;
    let blobs = ensure_blobs(state).await?;
    let (db_arc, user) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        // If handles already spawned, no-op.
        if !s.receive_handles.lock().await.is_empty() {
            return Ok(());
        }
        (s.db.clone(), s.user.clone())
    };
    let ctx = ReceiveContext {
        docs,
        blobs,
        conn: db_arc,
        user,
    };
    let handles = mv_sync::spawn_receive_loop(ctx)
        .await
        .map_err(AppError::from)?;
    let guard = state.inner.lock().await;
    let s = guard.session.as_ref().ok_or(AppError::Locked)?;
    let mut slot = s.receive_handles.lock().await;
    slot.extend(handles);
    Ok(())
}

// --- Commands ------------------------------------------------------------

#[tauri::command]
pub async fn share_album_with_peer(
    state: State<'_, AppState>,
    album_id: i64,
    peer_node_id_hex: String,
) -> Result<ShareInviteView, String> {
    wire(share_album_with_peer_impl(&state, album_id, peer_node_id_hex).await)
}

async fn share_album_with_peer_impl(
    state: &AppState,
    album_id: i64,
    peer_node_id_hex: String,
) -> AppResult<ShareInviteView> {
    ensure_receive_loop(state).await?;

    let node_id = hex::decode(&peer_node_id_hex)
        .map_err(|_| AppError::BadRequest("peer_node_id_hex must be hex".into()))?;
    if node_id.len() != 32 {
        return Err(AppError::BadRequest("peer_node_id must be 32 bytes".into()));
    }

    // Resolve collection key + recipient identity pub from the DB.
    let (ck_bytes, mk_bytes, user_id, db_arc, recipient_pub) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        let user_id = s.user.user_id;
        let default_cid = s.default_collection_id;
        let default_ck_bytes = *s.default_collection_key.as_bytes();
        let mk_bytes = *s.user.master_key.as_bytes();
        let unlocked_override_bytes: Option<[u8; 32]> =
            s.unlocked_albums.get(&album_id).map(|k| *k.as_bytes());
        let db_arc = s.db.clone();

        let (ck_bytes_inner, identity_pub): ([u8; 32], Vec<u8>) = {
            let c = db_arc.lock().await;
            let override_ck =
                unlocked_override_bytes.map(mv_core::crypto::CollectionKey::from_bytes);
            let ck = collection_key_for(
                mk_bytes,
                &c,
                user_id,
                album_id,
                default_cid,
                default_ck_bytes,
                override_ck.as_ref(),
            )?;
            let row: (Vec<u8>,) = c
                .query_row(
                    r"SELECT peer_identity_pub FROM peer_accept
                      WHERE owner_user_id = ?1 AND peer_node_id = ?2",
                    rusqlite::params![user_id, &node_id[..]],
                    |r| Ok((r.get(0)?,)),
                )
                .map_err(|_| AppError::BadRequest("peer not accepted".into()))?;
            (*ck.as_bytes(), row.0)
        };
        if identity_pub.len() != 32 {
            return Err(AppError::BadRequest("peer identity_pub malformed".into()));
        }
        (ck_bytes_inner, mk_bytes, user_id, db_arc, identity_pub)
    };

    let docs = ensure_docs(state).await?;
    let blobs = ensure_blobs(state).await?;
    let ctx = ShareContext {
        docs,
        blobs,
        conn: db_arc,
        owner_user_id: user_id,
        collection_key_bytes: ck_bytes,
        master_key_bytes: mk_bytes,
    };

    let mut recipient_arr = [0u8; 32];
    recipient_arr.copy_from_slice(&recipient_pub);
    let receipt = publish_album_to_peer(&ctx, album_id, &recipient_arr)
        .await
        .map_err(AppError::from)?;
    let ticket = generate_share_ticket(&ctx, album_id)
        .await
        .map_err(AppError::from)?;

    Ok(ShareInviteView {
        namespace_ticket_base32: ticket,
        collection_id: album_id,
        asset_count: receipt.asset_count as u32,
    })
}

#[tauri::command]
pub async fn list_album_shares(
    state: State<'_, AppState>,
    album_id: i64,
) -> Result<Vec<ShareRecipientView>, String> {
    wire(list_album_shares_impl(&state, album_id).await)
}

async fn list_album_shares_impl(
    state: &AppState,
    album_id: i64,
) -> AppResult<Vec<ShareRecipientView>> {
    let (db_arc, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<ShareRecipientView>> {
        let c = db_arc.blocking_lock();
        let pairs = db::list_peer_wrappings_for_collection(&c, album_id)?;
        let _ = user_id; // reserved for future owner-check (e.g. hidden albums)
        let mut out = Vec::with_capacity(pairs.len());
        for (identity_pub, _wrapped) in pairs {
            // Reverse-map back to the node_id via peer_accept.
            let row: Option<(Vec<u8>, Option<String>, i64)> = c
                .query_row(
                    r"SELECT peer_node_id, relay_url, added_at FROM peer_accept
                      WHERE peer_identity_pub = ?1",
                    rusqlite::params![&identity_pub],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .ok();
            let (node_id, relay_url, shared_at) = match row {
                Some((n, r, a)) => (n, r, a),
                // Wrapping exists but peer was forgotten; still list
                // with a blank node_id for UI.
                None => (Vec::new(), None, 0),
            };
            out.push(ShareRecipientView {
                peer_node_id_hex: hex::encode(&node_id),
                peer_identity_pub_hex: hex::encode(&identity_pub),
                relay_url,
                shared_at,
            });
        }
        Ok(out)
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn revoke_album_share(
    state: State<'_, AppState>,
    album_id: i64,
    peer_node_id_hex: String,
) -> Result<(), String> {
    wire(revoke_album_share_impl(&state, album_id, peer_node_id_hex).await)
}

async fn revoke_album_share_impl(
    state: &AppState,
    album_id: i64,
    peer_node_id_hex: String,
) -> AppResult<()> {
    let node_id = hex::decode(&peer_node_id_hex)
        .map_err(|_| AppError::BadRequest("peer_node_id_hex must be hex".into()))?;

    let (ck_bytes, mk_bytes, user_id, db_arc, recipient_pub) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        let user_id = s.user.user_id;
        let default_cid = s.default_collection_id;
        let default_ck_bytes = *s.default_collection_key.as_bytes();
        let mk_bytes = *s.user.master_key.as_bytes();
        let unlocked_override_bytes: Option<[u8; 32]> =
            s.unlocked_albums.get(&album_id).map(|k| *k.as_bytes());
        let db_arc = s.db.clone();

        let (ck_bytes_inner, identity_pub): ([u8; 32], Vec<u8>) = {
            let c = db_arc.lock().await;
            let override_ck =
                unlocked_override_bytes.map(mv_core::crypto::CollectionKey::from_bytes);
            let ck = collection_key_for(
                mk_bytes,
                &c,
                user_id,
                album_id,
                default_cid,
                default_ck_bytes,
                override_ck.as_ref(),
            )?;
            let row: (Vec<u8>,) = c
                .query_row(
                    r"SELECT peer_identity_pub FROM peer_accept
                      WHERE owner_user_id = ?1 AND peer_node_id = ?2",
                    rusqlite::params![user_id, &node_id[..]],
                    |r| Ok((r.get(0)?,)),
                )
                .map_err(|_| AppError::BadRequest("peer not accepted".into()))?;
            (*ck.as_bytes(), row.0)
        };
        (ck_bytes_inner, mk_bytes, user_id, db_arc, identity_pub)
    };

    let docs = ensure_docs(state).await?;
    let blobs = ensure_blobs(state).await?;
    let mut ctx = ShareContext {
        docs,
        blobs,
        conn: db_arc,
        owner_user_id: user_id,
        collection_key_bytes: ck_bytes,
        master_key_bytes: mk_bytes,
    };

    let mut recipient_arr = [0u8; 32];
    recipient_arr.copy_from_slice(&recipient_pub);
    revoke_peer(&mut ctx, album_id, &recipient_arr)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn list_incoming_shares(
    state: State<'_, AppState>,
) -> Result<Vec<IncomingShareView>, String> {
    wire(list_incoming_shares_impl(&state).await)
}

async fn list_incoming_shares_impl(state: &AppState) -> AppResult<Vec<IncomingShareView>> {
    let (db_arc, user_id, identity_pub) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id, s.user.identity.public.0)
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<IncomingShareView>> {
        let c = db_arc.blocking_lock();
        let mut stmt = c.prepare(
            r"SELECT sn.collection_id, sn.namespace_id, sn.author_id, c.name_ct, c.kind
              FROM shared_namespace sn
              JOIN collection c ON c.id = sn.collection_id
              WHERE sn.role = 'peer' AND c.owner_id = ?1
              ORDER BY sn.created_at DESC",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![user_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for (cid, ns, author, name_ct, _kind) in rows {
            // Try decrypting the album name with our wrapping; if
            // missing, state=pending.
            let sealed_ck = db::get_peer_wrapped_collection_key(
                &c,
                cid,
                &identity_pub,
                mv_core::share::WRAPPING_PEER,
            )?;
            let state = if sealed_ck.is_some() {
                "accepted"
            } else {
                "pending"
            };
            let album_name = if name_ct == b"(incoming share)".as_slice() || name_ct.is_empty() {
                None
            } else {
                // If we can open it, surface the name.
                sealed_ck.as_ref().and_then(|s| {
                    mv_core::share::open_shared_collection(s, &unlocked_user_identity_stub())
                        .ok()
                        .and_then(|_ck| {
                            // Phase 3.2 MVP: for this list we only flag
                            // "accepted" vs "pending"; album names
                            // render through the normal Albums list
                            // because collection.name_ct has been
                            // overwritten by the meta event.
                            None
                        })
                })
            };
            let _ = author;
            out.push(IncomingShareView {
                collection_id: cid,
                namespace_id_hex: hex::encode(&ns),
                sender_identity_pub_hex: String::new(),
                state: state.to_string(),
                album_name,
            });
        }
        Ok(out)
    })
    .await
    .map_err(AppError::from)?
}

// We can't actually call open_shared_collection without the session's
// UnlockedUser inside spawn_blocking (no Send issues but the method
// needs the identity keypair). For now we leave album_name=None in the
// list — the main Albums list already surfaces the decrypted name.
fn unlocked_user_identity_stub() -> mv_core::crypto::envelope::PeerKeypair {
    mv_core::crypto::envelope::PeerKeypair::from_bytes([0u8; 32], [0u8; 32])
}

#[tauri::command]
pub async fn accept_incoming_share(
    state: State<'_, AppState>,
    namespace_ticket_base32: String,
) -> Result<i64, String> {
    wire(accept_incoming_share_impl(&state, namespace_ticket_base32).await)
}

async fn accept_incoming_share_impl(state: &AppState, base32: String) -> AppResult<i64> {
    let docs = ensure_docs(state).await?;
    let blobs = ensure_blobs(state).await?;
    let (db_arc, user) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.clone())
    };
    let ctx = ReceiveContext {
        docs,
        blobs,
        conn: db_arc,
        user,
    };
    let cid = accept_namespace_ticket(&ctx, &base32)
        .await
        .map_err(AppError::from)?;
    // Make sure the broader loop is running too so its catch-up sees
    // the newly-imported namespace's events.
    ensure_receive_loop(state).await?;
    Ok(cid)
}

// =========== Public share-links (D7) ==========================================

use mv_core::public_link;
use rusqlite::OptionalExtension;

#[tauri::command]
pub async fn create_public_link(
    state: State<'_, AppState>,
    collection_id: i64,
    password: Option<String>,
    expires_at: Option<i64>,
) -> Result<crate::dto::PublicLinkView, String> {
    wire(create_public_link_impl(&state, collection_id, password, expires_at).await)
}

async fn create_public_link_impl(
    state: &AppState,
    collection_id: i64,
    password: Option<String>,
    expires_at: Option<i64>,
) -> AppResult<crate::dto::PublicLinkView> {
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };

    let pw_secret = password.map(secrecy::SecretString::from);
    let link = tokio::task::spawn_blocking(move || -> AppResult<crate::dto::PublicLinkView> {
        let new = public_link::generate(pw_secret.as_ref())?;
        let guard = db_handle.blocking_lock();
        // Verify collection is owned by the session user. Reject attempts to
        // publish someone else's collection even if they somehow guess the id.
        let owner_check: Option<i64> = guard
            .query_row(
                "SELECT owner_id FROM collection WHERE id = ?1",
                rusqlite::params![collection_id],
                |r| r.get(0),
            )
            .optional()?;
        match owner_check {
            Some(owner) if owner == user_id => {}
            _ => return Err(AppError::NotFound),
        }

        let now = chrono::Utc::now().timestamp();
        let id = mv_core::db::queries::insert_public_link(
            &guard,
            collection_id,
            user_id,
            &new.pub_id,
            new.has_password,
            new.password_salt.as_ref(),
            new.wrapped_key.as_deref(),
            expires_at,
            now,
        )?;
        let url_fragment = new.url_fragment();
        Ok(crate::dto::PublicLinkView {
            id,
            collection_id,
            pub_id_b32: new.pub_id_b32,
            url_fragment,
            has_password: new.has_password,
            expires_at,
            created_at: now,
        })
    })
    .await
    .map_err(AppError::from)??;
    Ok(link)
}

#[tauri::command]
pub async fn list_public_links(
    state: State<'_, AppState>,
) -> Result<Vec<crate::dto::PublicLinkView>, String> {
    wire(list_public_links_impl(&state).await)
}

async fn list_public_links_impl(state: &AppState) -> AppResult<Vec<crate::dto::PublicLinkView>> {
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<Vec<crate::dto::PublicLinkView>> {
        let guard = db_handle.blocking_lock();
        let rows = mv_core::db::queries::list_public_links_for_user(&guard, user_id)?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let mut pub_bytes = [0u8; 16];
                if r.pub_id.len() == 16 {
                    pub_bytes.copy_from_slice(&r.pub_id);
                }
                let pub_id_b32 =
                    base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &pub_bytes)
                        .to_ascii_lowercase();
                crate::dto::PublicLinkView {
                    id: r.id,
                    collection_id: r.collection_id,
                    pub_id_b32,
                    // After a list, we no longer have the viewer key — the
                    // URL fragment is only surfaced at create time. UIs
                    // that need to re-share must create a new link.
                    url_fragment: String::new(),
                    has_password: r.has_password,
                    expires_at: r.expires_at,
                    created_at: r.created_at,
                }
            })
            .collect())
    })
    .await
    .map_err(AppError::from)?
}

#[tauri::command]
pub async fn revoke_public_link(state: State<'_, AppState>, id: i64) -> Result<bool, String> {
    wire(revoke_public_link_impl(&state, id).await)
}

async fn revoke_public_link_impl(state: &AppState, id: i64) -> AppResult<bool> {
    let (db_handle, user_id) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        (s.db.clone(), s.user.user_id)
    };
    tokio::task::spawn_blocking(move || -> AppResult<bool> {
        let guard = db_handle.blocking_lock();
        mv_core::db::queries::delete_public_link(&guard, id, user_id).map_err(AppError::from)
    })
    .await
    .map_err(AppError::from)?
}
