//! On-device ML pipeline — Phase 2.
//!
//! Public surface covers CLIP (visual + textual), face detection/embedding,
//! and perceptual hashing. Model-backed pieces live behind the `ml-models`
//! feature flag; when disabled, [`MlRuntime::load`] returns
//! `Err(Error::ModelsUnavailable)` and the UI falls back to "install models"
//! prompts.
//!
//! `phash` is pure Rust with no model — it always builds.

pub mod nearp;
pub mod phash;
pub mod runtime;

// Phase 2.1 — feature-gated ONNX-backed pipeline. Unit-testable pieces
// (manifest verifier) build without the flag because they only need `sha2` at
// test time; keep them behind `ml-models` to avoid dragging the dep into the
// default build.
#[cfg(feature = "ml-models")]
pub mod manifest;

#[cfg(feature = "ml-models")]
pub mod loader;

#[cfg(feature = "ml-models")]
pub mod clip;

#[cfg(feature = "ml-models")]
pub mod tokenizer;

#[cfg(feature = "ml-models")]
pub mod faces;

#[cfg(feature = "ml-models")]
pub mod worker_exec;

pub use runtime::{ExecutionProvider, MlConfig, MlJobKind, MlRuntime, MlWorker};

/// Compile-time flag: true when the `ml-models` feature is enabled. Callers
/// use this for UI/status rendering (e.g. "install models" vs "running").
#[cfg(feature = "ml-models")]
pub const MODELS_ENABLED: bool = true;

#[cfg(not(feature = "ml-models"))]
pub const MODELS_ENABLED: bool = false;
