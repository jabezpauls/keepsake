//! Hot-path query helpers for Phase 1.
//!
//! Everything a source-ingest → timeline → album flow needs. No ORM — each
//! function is an explicit SQL statement so the data flow is reviewable.

use rusqlite::{params, Connection, OptionalExtension};

use crate::crypto::keystore::UserRecord;
use crate::{Error, Result};

// --------- USER ---------------------------------------------------------------

pub fn insert_user(conn: &Connection, record: &UserRecord, created_at_utc: i64) -> Result<i64> {
    conn.execute(
        r"INSERT INTO user (
            username_ct, main_salt, wrapped_master_key, hidden_wrapped_master_key, hidden_salt,
            identity_pub, wrapped_identity_sk, iroh_node_pub, wrapped_iroh_sk, created_at
          ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            record.username_ct,
            record.main_salt.as_slice(),
            record.wrapped_master_key,
            record.hidden_wrapped_master_key,
            record.hidden_salt.as_slice(),
            record.identity_pub.as_slice(),
            record.wrapped_identity_sk,
            record.iroh_node_pub.as_slice(),
            record.wrapped_iroh_sk,
            created_at_utc,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn user_exists(conn: &Connection) -> Result<bool> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM user", [], |r| r.get(0))?;
    Ok(n > 0)
}

pub fn get_user_record(conn: &Connection, user_id: i64) -> Result<UserRecord> {
    conn.query_row(
        r"SELECT username_ct, main_salt, wrapped_master_key,
                 hidden_wrapped_master_key, hidden_salt,
                 identity_pub, wrapped_identity_sk,
                 iroh_node_pub, wrapped_iroh_sk
          FROM user WHERE id = ?1",
        params![user_id],
        |r| {
            Ok(UserRecord {
                username_ct: r.get::<_, Vec<u8>>(0)?,
                main_salt: fixed_16(&r.get::<_, Vec<u8>>(1)?)?,
                wrapped_master_key: r.get::<_, Vec<u8>>(2)?,
                hidden_wrapped_master_key: r.get::<_, Vec<u8>>(3)?,
                hidden_salt: fixed_16(&r.get::<_, Vec<u8>>(4)?)?,
                identity_pub: fixed_32(&r.get::<_, Vec<u8>>(5)?)?,
                wrapped_identity_sk: r.get::<_, Vec<u8>>(6)?,
                iroh_node_pub: fixed_32(&r.get::<_, Vec<u8>>(7)?)?,
                wrapped_iroh_sk: r.get::<_, Vec<u8>>(8)?,
            })
        },
    )
    .map_err(Error::from)
}

fn fixed_16(b: &[u8]) -> rusqlite::Result<[u8; 16]> {
    b.try_into().map_err(|_| {
        rusqlite::Error::InvalidColumnType(
            0,
            "expected 16 bytes".into(),
            rusqlite::types::Type::Blob,
        )
    })
}

fn fixed_32(b: &[u8]) -> rusqlite::Result<[u8; 32]> {
    b.try_into().map_err(|_| {
        rusqlite::Error::InvalidColumnType(
            0,
            "expected 32 bytes".into(),
            rusqlite::types::Type::Blob,
        )
    })
}

// --------- SOURCE -------------------------------------------------------------

pub struct SourceRow {
    pub id: i64,
    pub name_ct: Vec<u8>,
    pub root_path_ct: Vec<u8>,
    pub device_hint_ct: Option<Vec<u8>>,
    pub imported_at: i64,
    pub bytes_total: i64,
    pub file_count: i64,
    pub linked_only: bool,
}

pub fn insert_source(
    conn: &Connection,
    owner_id: i64,
    name_ct: &[u8],
    root_path_ct: &[u8],
    device_hint_ct: Option<&[u8]>,
    linked_only: bool,
    imported_at: i64,
) -> Result<i64> {
    conn.execute(
        r"INSERT INTO source (
            owner_id, name_ct, root_path_ct, device_hint_ct,
            imported_at, bytes_total, file_count, linked_only
          ) VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, ?6)",
        params![
            owner_id,
            name_ct,
            root_path_ct,
            device_hint_ct,
            imported_at,
            linked_only as i64
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn bump_source_stats(
    conn: &Connection,
    source_id: i64,
    delta_bytes: i64,
    delta_files: i64,
) -> Result<()> {
    conn.execute(
        r"UPDATE source SET bytes_total = bytes_total + ?1, file_count = file_count + ?2 WHERE id = ?3",
        params![delta_bytes, delta_files, source_id],
    )?;
    Ok(())
}

pub fn list_sources(conn: &Connection, owner_id: i64) -> Result<Vec<SourceRow>> {
    let mut stmt = conn.prepare(
        r"SELECT id, name_ct, root_path_ct, device_hint_ct, imported_at, bytes_total, file_count, linked_only
          FROM source WHERE owner_id = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![owner_id], |r| {
            Ok(SourceRow {
                id: r.get(0)?,
                name_ct: r.get(1)?,
                root_path_ct: r.get(2)?,
                device_hint_ct: r.get(3)?,
                imported_at: r.get(4)?,
                bytes_total: r.get(5)?,
                file_count: r.get(6)?,
                linked_only: r.get::<_, i64>(7)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// --------- ASSET --------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AssetInsert<'a> {
    pub blake3_plaintext: &'a [u8; 32],
    pub mime: &'a str,
    pub bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub taken_at_utc_day: Option<i64>,
    pub is_video: bool,
    pub is_raw: bool,
    pub is_screenshot: bool,
    pub is_live: bool,
    pub is_motion: bool,
    pub source_id: i64,
    pub cas_ref: &'a str,
    pub imported_at: i64,
    pub filename_ct: &'a [u8],
    pub taken_at_utc_ct: Option<&'a [u8]>,
    pub gps_ct: Option<&'a [u8]>,
    pub device_ct: Option<&'a [u8]>,
    pub lens_ct: Option<&'a [u8]>,
    pub exif_all_ct: Option<&'a [u8]>,
    pub wrapped_file_key: &'a [u8],
}

#[derive(Debug, PartialEq, Eq)]
pub enum InsertResult {
    Inserted(i64),
    Existing(i64),
}

/// Insert an asset or, if `blake3_plaintext` already exists, return its id.
///
/// Uses `INSERT OR IGNORE` + a follow-up `SELECT` so Phase 1 can treat
/// duplicate content as a no-op. Callers still record a per-source
/// `asset_location` row for provenance.
pub fn insert_asset_if_new(conn: &Connection, a: &AssetInsert<'_>) -> Result<InsertResult> {
    let affected = conn.execute(
        r"INSERT OR IGNORE INTO asset (
            blake3_plaintext, mime, bytes, width, height, duration_ms,
            taken_at_utc_day, is_video, is_raw, is_screenshot, is_live, is_motion,
            source_id, cas_ref, imported_at,
            filename_ct, taken_at_utc_ct, gps_ct, device_ct, lens_ct, exif_all_ct,
            wrapped_file_key
          ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
        params![
            a.blake3_plaintext.as_slice(),
            a.mime,
            a.bytes,
            a.width,
            a.height,
            a.duration_ms,
            a.taken_at_utc_day,
            a.is_video as i64,
            a.is_raw as i64,
            a.is_screenshot as i64,
            a.is_live as i64,
            a.is_motion as i64,
            a.source_id,
            a.cas_ref,
            a.imported_at,
            a.filename_ct,
            a.taken_at_utc_ct,
            a.gps_ct,
            a.device_ct,
            a.lens_ct,
            a.exif_all_ct,
            a.wrapped_file_key,
        ],
    )?;
    // Look up id regardless — if affected == 0 we want the existing one.
    let id: i64 = conn.query_row(
        "SELECT id FROM asset WHERE blake3_plaintext = ?1",
        params![a.blake3_plaintext.as_slice()],
        |r| r.get(0),
    )?;
    if affected > 0 {
        Ok(InsertResult::Inserted(id))
    } else {
        Ok(InsertResult::Existing(id))
    }
}

pub fn insert_asset_location(
    conn: &Connection,
    asset_id: i64,
    source_id: i64,
    original_path_ct: &[u8],
    mtime: i64,
) -> Result<()> {
    conn.execute(
        r"INSERT OR IGNORE INTO asset_location (asset_id, source_id, original_path_ct, mtime)
          VALUES (?1, ?2, ?3, ?4)",
        params![asset_id, source_id, original_path_ct, mtime],
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct TimelineEntry {
    pub id: i64,
    pub taken_at_utc_day: Option<i64>,
    pub mime: String,
    pub cas_ref: String,
    pub is_video: bool,
    pub is_live: bool,
    pub wrapped_file_key: Vec<u8>,
}

/// Cursor = last `(taken_at_utc_day, id)` seen. Use `(i64::MAX, i64::MAX)` as
/// the initial cursor.
pub fn list_timeline_page(
    conn: &Connection,
    cursor_day: i64,
    cursor_id: i64,
    limit: u32,
) -> Result<Vec<TimelineEntry>> {
    let mut stmt = conn.prepare(
        r"SELECT id, taken_at_utc_day, mime, cas_ref, is_video, is_live, wrapped_file_key
          FROM asset
          WHERE (COALESCE(taken_at_utc_day, 0) < ?1)
             OR (COALESCE(taken_at_utc_day, 0) = ?1 AND id < ?2)
          ORDER BY COALESCE(taken_at_utc_day, 0) DESC, id DESC
          LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![cursor_day, cursor_id, limit], |r| {
            Ok(TimelineEntry {
                id: r.get(0)?,
                taken_at_utc_day: r.get(1)?,
                mime: r.get(2)?,
                cas_ref: r.get(3)?,
                is_video: r.get::<_, i64>(4)? != 0,
                is_live: r.get::<_, i64>(5)? != 0,
                wrapped_file_key: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// --------- COLLECTIONS --------------------------------------------------------

pub fn insert_collection(
    conn: &Connection,
    owner_id: i64,
    kind: &str,
    name_ct: &[u8],
    has_password: bool,
    password_salt: Option<&[u8; 16]>,
    created_at: i64,
) -> Result<i64> {
    conn.execute(
        r"INSERT INTO collection (owner_id, kind, name_ct, has_password, password_salt, created_at)
          VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            owner_id,
            kind,
            name_ct,
            has_password as i64,
            password_salt.map(|s| s.as_slice()),
            created_at
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn add_to_collection(
    conn: &Connection,
    collection_id: i64,
    asset_id: i64,
    added_at: i64,
) -> Result<()> {
    conn.execute(
        r"INSERT OR IGNORE INTO collection_member (collection_id, asset_id, added_at)
          VALUES (?1, ?2, ?3)",
        params![collection_id, asset_id, added_at],
    )?;
    Ok(())
}

pub fn upsert_collection_key(
    conn: &Connection,
    collection_id: i64,
    user_id: i64,
    wrapping: &str,
    wrapped_key: &[u8],
) -> Result<()> {
    conn.execute(
        r"INSERT INTO collection_key (collection_id, user_id, wrapping, wrapped_key)
          VALUES (?1, ?2, ?3, ?4)
          ON CONFLICT(collection_id, user_id, wrapping) DO UPDATE SET wrapped_key = excluded.wrapped_key",
        params![collection_id, user_id, wrapping, wrapped_key],
    )?;
    Ok(())
}

pub fn get_collection_key(
    conn: &Connection,
    collection_id: i64,
    user_id: i64,
    wrapping: &str,
) -> Result<Option<Vec<u8>>> {
    let r = conn
        .query_row(
            r"SELECT wrapped_key FROM collection_key
              WHERE collection_id = ?1 AND user_id = ?2 AND wrapping = ?3",
            params![collection_id, user_id, wrapping],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .optional()?;
    Ok(r)
}

pub fn set_album_password_salt(
    conn: &Connection,
    collection_id: i64,
    salt: &[u8; 16],
) -> Result<()> {
    conn.execute(
        r"UPDATE collection SET has_password = 1, password_salt = ?1 WHERE id = ?2",
        params![salt.as_slice(), collection_id],
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct CollectionRow {
    pub id: i64,
    pub owner_id: i64,
    pub kind: String,
    pub name_ct: Vec<u8>,
    pub has_password: bool,
    pub password_salt: Option<[u8; 16]>,
    pub created_at: i64,
}

fn row_to_collection(r: &rusqlite::Row<'_>) -> rusqlite::Result<CollectionRow> {
    let salt: Option<Vec<u8>> = r.get(5)?;
    let password_salt = salt
        .map(|s| {
            s.as_slice().try_into().map_err(|_| {
                rusqlite::Error::InvalidColumnType(
                    5,
                    "expected 16 bytes".into(),
                    rusqlite::types::Type::Blob,
                )
            })
        })
        .transpose()?;
    Ok(CollectionRow {
        id: r.get(0)?,
        owner_id: r.get(1)?,
        kind: r.get(2)?,
        name_ct: r.get(3)?,
        has_password: r.get::<_, i64>(4)? != 0,
        password_salt,
        created_at: r.get(6)?,
    })
}

pub fn list_collections(
    conn: &Connection,
    owner_id: i64,
    include_hidden: bool,
) -> Result<Vec<CollectionRow>> {
    let mut stmt = if include_hidden {
        conn.prepare(
            r"SELECT id, owner_id, kind, name_ct, has_password, password_salt, created_at
              FROM collection WHERE owner_id = ?1 ORDER BY id",
        )?
    } else {
        conn.prepare(
            r"SELECT id, owner_id, kind, name_ct, has_password, password_salt, created_at
              FROM collection WHERE owner_id = ?1 AND kind != 'hidden_vault' ORDER BY id",
        )?
    };
    let rows = stmt
        .query_map(params![owner_id], row_to_collection)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_collection(conn: &Connection, id: i64) -> Result<Option<CollectionRow>> {
    let r = conn
        .query_row(
            r"SELECT id, owner_id, kind, name_ct, has_password, password_salt, created_at
              FROM collection WHERE id = ?1",
            params![id],
            row_to_collection,
        )
        .optional()?;
    Ok(r)
}

pub fn count_collection_members(conn: &Connection, collection_id: i64) -> Result<i64> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM collection_member WHERE collection_id = ?1",
        params![collection_id],
        |r| r.get(0),
    )?;
    Ok(n)
}

pub fn list_collection_page(
    conn: &Connection,
    collection_id: i64,
    cursor_day: i64,
    cursor_id: i64,
    limit: u32,
) -> Result<Vec<TimelineEntry>> {
    let mut stmt = conn.prepare(
        r"SELECT a.id, a.taken_at_utc_day, a.mime, a.cas_ref, a.is_video, a.is_live, a.wrapped_file_key
          FROM asset a
          JOIN collection_member m ON m.asset_id = a.id
          WHERE m.collection_id = ?1
            AND ((COALESCE(a.taken_at_utc_day, 0) < ?2)
              OR (COALESCE(a.taken_at_utc_day, 0) = ?2 AND a.id < ?3))
          ORDER BY COALESCE(a.taken_at_utc_day, 0) DESC, a.id DESC
          LIMIT ?4",
    )?;
    let rows = stmt
        .query_map(params![collection_id, cursor_day, cursor_id, limit], |r| {
            Ok(TimelineEntry {
                id: r.get(0)?,
                taken_at_utc_day: r.get(1)?,
                mime: r.get(2)?,
                cas_ref: r.get(3)?,
                is_video: r.get::<_, i64>(4)? != 0,
                is_live: r.get::<_, i64>(5)? != 0,
                wrapped_file_key: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[derive(Debug, Clone)]
pub struct AssetRow {
    pub id: i64,
    pub blake3_plaintext: [u8; 32],
    pub mime: String,
    pub bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub taken_at_utc_day: Option<i64>,
    pub is_video: bool,
    pub is_raw: bool,
    pub is_screenshot: bool,
    pub is_live: bool,
    pub is_motion: bool,
    pub source_id: i64,
    pub cas_ref: String,
    pub imported_at: i64,
    pub filename_ct: Vec<u8>,
    pub taken_at_utc_ct: Option<Vec<u8>>,
    pub gps_ct: Option<Vec<u8>>,
    pub device_ct: Option<Vec<u8>>,
    pub lens_ct: Option<Vec<u8>>,
    pub exif_all_ct: Option<Vec<u8>>,
    pub wrapped_file_key: Vec<u8>,
}

pub fn get_asset(conn: &Connection, id: i64) -> Result<Option<AssetRow>> {
    let r = conn
        .query_row(
            r"SELECT id, blake3_plaintext, mime, bytes, width, height, duration_ms,
                     taken_at_utc_day, is_video, is_raw, is_screenshot, is_live, is_motion,
                     source_id, cas_ref, imported_at,
                     filename_ct, taken_at_utc_ct, gps_ct, device_ct, lens_ct, exif_all_ct,
                     wrapped_file_key
              FROM asset WHERE id = ?1",
            params![id],
            |r| {
                let blake = fixed_32(&r.get::<_, Vec<u8>>(1)?)?;
                Ok(AssetRow {
                    id: r.get(0)?,
                    blake3_plaintext: blake,
                    mime: r.get(2)?,
                    bytes: r.get(3)?,
                    width: r.get(4)?,
                    height: r.get(5)?,
                    duration_ms: r.get(6)?,
                    taken_at_utc_day: r.get(7)?,
                    is_video: r.get::<_, i64>(8)? != 0,
                    is_raw: r.get::<_, i64>(9)? != 0,
                    is_screenshot: r.get::<_, i64>(10)? != 0,
                    is_live: r.get::<_, i64>(11)? != 0,
                    is_motion: r.get::<_, i64>(12)? != 0,
                    source_id: r.get(13)?,
                    cas_ref: r.get(14)?,
                    imported_at: r.get(15)?,
                    filename_ct: r.get(16)?,
                    taken_at_utc_ct: r.get(17)?,
                    gps_ct: r.get(18)?,
                    device_ct: r.get(19)?,
                    lens_ct: r.get(20)?,
                    exif_all_ct: r.get(21)?,
                    wrapped_file_key: r.get(22)?,
                })
            },
        )
        .optional()?;
    Ok(r)
}

pub fn list_collection_member_ids(conn: &Connection, collection_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT asset_id FROM collection_member WHERE collection_id = ?1 ORDER BY asset_id",
    )?;
    let rows = stmt
        .query_map(params![collection_id], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// --------- EDITS + DERIVATIVES ------------------------------------------------

pub fn set_edit(conn: &Connection, asset_id: i64, ops_ct: &[u8]) -> Result<()> {
    conn.execute(
        r"INSERT INTO edit (asset_id, ops_ct) VALUES (?1, ?2)
          ON CONFLICT(asset_id) DO UPDATE SET ops_ct = excluded.ops_ct",
        params![asset_id, ops_ct],
    )?;
    Ok(())
}

pub fn insert_derivative(
    conn: &Connection,
    asset_id: i64,
    kind: &str,
    cas_ref: &str,
) -> Result<()> {
    conn.execute(
        r"INSERT OR REPLACE INTO derivative (asset_id, kind, cas_ref) VALUES (?1, ?2, ?3)",
        params![asset_id, kind, cas_ref],
    )?;
    Ok(())
}

pub fn get_derivative(conn: &Connection, asset_id: i64, kind: &str) -> Result<Option<String>> {
    let r = conn
        .query_row(
            r"SELECT cas_ref FROM derivative WHERE asset_id = ?1 AND kind = ?2",
            params![asset_id, kind],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    Ok(r)
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::keystore;
    use secrecy::SecretString;

    fn open_mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate::apply(&conn).unwrap();
        conn
    }

    #[test]
    fn schema_creates_all_tables() {
        let conn = open_mem();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for t in [
            "asset",
            "asset_location",
            "collection",
            "collection_key",
            "collection_member",
            "derivative",
            "edit",
            "face",
            "person",
            "phash",
            "source",
            "user",
        ] {
            assert!(names.iter().any(|n| n == t), "missing table {t}");
        }
    }

    #[test]
    fn user_insert_get_round_trip() {
        let conn = open_mem();
        let (record, _) =
            keystore::create_user("u", &SecretString::from("long-password-xxxxxxx")).unwrap();
        let id = insert_user(&conn, &record, 123).unwrap();
        let got = get_user_record(&conn, id).unwrap();
        assert_eq!(got.username_ct, record.username_ct);
        assert_eq!(got.main_salt, record.main_salt);
        assert_eq!(got.wrapped_master_key, record.wrapped_master_key);
        assert_eq!(got.identity_pub, record.identity_pub);
        assert_eq!(got.iroh_node_pub, record.iroh_node_pub);
        assert!(user_exists(&conn).unwrap());
    }

    fn seed_user_and_source(conn: &Connection) -> (i64, i64) {
        let (record, _) =
            keystore::create_user("u", &SecretString::from("long-password-xxxxxxx")).unwrap();
        let uid = insert_user(conn, &record, 0).unwrap();
        let sid = insert_source(conn, uid, b"name-ct", b"root-ct", None, false, 0).unwrap();
        (uid, sid)
    }

    #[test]
    fn asset_insert_dedupes_on_blake3() {
        let conn = open_mem();
        let (_uid, sid) = seed_user_and_source(&conn);
        let hash = [7u8; 32];
        let a = AssetInsert {
            blake3_plaintext: &hash,
            mime: "image/jpeg",
            bytes: 100,
            width: Some(640),
            height: Some(480),
            duration_ms: None,
            taken_at_utc_day: Some(20_000),
            is_video: false,
            is_raw: false,
            is_screenshot: false,
            is_live: false,
            is_motion: false,
            source_id: sid,
            cas_ref: "deadbeef",
            imported_at: 0,
            filename_ct: b"fn-ct",
            taken_at_utc_ct: None,
            gps_ct: None,
            device_ct: None,
            lens_ct: None,
            exif_all_ct: None,
            wrapped_file_key: b"wfk",
        };
        let first = insert_asset_if_new(&conn, &a).unwrap();
        let second = insert_asset_if_new(&conn, &a).unwrap();
        assert!(matches!(first, InsertResult::Inserted(_)));
        assert!(matches!(second, InsertResult::Existing(_)));
        match (first, second) {
            (InsertResult::Inserted(id1), InsertResult::Existing(id2)) => assert_eq!(id1, id2),
            _ => unreachable!(),
        }
    }

    #[test]
    fn timeline_pagination_orders_by_day_desc() {
        let conn = open_mem();
        let (_uid, sid) = seed_user_and_source(&conn);
        for (i, day) in [100_i64, 200, 150, 50].iter().enumerate() {
            let mut hash = [0u8; 32];
            hash[0] = i as u8;
            let a = AssetInsert {
                blake3_plaintext: &hash,
                mime: "image/jpeg",
                bytes: 10,
                width: None,
                height: None,
                duration_ms: None,
                taken_at_utc_day: Some(*day),
                is_video: false,
                is_raw: false,
                is_screenshot: false,
                is_live: false,
                is_motion: false,
                source_id: sid,
                cas_ref: &format!("ref{i}"),
                imported_at: 0,
                filename_ct: b"fn",
                taken_at_utc_ct: None,
                gps_ct: None,
                device_ct: None,
                lens_ct: None,
                exif_all_ct: None,
                wrapped_file_key: b"wfk",
            };
            insert_asset_if_new(&conn, &a).unwrap();
        }
        let page = list_timeline_page(&conn, i64::MAX, i64::MAX, 10).unwrap();
        let days: Vec<_> = page.iter().map(|e| e.taken_at_utc_day.unwrap()).collect();
        assert_eq!(days, vec![200, 150, 100, 50]);
    }

    #[test]
    fn collection_members_and_keys() {
        let conn = open_mem();
        let (uid, _sid) = seed_user_and_source(&conn);
        let cid = insert_collection(&conn, uid, "album", b"name-ct", false, None, 0).unwrap();
        upsert_collection_key(&conn, cid, uid, "master", b"wrapped-master").unwrap();
        let k = get_collection_key(&conn, cid, uid, "master").unwrap();
        assert_eq!(k.as_deref(), Some(b"wrapped-master".as_slice()));
        // A second upsert updates the value.
        upsert_collection_key(&conn, cid, uid, "master", b"updated").unwrap();
        let k2 = get_collection_key(&conn, cid, uid, "master").unwrap();
        assert_eq!(k2.as_deref(), Some(b"updated".as_slice()));
    }

    #[test]
    fn derivative_roundtrip() {
        let conn = open_mem();
        let (_uid, sid) = seed_user_and_source(&conn);
        let hash = [1u8; 32];
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
        let id = match insert_asset_if_new(&conn, &a).unwrap() {
            InsertResult::Inserted(x) | InsertResult::Existing(x) => x,
        };
        insert_derivative(&conn, id, "thumb256", "deadbeef").unwrap();
        assert_eq!(
            get_derivative(&conn, id, "thumb256").unwrap().as_deref(),
            Some("deadbeef")
        );
    }
}
