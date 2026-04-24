//! D6: two users coexisting on one vault. Pins the "iterate users, try
//! password against each, first success wins" algorithm that the auth
//! command uses. We can't hit the Tauri command directly, but we can
//! exercise the same primitives (list_user_ids + keystore::unlock +
//! username_ct decryption) so regressions on the db/crypto side get
//! caught here.

use mv_core::crypto::keystore;
use mv_core::db;
use secrecy::SecretString;

fn try_unlock_as(
    conn: &rusqlite::Connection,
    typed_username: &str,
    typed_password: &str,
) -> Option<i64> {
    let ids = db::list_user_ids(conn).unwrap();
    let pw = SecretString::from(typed_password.to_string());
    for (user_id, _ipk, _ca) in ids {
        let record = db::get_user_record(conn, user_id).unwrap();
        let Ok(unlocked) = keystore::unlock(&record, &pw, user_id) else {
            continue;
        };
        let Ok(uname_bytes) =
            mv_core::crypto::open_row(&record.username_ct, 0, unlocked.master_key.as_bytes())
        else {
            continue;
        };
        let Ok(actual) = String::from_utf8(uname_bytes) else {
            continue;
        };
        if actual == typed_username {
            return Some(user_id);
        }
    }
    None
}

fn setup_two_users() -> (rusqlite::Connection, i64, i64) {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    db::migrate::apply(&conn).unwrap();

    let (rec_a, _) =
        keystore::create_user("alice", &SecretString::from("alice-main-key-xxx")).unwrap();
    let (rec_b, _) =
        keystore::create_user("bob", &SecretString::from("bob-main-key-yyyyy")).unwrap();
    let a = db::insert_user(&conn, &rec_a, 100).unwrap();
    let b = db::insert_user(&conn, &rec_b, 200).unwrap();
    (conn, a, b)
}

#[test]
fn each_user_unlocks_with_own_password() {
    let (conn, a, b) = setup_two_users();
    assert_eq!(try_unlock_as(&conn, "alice", "alice-main-key-xxx"), Some(a));
    assert_eq!(try_unlock_as(&conn, "bob", "bob-main-key-yyyyy"), Some(b));
}

#[test]
fn cross_user_password_fails() {
    let (conn, _a, _b) = setup_two_users();
    // Alice's password on Bob's username: no user matches both.
    assert_eq!(try_unlock_as(&conn, "bob", "alice-main-key-xxx"), None);
    assert_eq!(try_unlock_as(&conn, "alice", "bob-main-key-yyyyy"), None);
}

#[test]
fn wrong_password_fails_entirely() {
    let (conn, _a, _b) = setup_two_users();
    assert_eq!(try_unlock_as(&conn, "alice", "not-the-password-aa"), None);
}

#[test]
fn unknown_username_fails() {
    let (conn, _a, _b) = setup_two_users();
    assert_eq!(try_unlock_as(&conn, "carol", "alice-main-key-xxx"), None);
}

#[test]
fn users_get_distinct_identity_pubs() {
    let (conn, _a, _b) = setup_two_users();
    let rows = db::list_user_ids(&conn).unwrap();
    assert_eq!(rows.len(), 2);
    assert_ne!(
        rows[0].1, rows[1].1,
        "each user gets its own X25519 identity keypair"
    );
}
