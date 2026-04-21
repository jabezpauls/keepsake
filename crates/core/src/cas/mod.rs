//! Content-addressed encrypted store.
//!
//! All blobs live at `<root>/cas/<AA>/<HEX_BLAKE3_OF_PLAINTEXT>`, encrypted
//! per `plans/architecture.md` §3. The store is append-only outside of
//! explicit `gc()` calls.

pub mod store;

pub use store::{CasStore, GcReport};
