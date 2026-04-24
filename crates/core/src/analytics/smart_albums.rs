//! Smart albums (§9): rule-compiled collections whose membership is
//! materialised on demand.
//!
//! Shape:
//! 1. User authors a [`SmartRule`] via the chip builder (same surface as
//!    the normal search chips).
//! 2. [`SmartRule::compile`] converts it into a [`crate::search::SearchQuery`]
//!    — the same executor the Search view uses.
//! 3. [`materialize`] runs the search and writes the hit set into
//!    `collection_member_smart`. The UI reads from that table.
//!
//! Rule semantics today are "all clauses must match" (flat AND). Nested
//! AND/OR trees, `NOT`, and text-CLIP predicates are out of scope for D4
//! — the goal is parity with the existing search chip set.
//!
//! See `plans/phase-3-peers-smart.md §9`.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::crypto::CollectionKey;
use crate::search::{search, SearchQuery};
use crate::Result;

/// JSON-serialisable rule spec. Matches the fields of [`SearchQuery`]
/// that don't require an on-the-fly CLIP runtime so materialisation
/// works with models unavailable.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SmartRule {
    #[serde(default)]
    pub is_raw: Option<bool>,
    #[serde(default)]
    pub is_video: Option<bool>,
    #[serde(default)]
    pub is_screenshot: Option<bool>,
    #[serde(default)]
    pub is_live: Option<bool>,
    #[serde(default)]
    pub has_faces: Option<bool>,
    #[serde(default)]
    pub camera_make: Option<String>,
    #[serde(default)]
    pub lens: Option<String>,
    #[serde(default)]
    pub source_id: Option<i64>,
    #[serde(default)]
    pub person_ids: Vec<i64>,
    #[serde(default)]
    pub after_day: Option<i64>,
    #[serde(default)]
    pub before_day: Option<i64>,
}

impl SmartRule {
    /// Cap the materialised membership. 10k is plenty for any user-facing
    /// smart album; past that the UI has other problems.
    pub const LIMIT: u32 = 10_000;

