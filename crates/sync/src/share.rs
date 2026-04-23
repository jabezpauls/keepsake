//! Outbound album sharing — C8 publish + C10 revoke/rotate (C10 lands in
//! a follow-up commit; the skeleton here is just publish).
//!
//! Given an unlocked `CollectionKey` for a local album and a peer's
//! X25519 identity public key, [`publish_album_to_peer`] does the full
//! outbound dance described in `plans/phase-3-peers-smart.md §3.1`:
//!
//! 1. Look up (or create) the shared iroh-docs namespace for this album.
//! 2. Seal the collection key with the recipient's X25519 pub and write
//!    it to `c/key/<recipient>`.
//! 3. Persist a sender-side copy of the wrapping in
//!    `collection_key (wrapping='peer_x25519')` so the sender can list
//!    "who has access" later.
//! 4. Seal the album's `name` (decrypted under the collection key from
//!    the local `collection.name_ct` column) as a `CollectionMeta` row
//!    and write to `c/meta/<cid>`.
//! 5. For each member asset: build a `MemberEntry` from the local
//!    `asset` row (all sealed-under-collection-key bytes are passed
//!    through verbatim; `wrapped_file_key` is already wrapped under the
//!    collection key at ingest time). Seal the whole JSON under the
//!    collection key, write to `c/mem/<asset_id>`, and
//!    [`BlobsBridge::announce_asset`] the ciphertext so the recipient
//!    can fetch it.

use std::sync::Arc;

use iroh_docs::api::protocol::{AddrInfoOptions, ShareMode};
use iroh_docs::api::Doc;
use mv_core::crypto::envelope::{open_row, seal_row, wrap_collection_key};
use mv_core::crypto::{CollectionKey, MasterKey};
use mv_core::db::queries as q;
use mv_core::share::{
    seal_collection_for_peer, seal_collection_meta, seal_member_entry, CollectionMeta, MemberEntry,
    WRAPPING_PEER,
};
use mv_core::{Error, Result};
use rusqlite::Connection;
use tokio::sync::Mutex;

use crate::blobs::BlobsBridge;
use crate::docs::{DocsBridge, DocsKey};

/// Context shared across every outbound share operation. Owned by
/// short-lived share commands; nothing inside holds the DB lock across
/// await points — each await re-acquires.
pub struct ShareContext {
    pub docs: Arc<DocsBridge>,
    pub blobs: Arc<BlobsBridge>,
    pub conn: Arc<Mutex<Connection>>,
    pub owner_user_id: i64,
    /// Unlocked collection-key bytes. Lives only as long as the share
    /// context itself; callers resolve via
    /// `app/src-tauri/.../commands/albums::collection_key_for`.
    pub collection_key_bytes: [u8; 32],
    /// Unlocked master-key bytes — used to re-wrap the collection key
    /// for the owner after [`rotate_collection_key`]. Callers get this
    /// from `session.user.master_key`.
    pub master_key_bytes: [u8; 32],
}

/// Summary returned by [`publish_album_to_peer`]. Useful for audit
/// surfaces + C14's `share_status` row.
#[derive(Debug, Clone)]
pub struct ShareReceipt {
    pub namespace_id: [u8; 32],
    pub asset_count: usize,
    pub meta_published: bool,
}

