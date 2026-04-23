//! Post-ingest ML-job enqueue helper.
//!
//! Every new asset that lands via an ingest adapter gets two jobs added to the
//! plaintext `ml_job` queue: one `embed_asset` (CLIP 768-d vector) and one
//! `detect_faces` (SCRFD + ArcFace face rows). The `MlWorker` drains these
//! once the runtime is loaded; off-flag or missing-weights builds simply let
//! the queue grow until it can be processed.
//!
//! Idempotent: [`crate::db::enqueue_ml_job`] dedupes on `(kind, asset_id)`,
//! so calling this twice for the same asset is a no-op.

use crate::db;
use crate::Result;

/// Enqueue the standard post-ingest ML jobs for `asset_id`. Returns the
/// number of jobs actually inserted (0, 1, or 2 depending on existing dedupe
/// state) so callers can log queue growth during bulk operations.
pub fn enqueue_post_ingest(
    conn: &rusqlite::Connection,
    asset_id: i64,
    now: i64,
) -> Result<u32> {
    let mut inserted = 0u32;
    if enqueue_dedup(conn, "embed_asset", asset_id, now)? {
        inserted += 1;
    }
    if enqueue_dedup(conn, "detect_faces", asset_id, now)? {
        inserted += 1;
    }
    Ok(inserted)
}

/// Wrap [`db::enqueue_ml_job`] so we report whether the row was actually
/// inserted vs. suppressed by the existing `(kind, asset_id)` uniqueness.
/// `enqueue_ml_job` returns the same id regardless of whether the row was
/// new or pre-existing, so we fingerprint by sampling the pending-kind count
/// before and after.
fn enqueue_dedup(
    conn: &rusqlite::Connection,
    kind: &str,
    asset_id: i64,
    now: i64,
) -> Result<bool> {
    let before: i64 = conn.query_row(
        r"SELECT COUNT(*) FROM ml_job WHERE kind = ?1 AND asset_id = ?2",
        rusqlite::params![kind, asset_id],
        |r| r.get(0),
    )?;
    db::enqueue_ml_job(conn, kind, Some(asset_id), now)?;
    let after: i64 = conn.query_row(
        r"SELECT COUNT(*) FROM ml_job WHERE kind = ?1 AND asset_id = ?2",
        rusqlite::params![kind, asset_id],
        |r| r.get(0),
    )?;
    Ok(after > before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::keystore;
    use crate::db::{self, AssetInsert};
    use secrecy::SecretString;

    fn open() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate::apply(&conn).unwrap();
        conn
    }

    fn seed_asset(conn: &rusqlite::Connection, seed: u8) -> i64 {
        let (record, _) =
            keystore::create_user("u", &SecretString::from("very-long-pw-xyz")).unwrap();
        let uid = db::insert_user(conn, &record, 0).unwrap();
        let sid = db::insert_source(conn, uid, b"s", b"r", None, false, 0).unwrap();
        let mut hash = [0u8; 32];
        hash[0] = seed;
        let a = AssetInsert {
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
        match db::insert_asset_if_new(conn, &a).unwrap() {
            db::InsertResult::Inserted(id) | db::InsertResult::Existing(id) => id,
        }
    }

    #[test]
    fn first_enqueue_inserts_two_jobs() {
        let conn = open();
        let asset_id = seed_asset(&conn, 1);
        let n = enqueue_post_ingest(&conn, asset_id, 0).unwrap();
        assert_eq!(n, 2);
        let counts = db::count_ml_jobs_by_state(&conn).unwrap();
        let pending = counts
            .iter()
            .find(|(s, _)| s == "pending")
            .map(|(_, n)| *n)
            .unwrap_or(0);
        assert_eq!(pending, 2);
    }

    #[test]
    fn repeat_enqueue_is_noop() {
        let conn = open();
        let asset_id = seed_asset(&conn, 2);
        enqueue_post_ingest(&conn, asset_id, 0).unwrap();
        let n = enqueue_post_ingest(&conn, asset_id, 10).unwrap();
        assert_eq!(n, 0, "second call must dedupe");
        let counts = db::count_ml_jobs_by_state(&conn).unwrap();
        let pending = counts
            .iter()
            .find(|(s, _)| s == "pending")
            .map(|(_, n)| *n)
            .unwrap_or(0);
        assert_eq!(pending, 2);
    }
}
