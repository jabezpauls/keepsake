//! Additive migrations.
//!
//! Phase 1 is schema version 1. Future phases increment `user_version` and
//! add new tables/indexes here. Existing columns are never changed — doing so
//! requires a major migration release per `plans/architecture.md` §4.4.

use rusqlite::Connection;

use crate::Result;

/// Target schema version shipped by this build.
pub const CURRENT_VERSION: i32 = 1;

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

    // version == 0 → run initial DDL.
    super::schema::init(conn)?;
    conn.pragma_update(None, "user_version", CURRENT_VERSION)?;
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
}