/// Publish (or re-publish) an album to a single peer recipient. iroh-
/// docs CRDT semantics make this idempotent — resharing writes the
/// same entries; last-writer-wins leaves no duplicate state.
pub async fn publish_album_to_peer(
    ctx: &ShareContext,
    collection_id: i64,
    recipient_identity_pub: &[u8; 32],
) -> Result<ShareReceipt> {
    // 1. Ensure the shared namespace exists on the sender side. `open_shared`
    //    errs with `NotFound` when missing; we catch that and create.
    let ns = match ctx
        .docs
        .open_shared(&ctx.conn, collection_id, "owner")
        .await
    {
        Ok(n) => n,
        Err(Error::NotFound) | Err(Error::Db(_)) => {
            ctx.docs.create_shared(&ctx.conn, collection_id).await?
        }
        Err(e) => return Err(e),
    };
    let namespace_id: [u8; 32] = *AsRef::<[u8; 32]>::as_ref(&ns.doc.id());

    // Reconstruct the collection key in a short-lived local so we can
    // pass it to the core seal helpers.
    let ck = CollectionKey::from_bytes(ctx.collection_key_bytes);

    // 2. Seal the collection key for this recipient and write it.
    let sealed_key = seal_collection_for_peer(&ck, recipient_identity_pub)?;
    set_doc_bytes(
        &ns.doc,
        ns.author,
        DocsKey::collection_key(recipient_identity_pub),
        sealed_key.clone(),
    )
    .await?;

    // 3. Mirror the wrapping on the sender side so `list_album_shares`
    //    can enumerate recipients out of the DB without re-reading the
    //    namespace. Ignore the return — idempotent upsert.
    {
        let c = ctx.conn.lock().await;
        q::upsert_peer_wrapped_collection_key(
            &c,
            collection_id,
            recipient_identity_pub,
            WRAPPING_PEER,
            &sealed_key,
        )?;
    }

    // 4. Re-seal collection meta under the collection key.
    let meta_published = publish_meta(ctx, &ns.doc, ns.author, collection_id, &ck).await?;

    // 5. Publish every member + announce its ciphertext.
    let asset_ids = {
        let c = ctx.conn.lock().await;
        q::list_collection_member_ids(&c, collection_id)?
    };
    for asset_id in &asset_ids {
        publish_member(ctx, &ns.doc, ns.author, *asset_id, &ck).await?;
        // Announce the ciphertext blob (idempotent per BlobsBridge).
        let _ = ctx.blobs.announce_asset(*asset_id).await?;
    }

    Ok(ShareReceipt {
        namespace_id,
        asset_count: asset_ids.len(),
        meta_published,
    })
}

async fn publish_meta(
    ctx: &ShareContext,
    doc: &Doc,
    author: iroh_docs::AuthorId,
    collection_id: i64,
    ck: &CollectionKey,
) -> Result<bool> {
    // Pull the collection row, decrypt its name with the master key —
    // wait, no: `collection.name_ct` was sealed with the master key in
    // Phase 1, but Phase 3.2 switched albums to have their name sealed
    // under the collection key (that's what `seal_row(name, 0, ck)` in
    // albums.rs does). We read it and decrypt accordingly.
    let row = {
        let c = ctx.conn.lock().await;
        q::get_collection(&c, collection_id)?.ok_or(Error::NotFound)?
    };
    // Album names are sealed under the collection key at create time
    // (see app/src-tauri/src/commands/albums.rs::create_album).
    let name_plain = open_row(&row.name_ct, 0, ck.as_bytes())?;
    let name = String::from_utf8(name_plain).map_err(|_| Error::BlobFormat)?;
    // Cover-asset metadata is a follow-up — the receiver renders without
    // a cover just fine for the C8 slice.
    let meta = CollectionMeta {
        kind: row.kind,
        name,
        cover_blake3_plaintext: None,
    };
    let sealed = seal_collection_meta(&meta, ck)?;
    set_doc_bytes(doc, author, DocsKey::collection_meta(collection_id), sealed).await?;
    Ok(true)
}

