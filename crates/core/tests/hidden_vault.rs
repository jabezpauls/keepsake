//! Hidden-vault acceptance test (phase-1-foundation.md §9 #8).
//!
//! Plausible-deniability: without the hidden password, the on-disk layout
//! and every API response must be byte-indistinguishable between a vault
//! that has a hidden sub-vault and one that does not.

mod support;

use mv_core::crypto::keystore;
use secrecy::SecretString;

#[test]
fn without_hidden_password_layout_matches_vault_without_one() {
    // Two vaults: A has a hidden password set, B does not.
    let (record_a, mut user_a) =
        keystore::create_user("alice", &SecretString::from("main-main-main-main")).unwrap();
    let (record_b, _user_b) =
        keystore::create_user("bob", &SecretString::from("main-main-main-main")).unwrap();
    let mut record_a = record_a;
    keystore::set_hidden_vault_password(
        &mut record_a,
        &mut user_a,
        &SecretString::from("hidden-hidden-hidden"),
    )
    .unwrap();

    // Structural indistinguishability: same-length, non-zero blobs.
    assert_eq!(
        record_a.hidden_wrapped_master_key.len(),
        record_b.hidden_wrapped_master_key.len()
    );
    assert_eq!(record_a.hidden_salt.len(), record_b.hidden_salt.len());
}

#[test]
fn wrong_hidden_password_is_opaque() {
    let (mut record, mut user) =
        keystore::create_user("alice", &SecretString::from("main-main-main-main")).unwrap();
    keystore::set_hidden_vault_password(
        &mut record,
        &mut user,
        &SecretString::from("hidden-hidden-hidden"),
    )
    .unwrap();

    // Simulate lock + re-unlock.
    let mut re = keystore::unlock(
        &record,
        &SecretString::from("main-main-main-main"),
        user.user_id,
    )
    .unwrap();
    assert!(re.hidden_master_key.is_none());

    // Wrong hidden password must fail with the same variant as "no hidden
    // vault set" to prevent a presence oracle.
    let err =
        keystore::unlock_hidden_vault(&record, &mut re, &SecretString::from("wrong-hidden-pw-xx"));
    assert!(matches!(err, Err(mv_core::Error::KeyOrData)));
    assert!(re.hidden_master_key.is_none());

    // Correct password populates the hidden key.
    keystore::unlock_hidden_vault(
        &record,
        &mut re,
        &SecretString::from("hidden-hidden-hidden"),
    )
    .unwrap();
    assert!(re.hidden_master_key.is_some());
}

#[test]
fn unlock_hidden_on_vault_without_one_fails_identically() {
    let (record, mut user) =
        keystore::create_user("alice", &SecretString::from("main-main-main-main")).unwrap();
    // We never call set_hidden_vault_password — the placeholder blob is
    // random bytes that will not authenticate under any password.
    let err = keystore::unlock_hidden_vault(
        &record,
        &mut user,
        &SecretString::from("try-this-password-x"),
    );
    assert!(matches!(err, Err(mv_core::Error::KeyOrData)));
}
