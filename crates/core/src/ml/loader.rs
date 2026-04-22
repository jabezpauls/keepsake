//! ONNX session loader — Phase 2.1.
//!
//! `load_all` walks the pinned manifest, verifies SHA-256 per file, and then
//! builds one `ort::Session` per model with a CUDA → CoreML → CPU provider
//! fallback. Callers get either `Ok(Sessions)` with all four sessions live,
//! or a specific error variant (`ModelsUnavailable` / `MlModelChecksum` /
//! `MlModelShape` / `Ingest`) so the UI can render an actionable prompt.
//!
//! The actual model files are never redistributed — `model_dir` is populated
//! by `scripts/download_models.sh` using user-supplied URLs. See plan
//! `wise-strolling-otter.md` for the BYO-URLs rationale.

use std::path::Path;
use std::sync::Arc;

use ort::execution_providers::{
    CPUExecutionProvider, CoreMLExecutionProvider, CUDAExecutionProvider, ExecutionProviderDispatch,
};
use ort::session::Session;

use super::manifest;
use super::runtime::ExecutionProvider;
use crate::{Error, Result};

/// The four ONNX sessions the Phase 2.1 pipeline needs.
///
/// `Session` is not `Clone`, so we wrap each in `Arc` for the few places that
/// share across worker tasks (`worker_exec::run_*` and `search::maybe_clip_rerank`).
pub struct Sessions {
    pub clip_visual: Arc<Session>,
    pub clip_textual: Arc<Session>,
    pub scrfd: Arc<Session>,
    pub arcface: Arc<Session>,
    /// Human-readable name of the provider the sessions actually loaded with.
    /// Reported back through `ml_status` so the UI can show "running on Cuda".
    pub provider_label: String,
}

/// Verify every manifest entry, then build one session per ONNX file. Returns
/// the loaded sessions plus the provider label that was selected.
///
/// Provider selection order is fallback-style, not exclusive: we hand
/// `ort::SessionBuilder::with_execution_providers` the full preference list
/// and ORT picks the first one that actually initialises on the host. That
/// means `ExecutionProvider::Auto` yields a CUDA-first box that silently
/// falls through to CPU when CUDA isn't present.
pub fn load_all(model_dir: &Path, preferred: ExecutionProvider) -> Result<Sessions> {
    manifest::verify_all(model_dir)?;

    let providers = build_provider_list(preferred);
    let provider_label = provider_list_label(&preferred);

    let clip_visual = build_session(model_dir, "clip_visual.onnx", &providers)?;
    let clip_textual = build_session(model_dir, "clip_textual.onnx", &providers)?;
    let scrfd = build_session(model_dir, "scrfd.onnx", &providers)?;
    let arcface = build_session(model_dir, "arcface.onnx", &providers)?;

    // Output-shape assertions — these are the cheapest way to catch a model
    // export that doesn't match what our post-processor expects. SCRFD is the
    // riskiest: the `scrfd_10g_bnkps` export emits 9 tensors (3 strides × {score,
    // bbox, kps}); other exports collapse into 1. CLIP + ArcFace are each a
    // single pooled embedding.
    if scrfd.outputs.len() != 9 {
        return Err(Error::MlModelShape(
            "scrfd.onnx: expected 9 output tensors (3 strides × score/bbox/kps)",
        ));
    }
    if clip_visual.outputs.len() != 1 {
        return Err(Error::MlModelShape(
            "clip_visual.onnx: expected 1 output (pooled 768-d embedding)",
        ));
    }
    if clip_textual.outputs.len() != 1 {
        return Err(Error::MlModelShape(
            "clip_textual.onnx: expected 1 output (pooled 768-d embedding)",
        ));
    }
    if arcface.outputs.len() != 1 {
        return Err(Error::MlModelShape(
            "arcface.onnx: expected 1 output (512-d embedding)",
        ));
    }

    Ok(Sessions {
        clip_visual: Arc::new(clip_visual),
        clip_textual: Arc::new(clip_textual),
        scrfd: Arc::new(scrfd),
        arcface: Arc::new(arcface),
        provider_label,
    })
}

