//! Ingest pipeline — scan + encrypt + store + provenance.

pub mod provenance;
pub mod sidecar;

pub use sidecar::{read_xmp_sidecar, write_xmp_sidecar, XmpFields};
