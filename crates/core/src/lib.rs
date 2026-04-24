//! Media Vault core library.
//!
//! Phase 1 scaffolding. Modules are populated progressively as phase tasks complete;
//! declarations exist here from the start so the dependency graph compiles cleanly.

pub mod analytics;
pub mod blind_text;
pub mod cas;
pub mod crypto;
pub mod db;
pub mod error;
pub mod geocode;
pub mod ingest;
pub mod media;
pub mod ml;
pub mod public_link;
pub mod search;
pub mod share;

pub use error::{Error, Result};
