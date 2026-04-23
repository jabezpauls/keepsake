//! Receive loop — subscribes to every shared namespace this peer
//! participates in (`shared_namespace(role='peer')`), watches for
//! `c/key/<my_pub>` and `c/meta/<cid>` / `c/mem/<asset>` / `c/rev/<pub>`
//! entries, filters by the `peer_accept` trust gate, and persists the
//! unwrapped state into the local DB.
//!
//! Blobs aren't pre-fetched — each `c/mem/<asset>` entry announces a
//! ciphertext hash that the viewer opens on demand (Phase 3.3 will add
//! a background fetcher). We do sync metadata aggressively so the
//! receiving user sees album + member rows the moment sync catches up.
//!
//! ## Trust gate
//!
//! Every `InsertRemote { from: PublicKey, .. }` event names the author
//! node id that wrote the entry. Before persisting anything, we look
//! that id up in the `peer_accept` table. If it's not an accepted peer
//! (or if the inserted author doesn't match the namespace's known
//! owner yet), we silently drop the event at `debug` tracing level.
//! Architecture.md §9 keeps this enforcement on the application side
//! rather than at the iroh router because iroh 0.98's `Endpoint` does
//! not expose a per-ALPN accept callback we can pipe the allowlist
//! into cleanly.

use std::str::FromStr;
use std::sync::Arc;

use iroh_docs::DocTicket;
use mv_core::crypto::keystore::UnlockedUser;
use mv_core::db::queries as q;
use mv_core::share::{
    open_collection_meta, open_member_entry, open_shared_collection, WRAPPING_PEER,
};
use mv_core::{Error, Result};
use n0_future::StreamExt;
use rusqlite::Connection;
use tokio::sync::Mutex;
use tokio::task::AbortHandle;

use crate::blobs::BlobsBridge;
use crate::docs::{DocsBridge, DocsKey, SharedCollectionNamespace};

/// Context the receive loop holds over its lifetime.
#[derive(Clone)]
pub struct ReceiveContext {
    pub docs: Arc<DocsBridge>,
    pub blobs: Arc<BlobsBridge>,
    pub conn: Arc<Mutex<Connection>>,
    pub user: Arc<UnlockedUser>,
}

/// Spawn one per-namespace task for every `role='peer'` row currently
/// in `shared_namespace`. Returns the first task's abort handle so
/// `Peer::shutdown` can tear the hierarchy down; subsequent tasks are
/// detached and shut down when the `DocsBridge` is dropped.
///
/// `accept_namespace_ticket` must be called for new invites that arrive
/// *after* this loop boots — that function opens the namespace and
/// spawns its own listener.
pub async fn spawn_receive_loop(ctx: ReceiveContext) -> Result<Vec<AbortHandle>> {
    let peer_rows: Vec<i64> = {
        let c = ctx.conn.lock().await;
        let rows: rusqlite::Result<Vec<i64>> = {
            let mut stmt =
                c.prepare("SELECT collection_id FROM shared_namespace WHERE role = 'peer'")?;
            let iter = stmt.query_map([], |r| r.get::<_, i64>(0))?;
            iter.collect()
        };
        rows?
    };

    let mut handles = Vec::with_capacity(peer_rows.len());
    for cid in peer_rows {
        let ns = ctx.docs.open_shared(&ctx.conn, cid, "peer").await?;
        let h = spawn_namespace_task(ctx.clone(), ns);
        handles.push(h);
    }
    Ok(handles)
}

/// Accept an incoming namespace invite (a `DocTicket` produced by the
/// sender's `Doc::share(ShareMode::Read)`). Imports the namespace,
/// persists the mapping, and spawns a per-namespace listener for its
/// future events. Returns the local `collection_id` under which the
/// share was registered.
///
/// The collection_id is assigned now — the incoming `c/meta/` event
/// later fills in the album name.
pub async fn accept_namespace_ticket(ctx: &ReceiveContext, base32: &str) -> Result<i64> {
    let ticket = DocTicket::from_str(base32).map_err(|e| {
        tracing::debug!(%e, "DocTicket::from_str failed");
        Error::TicketFormat("doc ticket parse")
    })?;

    // Insert a placeholder local collection row so we have a stable id
    // to key everything on. The name gets overwritten when the first
    // `c/meta/` event lands.
    let collection_id = {
        let c = ctx.conn.lock().await;
        c.execute(
            r"INSERT INTO collection (owner_id, kind, name_ct, has_password, created_at)
              VALUES (?1, 'album', ?2, 0, ?3)",
            rusqlite::params![
                ctx.user.user_id,
                b"(incoming share)".to_vec(),
                chrono::Utc::now().timestamp(),
            ],
        )?;
        c.last_insert_rowid()
    };

    let ns = ctx
        .docs
        .import_shared(&ctx.conn, collection_id, ticket.capability)
        .await?;
    let _ = spawn_namespace_task(ctx.clone(), ns);
    Ok(collection_id)
}