async fn publish_member(
    ctx: &ShareContext,
    doc: &Doc,
    author: iroh_docs::AuthorId,
    asset_id: i64,
    ck: &CollectionKey,
) -> Result<()> {
    // Fetch the asset row + its ciphertext hash.
    let (row, ct_hash) = {
        let c = ctx.conn.lock().await;
        let row = q::get_asset(&c, asset_id)?.ok_or(Error::NotFound)?;
        let ct_hash: Option<Vec<u8>> = c.query_row(
            "SELECT ciphertext_blake3 FROM asset WHERE id = ?1",
            [asset_id],
            |r| r.get(0),
        )?;
        (row, ct_hash)
    };

    // If the ciphertext hash isn't cached yet, compute it now (the
    // blobs bridge will do this anyway on announce, but we want it
    // inside the member entry for the receiver).
    let ct_hash_arr: Option<[u8; 32]> = if let Some(h) = ct_hash {
        if h.len() == 32 {
            let mut a = [0u8; 32];
            a.copy_from_slice(&h);
            Some(a)
        } else {
            None
        }
    } else {
        let h = ctx.blobs.cas().compute_ciphertext_blake3(&row.cas_ref)?;
        let c = ctx.conn.lock().await;
        q::set_ciphertext_blake3(&c, asset_id, &h)?;
        Some(h)
    };

    let entry = MemberEntry {
        cas_ref: row.cas_ref,
        blake3_plaintext: row.blake3_plaintext,
        ciphertext_blake3: ct_hash_arr,
        wrapped_file_key: row.wrapped_file_key,
        mime: row.mime,
        bytes: row.bytes,
        width: row.width,
        height: row.height,
        duration_ms: row.duration_ms,
        taken_at_utc_day: row.taken_at_utc_day,
        is_video: row.is_video,
        is_raw: row.is_raw,
        is_screenshot: row.is_screenshot,
        is_live: row.is_live,
        is_motion: row.is_motion,
        filename_ct: row.filename_ct,
        taken_at_utc_ct: row.taken_at_utc_ct,
        gps_ct: row.gps_ct,
        device_ct: row.device_ct,
        lens_ct: row.lens_ct,
        exif_all_ct: row.exif_all_ct,
    };
    let sealed = seal_member_entry(&entry, asset_id, ck)?;
    set_doc_bytes(doc, author, DocsKey::collection_member(asset_id), sealed).await?;
    Ok(())
}

/// Write a `c/rev/<recipient>` tombstone, delete the recipient's DB
/// wrapping, and rotate the collection key so future metadata is only
/// readable by the remaining recipients (+ the owner).
///
/// **Ciphertext blobs are NOT re-encrypted.** Old CAS blobs remain
/// decryptable by any device that already has the old collection key
/// (the revoked peer, if they cached it). This matches the design in
/// `plans/phase-3-peers-smart.md §3.3` — a future Phase 3.3 background
/// worker can re-encrypt on demand; for now the practical effect is
/// "revoked peer cannot learn about *new* members added after revoke,
/// and cannot open *new* metadata updates".
pub async fn revoke_peer(
    ctx: &mut ShareContext,
    collection_id: i64,
    recipient_identity_pub: &[u8; 32],
) -> Result<()> {
    // Open (we expect this) the existing owner-side namespace.
    let ns = ctx
        .docs
        .open_shared(&ctx.conn, collection_id, "owner")
        .await?;

    // Write a tombstone sealed under the *current* ck so receivers with
    // the old key can authenticate it. Payload is a serialised
    // timestamp — the key itself (`c/rev/<recipient>`) carries the
    // meaning.
    let ck_old = CollectionKey::from_bytes(ctx.collection_key_bytes);
    let tombstone = seal_row(
        &chrono::Utc::now().timestamp().to_be_bytes(),
        0,
        ck_old.as_bytes(),
    )?;
    set_doc_bytes(
        &ns.doc,
        ns.author,
        DocsKey::collection_revocation(recipient_identity_pub),
        tombstone,
    )
    .await?;

    // Drop the DB wrapping for the revoked recipient.
    {
        let c = ctx.conn.lock().await;
        q::delete_peer_wrapped_collection_key(
            &c,
            collection_id,
            recipient_identity_pub,
            WRAPPING_PEER,
        )?;
    }

    // Rotate the collection key for the remaining members.
    rotate_collection_key(ctx, collection_id).await?;
    Ok(())
}

