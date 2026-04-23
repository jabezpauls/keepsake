//! Phase-2 acceptance criterion 5: 500k-asset synthetic library, first
//! timeline page ≤ 200 ms cold (single-threaded). Ignored by default —
//! compile cost and 500k-row insert make this unsuitable for CI's baseline
//! `cargo test` run. Execute with:
//!
//! ```bash
//! cargo test -p mv-core --test perf_timeline -- --ignored perf_timeline
//! ```
//!
//! The 60 fps scroll SLO is covered in the Tauri shell via Playwright
//! (separate harness) — this test only validates the SQL-layer page query.

mod support;

use std::time::Instant;

use mv_core::db;
use support::{bulk, Vault};

const TARGET_ASSETS: u32 = 500_000;
const COLD_FIRST_PAGE_SLO_MS: u128 = 200;
const PAGE_LIMIT: u32 = 120;

#[test]
#[ignore = "perf: 500k-asset cold timeline SLO"]
fn perf_timeline_500k_cold_first_page_under_200ms() {
    let vault = Vault::create("very-long-password-xyz");
    let t_seed = Instant::now();
    bulk::generate_synthetic_library(&vault.conn, vault.source_id, TARGET_ASSETS, 0xC0FFEE);
    eprintln!("seeded {TARGET_ASSETS} assets in {:?}", t_seed.elapsed());

    // `sqlite`'s page cache benefits from warmup — so we *close* and reopen
    // the connection to measure a genuine cold hit. We keep the TempDir
    // alive (via _tmp) so the on-disk file survives the reopen; only the
    // Connection handle gets dropped.
    let db_path = vault.tmp.path().join("index.db");
    let Vault { tmp, conn, .. } = vault;
    let _tmp = tmp; // keep the tempdir alive through the reopen
    drop(conn);
    let conn = db::schema::open(&db_path).expect("reopen");

    let t_query = Instant::now();
    let page = db::list_timeline_page(&conn, i64::MAX, i64::MAX, PAGE_LIMIT, false)
        .expect("list_timeline_page");
    let cold = t_query.elapsed();
    eprintln!(
        "cold first-page ({} rows) in {:?} (SLO {} ms)",
        page.len(),
        cold,
        COLD_FIRST_PAGE_SLO_MS
    );
    assert_eq!(page.len() as u32, PAGE_LIMIT);
    assert!(
        cold.as_millis() <= COLD_FIRST_PAGE_SLO_MS,
        "cold first-page latency {} ms over SLO {} ms",
        cold.as_millis(),
        COLD_FIRST_PAGE_SLO_MS
    );
}
