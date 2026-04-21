//! Provenance — register a source, record where each asset was found.
//!
//! All writes are performed inside a caller-provided transaction so ingest
//! stays atomic per asset.

use rusqlite::Connection;

use crate::db;
use crate::Result;

/// Register a new source (backup dump). Returns the new source id.
pub fn register_source(
    conn: &Connection,
    owner_id: i64,
    name_ct: &[u8],
    root_path_ct: &[u8],
    device_hint_ct: Option<&[u8]>,
    linked_only: bool,
    imported_at: i64,
) -> Result<i64> {
    db::insert_source(
        conn,
        owner_id,
        name_ct,
        root_path_ct,
        device_hint_ct,
        linked_only,
        imported_at,
    )
}

/// Record a physical location for an asset. Safe to call multiple times per
/// `(asset, source, path)` — duplicates are idempotent.
pub fn record_location(
    conn: &Connection,
    asset_id: i64,
    source_id: i64,
    original_path_ct: &[u8],
    mtime: i64,
) -> Result<()> {
    db::insert_asset_location(conn, asset_id, source_id, original_path_ct, mtime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::keystore;
    use crate::db;
    use secrecy::SecretString;

    fn open_mem_with_user() -> (Connection, i64) {
        let conn = Connection::open_in_memory().unwrap();
        db::migrate::apply(&conn).unwrap();
        let (record, _) =
            keystore::create_user("u", &SecretString::from("a-long-password-xx")).unwrap();
        let uid = db::insert_user(&conn, &record, 0).unwrap();
        (conn, uid)
    }

    #[test]
    fn register_source_and_record_location() {
        let (conn, uid) = open_mem_with_user();
        let sid = register_source(&conn, uid, b"my-dump", b"/tmp/src", None, false, 100).unwrap();
        let sources = db::list_sources(&conn, uid).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, sid);
        assert_eq!(sources[0].imported_at, 100);
    }

    #[test]
    fn record_location_is_idempotent() {
        let (conn, uid) = open_mem_with_user();
        let sid = register_source(&conn, uid, b"s", b"p", None, false, 0).unwrap();
        let hash = [3u8; 32];
        let a = db::AssetInsert {
            blake3_plaintext: &hash,
            mime: "image/jpeg",
            bytes: 0,
            width: None,
            height: None,
            duration_ms: None,
            taken_at_utc_day: None,
            is_video: false,
            is_raw: false,
            is_screenshot: false,
            is_live: false,
            is_motion: false,
            source_id: sid,
            cas_ref: "x",
            imported_at: 0,
            filename_ct: b"f",
            taken_at_utc_ct: None,
            gps_ct: None,
            device_ct: None,
            lens_ct: None,
            exif_all_ct: None,
            wrapped_file_key: b"w",
        };
        let aid = match db::insert_asset_if_new(&conn, &a).unwrap() {
            db::InsertResult::Inserted(x) | db::InsertResult::Existing(x) => x,
        };
        record_location(&conn, aid, sid, b"path-1", 1).unwrap();
        record_location(&conn, aid, sid, b"path-1", 1).unwrap();
        record_location(&conn, aid, sid, b"path-2", 2).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM asset_location WHERE asset_id = ?1",
                [aid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
    }
}
