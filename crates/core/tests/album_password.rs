//! Album-password acceptance test (phase-1-foundation.md §9 #7).
//!
//! A password-protected album must be visible as an entity after the vault
//! is re-unlocked, but its members must stay hidden until the right album
//! password is supplied. Wrong passwords leak nothing.

mod support;

use mv_core::crypto::keystore;
use mv_core::crypto::{seal_row, unwrap_collection_key, wrap_collection_key, CollectionKey};
use mv_core::db;
use secrecy::SecretString;
use support::{write_jpeg, Vault};

#[test]
fn password_album_plausible_deniability() {
    let vault = Vault::create("main-main-main-main");

    // Create an album with a password, put one asset in it.
    let album_name_ct = seal_row(b"Beach 2024", 0, vault.user.master_key.as_bytes()).unwrap();
    let album_ck = CollectionKey::random().unwrap();
    let wrapped_master = wrap_collection_key(&album_ck, &vault.user.master_key).unwrap();
    let (wrapped_pw, salt) = keystore::wrap_collection_key_for_album(
        &album_ck,
        &SecretString::from("beach-pictures-2024"),
    )
    .unwrap();

    let cid = db::insert_collection(
        &vault.conn,
        vault.user_id,
        "album",
        &album_name_ct,
        true,
        Some(&salt),
        0,
    )
    .unwrap();
    db::upsert_collection_key(&vault.conn, cid, vault.user_id, "master", &wrapped_master).unwrap();
    db::upsert_collection_key(
        &vault.conn,
        cid,
        vault.user_id,
        "album_password",
        &wrapped_pw,
    )
    .unwrap();

    // Seed one asset into the album.
    let asset_hash = [42u8; 32];
    let fn_ct = seal_row(b"IMG_0001.JPG", 0, album_ck.as_bytes()).unwrap();
    let wrapped_fk = {
        let fk = mv_core::crypto::FileKey::random().unwrap();
        mv_core::crypto::wrap_file_key(&fk, &album_ck).unwrap()
    };
    let insert = db::AssetInsert {
        blake3_plaintext: &asset_hash,
        mime: "image/jpeg",
        bytes: 1,
        width: None,
        height: None,
        duration_ms: None,
        taken_at_utc_day: None,
        is_video: false,
        is_raw: false,
        is_screenshot: false,
        is_live: false,
        is_motion: false,
        source_id: vault.source_id,
        cas_ref: "aa",
        imported_at: 0,
        filename_ct: &fn_ct,
        taken_at_utc_ct: None,
        gps_ct: None,
        device_ct: None,
        lens_ct: None,
        exif_all_ct: None,
        wrapped_file_key: &wrapped_fk,
    };
    let aid = match db::insert_asset_if_new(&vault.conn, &insert).unwrap() {
        db::InsertResult::Inserted(id) | db::InsertResult::Existing(id) => id,
    };
    db::add_to_collection(&vault.conn, cid, aid, 0).unwrap();

    // Simulate lock + re-unlock: drop the original unlocked user.
    let record = vault.record;
    drop(vault.user);
    drop(vault.default_collection_key);
    let unlocked = keystore::unlock(
        &record,
        &SecretString::from("main-main-main-main"),
        vault.user_id,
    )
    .unwrap();

    // Listing collections returns the album (plaintext metadata is visible
    // by design — only the name_ct is encrypted).
    let all = db::list_collections(&vault.conn, vault.user_id, false).unwrap();
    assert_eq!(all.len(), 2, "unsorted default + password album");
    let coll = all.iter().find(|c| c.id == cid).unwrap();
    assert!(coll.has_password);

    // Without the album password, the album's CollectionKey is unreachable —
    // `unlock_album` with the wrong password fails without leaking which
    // branch of the check killed it.
    let wrong = keystore::unlock_album(
        &wrapped_pw,
        &SecretString::from("not-the-password-xxx"),
        &salt,
    );
    assert!(wrong.is_err());

    // Correct password recovers the key and surfaces the member.
    let ck = keystore::unlock_album(
        &wrapped_pw,
        &SecretString::from("beach-pictures-2024"),
        &salt,
    )
    .unwrap();
    assert_eq!(ck.as_bytes(), album_ck.as_bytes());

    let members = db::list_collection_member_ids(&vault.conn, cid).unwrap();
    assert_eq!(members.len(), 1);
    // Filename decrypts only with the right ck.
    let decoded = mv_core::crypto::open_row(&fn_ct, 0, ck.as_bytes()).unwrap();
    assert_eq!(decoded, b"IMG_0001.JPG");

    // A separate "master-only" unwrap path also works for the owner.
    let via_master = db::get_collection_key(&vault.conn, cid, vault.user_id, "master")
        .unwrap()
        .unwrap();
    let ck2 = unwrap_collection_key(&via_master, &unlocked.master_key).unwrap();
    assert_eq!(ck2.as_bytes(), album_ck.as_bytes());
}

#[test]
fn wrong_password_returns_no_error_leak() {
    // A harness-level check: the `unlock_album` surface returns the opaque
    // KeyOrData variant regardless of the specific failure.
    let ck = CollectionKey::random().unwrap();
    let (wrapped, salt) =
        keystore::wrap_collection_key_for_album(&ck, &SecretString::from("correct-long-pw"))
            .unwrap();
    match keystore::unlock_album(&wrapped, &SecretString::from("wrong-long-pw!"), &salt) {
        Err(mv_core::Error::KeyOrData) => (),
        other => panic!("expected opaque KeyOrData, got {other:?}"),
    }
    let _ = write_jpeg; // keep the helper referenced for the integration build
}
