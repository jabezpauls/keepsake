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

use crate::crypto::envelope::{open_from_peer, seal_for_peer, PeerKeypair, PeerPublicKey};
use crate::crypto::CollectionKey;
use crate::{Error, Result};

/// Wrapping kind stored in `collection_key.wrapping` for peer recipients.
pub const WRAPPING_PEER: &str = "peer_x25519";

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
}