/// Rotate the collection key:
///   1. Generate a fresh `CollectionKey`.
///   2. Seal it for every remaining peer recipient and overwrite their
///      `c/key/<pub>` entry + sender-side DB wrapping.
///   3. Re-wrap it under the owner's master key + overwrite the
///      `wrapping='master'` row.
///
/// On return, `ctx.collection_key_bytes` carries the new key so follow-up
/// operations (e.g. meta updates) seal under the rotated key.
///
/// Album-password wrappings are *not* rotated here — we'd need the user's
/// password and re-wrapping without it would break unlock. Callers must
/// rotate password wrappings separately (future commit: surface a
/// "rotate-with-password" command in the UI). For the C14 acceptance
/// test, albums under test aren't password-protected.
pub async fn rotate_collection_key(ctx: &mut ShareContext, collection_id: i64) -> Result<()> {
    let new_ck = CollectionKey::random()?;

    // Re-seal for every remaining recipient (the revoked one was
    // already deleted from the DB, so it won't appear in the list).
    let recipients = {
        let c = ctx.conn.lock().await;
        q::list_peer_wrappings_for_collection(&c, collection_id)?
    };
    let ns = ctx
        .docs
        .open_shared(&ctx.conn, collection_id, "owner")
        .await?;
    for (pub_bytes, _old_wrap) in recipients {
        if pub_bytes.len() != 32 {
            continue;
        }
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&pub_bytes);
        let sealed_new = seal_collection_for_peer(&new_ck, &pk)?;
        set_doc_bytes(
            &ns.doc,
            ns.author,
            DocsKey::collection_key(&pk),
            sealed_new.clone(),
        )
        .await?;
        let c = ctx.conn.lock().await;
        q::upsert_peer_wrapped_collection_key(&c, collection_id, &pk, WRAPPING_PEER, &sealed_new)?;
    }

    // Re-wrap for the owner under master.
    let mk = MasterKey::from_bytes(ctx.master_key_bytes);
    let master_wrapped = wrap_collection_key(&new_ck, &mk)?;
    {
        let c = ctx.conn.lock().await;
        q::upsert_collection_key(
            &c,
            collection_id,
            ctx.owner_user_id,
            "master",
            &master_wrapped,
        )?;
    }

    // Swap the in-memory key so callers continue under the rotated key.
    ctx.collection_key_bytes = *new_ck.as_bytes();
    Ok(())
}

/// Generate a read-only `DocTicket` the recipient pastes into their
/// "Accept invite" textarea to join the namespace. Returns its base32
/// string form (the Display impl is the canonical serialisation).
pub async fn generate_share_ticket(ctx: &ShareContext, collection_id: i64) -> Result<String> {
    let ns = ctx
        .docs
        .open_shared(&ctx.conn, collection_id, "owner")
        .await?;
    let ticket = ns
        .doc
        .share(ShareMode::Read, AddrInfoOptions::RelayAndAddresses)
        .await
        .map_err(|e| {
            tracing::debug!(%e, "doc.share failed");
            Error::PeerUnreachable
        })?;
    Ok(ticket.to_string())
}

/// Thin adapter: write bytes to an iroh-docs key, mapping the library's
/// anyhow::Error into our `mv_core::Error`. We don't care which bucket
/// the error falls into — the whole share operation aborts.
async fn set_doc_bytes(
    doc: &Doc,
    author: iroh_docs::AuthorId,
    key: Vec<u8>,
    value: Vec<u8>,
) -> Result<()> {
    let _ = doc
        .set_bytes(author, bytes::Bytes::from(key), bytes::Bytes::from(value))
        .await
        .map_err(|e| {
            tracing::debug!(%e, "iroh-docs set_bytes failed");
            Error::PeerUnreachable
        })?;
    Ok(())
}
