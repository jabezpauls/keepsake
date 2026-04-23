//! Shared test helpers for integration tests.
//!
//! Generates fixtures on the fly rather than checking in binary files, so CI
//! never carries stale asset data and individual tests can shape the dump to
//! exactly what they're asserting against.

#![allow(dead_code, reason = "helpers used by selected integration tests")]

pub mod bulk;

use std::path::{Path, PathBuf};

use mv_core::cas::CasStore;
use mv_core::crypto::keystore::{self, UnlockedUser, UserRecord};
use mv_core::crypto::{wrap_collection_key, CollectionKey};
use mv_core::db;
use rusqlite::Connection;
use secrecy::SecretString;
use tempfile::TempDir;

pub struct Vault {
    pub tmp: TempDir,
    pub conn: Connection,
    pub cas: CasStore,
    pub user_id: i64,
    pub record: UserRecord,
    pub user: UnlockedUser,
    pub default_collection_id: i64,
    pub default_collection_key: CollectionKey,
    pub source_id: i64,
}

impl Vault {
    pub fn create(password: &str) -> Self {
        let tmp = TempDir::new().unwrap();
        let cas = CasStore::open(tmp.path()).unwrap();
        let conn = db::schema::open(&tmp.path().join("index.db")).unwrap();
        let (record, mut unlocked) =
            keystore::create_user("alice", &SecretString::from(password.to_string())).unwrap();
        let user_id = db::insert_user(&conn, &record, 0).unwrap();
        unlocked.user_id = user_id;

        let default_ck = CollectionKey::random().unwrap();
        let name_ct =
            mv_core::crypto::seal_row(b"Unsorted", 0, unlocked.master_key.as_bytes()).unwrap();
        let cid = db::insert_collection(&conn, user_id, "album", &name_ct, false, None, 0).unwrap();
        let wrapped = wrap_collection_key(&default_ck, &unlocked.master_key).unwrap();
        db::upsert_collection_key(&conn, cid, user_id, "master", &wrapped).unwrap();

        let source_id =
            db::insert_source(&conn, user_id, b"name", b"root", None, false, 0).unwrap();
        Self {
            tmp,
            conn,
            cas,
            user_id,
            record,
            user: unlocked,
            default_collection_id: cid,
            default_collection_key: default_ck,
            source_id,
        }
    }
}

/// Write a small deterministic JPEG to `path`. `seed` controls pixel values so
/// distinct seeds → distinct plaintext hashes.
pub fn write_jpeg(path: &Path, seed: u8) {
    let img: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> =
        image::ImageBuffer::from_fn(16, 12, |x, y| {
            image::Rgb([(x as u8).wrapping_add(seed), y as u8, seed])
        });
    img.save_with_format(path, image::ImageFormat::Jpeg)
        .unwrap();
}

/// Write an empty-but-extensioned placeholder — enough for the ingest
/// pipeline's extension-based candidate check, not enough to probe.
pub fn touch(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
}

/// Build a synthetic iPhone-style DCIM dump rooted at `root`.
///
/// The structure mirrors `phase-1-foundation.md` §5 Acceptance criteria:
/// a mix of JPEGs, HEIC+MOV live pairs, burst groups, a screenshot, and a
/// short video. Total = 10 JPEGs + (3 HEIC + 3 paired MOV) + 2 burst stems
/// + 1 screenshot + 1 video ≈ 20 files / 16 distinct plaintexts.
pub fn make_iphone_dump(root: &Path) -> Vec<PathBuf> {
    let dcim = root.join("DCIM").join("100APPLE");
    std::fs::create_dir_all(&dcim).unwrap();
    let mut out = Vec::new();

    // 10 plain JPEGs.
    for i in 0..10 {
        let p = dcim.join(format!("IMG_{:04}.JPG", 1000 + i));
        write_jpeg(&p, i as u8);
        out.push(p);
    }

    // 3 Live Photo pairs (HEIC + MOV, same stem). We cheat by writing JPEG
    // bytes into the .HEIC file (the pair detector only inspects filenames),
    // and a minimal JPEG-masquerading MOV that survives the extension-based
    // candidate check.
    for i in 0..3 {
        let stem = format!("IMG_{:04}", 2000 + i);
        let heic = dcim.join(format!("{stem}.HEIC"));
        let mov = dcim.join(format!("{stem}.MOV"));
        write_jpeg(&heic, 100 + i as u8);
        touch(&mov, &[0x42; 64]);
        out.push(heic);
        out.push(mov);
    }

    // Burst: pairs of IMG_xxxx + IMG_Exxxx for two stems (total 4 files,
    // grouped as 2 bursts).
    for i in 0..2 {
        let num = 3000 + i;
        let a = dcim.join(format!("IMG_{num:04}.JPG"));
        let b = dcim.join(format!("IMG_E{num:04}.JPG"));
        write_jpeg(&a, 150 + i as u8);
        write_jpeg(&b, 160 + i as u8);
        out.push(a);
        out.push(b);
    }

    // Screenshot — iOS names these `IMG_xxxx.PNG` under Screenshots/. The
    // probe heuristic flips `is_screenshot=true` when the file is a PNG
    // whose filename matches a screenshot pattern.
    let ss = dcim.join("IMG_Screenshot_001.PNG");
    let img: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> =
        image::ImageBuffer::from_fn(32, 48, |x, y| image::Rgb([x as u8, y as u8, 200]));
    img.save_with_format(&ss, image::ImageFormat::Png).unwrap();
    out.push(ss);

    // Short "video" placeholder.
    let vid = dcim.join("IMG_4000.MP4");
    touch(&vid, &[0x7E; 64]);
    out.push(vid);

    out
}
