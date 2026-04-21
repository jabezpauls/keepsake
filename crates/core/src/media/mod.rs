//! Media probing, pairing, and derivative generation.
//!
//! Phase 1 targets the most common formats (JPEG / PNG / HEIC / H.264-MP4 /
//! common RAW). Anything that can't be probed returns minimal metadata — we
//! never fail ingest because of an unreadable EXIF block.

pub mod derive;
pub mod probe;

// pairs.rs is added in Step 9 of the execution plan.

pub use derive::{derive_thumbnails, ThumbnailOutput, ThumbnailSize};
pub use probe::{probe_path, MediaProbe};
