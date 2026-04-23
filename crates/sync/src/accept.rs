//! Pairing-ticket acceptance — verifies, deduplicates, and persists.
//!
//! `accept_ticket` is the only entry point. Ticket rejection paths all raise
//! distinct `Error` variants:
//! * Malformed layout → `TicketFormat(&'static str)`.
//! * Signature mismatch → `TicketSignature`.
//! * SQL failure → `Db(...)`.
//!
//! No "accepted ourselves" check yet — two users pairing from different
//! vaults should be able to exchange tickets bidirectionally. A future
//! commit can add the `iroh_node_pub == local_node_id` guard once multi-user
//! context is threaded in.

use mv_core::db::{self, PeerAcceptRow};
use mv_core::Result;
use rusqlite::Connection;

use crate::ticket::PairingTicket;

/// Accept a base32-encoded pairing ticket on behalf of `owner_user_id`.
/// Verifies the Ed25519 signature, then upserts into `peer_accept`.
/// Idempotent: re-accepting the same ticket refreshes `added_at`.
pub fn accept_base32(
    conn: &Connection,
    owner_user_id: i64,
    base32_str: &str,
) -> Result<PeerAcceptRow> {
    let ticket = PairingTicket::from_base32(base32_str)?;
    accept_ticket(conn, owner_user_id, &ticket)
}

/// Accept a pre-parsed ticket. Separate so tests can craft tickets
/// directly.
pub fn accept_ticket(
    conn: &Connection,
    owner_user_id: i64,
    ticket: &PairingTicket,
) -> Result<PeerAcceptRow> {
    ticket.verify()?;
    let row = PeerAcceptRow {
        peer_node_id: ticket.iroh_node_pub.to_vec(),
        peer_identity_pub: ticket.identity_pub.to_vec(),
        owner_user_id,
        relay_url: if ticket.relay_url.is_empty() {
            None
        } else {
            Some(ticket.relay_url.clone())
        },
        added_at: chrono::Utc::now().timestamp(),
        note_ct: None,
    };
    db::upsert_peer_accept(conn, &row)?;
    Ok(row)
}

/// List peers this user has accepted, newest-first.
pub fn list_for_user(conn: &Connection, owner_user_id: i64) -> Result<Vec<PeerAcceptRow>> {
    db::list_peer_accepts(conn, owner_user_id)
}

/// Forget a peer by its 32-byte node id.
pub fn forget(conn: &Connection, owner_user_id: i64, peer_node_id: &[u8]) -> Result<bool> {
    db::delete_peer_accept(conn, owner_user_id, peer_node_id)
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SECRET_KEY_LENGTH;
    use mv_core::db::schema;
    use mv_core::Error;
    use tempfile::TempDir;

    fn open_vault() -> (TempDir, Connection, i64) {
        use mv_core::crypto::keystore;
        use secrecy::SecretString;
        let tmp = TempDir::new().unwrap();
        let conn = schema::open(&tmp.path().join("index.db")).unwrap();
        let (record, _unlocked) =
            keystore::create_user("owner", &SecretString::from("very-long-pw-xyz")).unwrap();
        let uid = mv_core::db::insert_user(&conn, &record, 0).unwrap();
        (tmp, conn, uid)
    }

    fn canned_ticket(seed: [u8; SECRET_KEY_LENGTH]) -> PairingTicket {
        let identity_pub = [0x33; 32];
        PairingTicket::sign(&seed, &identity_pub, "lan".into(), 42).unwrap()
    }

    #[test]
    fn accept_ticket_writes_peer_accept_row() {
        let (_tmp, conn, uid) = open_vault();
        let t = canned_ticket([0x11; SECRET_KEY_LENGTH]);
        let row = accept_ticket(&conn, uid, &t).unwrap();
        assert_eq!(row.peer_node_id, t.iroh_node_pub.to_vec());
        assert_eq!(row.peer_identity_pub, t.identity_pub.to_vec());
        assert_eq!(row.relay_url.as_deref(), Some("lan"));

        let listed = list_for_user(&conn, uid).unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[test]
    fn accept_is_idempotent_across_re_imports() {
        let (_tmp, conn, uid) = open_vault();
        let t = canned_ticket([0x22; SECRET_KEY_LENGTH]);
        accept_ticket(&conn, uid, &t).unwrap();
        accept_ticket(&conn, uid, &t).unwrap();
        let listed = list_for_user(&conn, uid).unwrap();
        assert_eq!(listed.len(), 1, "duplicate import must not double-row");
    }

    #[test]
    fn base32_entry_point_round_trips() {
        let (_tmp, conn, uid) = open_vault();
        let t = canned_ticket([0x33; SECRET_KEY_LENGTH]);
        let s = t.to_base32();
        let row = accept_base32(&conn, uid, &s).unwrap();
        assert_eq!(row.peer_node_id, t.iroh_node_pub.to_vec());
    }

    #[test]
    fn tampered_ticket_is_rejected_before_db_write() {
        let (_tmp, conn, uid) = open_vault();
        let mut t = canned_ticket([0x44; SECRET_KEY_LENGTH]);
        t.timestamp ^= 0xFFFF;
        let r = accept_ticket(&conn, uid, &t);
        assert!(matches!(r, Err(Error::TicketSignature)));
        let listed = list_for_user(&conn, uid).unwrap();
        assert!(listed.is_empty(), "reject must not persist partial row");
    }

    #[test]
    fn forget_round_trip() {
        let (_tmp, conn, uid) = open_vault();
        let t = canned_ticket([0x55; SECRET_KEY_LENGTH]);
        accept_ticket(&conn, uid, &t).unwrap();
        let removed = forget(&conn, uid, &t.iroh_node_pub).unwrap();
        assert!(removed);
        let listed = list_for_user(&conn, uid).unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn cross_user_isolation() {
        use mv_core::crypto::keystore;
        use secrecy::SecretString;
        let (_tmp, conn, uid_a) = open_vault();
        let (rec_b, _) =
            keystore::create_user("bob", &SecretString::from("very-long-pw-bob")).unwrap();
        let uid_b = mv_core::db::insert_user(&conn, &rec_b, 0).unwrap();

        let t = canned_ticket([0x66; SECRET_KEY_LENGTH]);
        accept_ticket(&conn, uid_a, &t).unwrap();
        let a = list_for_user(&conn, uid_a).unwrap();
        let b = list_for_user(&conn, uid_b).unwrap();
        assert_eq!(a.len(), 1);
        assert!(b.is_empty(), "different user must not see another's peers");
    }
}