fn build_session(
    model_dir: &Path,
    name: &'static str,
    providers: &[ExecutionProviderDispatch],
) -> Result<Session> {
    let path = model_dir.join(name);
    Session::builder()
        .map_err(|e| ort_to_error(name, &e))?
        .with_execution_providers(providers)
        .map_err(|e| ort_to_error(name, &e))?
        .commit_from_file(&path)
        .map_err(|e| ort_to_error(name, &e))
}

fn ort_to_error(name: &'static str, err: &ort::Error) -> Error {
    // Deliberate: the error chain goes to `debug!` (so operators can see the
    // full ORT detail) while the caller-facing variant stays opaque. Details
    // leaking through `Display` would make the UI surface internal ORT paths.
    tracing::debug!(model = name, %err, "ort session load failed");
    Error::Ingest(format!("ort load {name} failed"))
}

fn build_provider_list(preferred: ExecutionProvider) -> Vec<ExecutionProviderDispatch> {
    // CPU is always on the list as the universal fallback.
    let cpu: ExecutionProviderDispatch = CPUExecutionProvider::default().build();
    match preferred {
        ExecutionProvider::Cpu => vec![cpu],
        ExecutionProvider::Cuda => vec![CUDAExecutionProvider::default().build(), cpu],
        ExecutionProvider::CoreMl => vec![CoreMLExecutionProvider::default().build(), cpu],
        // Auto: try CUDA, then CoreML, then CPU. ORT ignores providers that
        // weren't compiled in, so on a box without `ml-cuda` the CUDA entry
        // simply never activates.
        ExecutionProvider::Auto => vec![
            CUDAExecutionProvider::default().build(),
            CoreMLExecutionProvider::default().build(),
            cpu,
        ],
    }
}

fn provider_list_label(preferred: &ExecutionProvider) -> String {
    match preferred {
        ExecutionProvider::Auto => "Auto".to_string(),
        ExecutionProvider::Cpu => "Cpu".to_string(),
        ExecutionProvider::Cuda => "Cuda".to_string(),
        ExecutionProvider::CoreMl => "CoreMl".to_string(),
    }
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_model_dir_returns_models_unavailable() {
        let tmp = TempDir::new().unwrap();
        let r = load_all(tmp.path(), ExecutionProvider::Cpu);
        assert!(matches!(r, Err(Error::ModelsUnavailable)));
    }

    #[test]
    fn provider_list_always_includes_cpu() {
        for ep in [
            ExecutionProvider::Auto,
            ExecutionProvider::Cpu,
            ExecutionProvider::Cuda,
            ExecutionProvider::CoreMl,
        ] {
            let list = build_provider_list(ep);
            assert!(!list.is_empty());
            // CPU is always the last entry — that's the fallback contract.
            let labels: Vec<String> = list.iter().map(|p| format!("{:?}", p)).collect();
            assert!(
                labels.last().unwrap().contains("CPU"),
                "CPU not at end of list for {:?}: {:?}",
                ep,
                labels
            );
        }
    }

    #[test]
    fn provider_labels_are_stable() {
        assert_eq!(provider_list_label(&ExecutionProvider::Auto), "Auto");
        assert_eq!(provider_list_label(&ExecutionProvider::Cpu), "Cpu");
        assert_eq!(provider_list_label(&ExecutionProvider::Cuda), "Cuda");
        assert_eq!(provider_list_label(&ExecutionProvider::CoreMl), "CoreMl");
    }

    // Tier-B: requires real model weights at MV_MODELS.
    // Run with: MV_MODELS=/path cargo test -p mv-core --features ml-models -- --ignored
    #[test]
    #[ignore = "requires MV_MODELS=/path with real weights"]
    fn load_all_succeeds_with_real_weights() {
        let Some(dir) = std::env::var_os("MV_MODELS") else {
            panic!("MV_MODELS not set — invoked with --ignored but env missing");
        };
        let sessions = load_all(Path::new(&dir), ExecutionProvider::Auto)
            .expect("real weights should load cleanly");
        // Spot-check input/output counts match CLIP/ArcFace expectations.
        assert_eq!(sessions.clip_visual.outputs.len(), 1);
        assert_eq!(sessions.arcface.outputs.len(), 1);
    }
}
