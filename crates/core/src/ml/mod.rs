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

pub use runtime::{ExecutionProvider, MlConfig, MlJobKind, MlRuntime, MlWorker};
