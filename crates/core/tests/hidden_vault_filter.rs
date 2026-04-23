//! Acceptance criterion: hidden-vault assets never surface in timeline, map,
//! or search when the hidden vault is locked. Architecture.md §9 (plausible
//! deniability) requires a default-hide posture.

mod support;

use mv_core::db;
use support::Vault;

#[test]
fn hidden_vault_assets_are_excluded_by_default() {
    let vault = Vault::create("very-long-password-xyz");
    let conn = &vault.conn;

    // Two assets: one in the default album, one in a hidden-vault collection.
    let normal = seed_asset(conn, vault.source_id, 1, Some(100));
    let hidden = seed_asset(conn, vault.source_id, 2, Some(200));

    // Default album already exists at `vault.default_collection_id`; mark
    // `normal` as a member so it has a non-hidden route.
    db::add_to_collection(conn, vault.default_collection_id, normal, 0).unwrap();

    // Create a hidden-vault collection and add `hidden` to it.
    let hv_name = mv_core::crypto::seal_row(b"Vault", 0, vault.user.master_key.as_bytes()).unwrap();
    let hv_id = db::insert_collection(conn, vault.user_id, "hidden_vault", &hv_name, false, None, 0)
        .unwrap();
    db::add_to_collection(conn, hv_id, hidden, 0).unwrap();

    // Locked (default): hidden is excluded everywhere.
    let default_f = db::AssetFilter {
        limit: Some(10),
        ..Default::default()
    };
    let ids: Vec<i64> = db::filter_assets(conn, &default_f)
        .unwrap()
        .into_iter()
        .map(|a| a.id)
        .collect();
    assert!(ids.contains(&normal));
    assert!(!ids.contains(&hidden), "hidden asset must not leak locked");

    // Unlock → hidden shows up.
    let unlocked_f = db::AssetFilter {
        limit: Some(10),
        hidden_vault_unlocked: true,
        ..Default::default()
    };
    let ids: Vec<i64> = db::filter_assets(conn, &unlocked_f)
        .unwrap()
        .into_iter()
        .map(|a| a.id)
        .collect();
    assert!(ids.contains(&normal));
    assert!(ids.contains(&hidden), "unlock surface hidden");

    // Map path: list_assets_with_gps uses the same filter shape.
    let locked_geo = db::list_assets_with_gps(conn, &default_f).unwrap();
    assert!(!locked_geo.iter().any(|(id, _, _)| *id == hidden));
    let unlocked_geo = db::list_assets_with_gps(conn, &unlocked_f).unwrap();
    assert!(unlocked_geo.iter().any(|(id, _, _)| *id == hidden));
}

fn seed_asset(conn: &rusqlite::Connection, source_id: i64, seed: u8, day: Option<i64>) -> i64 {
    let mut hash = [0u8; 32];
    hash[0] = seed;
    let a = db::AssetInsert {
        blake3_plaintext: &hash,
        mime: "image/jpeg",
        bytes: 0,
        width: None,
        height: None,
        duration_ms: None,
        taken_at_utc_day: day,
        is_video: false,
        is_raw: false,
        is_screenshot: false,
        is_live: false,
        is_motion: false,
        source_id,
        cas_ref: "x",
        imported_at: 0,
        filename_ct: b"f",
        taken_at_utc_ct: None,
        // `list_assets_with_gps` filters on `gps_ct IS NOT NULL`, so seed a
        // non-null placeholder. The value itself doesn't need to be a valid
        // AEAD payload for this test — `list_assets_with_gps` just emits the
        // bytes back and the decrypt step lives in the caller.
        gps_ct: Some(b"placeholder"),
        device_ct: None,
        lens_ct: None,
        exif_all_ct: None,
        wrapped_file_key: b"w",
    };
    match db::insert_asset_if_new(conn, &a).unwrap() {
        db::InsertResult::Inserted(id) | db::InsertResult::Existing(id) => id,
    }
}
