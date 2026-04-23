//! Bulk-asset generator for perf-bucket integration tests.
//!
//! Inserts N asset rows with randomised plaintext metadata (no disk files,
//! no CAS blobs). Fast enough for 500k-row SLO scenarios — the entire 500k
//! insert runs in ~1 s wrapped in a single transaction.
//!
//! Intentionally NOT linked into `support::Vault::create` — perf tests that
//! need it opt-in via `bulk::generate_synthetic_library(&vault, count, seed)`.

use mv_core::db::{self, AssetInsert};

/// Tiny xorshift-32 RNG. Deterministic given the seed, zero external deps,
/// good-enough distribution for plaintext-only synthetic data.
#[derive(Clone, Copy)]
struct XorShift32(u32);

impl XorShift32 {
    fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0xa5a5_a5a5 } else { seed })
    }
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
}

/// Bulk-insert `count` synthetic asset rows into `conn` via one transaction.
/// Returns the freshly-inserted asset ids in order.
///
/// Distribution:
/// - `taken_at_utc_day` uniform over the past ~3 years (days 18000..19200).
/// - `is_video` ~5%, `is_raw` ~3%, `is_screenshot` ~8%, `is_live` ~4%.
/// - `source_id` is always the provided one.
/// - `blake3_plaintext` is a seeded hash so rows are unique.
///
/// Callers should seed the same `seed` for reproducible test output.
pub fn generate_synthetic_library(
    conn: &rusqlite::Connection,
    source_id: i64,
    count: u32,
    seed: u32,
) -> Vec<i64> {
    let mut rng = XorShift32::new(seed);
    let mut ids = Vec::with_capacity(count as usize);
    let tx = conn.unchecked_transaction().expect("begin tx");
    for i in 0..count {
        let mut hash = [0u8; 32];
        let h = (seed as u64).wrapping_mul(i as u64 + 1);
        hash[..8].copy_from_slice(&h.to_le_bytes());
        hash[8..16].copy_from_slice(&(rng.next() as u64 + rng.next() as u64 * (u32::MAX as u64 + 1)).to_le_bytes());

        let r = rng.next();
        let day = 18000 + (r % 1200) as i64;
        let is_video = rng.next() % 20 == 0;
        let is_raw = rng.next() % 33 == 0;
        let is_screenshot = rng.next() % 12 == 0;
        let is_live = rng.next() % 25 == 0;

        let a = AssetInsert {
            blake3_plaintext: &hash,
            mime: if is_video { "video/mp4" } else { "image/jpeg" },
            bytes: 1024,
            width: Some(1920),
            height: Some(1080),
            duration_ms: None,
            taken_at_utc_day: Some(day),
            is_video,
            is_raw,
            is_screenshot,
            is_live,
            is_motion: false,
            source_id,
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
        match db::insert_asset_if_new(conn, &a).expect("insert") {
            db::InsertResult::Inserted(id) | db::InsertResult::Existing(id) => ids.push(id),
        }
    }
    tx.commit().expect("commit");
    ids
}
