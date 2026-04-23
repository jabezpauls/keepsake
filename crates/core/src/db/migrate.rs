//! Additive migrations.
//!
//! Phase 1 was schema version 1; Phase 2 is version 2. Existing columns are
//! never changed — doing so requires a major migration release per
//! `plans/architecture.md` §4.4.

use rusqlite::Connection;

use crate::Result;

/// Target schema version shipped by this build.
pub const CURRENT_VERSION: i32 = 6;

/// Apply any migrations needed to bring `conn` up to [`CURRENT_VERSION`].
pub fn apply(conn: &Connection) -> Result<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == CURRENT_VERSION {
        return Ok(());
    }
    if version > CURRENT_VERSION {
        // DB was written by a newer build. We refuse to roll back silently —
        // callers see this as a plain Db error when they try to query.
        return Err(crate::Error::Db(rusqlite::Error::ExecuteReturnedResults));
    }

    if version < 1 {
        super::schema::init(conn)?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    if version < 2 {
        conn.execute_batch(super::schema::DDL_V2)?;
        conn.pragma_update(None, "user_version", 2)?;
    }
    if version < 3 {
        conn.execute_batch(super::schema::DDL_V3)?;
        conn.pragma_update(None, "user_version", 3)?;
    }
    if version < 4 {
        conn.execute_batch(super::schema::DDL_V4)?;
        conn.pragma_update(None, "user_version", 4)?;
    }
    if version < 5 {
        conn.execute_batch(super::schema::DDL_V5)?;
        conn.pragma_update(None, "user_version", 5)?;
    }
    if version < 6 {
        conn.execute_batch(super::schema::DDL_V6)?;
        conn.pragma_update(None, "user_version", 6)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_on_fresh_db_reaches_current_version() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();
        let v: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, CURRENT_VERSION);
    }

    #[test]
    fn apply_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();
        apply(&conn).unwrap();
        apply(&conn).unwrap();
        let v: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, CURRENT_VERSION);
    }

    #[test]
    fn v1_to_v2_upgrade_creates_new_tables() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::schema::init(&conn).unwrap();
        conn.pragma_update(None, "user_version", 1_i32).unwrap();

        apply(&conn).unwrap();
        for t in ["ml_job", "nd_cluster", "asset_vec"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "missing table after upgrade: {t}");
        }
        // path_hash column exists on asset_location.
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(asset_location)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(cols.iter().any(|c| c == "path_hash"));
    }

    #[test]
    fn v2_to_v3_creates_peer_accept() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::schema::init(&conn).unwrap();
        conn.execute_batch(super::super::schema::DDL_V2).unwrap();
        conn.pragma_update(None, "user_version", 2_i32).unwrap();

        apply(&conn).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='peer_accept'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn v3_to_v4_reshapes_collection_key_and_adds_ciphertext_blake3() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::schema::init(&conn).unwrap();
        conn.execute_batch(super::super::schema::DDL_V2).unwrap();
        conn.execute_batch(super::super::schema::DDL_V3).unwrap();
        conn.pragma_update(None, "user_version", 3_i32).unwrap();

        apply(&conn).unwrap();

        // collection_key now has peer_identity_pub.
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(collection_key)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(cols.iter().any(|c| c == "peer_identity_pub"));

        // user_id became nullable (partial unique index handles dedupe).
        let user_id_notnull: i64 = conn
            .query_row(
                "SELECT \"notnull\" FROM pragma_table_info('collection_key') WHERE name='user_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(user_id_notnull, 0, "user_id must be nullable post-v4");

        // asset.ciphertext_blake3 column exists.
        let acols: Vec<String> = conn
            .prepare("PRAGMA table_info(asset)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(acols.iter().any(|c| c == "ciphertext_blake3"));

        // Partial unique indexes are present.
        let idxs: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='collection_key'",
            )
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(idxs.iter().any(|n| n == "idx_collection_key_local"));
        assert!(idxs.iter().any(|n| n == "idx_collection_key_peer"));
    }

    #[test]
    fn v4_to_v5_creates_shared_namespace() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::schema::init(&conn).unwrap();
        conn.execute_batch(super::super::schema::DDL_V2).unwrap();
        conn.execute_batch(super::super::schema::DDL_V3).unwrap();
        conn.execute_batch(super::super::schema::DDL_V4).unwrap();
        conn.pragma_update(None, "user_version", 4_i32).unwrap();

        apply(&conn).unwrap();

        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='shared_namespace'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "shared_namespace table must exist after v5");

        let idx: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_shared_namespace_ns'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1);
    }

    #[test]
    fn v5_to_v6_creates_share_status() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::schema::init(&conn).unwrap();
        conn.execute_batch(super::super::schema::DDL_V2).unwrap();
        conn.execute_batch(super::super::schema::DDL_V3).unwrap();
        conn.execute_batch(super::super::schema::DDL_V4).unwrap();
        conn.execute_batch(super::super::schema::DDL_V5).unwrap();
        conn.pragma_update(None, "user_version", 5_i32).unwrap();

        apply(&conn).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='share_status'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }
}
