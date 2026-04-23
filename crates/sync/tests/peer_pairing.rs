//! Tier-B: two real `Peer` instances bind UDP endpoints on loopback, exchange
//! a signed pairing ticket, and end up in each other's `peer_accept` tables.
//!
//! Ignored by default because binding real UDP sockets inside `cargo test`
//! surfaces flaky failures in sandboxed CI environments that don't allow
//! AF_INET. Run with:
//!
//! ```bash
//! MV_IROH_ENABLE=1 cargo test -p mv-sync --test peer_pairing -- --ignored
//! ```

use mv_core::crypto::keystore;
use mv_core::db::{self, schema};
use mv_sync::{accept_base32, list_for_user, Peer, PeerConfig};
use secrecy::SecretString;

fn enabled() -> bool {
    std::env::var("MV_IROH_ENABLE").is_ok_and(|v| v == "1")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Tier-B: requires MV_IROH_ENABLE=1 and a sandbox that allows UDP bind"]
async fn two_peers_exchange_tickets_end_to_end() {
    if !enabled() {
        eprintln!("skip: MV_IROH_ENABLE not set");
        return;
    }
    mv_core::crypto::init().unwrap();

    // Two independently-seeded users on the same on-disk vault.
    let tmp = tempfile::tempdir().unwrap();
    let conn = schema::open(&tmp.path().join("index.db")).unwrap();

    let (rec_a, unlocked_a) =
        keystore::create_user("alice", &SecretString::from("very-long-pw-alice")).unwrap();
    let uid_a = db::insert_user(&conn, &rec_a, 0).unwrap();

    let (rec_b, unlocked_b) =
        keystore::create_user("bob", &SecretString::from("very-long-pw-bob")).unwrap();
    let uid_b = db::insert_user(&conn, &rec_b, 0).unwrap();

    // Boot two real peers — each binds its own ephemeral UDP port on
    // 127.0.0.1 with RelayMode::Disabled.
    let peer_a = Peer::start(&unlocked_a, PeerConfig::default())
        .await
        .unwrap();
    let peer_b = Peer::start(&unlocked_b, PeerConfig::default())
        .await
        .unwrap();

    // A issues a ticket → B accepts.
    let seed_a = *unlocked_a.iroh_node.secret_bytes();
    let ticket_a = peer_a.ticket(&seed_a).unwrap();
    accept_base32(&conn, uid_b, &ticket_a.to_base32()).unwrap();

    // B issues a ticket → A accepts.
    let seed_b = *unlocked_b.iroh_node.secret_bytes();
    let ticket_b = peer_b.ticket(&seed_b).unwrap();
    accept_base32(&conn, uid_a, &ticket_b.to_base32()).unwrap();

    // Each side sees exactly one paired peer — the other one.
    let a_peers = list_for_user(&conn, uid_a).unwrap();
    let b_peers = list_for_user(&conn, uid_b).unwrap();
    assert_eq!(a_peers.len(), 1);
    assert_eq!(b_peers.len(), 1);
    assert_eq!(a_peers[0].peer_node_id, peer_b.node_id_bytes().to_vec());
    assert_eq!(b_peers[0].peer_node_id, peer_a.node_id_bytes().to_vec());
    assert_eq!(a_peers[0].peer_identity_pub, peer_b.identity_pub().to_vec());
    assert_eq!(b_peers[0].peer_identity_pub, peer_a.identity_pub().to_vec());
}
