//! End-to-end ingest-flow test against a synthetic iPhone dump.
//!
//! Covers phase-1-foundation.md §9 Acceptance #5: after ingesting a synthetic
//! iPhone dump the vault holds the expected asset count, pair detection has
//! fired, and re-ingest of the same dir is a full dedupe.

mod support;

use std::sync::Arc;

use mv_core::ingest::{GenericAdapter, IngestAdapter, IngestCtx, Progress};
use mv_core::media::detect_pairs;
use support::{make_iphone_dump, Vault};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn iphone_dump_round_trip_and_dedupe() {
    let vault = Vault::create("very-long-password-xyz");
    let dump = tempfile::tempdir().unwrap();
    let files = make_iphone_dump(dump.path());
    assert!(files.len() >= 20);

    // Pair detection is a pure file-list pass; we assert on its shape so any
    // regression in the heuristic surfaces independently of ingest.
    let report = detect_pairs(&files);
    assert_eq!(report.live.len(), 3, "three HEIC/MOV live pairs");
    assert_eq!(report.bursts.len(), 2, "two burst groups");

    let ctx = IngestCtx {
        cas: Arc::new(vault.cas),
        db: Arc::new(tokio::sync::Mutex::new(vault.conn)),
        user: Arc::new(vault.user),
        default_collection_key: Arc::new(vault.default_collection_key),
    };
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let progress = Progress(tx);

    let report = GenericAdapter::new()
        .ingest(&ctx, dump.path(), vault.source_id, progress.clone())
        .await
        .unwrap();

    // Every file is ingested (dummy .MOV / .HEIC files still get an asset row
    // via MIME-by-extension + empty probe).
    assert!(report.inserted >= 18, "inserted={}", report.inserted);
    assert_eq!(report.errors, 0);

    // Re-ingest must be a pure-dedupe pass.
    let report2 = GenericAdapter::new()
        .ingest(&ctx, dump.path(), vault.source_id, progress)
        .await
        .unwrap();
    assert_eq!(report2.inserted, 0, "re-ingest must not insert new assets");
    assert!(report2.deduped >= 18, "deduped={}", report2.deduped);

    // Sanity-check the DB state.
    let db = ctx.db.lock().await;
    let asset_count: i64 = db
        .query_row("SELECT COUNT(*) FROM asset", [], |r| r.get(0))
        .unwrap();
    assert_eq!(asset_count as u64, report.inserted);

    // Every ingest pass appends a provenance row per (asset, source, path).
    // Because the path ciphertext changes each pass (fresh nonce), Phase 1
    // doesn't deduplicate at the location level — two passes yield ~2×
    // rows. Location-level dedupe is a Phase-2 migration (adds a plaintext
    // path_hash to key on). We only assert a *lower bound* here so later
    // phases can tighten without regressing this test.
    let loc_count: i64 = db
        .query_row("SELECT COUNT(*) FROM asset_location", [], |r| r.get(0))
        .unwrap();
    assert!(loc_count >= asset_count);
}
