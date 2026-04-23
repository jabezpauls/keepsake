//! Phase 3.2 — collection-key sharing envelope.
//!
//! A single collection is wrapped for multiple recipients via distinct
//! `collection_key` rows (§4.2). Local users use `master` or `album_password`
//! wrappings; peer recipients use `peer_x25519`, where the row stores a
//! `crypto_box_seal`-sealed `CollectionKey.as_bytes()`. This module is the
//! pure-crypto layer above the DB: callers handle the DB row writes via
//! `db::upsert_peer_wrapped_collection_key`.
//!
//! Reuses `envelope::seal_for_peer` / `open_from_peer` — no new crypto
//! primitives. Purpose here is type-safety: we want callers to work with
//! `CollectionKey` and a recipient's identity public key rather than with
//! raw byte slices, and to get distinct error variants for share-specific
//! failures so the Tauri boundary can surface them cleanly.

use serde::{Deserialize, Serialize};

use crate::crypto::envelope::{
    open_from_peer, open_row, seal_for_peer, seal_row, PeerKeypair, PeerPublicKey,
};
use crate::crypto::CollectionKey;
use crate::{Error, Result};

/// Wrapping kind stored in `collection_key.wrapping` for peer recipients.
pub const WRAPPING_PEER: &str = "peer_x25519";

/// Associated-data sentinel used when `seal_row`-wrapping the
/// collection-meta entry for iroh-docs. Real `asset_id`s are i64, so
/// `u64::MAX` can't collide. Member entries use their own asset_id as
/// the AD. Keeping AD distinct is a belt-and-braces hedge against
/// accidental key-reuse across payload kinds in the same namespace.
const META_ROW_ID: u64 = u64::MAX;

/// Plaintext shape of the `collection/meta/…` value carried over
/// iroh-docs. Serialized as JSON inside a `seal_row` envelope under the
/// collection key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionMeta {
    pub kind: String,
    pub name: String,
    pub cover_blake3_plaintext: Option<[u8; 32]>,
}

/// Plaintext shape of each `collection/members/…` value. Top-level
/// fields that aren't subject-matter metadata (mime, bytes, dims) stay
/// plaintext so that multiple recipients can dedupe identical content
/// without unsealing first. Subject-matter fields
/// (`filename_ct`/`gps_ct`/etc.) are pass-through ciphertext sealed
/// under the same collection key at ingest time; we don't re-encrypt
/// them at share time — the single outer `seal_row` envelope around the
/// whole JSON is the confidentiality boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberEntry {
    pub cas_ref: String,
    pub blake3_plaintext: [u8; 32],
    pub ciphertext_blake3: Option<[u8; 32]>,
    pub wrapped_file_key: Vec<u8>,
    pub mime: String,
    pub bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub taken_at_utc_day: Option<i64>,
    pub is_video: bool,
    pub is_raw: bool,
    pub is_screenshot: bool,
    pub is_live: bool,
    pub is_motion: bool,
    pub filename_ct: Vec<u8>,
    pub taken_at_utc_ct: Option<Vec<u8>>,
    pub gps_ct: Option<Vec<u8>>,
    pub device_ct: Option<Vec<u8>>,
    pub lens_ct: Option<Vec<u8>>,
    pub exif_all_ct: Option<Vec<u8>>,
}

/// Seal `meta` under the collection key for writing to
/// `collection/meta/<cid>` in iroh-docs.
pub fn seal_collection_meta(meta: &CollectionMeta, ck: &CollectionKey) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(meta).map_err(|_| Error::BlobFormat)?;
    seal_row(&json, META_ROW_ID, ck.as_bytes())
}

/// Open a `collection/meta/<cid>` entry that was sealed by
/// [`seal_collection_meta`]. Returns the parsed `CollectionMeta`.
pub fn open_collection_meta(value: &[u8], ck: &CollectionKey) -> Result<CollectionMeta> {
    let json = open_row(value, META_ROW_ID, ck.as_bytes())?;
    serde_json::from_slice(&json).map_err(|_| Error::BlobFormat)
}

/// Seal a per-asset `MemberEntry` under the collection key. AD is the
/// sender's `asset_id` cast to u64, keeping sealed entries distinct
/// within the namespace.
pub fn seal_member_entry(
    entry: &MemberEntry,
    asset_id: i64,
    ck: &CollectionKey,
) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(entry).map_err(|_| Error::BlobFormat)?;
    seal_row(&json, asset_id as u64, ck.as_bytes())
}

/// Open a `MemberEntry` sealed by [`seal_member_entry`]. `asset_id` must
/// match the AD used at seal time (the sender's id, which is also the
/// key suffix in `DocsKey::collection_member`).
pub fn open_member_entry(value: &[u8], asset_id: i64, ck: &CollectionKey) -> Result<MemberEntry> {
    let json = open_row(value, asset_id as u64, ck.as_bytes())?;
    serde_json::from_slice(&json).map_err(|_| Error::BlobFormat)
}

/// Seal the 32-byte `CollectionKey` for a peer's X25519 identity public key
/// using libsodium sealed-box. Output is a dense ciphertext (48-byte
/// overhead for the sealed-box header + MAC) ready to write to
/// `collection_key.wrapped_key`.
pub fn seal_collection_for_peer(
    ck: &CollectionKey,
    recipient_identity_pub: &[u8; 32],
) -> Result<Vec<u8>> {
    let pk = PeerPublicKey(*recipient_identity_pub);
    seal_for_peer(ck.as_bytes(), &pk)
}

