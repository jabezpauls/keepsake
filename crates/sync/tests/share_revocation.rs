//! Tier-B: Alice shares an album with Bob + Carol, revokes Bob,
//! rotates the key, and verifies:
//!
//! 1. Bob's original wrapping is gone; new key envelope absent.
//! 2. Carol keeps access after rotation (new wrapping present,
//!    `open_shared_collection` succeeds with the rotated bytes).
//! 3. Audit ledger accumulates `active` × 2 + `revoked` × 1 +
//!    `rotated` × 1 rows (Carol-only rotation, since Bob was removed
//!    before rotate enumerates remaining recipients).
//!
//! Does not exercise iroh networking — uses the same crypto + DB
//! layer as `two_peer_share.rs`.
//!
//! Run with:
//!
//! ```bash
//! MV_IROH_ENABLE=1 cargo test -p mv-sync --test share_revocation -- --ignored
//! ```

use mv_core::crypto::keystore;
use mv_core::db::{self, queries as q, schema};
use mv_core::share::{open_shared_collection, seal_collection_for_peer, WRAPPING_PEER};
use mv_core::{crypto::CollectionKey, Error};
use secrecy::SecretString;

fn enabled() -> bool {
    std::env::var("MV_IROH_ENABLE").is_ok_and(|v| v == "1")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Tier-B: requires MV_IROH_ENABLE=1"]
async fn revoke_bob_keeps_carol_working() {
    if !enabled() {
        eprintln!("skip: MV_IROH_ENABLE not set");
        return;
    }
    mv_core::crypto::init().unwrap();

    // Three users, one vault (Alice owns).
    let tmp = tempfile::tempdir().unwrap();
    let conn = schema::open(&tmp.path().join("index.db")).unwrap();
    let (rec_a, unlocked_a) =
        keystore::create_user("alice", &SecretString::from("very-long-pw-alice-rev")).unwrap();
    let uid_a = db::insert_user(&conn, &rec_a, 0).unwrap();
    let (_rec_b, unlocked_b) =
        keystore::create_user("bob", &SecretString::from("very-long-pw-bob-rev")).unwrap();
    let (_rec_c, unlocked_c) =
        keystore::create_user("carol", &SecretString::from("very-long-pw-carol-rev")).unwrap();

    // Alice creates an album keyed by a random ck1.
    let ck1 = CollectionKey::random().unwrap();
    let name_ct = mv_core::crypto::seal_row(b"Trip", 0, unlocked_a.master_key.as_bytes()).unwrap();
    let cid = db::insert_collection(&conn, uid_a, "album", &name_ct, false, None, 0).unwrap();
    let master_wrap = mv_core::crypto::wrap_collection_key(&ck1, &unlocked_a.master_key).unwrap();
    db::upsert_collection_key(&conn, cid, uid_a, "master", &master_wrap).unwrap();

    // Share ck1 with Bob + Carol.
    let sealed_b = seal_collection_for_peer(&ck1, &unlocked_b.identity.public.0).unwrap();
    let sealed_c = seal_collection_for_peer(&ck1, &unlocked_c.identity.public.0).unwrap();
    q::upsert_peer_wrapped_collection_key(
        &conn,
        cid,
        &unlocked_b.identity.public.0,
        WRAPPING_PEER,
        &sealed_b,
    )
    .unwrap();
    q::record_share_status(
        &conn,
        cid,
        &unlocked_b.identity.public.0,
        q::SHARE_STATUS_ACTIVE,
        100,
    )
    .unwrap();
    q::upsert_peer_wrapped_collection_key(
        &conn,
        cid,
        &unlocked_c.identity.public.0,
        WRAPPING_PEER,
        &sealed_c,
    )
    .unwrap();
    q::record_share_status(
        &conn,
        cid,
        &unlocked_c.identity.public.0,
        q::SHARE_STATUS_ACTIVE,
        101,
    )
    .unwrap();

    // --- Revoke Bob (the share.rs flow without iroh-docs) --------------
    q::delete_peer_wrapped_collection_key(&conn, cid, &unlocked_b.identity.public.0, WRAPPING_PEER)
        .unwrap();
    q::record_share_status(
        &conn,
        cid,
        &unlocked_b.identity.public.0,
        q::SHARE_STATUS_REVOKED,
        200,
    )
    .unwrap();

    // Rotate: new ck2, re-seal for Carol only.
    let ck2 = CollectionKey::random().unwrap();
    let remaining = q::list_peer_wrappings_for_collection(&conn, cid).unwrap();
    assert_eq!(remaining.len(), 1, "only Carol should remain post-revoke");
    for (pub_bytes, _) in remaining {
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&pub_bytes);
        let sealed_new = seal_collection_for_peer(&ck2, &pk).unwrap();
        q::upsert_peer_wrapped_collection_key(&conn, cid, &pk, WRAPPING_PEER, &sealed_new).unwrap();
        q::record_share_status(&conn, cid, &pk, q::SHARE_STATUS_ROTATED, 201).unwrap();
    }

    // Re-wrap for owner under master.
    let master_wrap2 = mv_core::crypto::wrap_collection_key(&ck2, &unlocked_a.master_key).unwrap();
    db::upsert_collection_key(&conn, cid, uid_a, "master", &master_wrap2).unwrap();

    // --- Assertions -----------------------------------------------------

    // 1. Bob's wrapping is gone.
    let bob_entry = q::get_peer_wrapped_collection_key(
        &conn,
        cid,
        &unlocked_b.identity.public.0,
        WRAPPING_PEER,
    )
    .unwrap();
    assert!(bob_entry.is_none(), "Bob's wrapping must be deleted");

    // Bob's best remaining handle — the ORIGINAL ck1 sealed envelope
    // — still decrypts to ck1, but that key no longer decrypts new
    // metadata (simulated: new cover meta sealed under ck2 cannot be
    // opened with ck1 via seal_row/open_row).
    let ck_bob_old = open_shared_collection(&sealed_b, &unlocked_b.identity).unwrap();
    assert_eq!(ck_bob_old.as_bytes(), ck1.as_bytes());
    let new_meta = mv_core::crypto::seal_row(b"Trip (renamed)", 0, ck2.as_bytes()).unwrap();
    let bob_sees_new_meta =
        mv_core::crypto::envelope::open_row(&new_meta, 0, ck_bob_old.as_bytes());
    assert!(
        matches!(bob_sees_new_meta, Err(Error::KeyOrData)),
        "Bob's old key cannot open post-rotation meta; got {bob_sees_new_meta:?}"
    );

    // 2. Carol's wrapping updated.
    let carol_wrap = q::get_peer_wrapped_collection_key(
        &conn,
        cid,
        &unlocked_c.identity.public.0,
        WRAPPING_PEER,
    )
    .unwrap()
    .unwrap();
    let ck_carol = open_shared_collection(&carol_wrap, &unlocked_c.identity).unwrap();
    assert_eq!(ck_carol.as_bytes(), ck2.as_bytes(), "Carol has ck2");
    let carol_sees_new_meta =
        mv_core::crypto::envelope::open_row(&new_meta, 0, ck_carol.as_bytes()).unwrap();
    assert_eq!(carol_sees_new_meta, b"Trip (renamed)");

    // 3. Ledger accumulated the expected transitions.
    let ledger = q::list_share_status_for_collection(&conn, cid).unwrap();
    let rows: Vec<&str> = ledger.iter().map(|(_, s, _)| s.as_str()).collect();
    assert!(rows.contains(&q::SHARE_STATUS_ACTIVE));
    assert!(rows.contains(&q::SHARE_STATUS_REVOKED));
    assert!(rows.contains(&q::SHARE_STATUS_ROTATED));
    assert_eq!(ledger.len(), 4, "active×2 + revoked×1 + rotated×1 = 4");
}
