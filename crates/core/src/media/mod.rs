//! Media probing, pairing, and derivative generation.
//!
//! Phase 1 targets the most common formats (JPEG / PNG / HEIC / H.264-MP4 /
//! common RAW). Anything that can't be probed returns minimal metadata — we
//! never fail ingest because of an unreadable EXIF block.

pub mod derive;
pub mod pairs;
pub mod probe;

pub use derive::{derive_thumbnails, ThumbnailOutput, ThumbnailSize};
pub use pairs::{detect_pairs, LivePair, PairReport, RawJpegPair};
pub use probe::{probe_path, MediaProbe};
