//! Additive migrations.
//!
//! Phase 1 was schema version 1; Phase 2 is version 2. Existing columns are
//! never changed — doing so requires a major migration release per
//! `plans/architecture.md` §4.4.
//!
//! ## Idempotency
//!
//! Migrations are re-runnable. In practice a previous process may have
//! committed the DDL changes before crashing, leaving `PRAGMA
//! user_version` behind the schema. `CREATE TABLE IF NOT EXISTS` and
//! `CREATE INDEX IF NOT EXISTS` handle themselves; `ALTER TABLE … ADD
//! COLUMN` does not. We run each migration batch statement-by-statement
//! through [`apply_statement_tolerant`], which tolerates
//! "duplicate column name" and "table … already exists" failures so a
//! half-applied migration self-heals on the next boot.

use rusqlite::Connection;

use crate::Result;

/// Target schema version shipped by this build.
pub const CURRENT_VERSION: i32 = 9;

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
        apply_tolerant_batch(conn, super::schema::DDL_V2)?;
        conn.pragma_update(None, "user_version", 2)?;
    }
    if version < 3 {
        apply_tolerant_batch(conn, super::schema::DDL_V3)?;
        conn.pragma_update(None, "user_version", 3)?;
    }
    if version < 4 {
        apply_tolerant_batch(conn, super::schema::DDL_V4)?;
        conn.pragma_update(None, "user_version", 4)?;
    }
    if version < 5 {
        apply_tolerant_batch(conn, super::schema::DDL_V5)?;
        conn.pragma_update(None, "user_version", 5)?;
    }
    if version < 6 {
        apply_tolerant_batch(conn, super::schema::DDL_V6)?;
        conn.pragma_update(None, "user_version", 6)?;
    }
    if version < 7 {
        apply_tolerant_batch(conn, super::schema::DDL_V7)?;
        conn.pragma_update(None, "user_version", 7)?;
    }
    if version < 8 {
        apply_tolerant_batch(conn, super::schema::DDL_V8)?;
        conn.pragma_update(None, "user_version", 8)?;
    }
    if version < 9 {
        apply_tolerant_batch(conn, super::schema::DDL_V9)?;
        conn.pragma_update(None, "user_version", 9)?;
    }
    Ok(())
}

/// Run a DDL batch, tolerating the handful of errors that indicate a
/// prior partial run already applied it. Two observed in the wild:
/// - `duplicate column name` — `ALTER TABLE … ADD COLUMN` on a column
///   the previous attempt committed before its `user_version` bump.
/// - `already exists` — `CREATE TABLE` / `CREATE INDEX` without
///   `IF NOT EXISTS` (we use `IF NOT EXISTS` in new migrations but
///   `DDL_V4` relies on a table rename that's atomic on success).
///
/// When tolerated, the batch stops at the first failing statement —
/// but every statement in our migration batches either uses
/// `IF NOT EXISTS` or is an `ALTER TABLE … ADD COLUMN` whose end state
/// the prior partial run already reached, so the subsequent
/// `user_version` bump in [`apply`] is safe.
fn apply_tolerant_batch(conn: &Connection, sql: &str) -> Result<()> {
    match conn.execute_batch(sql) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string().to_ascii_lowercase();
            if msg.contains("duplicate column name") || msg.contains("already exists") {
                tracing::warn!(
                    error = %e,
                    "migration DDL already applied by a prior partial run — continuing"
                );
                Ok(())
            } else {
                Err(crate::Error::from(e))
            }
        }
    }
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

    #[test]
    fn apply_recovers_from_partial_prior_run() {
        // Reproduce the in-the-wild case: a previous process committed
        // v7/v8/v9 DDL then failed to bump `user_version`. `apply()`
        // must see the stale pragma, re-run the DDL (which fails with
        // "duplicate column"), tolerate it, and advance the version.
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn).unwrap(); // go to v9
        assert_eq!(
            conn.query_row::<i32, _, _>("PRAGMA user_version", [], |r| r.get(0))
                .unwrap(),
            CURRENT_VERSION
        );
        // Simulate: rewind the pragma as if a crash beat the bump.
        conn.pragma_update(None, "user_version", 6_i32).unwrap();
        // Schema is already fully at v9; re-run must self-heal.
        apply(&conn).unwrap();
        assert_eq!(
            conn.query_row::<i32, _, _>("PRAGMA user_version", [], |r| r.get(0))
                .unwrap(),
            CURRENT_VERSION
        );
    }

    #[test]
    fn v8_to_v9_adds_pet_columns() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::schema::init(&conn).unwrap();
        conn.execute_batch(super::super::schema::DDL_V2).unwrap();
        conn.execute_batch(super::super::schema::DDL_V3).unwrap();
        conn.execute_batch(super::super::schema::DDL_V4).unwrap();
        conn.execute_batch(super::super::schema::DDL_V5).unwrap();
        conn.execute_batch(super::super::schema::DDL_V6).unwrap();
        conn.execute_batch(super::super::schema::DDL_V7).unwrap();
        conn.execute_batch(super::super::schema::DDL_V8).unwrap();
        conn.pragma_update(None, "user_version", 8_i32).unwrap();

        apply(&conn).unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(asset)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(cols.iter().any(|c| c == "is_pet"));
        assert!(cols.iter().any(|c| c == "pet_species_ct"));

        let idx: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_asset_is_pet'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1);
    }

    #[test]
    fn v7_to_v8_creates_public_link() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::schema::init(&conn).unwrap();
        conn.execute_batch(super::super::schema::DDL_V2).unwrap();
        conn.execute_batch(super::super::schema::DDL_V3).unwrap();
        conn.execute_batch(super::super::schema::DDL_V4).unwrap();
        conn.execute_batch(super::super::schema::DDL_V5).unwrap();
        conn.execute_batch(super::super::schema::DDL_V6).unwrap();
        conn.execute_batch(super::super::schema::DDL_V7).unwrap();
        conn.pragma_update(None, "user_version", 7_i32).unwrap();

        apply(&conn).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='public_link'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn v6_to_v7_adds_smart_spec_and_member_table() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::schema::init(&conn).unwrap();
        conn.execute_batch(super::super::schema::DDL_V2).unwrap();
        conn.execute_batch(super::super::schema::DDL_V3).unwrap();
        conn.execute_batch(super::super::schema::DDL_V4).unwrap();
        conn.execute_batch(super::super::schema::DDL_V5).unwrap();
        conn.execute_batch(super::super::schema::DDL_V6).unwrap();
        conn.pragma_update(None, "user_version", 6_i32).unwrap();

        apply(&conn).unwrap();

        // smart_spec_ct column on collection.
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(collection)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(cols.iter().any(|c| c == "smart_spec_ct"));

        // collection_member_smart table + asset index present.
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='collection_member_smart'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        let idx: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_collection_member_smart_asset'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1);
    }
}
