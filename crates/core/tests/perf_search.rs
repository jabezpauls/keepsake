//! Phase-2 acceptance criterion 6: 50k-asset metadata-filter + asset_vec
//! lookup cold ≤ 200 ms, warm ≤ 50 ms.
//!
//! This test measures the SQL-layer search hot path without a live
//! `MlRuntime` (runtime = None, so `maybe_clip_rerank` is a no-op). The
//! full CLIP-re-rank SLO lives under Tier-B in `tests/ml_tier_b.rs` since
//! it requires model weights.
//!
//! Run with:
//! ```bash
//! cargo test -p mv-core --test perf_search -- --ignored perf_search
//! ```

mod support;

use std::time::Instant;

use mv_core::db;
use mv_core::search::{search, SearchQuery};
use support::{bulk, Vault};

const TARGET_ASSETS: u32 = 50_000;
const COLD_SLO_MS: u128 = 200;
const WARM_SLO_MS: u128 = 50;
const LIMIT: u32 = 50;

#[test]
#[ignore = "perf: 50k-asset metadata-search SLO"]
fn perf_search_50k_cold_warm_slos() {
    let vault = Vault::create("very-long-password-xyz");
    let t_seed = Instant::now();
    let ids = bulk::generate_synthetic_library(
        &vault.conn,
        vault.source_id,
        TARGET_ASSETS,
        0x5EA_5EA,
    );
    // Populate asset_vec for half the library so the runtime-less search
    // still has rows to count against `list_asset_vecs` during re-rank
    // (the re-rank path is no-op here, but the upsert happens at ingest
    // time in production and we want the schema exercised).
    for (i, id) in ids.iter().enumerate().take((TARGET_ASSETS / 2) as usize) {
        let mut v = vec![0f32; 768];
        v[i % 768] = 1.0;
        db::upsert_asset_vec(&vault.conn, *id, &v).expect("upsert_asset_vec");
    }
    eprintln!(
        "seeded {TARGET_ASSETS} assets + {} asset_vec rows in {:?}",
        TARGET_ASSETS / 2,
        t_seed.elapsed()
    );

    // Cold: fresh connection to dodge sqlite's page cache. Keep the
    // TempDir alive (via _tmp) through the reopen — only the Connection
    // gets dropped.
    let db_path = vault.tmp.path().join("index.db");
    let vault_ck = mv_core::crypto::CollectionKey::from_bytes(*vault.default_collection_key.as_bytes());
    let Vault { tmp, conn, .. } = vault;
    let _tmp = tmp;
    drop(conn);
    let conn = db::schema::open(&db_path).expect("reopen");

    let q = SearchQuery {
        limit: LIMIT,
        ..Default::default()
    };

    let t_cold = Instant::now();
    let cold_hits = search(&conn, &q, Some(&vault_ck), None).expect("search cold");
    let cold = t_cold.elapsed();
    eprintln!(
        "cold top-{LIMIT} in {:?} (SLO {COLD_SLO_MS} ms), {} hits",
        cold,
        cold_hits.len()
    );
    assert!(!cold_hits.is_empty());
    assert!(
        cold.as_millis() <= COLD_SLO_MS,
        "cold search latency {} ms over SLO {} ms",
        cold.as_millis(),
        COLD_SLO_MS
    );

    // Warm: same connection, same query — should hit page cache.
    let t_warm = Instant::now();
    let _ = search(&conn, &q, Some(&vault_ck), None).expect("search warm");
    let warm = t_warm.elapsed();
    eprintln!("warm top-{LIMIT} in {:?} (SLO {WARM_SLO_MS} ms)", warm);
    assert!(
        warm.as_millis() <= WARM_SLO_MS,
        "warm search latency {} ms over SLO {} ms",
        warm.as_millis(),
        WARM_SLO_MS
    );
}
