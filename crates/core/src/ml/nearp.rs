//! Near-duplicate cluster rebuild pipeline.
//!
//! Reads every `(asset_id, phash)` row, clusters via LSH + union-find
//! ([`super::phash::cluster`]), picks a best-shot via sharpness on the 256px
//! thumbnail, and persists results to `nd_cluster`.
//!
//! Runs synchronously on the caller thread — callers that want background
//! execution wrap it in `tokio::task::spawn_blocking`. Rebuilding the whole
//! cluster table at once keeps state consistent; incremental updates are a
//! Phase-3 concern.

use std::collections::HashMap;

use crate::cas::CasStore;
use crate::crypto::{unwrap_file_key, CollectionKey};
use crate::db::{self, NdClusterMember};
use crate::ml::phash;
use crate::Result;

/// Default Hamming threshold for dhash near-dup detection. 6 bits out of 64
/// keeps false-positives near zero for most photo sets (see
/// `plans/phase-2-browsing.md` "Known decisions").
pub const DEFAULT_THRESHOLD: u32 = 6;

/// Rebuild `nd_cluster` from scratch. `ck_for_asset` lets the caller surface
/// the right collection key (default for plain assets, album key for password
/// albums) — we need it to unwrap each asset's FileKey so we can decrypt the
/// 256px thumbnail for sharpness scoring.
pub fn rebuild(
    conn: &rusqlite::Connection,
    cas: &CasStore,
    ck_for_asset: impl Fn(i64) -> Option<CollectionKey>,
    threshold: u32,
) -> Result<usize> {
    let phashes = db::list_phashes(conn)?;
    if phashes.len() < 2 {
        db::replace_nd_clusters(conn, &[])?;
        return Ok(0);
    }
    let clusters = phash::cluster(&phashes, threshold);

    let mut members: Vec<NdClusterMember> = Vec::new();
    for (cluster_id, asset_ids) in clusters.iter().enumerate() {
        // Sharpness-based best-shot pick. Errors reading any single asset
        // don't abort the cluster — the best-shot just falls back to the
        // first member.
        let mut best_id = asset_ids[0];
        let mut best_score: f64 = f64::NEG_INFINITY;
        for &aid in asset_ids {
            if let Some(score) = score_asset(conn, cas, aid, &ck_for_asset) {
                if score > best_score {
                    best_score = score;
                    best_id = aid;
                }
            }
        }
        for &aid in asset_ids {
            members.push(NdClusterMember {
                cluster_id: cluster_id as i64,
                asset_id: aid,
                is_best: aid == best_id,
            });
        }
    }
    db::replace_nd_clusters(conn, &members)?;
    Ok(clusters.len())
}

fn score_asset(
    conn: &rusqlite::Connection,
    cas: &CasStore,
    asset_id: i64,
    ck_for_asset: &impl Fn(i64) -> Option<CollectionKey>,
) -> Option<f64> {
    let ck = ck_for_asset(asset_id)?;
    let thumb_ref = db::get_derivative(conn, asset_id, "thumb256")
        .ok()
        .flatten()?;
    let asset = db::get_asset(conn, asset_id).ok().flatten()?;
    let fk = unwrap_file_key(&asset.wrapped_file_key, &ck).ok()?;
    let bytes = cas.get(&thumb_ref, &fk).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    Some(phash::sharpness(&img))
}

