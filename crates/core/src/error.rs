//! Placeholder for the Phase 1 Task-2 error hierarchy.
//!
//! The full enum is introduced in Step 2 of the execution plan. A minimal stand-in
//! is committed here so `cargo check --workspace` passes at Step 1.

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("not yet implemented")]
    Unimplemented,
}

pub type Result<T> = std::result::Result<T, Error>;
