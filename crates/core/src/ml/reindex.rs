//! Reindex sweep for pre-Phase-2.1 libraries.
//!
//! Walks plaintext tables and enqueues `embed_asset` / `detect_faces` jobs
//! for every asset that hasn't been processed yet. Safe to run repeatedly:
//! [`crate::db::enqueue_ml_job`] dedupes on `(kind, asset_id)`, so calling
//! `reindex_all` twice just returns the same counts the second time with
//! zero new enqueues.
//!
//! Runs against plaintext metadata only — no collection keys required, so
//! the command works pre-unlock and is safe to surface as an operator tool.

use crate::db;
use crate::Result;

/// Summary of what a reindex run queued. Returned to the UI so callers can
/// render "queued 1,234 assets for embedding" toasts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReindexReport {
    pub embed_queued: u32,
    pub detect_queued: u32,
    /// Total assets that had at least one sweep hit them. Useful for showing
    /// a single "N assets reindexed" number rather than two separate kinds.
    pub assets_touched: u32,
}

/// Enqueue every still-needed ML job. Idempotent.
///
/// Returns a report with exact counts of newly-inserted jobs (dedupes are
/// silently skipped and do not count).
pub fn reindex_all(conn: &rusqlite::Connection, now: i64) -> Result<ReindexReport> {
    let mut report = ReindexReport::default();
    let mut touched = std::collections::BTreeSet::<i64>::new();

    for id in db::sweep_assets_missing_embeddings(conn)? {
        if enqueue_new(conn, "embed_asset", id, now)? {
            report.embed_queued += 1;
        }
        touched.insert(id);
    }
    for id in db::sweep_assets_needing_faces(conn)? {
        if enqueue_new(conn, "detect_faces", id, now)? {
            report.detect_queued += 1;
        }
        touched.insert(id);
    }
    report.assets_touched = touched.len() as u32;
    Ok(report)
}

/// Enqueue and report whether the row actually landed (true = new,
/// false = dedupe). Counts before/after since `enqueue_ml_job` returns the
/// same id whether it wrote or matched an existing row.
fn enqueue_new(
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

    fn seed_assets(conn: &rusqlite::Connection, count: u8) -> Vec<i64> {
        let (record, _) =
            keystore::create_user("u", &SecretString::from("very-long-pw-xyz")).unwrap();
        let uid = db::insert_user(conn, &record, 0).unwrap();
        let sid = db::insert_source(conn, uid, b"s", b"r", None, false, 0).unwrap();
        let mut ids = Vec::with_capacity(count as usize);
        for seed in 0..count {
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
                db::InsertResult::Inserted(id) | db::InsertResult::Existing(id) => ids.push(id),
            }
        }
        ids
    }

    #[test]
    fn reindex_covers_all_assets_on_cold_library() {
        let conn = open();
        let ids = seed_assets(&conn, 5);
        let report = reindex_all(&conn, 0).unwrap();
        assert_eq!(report.embed_queued, 5);
        assert_eq!(report.detect_queued, 5);
        assert_eq!(report.assets_touched as usize, ids.len());
    }

    #[test]
    fn reindex_is_idempotent() {
        let conn = open();
        seed_assets(&conn, 3);
        reindex_all(&conn, 0).unwrap();
        let r2 = reindex_all(&conn, 10).unwrap();
        assert_eq!(r2.embed_queued, 0);
        assert_eq!(r2.detect_queued, 0);
        assert_eq!(r2.assets_touched, 3, "sweeps still see the same assets");
    }

    #[test]
    fn reindex_skips_assets_with_existing_vector() {
        let conn = open();
        let ids = seed_assets(&conn, 3);
        // Seed an asset_vec row for id[0] — the embed sweep should skip it.
        db::upsert_asset_vec(&conn, ids[0], &[0.1_f32; 768]).unwrap();
        let report = reindex_all(&conn, 0).unwrap();
        assert_eq!(report.embed_queued, 2, "skip the one with a vector");
        assert_eq!(report.detect_queued, 3, "face sweep unchanged");
    }

    #[test]
    fn reindex_skips_assets_with_existing_face_rows() {
        let conn = open();
        let ids = seed_assets(&conn, 3);
        // Seed a dummy face row for id[0]; detect sweep should skip it.
        db::insert_face(&conn, ids[0], 0.9, b"bbox", b"emb").unwrap();
        let report = reindex_all(&conn, 0).unwrap();
        assert_eq!(report.embed_queued, 3);
        assert_eq!(report.detect_queued, 2);
    }
}