/// Open a peer-sealed collection key using the recipient's own identity
/// keypair. Fails with `Error::KeyOrData` on any mismatch (wrong keypair,
/// tampered ciphertext, truncated input) — callers must not try to
/// distinguish the reasons (§1 opaque-crypto-failures rule).
pub fn open_shared_collection(
    wrapped: &[u8],
    recipient_keypair: &PeerKeypair,
) -> Result<CollectionKey> {
    let plain = open_from_peer(wrapped, recipient_keypair)?;
    if plain.len() != 32 {
        // Defensive: a sealed-box that decrypts to the wrong length is either
        // a corrupted share or a completely unrelated message. Treat as
        // key-or-data per the §1 rule.
        return Err(Error::KeyOrData);
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&plain);
    Ok(CollectionKey::from_bytes(bytes))
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
        crate::crypto::init().unwrap();
    }

    #[test]
    fn round_trip_through_two_identities() {
        init();
        let alice = PeerKeypair::generate().unwrap();
        let bob = PeerKeypair::generate().unwrap();
        let ck = CollectionKey::random().unwrap();

        // Alice seals for Bob; only Bob can open.
        let sealed = seal_collection_for_peer(&ck, &bob.public.0).unwrap();
        let opened = open_shared_collection(&sealed, &bob).unwrap();
        assert_eq!(opened.as_bytes(), ck.as_bytes());

        // Alice (the sender) cannot open her own seal with her own keypair —
        // sealed-box sender identity is ephemeral + discarded at seal time.
        let r = open_shared_collection(&sealed, &alice);
        assert!(matches!(r, Err(Error::KeyOrData)));
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        init();
        let bob = PeerKeypair::generate().unwrap();
        let ck = CollectionKey::random().unwrap();
        let mut sealed = seal_collection_for_peer(&ck, &bob.public.0).unwrap();
        // Flip a bit mid-ciphertext (well past the sealed-box header).
        let mid = sealed.len() / 2;
        sealed[mid] ^= 0xFF;
        let r = open_shared_collection(&sealed, &bob);
        assert!(matches!(r, Err(Error::KeyOrData)));
    }

    #[test]
    fn truncated_ciphertext_is_rejected() {
        init();
        let bob = PeerKeypair::generate().unwrap();
        let ck = CollectionKey::random().unwrap();
        let sealed = seal_collection_for_peer(&ck, &bob.public.0).unwrap();
        let r = open_shared_collection(&sealed[..sealed.len() - 2], &bob);
        assert!(matches!(r, Err(Error::KeyOrData)));
    }

    #[test]
    fn wrapping_constant_matches_schema() {
        // Sanity-pin: the schema DDL predicate in DDL_V4 + all DB helpers
        // hard-code the string 'peer_x25519'. Keep them in step.
        assert_eq!(WRAPPING_PEER, "peer_x25519");
    }

    #[test]
    fn collection_meta_round_trip() {
        init();
        let ck = CollectionKey::random().unwrap();
        let meta = CollectionMeta {
            kind: "album".into(),
            name: "Vacation 2025".into(),
            cover_blake3_plaintext: Some([0xAAu8; 32]),
        };
        let sealed = seal_collection_meta(&meta, &ck).unwrap();
        let back = open_collection_meta(&sealed, &ck).unwrap();
        assert_eq!(back.kind, meta.kind);
        assert_eq!(back.name, meta.name);
        assert_eq!(back.cover_blake3_plaintext, meta.cover_blake3_plaintext);

        // Wrong ck → KeyOrData.
        let other = CollectionKey::random().unwrap();
        let r = open_collection_meta(&sealed, &other);
        assert!(matches!(r, Err(Error::KeyOrData)));
    }

    #[test]
    fn member_entry_round_trip() {
        init();
        let ck = CollectionKey::random().unwrap();
        let entry = MemberEntry {
            cas_ref: "ab".to_string() + &"cd".repeat(31),
            blake3_plaintext: [0x11u8; 32],
            ciphertext_blake3: Some([0x22u8; 32]),
            wrapped_file_key: vec![0xDE; 64],
            mime: "image/jpeg".into(),
            bytes: 1_234_567,
            width: Some(4032),
            height: Some(3024),
            duration_ms: None,
            taken_at_utc_day: Some(20_250),
            is_video: false,
            is_raw: false,
            is_screenshot: false,
            is_live: true,
            is_motion: false,
            filename_ct: vec![0xAA; 32],
            taken_at_utc_ct: None,
            gps_ct: Some(vec![0xBB; 48]),
            device_ct: None,
            lens_ct: None,
            exif_all_ct: None,
        };
        let sealed = seal_member_entry(&entry, 17, &ck).unwrap();
        let back = open_member_entry(&sealed, 17, &ck).unwrap();
        assert_eq!(back.cas_ref, entry.cas_ref);
        assert_eq!(back.blake3_plaintext, entry.blake3_plaintext);
        assert_eq!(back.wrapped_file_key, entry.wrapped_file_key);
        assert_eq!(back.gps_ct, entry.gps_ct);

        // Wrong AD (asset_id mismatch) → KeyOrData.
        let r = open_member_entry(&sealed, 18, &ck);
        assert!(matches!(r, Err(Error::KeyOrData)));
    }
}