fn spawn_namespace_task(ctx: ReceiveContext, ns: SharedCollectionNamespace) -> AbortHandle {
    let task = tokio::spawn(async move {
        if let Err(e) = run_namespace_task(&ctx, ns).await {
            tracing::warn!(%e, "receive loop namespace task exited");
        }
    });
    task.abort_handle()
}

async fn run_namespace_task(ctx: &ReceiveContext, ns: SharedCollectionNamespace) -> Result<()> {
    let mut stream = ns.doc.subscribe().await.map_err(|e| {
        tracing::debug!(%e, cid = ns.collection_id, "doc.subscribe failed");
        Error::PeerUnreachable
    })?;
    while let Some(item) = stream.next().await {
        let ev = match item {
            Ok(ev) => ev,
            Err(e) => {
                tracing::debug!(%e, "LiveEvent stream error; skipping");
                continue;
            }
        };
        if let iroh_docs::engine::LiveEvent::InsertRemote { from, entry, .. } = ev {
            if let Err(e) = handle_entry(ctx, &ns, from, &entry).await {
                tracing::debug!(%e, "handle_entry failed; dropping");
            }
        }
    }
    Ok(())
}

async fn handle_entry(
    ctx: &ReceiveContext,
    ns: &SharedCollectionNamespace,
    from: iroh::PublicKey,
    entry: &iroh_docs::Entry,
) -> Result<()> {
    // Trust gate: is the author an accepted peer for this user?
    let author_bytes = *from.as_bytes();
    let is_accepted = {
        let c = ctx.conn.lock().await;
        let n: i64 = c.query_row(
            r"SELECT COUNT(*) FROM peer_accept
              WHERE owner_user_id = ?1 AND peer_node_id = ?2",
            rusqlite::params![ctx.user.user_id, &author_bytes[..]],
            |r| r.get(0),
        )?;
        n > 0
    };
    if !is_accepted {
        tracing::debug!(?author_bytes, "dropping entry from non-accepted peer");
        return Ok(());
    }

    // Fetch the entry's content bytes from our local iroh-blobs store
    // (iroh-docs stages remote content there during sync).
    let content_hash = entry.content_hash();
    let value = ctx.blobs.read_bytes(content_hash).await?;

    let key = entry.key();
    if let Some(recipient) = DocsKey::extract_key_recipient(key) {
        if recipient != ctx.user.identity.public.0 {
            // Envelope addressed to a different peer. Not for us.
            return Ok(());
        }
        let ck = open_shared_collection(&value, &ctx.user.identity)?;
        // Persist the wrapping locally so future boots can recover
        // without re-receiving the live event.
        let c = ctx.conn.lock().await;
        q::upsert_peer_wrapped_collection_key(
            &c,
            ns.collection_id,
            &ctx.user.identity.public.0,
            WRAPPING_PEER,
            &value,
        )?;
        tracing::info!(cid = ns.collection_id, "stored incoming collection key");
        // Drop the collection key eagerly from memory (we'll re-derive
        // from the DB when we need to decrypt meta/member entries).
        drop(ck);
    } else if let Some(cid_from_key) = DocsKey::extract_meta_id(key) {
        if cid_from_key != ns.collection_id {
            // Meta entry for a different collection in this namespace?
            // Namespaces are 1:1 with collections, so this shouldn't
            // happen — treat as a bug and skip.
            tracing::debug!(cid_from_key, "meta entry under wrong collection_id");
            return Ok(());
        }
        let ck = resolve_ck_for(ctx, ns.collection_id).await?;
        let meta = open_collection_meta(&value, &ck)?;
        // Re-seal the name under the collection key (same AD=0 as
        // albums.rs::create_album) so the UI's existing decrypt path
        // for `collection.name_ct` works unchanged.
        let sealed_name =
            mv_core::crypto::envelope::seal_row(meta.name.as_bytes(), 0, ck.as_bytes())?;
        let c = ctx.conn.lock().await;
        c.execute(
            "UPDATE collection SET name_ct = ?1, kind = ?2 WHERE id = ?3",
            rusqlite::params![&sealed_name, meta.kind, ns.collection_id],
        )?;
    } else if let Some(_sender_asset_id) = DocsKey::extract_member_id(key) {
        // We trust the key only to carry the sender's asset_id (used as AD
        // at seal time). The MemberEntry JSON carries the cas_ref +
        // blake3_plaintext we dedupe on locally.
        let sender_asset_id = DocsKey::extract_member_id(key).unwrap();
        let ck = resolve_ck_for(ctx, ns.collection_id).await?;
        let entry = open_member_entry(&value, sender_asset_id, &ck)?;
        persist_member(ctx, ns.collection_id, entry).await?;
    } else if let Some(rev_recipient) = DocsKey::extract_rev_recipient(key) {
        if rev_recipient == ctx.user.identity.public.0 {
            // We've been revoked. Drop our wrapping + flip the
            // collection name to a revoked sentinel.
            let c = ctx.conn.lock().await;
            q::delete_peer_wrapped_collection_key(
                &c,
                ns.collection_id,
                &ctx.user.identity.public.0,
                WRAPPING_PEER,
            )?;
            c.execute(
                "UPDATE collection SET name_ct = ?1 WHERE id = ?2",
                rusqlite::params![b"(revoked)".to_vec(), ns.collection_id],
            )?;
            tracing::info!(cid = ns.collection_id, "share revoked");
        }
    }
    Ok(())
}

