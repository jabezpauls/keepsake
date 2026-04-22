//! Search executor — Phase 2.
//!
//! Phase 2 scope:
//! - Plaintext filters (date range, flags, source) resolve entirely in SQL
//!   via [`crate::db::filter_assets`].
//! - `camera_make` / `lens` require decrypting per-row ciphertext — applied
//!   post-filter using the collection key the caller already unlocked.
//! - CLIP text search is gated on the `ml-models` feature flag; when the
//!   runtime is unavailable, text queries resolve to "no-op" (the other
//!   filters still apply) so the UI degrades gracefully.
//!
//! Vector search uses the plaintext [`crate::db::list_asset_vecs`] cache
//! (architecture.md §4.3 explicitly allows similarity-structure leakage in
//! exchange for query speed).

use chrono::{DateTime, Utc};

use crate::crypto::{open_row, CollectionKey};
use crate::db::{self, AssetFilter, AssetLite};
use crate::Result;

/// What the UI is asking for.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub text: Option<String>,
    pub person_ids: Vec<i64>,
    pub date_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    pub source_id: Option<i64>,
    pub has_faces: Option<bool>,
    pub is_video: Option<bool>,
    pub is_raw: Option<bool>,
    pub is_screenshot: Option<bool>,
    pub is_live: Option<bool>,
    pub camera_make: Option<String>,
    pub lens: Option<String>,
    pub limit: u32,
}

/// A single search hit. Ordered; caller iterates in order.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub asset_id: i64,
    /// `None` when the ordering was pure date. `Some(score)` when CLIP or a
    /// text filter produced a score.
    pub score: Option<f32>,
}

