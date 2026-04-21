//! Generic recursive folder ingest.
//!
//! Scans `root` for media files, hashes+encrypts each into the CAS, inserts
//! an `asset` row (deduping on plaintext hash), records provenance in
//! `asset_location`, and generates thumbnail derivatives.
//!
//! This is the shared core used by the iPhone + Takeout adapters (Step 10).

use std::path::Path;

use chrono::Utc;
use walkdir::WalkDir;

use crate::crypto::{seal_row, wrap_file_key, FileKey};
use crate::db;
use crate::ingest::sidecar::read_xmp_sidecar;
use crate::ingest::{IngestAdapter, IngestCtx, IngestReport, Progress, ProgressEvent, ScanReport};
use crate::media::{derive_thumbnails, probe_path, MediaProbe};
use crate::{Error, Result};

pub struct GenericAdapter;

impl GenericAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GenericAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl IngestAdapter for GenericAdapter {
    fn kind(&self) -> &'static str {
        "generic"
    }

    async fn scan(&self, root: &Path) -> Result<ScanReport> {
        let root = root.to_path_buf();
        tokio::task::spawn_blocking(move || scan_blocking(&root))
            .await
            .map_err(|e| Error::Ingest(format!("scan join: {e}")))?
    }

    async fn ingest(
        &self,
        ctx: &IngestCtx,
        root: &Path,
        source_id: i64,
        progress: Progress,
    ) -> Result<IngestReport> {
        let cas = ctx.cas.clone();
        let db = ctx.db.clone();
        let ck = ctx.default_collection_key.clone();
        let root = root.to_path_buf();

        tokio::task::spawn_blocking(move || {
            ingest_blocking(&root, source_id, &cas, &db, &ck, &progress)
        })
        .await
        .map_err(|e| Error::Ingest(format!("ingest join: {e}")))?
    }
}

fn scan_blocking(root: &Path) -> Result<ScanReport> {
    let mut count = 0u64;
    let mut bytes = 0u64;
    for entry in WalkDir::new(root).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        if !is_candidate(entry.path()) {
            continue;
        }
        if let Ok(md) = entry.metadata() {
            count += 1;
            bytes += md.len();
        }
    }
    Ok(ScanReport {
        file_count: count,
        total_bytes: bytes,
    })
}

fn ingest_blocking(
    root: &Path,
    source_id: i64,
    cas: &crate::cas::CasStore,
    db_mutex: &tokio::sync::Mutex<rusqlite::Connection>,
    collection_key: &crate::crypto::CollectionKey,
    progress: &Progress,
) -> Result<IngestReport> {
    progress.send(ProgressEvent::ScanStarted(root.to_path_buf()));
    let report = scan_blocking(root)?;
    let total = report.file_count;

    let mut report = IngestReport::default();
    let mut index = 0u64;

    for entry in WalkDir::new(root).into_iter().flatten() {
        if !entry.file_type().is_file() || !is_candidate(entry.path()) {
            continue;
        }
        let path = entry.path().to_path_buf();
        index += 1;
        progress.send(ProgressEvent::FileStarted {
            path: path.clone(),
            index,
            total,
        });

        match ingest_one(&path, source_id, cas, db_mutex, collection_key) {
            Ok(FileOutcome::Inserted { asset_id, bytes }) => {
                report.inserted += 1;
                progress.send(ProgressEvent::FileInserted { asset_id, bytes });
            }
            Ok(FileOutcome::Existing { asset_id }) => {
                report.deduped += 1;
                progress.send(ProgressEvent::FileDeduped { asset_id });
            }
            Ok(FileOutcome::Skipped(reason)) => {
                report.skipped += 1;
                progress.send(ProgressEvent::FileSkipped { path, reason });
            }
            Err(e) => {
                report.errors += 1;
                tracing::warn!(?e, path = ?path, "ingest error — skipping file");
                progress.send(ProgressEvent::FileSkipped {
                    path,
                    reason: format!("{e}"),
                });
            }
        }
    }

    progress.send(ProgressEvent::Done(report.clone()));
    Ok(report)
}

