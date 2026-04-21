//! Device-specific ingest adapters.
//!
//! All three share the heavy lifting in `generic.rs`; iPhone and Takeout
//! layer pre-processing on top of it (Phase 1 Tasks 7c/d — added in Step 10).

pub mod generic;
