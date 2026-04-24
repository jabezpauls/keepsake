//! Tier-C harness for Phase-2 acceptance criterion 2 (CLIP top-5 ≥ 80%).
//!
//! Reads a labelled CSV (`asset_id,caption`) and, for each caption, runs a
//! CLIP text search against an already-populated Media Vault. Reports the
//! percentage of captions whose ground-truth asset appears in the top-5.
//!
//! Not in CI. Run once you have a labelled dataset + real weights loaded:
//!
//! ```bash
//! export MV_MODELS=~/mv-models
//! export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
//! cargo run --release --features ml-models --example clip_topk -- \
//!   --vault ~/.local/share/media-vault \
//!   --labels ./fixtures/clip_queries.csv
//! ```
//!
//! Output line to record in `plans/phase-2-browsing.md`:
//! `CLIP top-5 accuracy: NN% (N/N)`.

#[cfg(not(feature = "ml-models"))]
fn main() {
    eprintln!("rebuild with --features ml-models to run clip_topk");
}

#[cfg(feature = "ml-models")]
fn main() {
    inner::run();
}

#[cfg(feature = "ml-models")]
mod inner {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use mv_core::db::schema;
    use mv_core::ml::{ExecutionProvider, MlConfig, MlRuntime};
    use mv_core::search::{search, SearchQuery};

    fn parse_args() -> (PathBuf, PathBuf) {
        let mut args = std::env::args().skip(1);
        let mut vault = None;
        let mut labels = None;
        while let Some(a) = args.next() {
            match a.as_str() {
                "--vault" => vault = args.next().map(PathBuf::from),
                "--labels" => labels = args.next().map(PathBuf::from),
                other => {
                    eprintln!("unknown arg: {other}");
                    std::process::exit(2);
                }
            }
        }
        (
            vault.expect("--vault <path> required"),
            labels.expect("--labels <path/to/labels.csv> required"),
        )
    }

    fn read_labels(path: &Path) -> Vec<(i64, String)> {
        let text = std::fs::read_to_string(path).expect("read labels csv");
        text.lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .filter_map(|l| {
                let mut parts = l.splitn(2, ',');
                let id: i64 = parts.next()?.trim().parse().ok()?;
                let caption = parts.next()?.trim().trim_matches('"').to_string();
                Some((id, caption))
            })
            .collect()
    }

    pub fn run() {
        let (vault, labels_path) = parse_args();
        let labels = read_labels(&labels_path);
        assert!(!labels.is_empty(), "labels CSV was empty");

        let conn = schema::open(&vault.join("index.db")).expect("open vault db");

        let models_dir =
            std::env::var_os("MV_MODELS").map_or_else(|| vault.join("models"), PathBuf::from);
        let rt = Arc::new(
            MlRuntime::load(MlConfig {
                model_dir: models_dir,
                execution_provider: ExecutionProvider::Auto,
                bundle: mv_core::ml::bundles::BundleId::Full,
            })
            .expect("load runtime"),
        );

        let mut hits = 0usize;
        for (ground_truth, caption) in &labels {
            let q = SearchQuery {
                text: Some(caption.clone()),
                limit: 5,
                ..Default::default()
            };
            let hits_vec = search(&conn, &q, None, Some(&rt)).expect("search");
            let top5: Vec<i64> = hits_vec.iter().take(5).map(|h| h.asset_id).collect();
            if top5.contains(ground_truth) {
                hits += 1;
            } else {
                eprintln!("miss: gt={ground_truth} caption={caption:?} top5={top5:?}");
            }
        }
        let pct = (hits as f64 / labels.len() as f64) * 100.0;
        println!(
            "CLIP top-5 accuracy: {:.1}% ({}/{})",
            pct,
            hits,
            labels.len()
        );
    }
}