enum FileOutcome {
    Inserted {
        asset_id: i64,
        bytes: u64,
    },
    Existing {
        asset_id: i64,
    },
    #[allow(
        dead_code,
        reason = "reserved for non-ingestable files found during scan"
    )]
    Skipped(String),
}

fn ingest_one(
    path: &Path,
    source_id: i64,
    cas: &crate::cas::CasStore,
    db_mutex: &tokio::sync::Mutex<rusqlite::Connection>,
    collection_key: &crate::crypto::CollectionKey,
) -> Result<FileOutcome> {
    // 1. Probe for metadata up front so dedupe-by-content still produces a
    //    MediaProbe for provenance purposes.
    let probe = probe_path(path)?;

    // 2. Optional sidecar — merged into the encrypted EXIF payload.
    let sidecar = read_xmp_sidecar(path).ok().flatten();

    // 3. Compute plaintext hash + encrypt into CAS with a fresh file key.
    let fk = FileKey::random()?;
    let file = std::fs::File::open(path)?;
    let (cas_ref, bytes) = cas.put_streaming(std::io::BufReader::new(file), &fk)?;

    // 4. Wrap the file key under the owning collection key.
    let wrapped_fk = wrap_file_key(&fk, collection_key)?;

    // 5. Seal per-row plaintexts. row_id = 0 here because we don't know the
    //    final asset id; the AD is intentionally weak for bootstrap rows. On
    //    re-open we re-seal with the actual id in a maintenance pass (Phase 2).
    let hash: [u8; 32] = blake3::hash(&std::fs::read(path)?).as_bytes().to_owned();
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let filename_ct = seal_row(filename.as_bytes(), 0, collection_key.as_bytes())?;

    let merged_exif = merge_exif(&probe, sidecar.as_ref());
    let exif_all_ct = seal_row(&merged_exif, 0, collection_key.as_bytes())?;

    let taken_at_utc_ct = probe
        .taken_at_utc
        .map(|t| seal_row(t.to_rfc3339().as_bytes(), 0, collection_key.as_bytes()))
        .transpose()?;

    let gps_ct = probe
        .gps
        .map(|(la, lo, alt)| {
            let v = serde_json::json!({"lat": la, "lon": lo, "alt": alt});
            seal_row(v.to_string().as_bytes(), 0, collection_key.as_bytes())
        })
        .transpose()?;

    let device_ct = probe
        .device
        .as_ref()
        .map(|d| seal_row(d.as_bytes(), 0, collection_key.as_bytes()))
        .transpose()?;

    let lens_ct = probe
        .lens
        .as_ref()
        .map(|d| seal_row(d.as_bytes(), 0, collection_key.as_bytes()))
        .transpose()?;

    let original_path_ct = seal_row(
        path.to_string_lossy().as_bytes(),
        source_id as u64,
        collection_key.as_bytes(),
    )?;

    let imported_at = Utc::now().timestamp();
    let taken_at_day = probe.taken_at_utc_day();
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(imported_at);

    let asset_id = {
        let guard = db_mutex.blocking_lock();
        let insert = db::AssetInsert {
            blake3_plaintext: &hash,
            mime: &probe.mime,
            bytes: bytes as i64,
            width: probe.width.map(i64::from),
            height: probe.height.map(i64::from),
            duration_ms: probe.duration_ms.map(|d| d as i64),
            taken_at_utc_day: taken_at_day,
            is_video: probe.mime.starts_with("video/"),
            is_raw: probe.is_raw,
            is_screenshot: probe.is_screenshot,
            is_live: probe.is_live,
            is_motion: probe.is_motion,
            source_id,
            cas_ref: &cas_ref,
            imported_at,
            filename_ct: &filename_ct,
            taken_at_utc_ct: taken_at_utc_ct.as_deref(),
            gps_ct: gps_ct.as_deref(),
            device_ct: device_ct.as_deref(),
            lens_ct: lens_ct.as_deref(),
            exif_all_ct: Some(&exif_all_ct),
            wrapped_file_key: &wrapped_fk,
        };
        match db::insert_asset_if_new(&guard, &insert)? {
            db::InsertResult::Inserted(id) => {
                db::insert_asset_location(&guard, id, source_id, &original_path_ct, mtime)?;
                db::bump_source_stats(&guard, source_id, bytes as i64, 1)?;
                Outcome::Inserted(id)
            }
            db::InsertResult::Existing(id) => {
                db::insert_asset_location(&guard, id, source_id, &original_path_ct, mtime)?;
                Outcome::Existing(id)
            }
        }
    };

    // 6. Generate thumbnails for freshly inserted visual assets.
    //    Thumbs reuse the asset's FileKey: the derivative table's
    //    (asset_id, kind, cas_ref) tuple implicitly binds them, and a single
    //    `unwrap_file_key(wrapped_file_key, collection_key)` recovers the key
    //    for both the original and its derivatives.
    if let Outcome::Inserted(id) = asset_id {
        if let Ok(thumbs) = derive_thumbnails(path) {
            for t in thumbs {
                let (tref, _) = cas.put_streaming(std::io::Cursor::new(&t.bytes), &fk)?;
                let guard = db_mutex.blocking_lock();
                db::insert_derivative(&guard, id, t.size.as_derivative_kind(), &tref)?;
            }
        }
    }

    Ok(match asset_id {
        Outcome::Inserted(id) => FileOutcome::Inserted {
            asset_id: id,
            bytes,
        },
        Outcome::Existing(id) => FileOutcome::Existing { asset_id: id },
    })
}