    /// Convert to a [`SearchQuery`] the executor can consume. `text` is
    /// always `None` — smart albums don't use CLIP search.
    pub fn compile(&self) -> SearchQuery {
        let date_range = match (self.after_day, self.before_day) {
            (Some(a), Some(b)) => Some((day_to_dt(a), day_to_dt(b))),
            // If only one endpoint is set, widen the other side — the
            // search executor requires both halves of `date_range`.
            (Some(a), None) => Some((day_to_dt(a), day_to_dt(i64::MAX / 86_400))),
            (None, Some(b)) => Some((day_to_dt(i64::MIN / 86_400), day_to_dt(b))),
            (None, None) => None,
        };
        SearchQuery {
            text: None,
            person_ids: self.person_ids.clone(),
            date_range,
            source_id: self.source_id,
            has_faces: self.has_faces,
            is_video: self.is_video,
            is_raw: self.is_raw,
            is_screenshot: self.is_screenshot,
            is_live: self.is_live,
            camera_make: self.camera_make.clone(),
            lens: self.lens.clone(),
            limit: Self::LIMIT,
            hidden_vault_unlocked: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.is_raw.is_none()
            && self.is_video.is_none()
            && self.is_screenshot.is_none()
            && self.is_live.is_none()
            && self.has_faces.is_none()
            && self.camera_make.is_none()
            && self.lens.is_none()
            && self.source_id.is_none()
            && self.person_ids.is_empty()
            && self.after_day.is_none()
            && self.before_day.is_none()
    }
}

fn day_to_dt(day: i64) -> DateTime<Utc> {
    let secs = day.saturating_mul(86_400);
    DateTime::<Utc>::from_timestamp(secs, 0).unwrap_or_else(Utc::now)
}

/// Refresh a smart album's `collection_member_smart` snapshot by running
/// `rule.compile()` through [`search`] and writing the asset ids.
///
/// `ck` must be the collection key that can open EXIF ciphertext
/// (typically the default collection key) — only required if the rule
/// has `camera_make` / `lens` predicates; otherwise `None` is fine.
///
/// Returns the number of rows written. Idempotent: existing snapshot is
/// deleted first.
pub fn materialize(
    conn: &Connection,
    collection_id: i64,
    rule: &SmartRule,
    ck: Option<&CollectionKey>,
    now: i64,
) -> Result<usize> {
    let query = rule.compile();
    let hits = search(conn, &query, ck, None)?;
    conn.execute(
        "DELETE FROM collection_member_smart WHERE collection_id = ?1",
        params![collection_id],
    )?;
    for hit in &hits {
        conn.execute(
            "INSERT OR IGNORE INTO collection_member_smart
                (collection_id, asset_id, snapshot_at)
                VALUES (?1, ?2, ?3)",
            params![collection_id, hit.asset_id, now],
        )?;
    }
    Ok(hits.len())
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

    fn seed(conn: &rusqlite::Connection) -> (i64, i64) {
        use crate::crypto::keystore;
        use secrecy::SecretString;
        let (record, _) =
            keystore::create_user("u", &SecretString::from("very-long-password-xyz")).unwrap();
        let uid = db::insert_user(conn, &record, 0).unwrap();
        let sid = db::insert_source(conn, uid, b"s", b"r", None, false, 0).unwrap();
        (uid, sid)
    }

    fn insert(
        conn: &rusqlite::Connection,
        sid: i64,
        seed: u8,
        day: i64,
        is_raw: bool,
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
            is_raw,
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
    fn compile_empty_is_no_op() {
        let r = SmartRule::default();
        assert!(r.is_empty());
        let q = r.compile();
        assert!(q.text.is_none());
        assert!(q.date_range.is_none());
    }

    #[test]
    fn compile_with_flags_copies_through() {
        let r = SmartRule {
            is_raw: Some(true),
            camera_make: Some("Sony".into()),
            ..Default::default()
        };
        let q = r.compile();
        assert_eq!(q.is_raw, Some(true));
        assert_eq!(q.camera_make.as_deref(), Some("Sony"));
    }

    #[test]
    fn materialize_writes_snapshot() {
        let conn = open();
        let (uid, sid) = seed(&conn);
        // Create a smart album row.
        let cid = db::insert_collection(&conn, uid, "smart_album", b"n", false, None, 0).unwrap();
        // Two RAW assets, one JPEG.
        insert(&conn, sid, 1, 100, true, None);
        insert(&conn, sid, 2, 101, true, None);
        insert(&conn, sid, 3, 102, false, None);

        let rule = SmartRule {
            is_raw: Some(true),
            ..Default::default()
        };
        let n = materialize(&conn, cid, &rule, None, 42).unwrap();
        assert_eq!(n, 2);

        let members: Vec<i64> = conn
            .prepare(
                "SELECT asset_id FROM collection_member_smart
                 WHERE collection_id = ?1 ORDER BY asset_id",
            )
            .unwrap()
            .query_map(params![cid], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn materialize_is_idempotent() {
        let conn = open();
        let (uid, sid) = seed(&conn);
        let cid = db::insert_collection(&conn, uid, "smart_album", b"n", false, None, 0).unwrap();
        insert(&conn, sid, 1, 100, true, None);
        let rule = SmartRule {
            is_raw: Some(true),
            ..Default::default()
        };
        materialize(&conn, cid, &rule, None, 1).unwrap();
        materialize(&conn, cid, &rule, None, 2).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM collection_member_smart WHERE collection_id = ?1",
                params![cid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        let snap: i64 = conn
            .query_row(
                "SELECT snapshot_at FROM collection_member_smart WHERE collection_id = ?1",
                params![cid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snap, 2);
    }

    #[test]
    fn materialize_with_decrypt_filter() {
        let conn = open();
        let (uid, sid) = seed(&conn);
        let cid = db::insert_collection(&conn, uid, "smart_album", b"n", false, None, 0).unwrap();
        let ck = CollectionKey::random().unwrap();
        let canon = seal_row(b"Canon R5", 0, ck.as_bytes()).unwrap();
        let sony = seal_row(b"SONY A7 IV", 0, ck.as_bytes()).unwrap();
        insert(&conn, sid, 1, 100, true, Some(&canon));
        insert(&conn, sid, 2, 101, true, Some(&sony));
        insert(&conn, sid, 3, 102, false, Some(&sony));

        let rule = SmartRule {
            is_raw: Some(true),
            camera_make: Some("sony".into()),
            ..Default::default()
        };
        let n = materialize(&conn, cid, &rule, Some(&ck), 0).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn date_range_widens_when_half_open() {
        let r = SmartRule {
            after_day: Some(100),
            ..Default::default()
        };
        let q = r.compile();
        // Both endpoints populated so the executor's AND logic works.
        let (a, b) = q.date_range.unwrap();
        assert!(a <= b);
    }

    #[test]
    fn spec_roundtrips_as_json() {
        let r = SmartRule {
            is_raw: Some(true),
            camera_make: Some("Sony".into()),
            person_ids: vec![1, 3],
            after_day: Some(100),
            ..Default::default()
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: SmartRule = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.is_raw, Some(true));
        assert_eq!(r2.person_ids, vec![1, 3]);
        assert_eq!(r2.after_day, Some(100));
    }
}
