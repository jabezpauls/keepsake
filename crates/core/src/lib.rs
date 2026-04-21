//! Media Vault core library.
//!
//! Phase 1 scaffolding. Modules are populated progressively as phase tasks complete;
//! declarations exist here from the start so the dependency graph compiles cleanly.

pub mod crypto;
pub mod error;

pub use error::{Error, Result};
