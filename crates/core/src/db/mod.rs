//! SQLite schema, migrations, and typed query helpers.
//!
//! Schema is FROZEN per `plans/architecture.md` §4. Phase 1 is version 1;
//! future phases add via additive migrations.

pub mod migrate;
pub mod queries;
pub mod schema;

pub use queries::*;
pub use schema::{init, open, open_readonly};