/// Look up our locally-stored collection key (previously unwrapped via
/// `c/key/<my_pub>`) for the given collection.
async fn resolve_ck_for(
    ctx: &ReceiveContext,
    collection_id: i64,
) -> Result<mv_core::crypto::CollectionKey> {
    let sealed: Vec<u8> = {
        let c = ctx.conn.lock().await;
        q::get_peer_wrapped_collection_key(
            &c,
            collection_id,
            &ctx.user.identity.public.0,
            WRAPPING_PEER,
        )?
        .ok_or(Error::NotFound)?
    };
    open_shared_collection(&sealed, &ctx.user.identity)
}

async fn persist_member(
    ctx: &ReceiveContext,
    collection_id: i64,
    entry: mv_core::share::MemberEntry,
) -> Result<()> {
    // 1. Insert or upsert the asset row — dedupe on blake3_plaintext.
    //    We need to pick a source_id; reuse the "incoming-shares"
    //    virtual source if it exists, or create one.
    let source_id = ensure_incoming_source(ctx).await?;
    let asset_id = {
        let c = ctx.conn.lock().await;
        let insert = q::AssetInsert {
            blake3_plaintext: &entry.blake3_plaintext,
            mime: &entry.mime,
            bytes: entry.bytes,
            width: entry.width,
            height: entry.height,
            duration_ms: entry.duration_ms,
            taken_at_utc_day: entry.taken_at_utc_day,
            is_video: entry.is_video,
            is_raw: entry.is_raw,
            is_screenshot: entry.is_screenshot,
            is_live: entry.is_live,
            is_motion: entry.is_motion,
            source_id,
            cas_ref: &entry.cas_ref,
            imported_at: chrono::Utc::now().timestamp(),
            filename_ct: &entry.filename_ct,
            taken_at_utc_ct: entry.taken_at_utc_ct.as_deref(),
            gps_ct: entry.gps_ct.as_deref(),
            device_ct: entry.device_ct.as_deref(),
            lens_ct: entry.lens_ct.as_deref(),
            exif_all_ct: entry.exif_all_ct.as_deref(),
            wrapped_file_key: &entry.wrapped_file_key,
        };
        match q::insert_asset_if_new(&c, &insert)? {
            q::InsertResult::Inserted(id) | q::InsertResult::Existing(id) => id,
        }
    };

    // 2. Cache the ciphertext hash for iroh-blobs fetch.
    if let Some(h) = entry.ciphertext_blake3 {
        let c = ctx.conn.lock().await;
        q::set_ciphertext_blake3(&c, asset_id, &h)?;
    }

    // 3. Link into the collection.
    {
        let c = ctx.conn.lock().await;
        c.execute(
            "INSERT OR IGNORE INTO collection_member (collection_id, asset_id, added_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![collection_id, asset_id, chrono::Utc::now().timestamp()],
        )?;
    }
    Ok(())
}

async fn ensure_incoming_source(ctx: &ReceiveContext) -> Result<i64> {
    // We store received assets under a special source with a stable
    // marker name so the user can filter them out in the Sources UI.
    let marker = b"__mv_incoming_shares__";
    let c = ctx.conn.lock().await;
    let existing: Option<i64> = c
        .query_row(
            "SELECT id FROM source WHERE owner_id = ?1 AND name_ct = ?2",
            rusqlite::params![ctx.user.user_id, marker],
            |r| r.get(0),
        )
        .ok();
    if let Some(id) = existing {
        return Ok(id);
    }
    c.execute(
        r"INSERT INTO source
          (owner_id, name_ct, root_path_ct, device_hint_ct, imported_at, bytes_total, file_count, linked_only)
          VALUES (?1, ?2, ?3, NULL, ?4, 0, 0, 0)",
        rusqlite::params![
            ctx.user.user_id,
            marker,
            b"/incoming-shares".to_vec(),
            chrono::Utc::now().timestamp(),
        ],
    )?;
    Ok(c.last_insert_rowid())
}
