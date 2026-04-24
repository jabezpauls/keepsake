//! Tier-C harness for Phase-2 acceptance criterion 3 (face pairwise-F1 ≥
//! 0.9). Reads a labelled CSV (`image_path,person_label`), detects faces,
//! embeds each, runs DBSCAN, and reports the pairwise F1 between the
//! produced clusters and the ground-truth person labels.
//!
//! Not in CI. Run once you have a labelled dataset + real weights loaded:
//!
//! ```bash
//! export MV_MODELS=~/mv-models
//! export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
//! cargo run --release --features ml-models --example face_f1 -- \
//!   --labels ./fixtures/face_identities.csv
//! ```
//!
//! CSV schema: `path/to/image.jpg,person_name`. One face per image expected;
//! multi-face images take the highest-score detection.

#[cfg(not(feature = "ml-models"))]
fn main() {
    eprintln!("rebuild with --features ml-models to run face_f1");
}

#[cfg(feature = "ml-models")]
fn main() {
    inner::run();
}

#[cfg(feature = "ml-models")]
mod inner {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use mv_core::ml::faces::{self, dbscan_cosine};
    use mv_core::ml::{ExecutionProvider, MlConfig, MlRuntime};

    fn parse_args() -> PathBuf {
        let mut args = std::env::args().skip(1);
        let mut labels = None;
        while let Some(a) = args.next() {
            match a.as_str() {
                "--labels" => labels = args.next().map(PathBuf::from),
                other => {
                    eprintln!("unknown arg: {other}");
                    std::process::exit(2);
                }
            }
        }
        labels.expect("--labels <path/to/face_labels.csv> required")
    }

    fn read_labels(path: &Path) -> Vec<(PathBuf, String)> {
        let text = std::fs::read_to_string(path).expect("read labels csv");
        text.lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .filter_map(|l| {
                let mut parts = l.splitn(2, ',');
                let p = parts.next()?.trim();
                let label = parts.next()?.trim().trim_matches('"').to_string();
                Some((PathBuf::from(p), label))
            })
            .collect()
    }

    fn pairwise_counts(labels: &[String], clusters: &[i32]) -> (u64, u64, u64) {
        let n = labels.len();
        let mut tp = 0u64;
        let mut fp = 0u64;
        let mut fn_ = 0u64;
        for i in 0..n {
            for j in (i + 1)..n {
                let same_cluster = clusters[i] >= 0 && clusters[i] == clusters[j];
                let same_label = labels[i] == labels[j];
                match (same_cluster, same_label) {
                    (true, true) => tp += 1,
                    (true, false) => fp += 1,
                    (false, true) => fn_ += 1,
                    (false, false) => {}
                }
            }
        }
        (tp, fp, fn_)
    }

    pub fn run() {
        let labels_path = parse_args();
        let rows = read_labels(&labels_path);
        assert!(!rows.is_empty(), "labels CSV was empty");

        let models_dir = std::env::var_os("MV_MODELS")
            .map(PathBuf::from)
            .expect("MV_MODELS required");
        let rt = Arc::new(
            MlRuntime::load(MlConfig {
                model_dir: models_dir,
                execution_provider: ExecutionProvider::Auto,
                bundle: mv_core::ml::bundles::BundleId::Full,
            })
            .expect("load runtime"),
        );

        let mut embeddings: Vec<Vec<f32>> = Vec::new();
        let mut gt: Vec<String> = Vec::new();
        let mut skipped = 0usize;
        for (path, label) in &rows {
            let disp = path.display();
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("skip {disp}: {e}");
                    skipped += 1;
                    continue;
                }
            };
            let Ok(img) = image::load_from_memory(&bytes) else {
                eprintln!("skip {disp}: image decode failed");
                skipped += 1;
                continue;
            };
            let rgb = img.to_rgb8();
            let detections = faces::detect_faces(&rt.scrfd(), &rgb).expect("detect_faces");
            let Some(best) = detections
                .into_iter()
                .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
            else {
                eprintln!("skip {disp}: no faces detected");
                skipped += 1;
                continue;
            };
            let emb = faces::embed_face(&rt.arcface(), &rgb, &best.landmarks).expect("embed_face");
            embeddings.push(emb);
            gt.push(label.clone());
        }
        eprintln!(
            "embedded {} faces ({} skipped) from {} rows",
            embeddings.len(),
            skipped,
            rows.len()
        );

        let clusters = dbscan_cosine(&embeddings, 0.4, 2);
        let n_clusters: HashMap<i32, usize> =
            clusters.iter().fold(HashMap::new(), |mut acc, &c| {
                *acc.entry(c).or_default() += 1;
                acc
            });
        eprintln!("clusters: {n_clusters:?}");

        let (tp, fp, fn_) = pairwise_counts(&gt, &clusters);
        let precision = if tp + fp == 0 {
            0.0
        } else {
            tp as f64 / (tp + fp) as f64
        };
        let recall = if tp + fn_ == 0 {
            0.0
        } else {
            tp as f64 / (tp + fn_) as f64
        };
        let f1 = if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        };
        println!("pairwise F1: {f1:.3}  (P={precision:.3} R={recall:.3})");
    }
}