/// Execute a search. `ck` is the currently-unlocked default collection key
/// (needed to decrypt `device_ct` / `lens_ct` for string filters). Pass `None`
/// if those filters aren't in the query — a None key with a string filter
/// returns an empty result rather than panicking.
pub fn search(
    conn: &rusqlite::Connection,
    q: &SearchQuery,
    ck: Option<&CollectionKey>,
) -> Result<Vec<SearchHit>> {
    // 1. Plaintext filters → candidate set.
    let limit_hint = q.limit.saturating_mul(4).max(50);
    let base_filter = AssetFilter {
        after_day: q.date_range.map(|(a, _)| a.timestamp() / 86_400),
        before_day: q.date_range.map(|(_, b)| b.timestamp() / 86_400),
        source_id: q.source_id,
        is_video: q.is_video,
        is_raw: q.is_raw,
        is_screenshot: q.is_screenshot,
        is_live: q.is_live,
        // We fetch more than `limit` up-front because post-filters may reject
        // some, but we still cap in the DB to avoid pulling 100k rows.
        limit: Some(limit_hint),
    };
    let mut candidates = db::filter_assets(conn, &base_filter)?;

    // 2. has_faces filter.
    if let Some(want) = q.has_faces {
        let face_asset_ids: std::collections::HashSet<i64> = conn
            .prepare("SELECT DISTINCT asset_id FROM face")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        candidates.retain(|a| face_asset_ids.contains(&a.id) == want);
    }

    // 3. person filter.
    if !q.person_ids.is_empty() {
        let placeholders = q
            .person_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT DISTINCT asset_id FROM face WHERE person_id IN ({placeholders})");
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<rusqlite::types::Value> = q.person_ids.iter().map(|&i| i.into()).collect();
        let person_assets: std::collections::HashSet<i64> = stmt
            .query_map(rusqlite::params_from_iter(params), |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        candidates.retain(|a| person_assets.contains(&a.id));
    }

    // 4. Decrypt-in-memory string filters.
    if q.camera_make.is_some() || q.lens.is_some() {
        let Some(key) = ck else {
            return Ok(Vec::new());
        };
        candidates.retain(|a| string_matches(a, key, q));
    }

    // 5. CLIP text scoring (feature-gated). Without the flag we fall back to
    //    date-ordered results, which is still useful.
    let hits = maybe_clip_rerank(conn, &candidates, q)?;

    Ok(hits.into_iter().take(q.limit as usize).collect())
}

fn string_matches(a: &AssetLite, key: &CollectionKey, q: &SearchQuery) -> bool {
    let decrypt_lower = |bytes: &[u8]| -> Option<String> {
        open_row(bytes, 0, key.as_bytes())
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .map(|s| s.to_lowercase())
    };
    if let Some(want) = &q.camera_make {
        let want_l = want.to_lowercase();
        let matched = a
            .device_ct
            .as_deref()
            .and_then(decrypt_lower)
            .is_some_and(|s| s.contains(&want_l));
        if !matched {
            return false;
        }
    }
    if let Some(want) = &q.lens {
        let want_l = want.to_lowercase();
        let matched = a
            .lens_ct
            .as_deref()
            .and_then(decrypt_lower)
            .is_some_and(|s| s.contains(&want_l));
        if !matched {
            return false;
        }
    }
    true
}

/// CLIP text re-ranking — stub. Wired up in Step 4 once `MlRuntime` lands.
/// Without the `ml-models` feature flag, text queries silently fall back to
/// date-ordered results so the search UI keeps working on model-less builds.
fn maybe_clip_rerank(
    _conn: &rusqlite::Connection,
    candidates: &[AssetLite],
    _q: &SearchQuery,
) -> Result<Vec<SearchHit>> {
    Ok(candidates
        .iter()
        .map(|a| SearchHit {
            asset_id: a.id,
            score: None,
        })
        .collect())
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{seal_row, CollectionKey};
    use crate::db::{self, AssetInsert};

    fn open() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate::apply(&conn).unwrap();
        conn
    }

    fn seed_user_source(conn: &rusqlite::Connection) -> i64 {
        use crate::crypto::keystore;
        use secrecy::SecretString;
        let (record, _) =
            keystore::create_user("u", &SecretString::from("very-long-password-xyz")).unwrap();
        let uid = db::insert_user(conn, &record, 0).unwrap();
        db::insert_source(conn, uid, b"s", b"r", None, false, 0).unwrap()
    }

    fn insert_asset(
        conn: &rusqlite::Connection,
        sid: i64,
        seed: u8,
        day: i64,
        device_ct: Option<&[u8]>,
    ) -> i64 {
        let mut hash = [0u8; 32];
        hash[0] = seed;
        let a = AssetInsert {
            blake3_plaintext: &hash,
            mime: "image/jpeg",
            bytes: 0,
            width: None,
            height: None,
            duration_ms: None,
            taken_at_utc_day: Some(day),
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
            device_ct,
            lens_ct: None,
            exif_all_ct: None,
            wrapped_file_key: b"w",
        };
        match db::insert_asset_if_new(conn, &a).unwrap() {
            db::InsertResult::Inserted(x) | db::InsertResult::Existing(x) => x,
        }
    }

    #[test]
    fn date_range_filter() {
        let conn = open();
        let sid = seed_user_source(&conn);
        insert_asset(&conn, sid, 1, 100, None);
        insert_asset(&conn, sid, 2, 200, None);
        insert_asset(&conn, sid, 3, 300, None);

        let range_start = DateTime::<Utc>::from_timestamp(150 * 86_400, 0).unwrap();
        let range_end = DateTime::<Utc>::from_timestamp(250 * 86_400, 0).unwrap();
        let q = SearchQuery {
            date_range: Some((range_start, range_end)),
            limit: 50,
            ..Default::default()
        };
        let hits = search(&conn, &q, None).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn camera_make_filter_decrypts_rows() {
        let conn = open();
        let sid = seed_user_source(&conn);
        let ck = CollectionKey::random().unwrap();
        let device_ct_canon = seal_row(b"Canon R5", 0, ck.as_bytes()).unwrap();
        let device_ct_sony = seal_row(b"Sony A7", 0, ck.as_bytes()).unwrap();
        insert_asset(&conn, sid, 1, 100, Some(&device_ct_canon));
        insert_asset(&conn, sid, 2, 101, Some(&device_ct_sony));
        insert_asset(&conn, sid, 3, 102, None);

        let q = SearchQuery {
            camera_make: Some("canon".into()),
            limit: 50,
            ..Default::default()
        };
        let hits = search(&conn, &q, Some(&ck)).unwrap();
        assert_eq!(hits.len(), 1);

        let q_sony = SearchQuery {
            camera_make: Some("sony".into()),
            limit: 50,
            ..Default::default()
        };
        let hits = search(&conn, &q_sony, Some(&ck)).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn text_query_without_models_degrades_to_date_order() {
        let conn = open();
        let sid = seed_user_source(&conn);
        insert_asset(&conn, sid, 1, 100, None);
        insert_asset(&conn, sid, 2, 200, None);

        let q = SearchQuery {
            text: Some("dog on beach".into()),
            limit: 50,
            ..Default::default()
        };
        let hits = search(&conn, &q, None).unwrap();
        // Without ml-models, all hits are returned unscored in date-desc order.
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.score.is_none()));
    }
}