enum Outcome {
    Inserted(i64),
    Existing(i64),
}

fn merge_exif(probe: &MediaProbe, sidecar: Option<&super::super::sidecar::XmpFields>) -> Vec<u8> {
    let mut obj = serde_json::Map::new();
    obj.insert("mime".into(), serde_json::Value::String(probe.mime.clone()));
    obj.insert("exif".into(), probe.exif_all_json.clone());
    if let Some(s) = sidecar {
        obj.insert(
            "sidecar".into(),
            serde_json::to_value(s).unwrap_or(serde_json::Value::Null),
        );
    }
    serde_json::to_vec(&serde_json::Value::Object(obj)).unwrap_or_default()
}

fn is_candidate(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    let hidden = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false);
    if hidden {
        return false;
    }
    matches!(
        ext.as_deref(),
        Some(
            "jpg"
                | "jpeg"
                | "png"
                | "heic"
                | "heif"
                | "webp"
                | "bmp"
                | "gif"
                | "tif"
                | "tiff"
                | "mp4"
                | "mov"
                | "m4v"
                | "mkv"
                | "webm"
                | "cr2"
                | "cr3"
                | "nef"
                | "arw"
                | "dng"
                | "raf"
                | "orf"
                | "rw2"
        )
    )
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::CasStore;
    use crate::crypto::keystore;
    use crate::crypto::CollectionKey;
    use crate::db;
    use secrecy::SecretString;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    type Vault = (
        TempDir,
        Arc<CasStore>,
        Arc<Mutex<rusqlite::Connection>>,
        i64,
        i64,
        Arc<CollectionKey>,
        Arc<keystore::UnlockedUser>,
    );

    fn setup_vault() -> Vault {
        let dir = TempDir::new().unwrap();
        let cas = Arc::new(CasStore::open(dir.path()).unwrap());
        let conn = rusqlite::Connection::open(dir.path().join("index.db")).unwrap();
        db::migrate::apply(&conn).unwrap();
        let (record, unlocked) =
            keystore::create_user("u", &SecretString::from("very-long-password-xyz")).unwrap();
        let user_id = db::insert_user(&conn, &record, 0).unwrap();
        let mut unlocked = unlocked;
        unlocked.user_id = user_id;
        let user = Arc::new(unlocked);

        let ck = Arc::new(CollectionKey::random().unwrap());
        let source_id = db::insert_source(&conn, user_id, b"s", b"root", None, false, 0).unwrap();
        (
            dir,
            cas,
            Arc::new(Mutex::new(conn)),
            user_id,
            source_id,
            ck,
            user,
        )
    }

    fn write_jpeg(p: &Path, seed: u8) {
        let img: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> =
            image::ImageBuffer::from_fn(16, 12, |x, y| {
                image::Rgb([(x as u8).wrapping_add(seed), y as u8, 0])
            });
        img.save_with_format(p, image::ImageFormat::Jpeg).unwrap();
    }

    #[tokio::test]
    async fn scan_counts_candidate_files() {
        let root = TempDir::new().unwrap();
        let a = root.path().join("a.jpg");
        let b = root.path().join("b.png");
        let c = root.path().join("ignored.txt");
        write_jpeg(&a, 1);
        image::RgbImage::new(8, 8)
            .save_with_format(&b, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&c, "hello").unwrap();
        let adapter = GenericAdapter::new();
        let r = adapter.scan(root.path()).await.unwrap();
        assert_eq!(r.file_count, 2);
        assert!(r.total_bytes > 0);
    }

    #[tokio::test]
    async fn ingest_inserts_and_dedupes() {
        let root = TempDir::new().unwrap();
        let a = root.path().join("a.jpg");
        let b = root.path().join("b.jpg"); // will be same plaintext as a → dedupe
        write_jpeg(&a, 1);
        std::fs::copy(&a, &b).unwrap();
        let c = root.path().join("c.jpg");
        write_jpeg(&c, 77);

        let (_d, cas, conn, _uid, sid, ck, user) = setup_vault();
        let ctx = IngestCtx {
            cas: cas.clone(),
            db: conn.clone(),
            user,
            default_collection_key: ck,
        };

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let progress = Progress(tx);
        let report = GenericAdapter::new()
            .ingest(&ctx, root.path(), sid, progress)
            .await
            .unwrap();

        assert_eq!(report.inserted, 2, "two distinct plaintexts");
        assert_eq!(report.deduped, 1, "copy should dedupe");
        assert_eq!(report.errors, 0);

        // Drain progress events for sanity; last event is Done.
        let mut last = None;
        while let Ok(ev) = rx.try_recv() {
            last = Some(ev);
        }
        assert!(matches!(last, Some(ProgressEvent::Done(_))));

        // Exactly 2 asset rows.
        let guard = conn.lock().await;
        let n: i64 = guard
            .query_row("SELECT COUNT(*) FROM asset", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
        // 3 location rows (two distinct + one dedupe hit).
        let loc: i64 = guard
            .query_row("SELECT COUNT(*) FROM asset_location", [], |r| r.get(0))
            .unwrap();
        assert_eq!(loc, 3);
        // 4 thumbnail derivatives (2 inserted assets × 2 sizes each).
        let d: i64 = guard
            .query_row("SELECT COUNT(*) FROM derivative", [], |r| r.get(0))
            .unwrap();
        assert_eq!(d, 4);
    }

    #[tokio::test]
    async fn reingest_of_same_root_is_full_dedupe() {
        let root = TempDir::new().unwrap();
        write_jpeg(&root.path().join("a.jpg"), 1);
        write_jpeg(&root.path().join("b.jpg"), 2);

        let (_d, cas, conn, _uid, sid, ck, user) = setup_vault();
        let ctx = IngestCtx {
            cas: cas.clone(),
            db: conn.clone(),
            user,
            default_collection_key: ck,
        };

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let progress = Progress(tx);
        let r1 = GenericAdapter::new()
            .ingest(&ctx, root.path(), sid, progress.clone())
            .await
            .unwrap();
        assert_eq!(r1.inserted, 2);
        let r2 = GenericAdapter::new()
            .ingest(&ctx, root.path(), sid, progress)
            .await
            .unwrap();
        assert_eq!(r2.inserted, 0);
        assert_eq!(r2.deduped, 2);
    }

    #[test]
    fn is_candidate_classifies_common_types() {
        assert!(is_candidate(Path::new("a.jpg")));
        assert!(is_candidate(Path::new("a.HEIC")));
        assert!(is_candidate(Path::new("img.CR2")));
        assert!(!is_candidate(Path::new("a.txt")));
        assert!(!is_candidate(Path::new(".hidden.jpg")));
    }
}
