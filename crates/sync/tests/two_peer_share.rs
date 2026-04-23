//! Tier-B round-trip: Alice seals a collection key for Bob's X25519
//! identity, writes it into a sealed envelope, Bob unwraps with his
//! own identity keypair, assert byte-for-byte equality. A third peer
//! (Eve) with an unrelated keypair must fail with `Error::KeyOrData`.
//!
//! We deliberately do NOT exercise full iroh-docs UDP sync in this
//! test — that's a larger integration harness. The core acceptance
//! invariants in `phase-3-peers-smart.md §10 #2` (sealed key
//! confidentiality + third-peer rejection) live entirely in the
//! crypto + DB layer, which this test pins.
//!
//! Run with:
//!
//! ```bash
//! MV_IROH_ENABLE=1 cargo test -p mv-sync --test two_peer_share -- --ignored
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
async fn alice_shares_album_with_bob_only() {
    if !enabled() {
        eprintln!("skip: MV_IROH_ENABLE not set");
        return;
    }
    mv_core::crypto::init().unwrap();

    // Two separate vaults.
    let tmp_a = tempfile::tempdir().unwrap();
    let tmp_b = tempfile::tempdir().unwrap();
    let tmp_e = tempfile::tempdir().unwrap();
    let conn_a = schema::open(&tmp_a.path().join("index.db")).unwrap();
    let conn_b = schema::open(&tmp_b.path().join("index.db")).unwrap();
    let conn_e = schema::open(&tmp_e.path().join("index.db")).unwrap();

    let (rec_a, unlocked_a) =
        keystore::create_user("alice", &SecretString::from("very-long-pw-alice-000")).unwrap();
    let uid_a = db::insert_user(&conn_a, &rec_a, 0).unwrap();
    let (rec_b, unlocked_b) =
        keystore::create_user("bob", &SecretString::from("very-long-pw-bob-000")).unwrap();
    let uid_b = db::insert_user(&conn_b, &rec_b, 0).unwrap();
    let (rec_e, unlocked_e) =
        keystore::create_user("eve", &SecretString::from("very-long-pw-eve-000")).unwrap();
    let _uid_e = db::insert_user(&conn_e, &rec_e, 0).unwrap();

    // Alice creates an album.
    let ck = CollectionKey::random().unwrap();
    let name_ct =
        mv_core::crypto::seal_row(b"Vacation 2025", 0, unlocked_a.master_key.as_bytes()).unwrap();
    let cid_a = db::insert_collection(&conn_a, uid_a, "album", &name_ct, false, None, 0).unwrap();
    let master_wrap = mv_core::crypto::wrap_collection_key(&ck, &unlocked_a.master_key).unwrap();
    db::upsert_collection_key(&conn_a, cid_a, uid_a, "master", &master_wrap).unwrap();

    // Alice records Bob as an accepted peer (the receive loop's trust gate).
    db::upsert_peer_accept(
        &conn_a,
        &db::queries::PeerAcceptRow {
            peer_node_id: unlocked_b.iroh_node.public.0.to_vec(),
            peer_identity_pub: unlocked_b.identity.public.0.to_vec(),
            owner_user_id: uid_a,
            relay_url: None,
            added_at: 0,
            note_ct: None,
        },
    )
    .unwrap();

    // Alice seals the collection key for Bob's X25519.
    let sealed = seal_collection_for_peer(&ck, &unlocked_b.identity.public.0).unwrap();

    // Alice stores the peer wrapping + audit ledger row (as the
    // publish flow does).
    q::upsert_peer_wrapped_collection_key(
        &conn_a,
        cid_a,
        &unlocked_b.identity.public.0,
        WRAPPING_PEER,
        &sealed,
    )
    .unwrap();
    q::record_share_status(
        &conn_a,
        cid_a,
        &unlocked_b.identity.public.0,
        q::SHARE_STATUS_ACTIVE,
        chrono::Utc::now().timestamp(),
    )
    .unwrap();

    // Bob: ship the sealed bytes onto his DB (as if received via
    // iroh-docs) + unwrap with his own keypair.
    let cid_b = db::insert_collection(&conn_b, uid_b, "album", b"(incoming share)", false, None, 0)
        .unwrap();
    q::upsert_peer_wrapped_collection_key(
        &conn_b,
        cid_b,
        &unlocked_b.identity.public.0,
        WRAPPING_PEER,
        &sealed,
    )
    .unwrap();
    let bob_stored = q::get_peer_wrapped_collection_key(
        &conn_b,
        cid_b,
        &unlocked_b.identity.public.0,
        WRAPPING_PEER,
    )
    .unwrap()
    .unwrap();
    let ck_bob = open_shared_collection(&bob_stored, &unlocked_b.identity).unwrap();
    assert_eq!(
        ck_bob.as_bytes(),
        ck.as_bytes(),
        "Bob must recover the exact collection key"
    );

    // Eve (not a recipient): unwrap fails with KeyOrData.
    let eve_result = open_shared_collection(&sealed, &unlocked_e.identity);
    assert!(
        matches!(eve_result, Err(Error::KeyOrData)),
        "Eve must get KeyOrData on Alice's sealed envelope; got {eve_result:?}"
    );

    // Audit ledger: Alice has one 'active' row for Bob.
    let ledger = q::list_share_status_for_collection(&conn_a, cid_a).unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].1, q::SHARE_STATUS_ACTIVE);
    assert_eq!(ledger[0].0, unlocked_b.identity.public.0.to_vec());
}