/// Build the cluster-id → `Vec<member>` index the UI wants.
pub fn clusters_by_id(conn: &rusqlite::Connection) -> Result<HashMap<i64, Vec<NdClusterMember>>> {
    let mut out: HashMap<i64, Vec<NdClusterMember>> = HashMap::new();
    for m in db::list_nd_clusters(conn)? {
        out.entry(m.cluster_id).or_default().push(m);
    }
    Ok(out)
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::CasStore;
    use crate::crypto::{seal_row, CollectionKey, FileKey};
    use crate::db::{self, AssetInsert};
    use image::{ImageBuffer, Rgb};
    use tempfile::TempDir;

    fn make_asset(
        conn: &rusqlite::Connection,
        cas: &CasStore,
        sid: i64,
        ck: &CollectionKey,
        pixels: impl Fn(u32, u32) -> [u8; 3],
        phash_override: u64,
    ) -> i64 {
        let mut hash = [0u8; 32];
        hash[0] = phash_override as u8;
        hash[1] = (phash_override >> 8) as u8;
        hash[2] = (phash_override >> 16) as u8;
        hash[3] = (phash_override >> 24) as u8;
        let fk = FileKey::random().unwrap();
        let wrapped = crate::crypto::wrap_file_key(&fk, ck).unwrap();
        // Thumb bytes = deterministic 64×64 WebP.
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(64, 64, |x, y| Rgb(pixels(x, y)));
        let mut thumb = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut thumb),
                image::ImageFormat::WebP,
            )
            .unwrap();
        let (tref, _) = cas
            .put_streaming(std::io::Cursor::new(&thumb), &fk)
            .unwrap();

        let filename_ct = seal_row(b"test.jpg", 0, ck.as_bytes()).unwrap();
        let a = AssetInsert {
            blake3_plaintext: &hash,
            mime: "image/jpeg",
            bytes: 0,
            width: Some(64),
            height: Some(64),
            duration_ms: None,
            taken_at_utc_day: None,
            is_video: false,
            is_raw: false,
            is_screenshot: false,
            is_live: false,
            is_motion: false,
            source_id: sid,
            cas_ref: &tref,
            imported_at: 0,
            filename_ct: &filename_ct,
            taken_at_utc_ct: None,
            gps_ct: None,
            device_ct: None,
            lens_ct: None,
            exif_all_ct: None,
            wrapped_file_key: &wrapped,
        };
        let aid = match db::insert_asset_if_new(conn, &a).unwrap() {
            db::InsertResult::Inserted(x) | db::InsertResult::Existing(x) => x,
        };
        db::insert_derivative(conn, aid, "thumb256", &tref).unwrap();
        db::upsert_phash(conn, aid, phash_override).unwrap();
        aid
    }

    #[test]
    fn rebuild_groups_near_duplicates_and_writes_nd_cluster() {
        let tmp = TempDir::new().unwrap();
        let cas = CasStore::open(tmp.path()).unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate::apply(&conn).unwrap();
        use crate::crypto::keystore;
        use secrecy::SecretString;
        let (record, _) =
            keystore::create_user("u", &SecretString::from("very-long-password-xyz")).unwrap();
        let uid = db::insert_user(&conn, &record, 0).unwrap();
        let sid = db::insert_source(&conn, uid, b"s", b"r", None, false, 0).unwrap();
        let ck = CollectionKey::random().unwrap();

        // Two near-dup pairs + one singleton.
        let base_a = 0x00FF_00FF_00FF_00FFu64;
        let base_b = 0xDEAD_BEEF_CAFE_BABEu64;
        let outlier = 0x1234_5678_9ABC_DEF0u64;
        let a1 = make_asset(&conn, &cas, sid, &ck, |x, y| [x as u8, y as u8, 0], base_a);
        let a2 = make_asset(
            &conn,
            &cas,
            sid,
            &ck,
            |x, y| [x as u8, y as u8, 1],
            base_a ^ 0b11,
        );
        let b1 = make_asset(
            &conn,
            &cas,
            sid,
            &ck,
            |x, y| [(x * 4) as u8, y as u8, 0],
            base_b,
        );
        let b2 = make_asset(
            &conn,
            &cas,
            sid,
            &ck,
            |x, y| [(x * 4) as u8, y as u8, 5],
            base_b ^ 0b110,
        );
        let _single = make_asset(&conn, &cas, sid, &ck, |_, _| [50, 50, 50], outlier);

        let ck_bytes = *ck.as_bytes();
        let n = rebuild(
            &conn,
            &cas,
            move |_| Some(CollectionKey::from_bytes(ck_bytes)),
            6,
        )
        .unwrap();
        assert_eq!(n, 2);
        let groups = clusters_by_id(&conn).unwrap();
        assert_eq!(groups.len(), 2);

        let mut sizes: Vec<_> = groups.values().map(|g| g.len()).collect();
        sizes.sort();
        assert_eq!(sizes, vec![2, 2]);

        // Each group has exactly one is_best.
        for group in groups.values() {
            let best = group.iter().filter(|m| m.is_best).count();
            assert_eq!(best, 1);
        }
        // a1/a2 should be in the same group.
        let a_group = groups
            .values()
            .find(|g| g.iter().any(|m| m.asset_id == a1))
            .unwrap();
        assert!(a_group.iter().any(|m| m.asset_id == a2));

        // Rebuilding again is stable.
        let n2 = rebuild(
            &conn,
            &cas,
            move |_| Some(CollectionKey::from_bytes(ck_bytes)),
            6,
        )
        .unwrap();
        assert_eq!(n2, 2);
        let groups2 = clusters_by_id(&conn).unwrap();
        assert_eq!(groups2.len(), 2);
        let _ = b1;
        let _ = b2;
    }

    #[test]
    fn rebuild_with_fewer_than_two_phashes_is_noop() {
        let tmp = TempDir::new().unwrap();
        let cas = CasStore::open(tmp.path()).unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate::apply(&conn).unwrap();
        let n = rebuild(&conn, &cas, |_| None, 6).unwrap();
        assert_eq!(n, 0);
        assert!(db::list_nd_clusters(&conn).unwrap().is_empty());
    }
}
