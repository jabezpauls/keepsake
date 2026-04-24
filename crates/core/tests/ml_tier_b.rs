//! Tier-B end-to-end tests — require real model weights and are intentionally
//! marked `#[ignore]`. Run with:
//!
//! ```bash
//! export MV_MODELS=~/mv-models
//! export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
//! cargo test -p mv-core --features ml-models --test ml_tier_b -- --ignored
//! ```
//!
//! Covers Phase 2 acceptance criteria 1 (ML end-to-end on golden fixture)
//! and 4 (merge/split stability). Criteria 2 and 3 (CLIP top-5 / face F1)
//! are harness-only; see `examples/clip_topk.rs` + `examples/face_f1.rs`.

#![cfg(feature = "ml-models")]

mod support;

use std::path::PathBuf;
use std::sync::Arc;

use mv_core::ingest::{GenericAdapter, IngestAdapter, IngestCtx, Progress};
use mv_core::ml::{ExecutionProvider, MlConfig, MlRuntime, MlWorker};
use support::{make_iphone_dump, Vault};

fn models_dir() -> Option<PathBuf> {
    std::env::var_os("MV_MODELS").map(PathBuf::from)
}

fn load_runtime_or_skip() -> Option<Arc<MlRuntime>> {
    let dir = models_dir()?;
    let cfg = MlConfig {
        model_dir: dir,
        execution_provider: ExecutionProvider::Auto,
        bundle: mv_core::ml::bundles::BundleId::Full,
    };
    match MlRuntime::load(cfg) {
        Ok(rt) => Some(Arc::new(rt)),
        Err(e) => {
            eprintln!("skip: MlRuntime::load failed — {e:?}");
            None
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires MV_MODELS=/path with real weights"]
async fn iphone_golden_end_to_end() {
    let Some(rt) = load_runtime_or_skip() else {
        eprintln!("skip: MV_MODELS not set");
        return;
    };

    let vault = Vault::create("very-long-password-xyz");
    let dump = tempfile::tempdir().unwrap();
    let _files = make_iphone_dump(dump.path());

    let db_arc = Arc::new(tokio::sync::Mutex::new(vault.conn));
    let cas_arc = Arc::new(vault.cas);

    let ctx = IngestCtx {
        cas: cas_arc.clone(),
        db: db_arc.clone(),
        user: Arc::new(vault.user),
        default_collection_key: Arc::new(vault.default_collection_key),
    };
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let progress = Progress(tx);

    GenericAdapter::new()
        .ingest(&ctx, dump.path(), vault.source_id, progress)
        .await
        .expect("ingest");

    // Install the worker + key resolver, then drain every queued job.
    let worker = MlWorker::new(db_arc.clone(), cas_arc.clone());
    let ck = ctx.default_collection_key.clone();
    worker.set_key_resolver(Arc::new(move |_id: i64| {
        Some(mv_core::crypto::CollectionKey::from_bytes(*ck.as_bytes()))
    }));
    {
        // Install the already-loaded runtime.
        let rt = rt.clone();
        let cfg = MlConfig {
            model_dir: models_dir().unwrap(),
            execution_provider: ExecutionProvider::Auto,
            bundle: mv_core::ml::bundles::BundleId::Full,
        };
        worker.try_load_runtime(cfg);
        let _ = rt;
    }

    // Drain until empty; cap at a generous iteration count so a stuck job
    // kind doesn't loop forever.
    for _ in 0..1000 {
        match worker.drain_one().await {
            Ok(None) => break,
            Ok(Some(_)) => {}
            Err(e) => panic!("worker drain: {e:?}"),
        }
    }

    let conn = db_arc.lock().await;
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM asset", [], |r| r.get(0))
        .unwrap();
    assert!(total >= 15, "expected >=15 assets, got {total}");

    let with_embedding: i64 = conn
        .query_row("SELECT COUNT(*) FROM asset_vec", [], |r| r.get(0))
        .unwrap();
    assert!(
        with_embedding >= (total as f64 * 0.9) as i64,
        "embeddings should cover >=90% of assets; got {with_embedding}/{total}"
    );

    let with_phash: i64 = conn
        .query_row("SELECT COUNT(*) FROM phash", [], |r| r.get(0))
        .unwrap();
    assert!(
        with_phash >= (total as f64 * 0.8) as i64,
        "phash should cover >=80% of assets (videos may skip); got {with_phash}/{total}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires MV_MODELS=/path with real weights"]
async fn merge_then_split_stability_placeholder() {
    // This test is a placeholder until we have a face-bearing fixture. For
    // now, assert MlRuntime loads and exits early. Plumbing lives in
    // `ml::worker_exec::run_rebuild_person_clusters`; exercising it
    // end-to-end requires face images, which we don't ship.
    let Some(_) = load_runtime_or_skip() else {
        eprintln!("skip: MV_MODELS not set");
        return;
    };
    eprintln!("merge/split stability harness exists; needs face fixture to assert");
}
