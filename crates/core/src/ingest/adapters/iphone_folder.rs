//! iPhone folder adapter.
//!
//! Wraps [`super::generic::GenericAdapter`] with iPhone-specific heuristics:
//!
//! - Recognises the DCIM/###APPLE directory structure (not a hard requirement,
//!   but the device hint is surfaced to the UI when present).
//! - Runs `media::pairs::detect_pairs` over the whole source before ingest so
//!   Live Photos / bursts are flagged.
//!
//! Phase 1 wires the hint + pairing through; deeper MakerNote-based device
//! inference is deferred to Phase 2.

use std::path::Path;

use walkdir::WalkDir;

use crate::ingest::{IngestAdapter, IngestCtx, IngestReport, Progress, ScanReport};
use crate::media::detect_pairs;
use crate::Result;

use super::generic::GenericAdapter;

pub struct IPhoneFolderAdapter {
    inner: GenericAdapter,
}

impl IPhoneFolderAdapter {
    pub fn new() -> Self {
        Self {
            inner: GenericAdapter::new(),
        }
    }
}

impl Default for IPhoneFolderAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// Heuristic: looks like a DCIM tree if any first-level subdir name is
/// `<digits>APPLE` (e.g. `100APPLE`, `101APPLE`).
pub fn looks_like_dcim_tree(root: &Path) -> bool {
    let dcim = root.join("DCIM");
    let scan_root = if dcim.is_dir() {
        dcim
    } else {
        root.to_path_buf()
    };
    for entry in std::fs::read_dir(&scan_root)
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        if let Some(n) = entry.file_name().to_str() {
            if n.ends_with("APPLE")
                && n.trim_end_matches("APPLE")
                    .chars()
                    .all(|c| c.is_ascii_digit())
            {
                return true;
            }
        }
    }
    false
}

fn all_candidate_paths(root: &Path) -> Vec<std::path::PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect()
}

#[async_trait::async_trait]
impl IngestAdapter for IPhoneFolderAdapter {
    fn kind(&self) -> &'static str {
        "iphone_folder"
    }

    async fn scan(&self, root: &Path) -> Result<ScanReport> {
        self.inner.scan(root).await
    }

    async fn ingest(
        &self,
        ctx: &IngestCtx,
        root: &Path,
        source_id: i64,
        progress: Progress,
    ) -> Result<IngestReport> {
        // Pre-pass: detect pairs. The flags themselves are surfaced to the
        // generic ingest implicitly because the `media::probe` layer checks
        // a supplementary pair table during insertion in Phase 2; for Phase 1
        // we log the detection so fixtures can assert on it.
        let paths = all_candidate_paths(root);
        let report = detect_pairs(&paths);
        tracing::info!(
            live_pairs = report.live.len(),
            motion = report.motion.len(),
            raw_jpeg = report.raw_jpeg.len(),
            bursts = report.bursts.len(),
            "iphone pair detection complete"
        );
        // Delegate the heavy lifting.
        self.inner.ingest(ctx, root, source_id, progress).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn dcim_heuristic_true_for_apple_subdir() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("DCIM").join("100APPLE")).unwrap();
        assert!(looks_like_dcim_tree(dir.path()));
    }

    #[test]
    fn dcim_heuristic_false_for_plain_folder() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("Pictures")).unwrap();
        assert!(!looks_like_dcim_tree(dir.path()));
    }

    #[test]
    fn adapter_kind_is_iphone_folder() {
        assert_eq!(IPhoneFolderAdapter::new().kind(), "iphone_folder");
    }
}
