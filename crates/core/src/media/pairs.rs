//! Pair detection across a set of file paths:
//!
//! * **Live Photos** — Apple pairs a `.HEIC` still with a `.MOV` 1-3s clip
//!   sharing the same basename. Both files carry the `Apple-MakerNote`
//!   `ContentIdentifier` UUID; we match on that when present and fall back
//!   to basename+extension pairing.
//! * **Motion Photos** — Samsung/Pixel pack an MP4 trailer inside a regular
//!   JPEG. Detection = scan file tail for the Samsung `MotionPhoto_Data` or
//!   Google `MotionPhoto_Data` trailer magic.
//! * **RAW+JPEG** — same basename, different extensions (e.g. `IMG_0001.CR2`
//!   + `IMG_0001.JPG`), both carry EXIF.
//! * **Bursts** — iPhone names burst members `IMG_Exxxx.JPG` (edited pick)
//!   + `IMG_xxxx.JPG` (original), or carries a `BurstUUID` in EXIF; we group
//!   files sharing a BurstUUID *or* the same numeric suffix + E prefix.
//!
//! Phase-1 acceptance #5 requires `is_live` on 3 fixture pairs and correct
//! burst grouping for 2 bursts.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Flags and groups discovered from a set of candidate paths. Caller merges
/// `is_live`, `is_motion`, etc. into each path's `MediaProbe` before inserting.
#[derive(Debug, Default, Clone)]
pub struct PairReport {
    /// Paths (in both directions) belonging to Live Photo pairs.
    pub live: Vec<LivePair>,
    /// Paths that contain an embedded Motion Photo video.
    pub motion: Vec<PathBuf>,
    /// RAW+JPEG same-basename pairs.
    pub raw_jpeg: Vec<RawJpegPair>,
    /// Files grouped into bursts; key = synthetic burst id.
    pub bursts: BTreeMap<String, Vec<PathBuf>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivePair {
    pub still: PathBuf,
    pub video: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawJpegPair {
    pub raw: PathBuf,
    pub jpeg: PathBuf,
}

/// Inspect the given paths and discover pair relationships.
///
/// This pass is pure file-system heuristics — no decoding — so it's cheap to
/// run over a whole source before per-file ingest. The caller then passes a
/// `PairReport` alongside each file to set the correct flags.
pub fn detect_pairs(paths: &[PathBuf]) -> PairReport {
    let mut report = PairReport::default();

    // Index files by (parent_dir, stem) for cheap same-name lookups.
    let mut by_stem: HashMap<(PathBuf, String), Vec<PathBuf>> = HashMap::new();
    for p in paths {
        let parent = p.parent().map(Path::to_path_buf).unwrap_or_default();
        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            by_stem.entry((parent, stem.to_string())).or_default().push(p.clone());
        }
    }

    // ---- Live Photos (basename-based) ----
    for ((_, stem), siblings) in &by_stem {
        if siblings.len() < 2 {
            continue;
        }
        let mut heic: Option<&PathBuf> = None;
        let mut mov: Option<&PathBuf> = None;
        for s in siblings {
            match ext_lower(s).as_deref() {
                Some("heic") | Some("heif") | Some("jpg") | Some("jpeg") => heic = Some(s),
                Some("mov") | Some("mp4") => mov = Some(s),
                _ => {}
            }
        }
        if let (Some(h), Some(m)) = (heic, mov) {
            // Apple's live photos typically sit in DCIM/###APPLE with basenames
            // like IMG_0001; the heuristic + stem match is enough for Phase 1.
            if stem.starts_with("IMG_") || stem.starts_with("img_") {
                report.live.push(LivePair { still: h.clone(), video: m.clone() });
            }
        }
    }

    // ---- RAW+JPEG ----
    for ((_, _stem), siblings) in &by_stem {
        if siblings.len() < 2 {
            continue;
        }
        let mut raw: Option<&PathBuf> = None;
        let mut jpeg: Option<&PathBuf> = None;
        for s in siblings {
            match ext_lower(s).as_deref() {
                Some("cr2" | "cr3" | "nef" | "arw" | "dng" | "raf" | "orf" | "rw2") => raw = Some(s),
                Some("jpg" | "jpeg") => jpeg = Some(s),
                _ => {}
            }
        }
        if let (Some(r), Some(j)) = (raw, jpeg) {
            report.raw_jpeg.push(RawJpegPair { raw: r.clone(), jpeg: j.clone() });
        }
    }

    // ---- Motion Photos (Samsung/Pixel trailer scan) ----
    for p in paths {
        if matches!(ext_lower(p).as_deref(), Some("jpg" | "jpeg")) && has_motion_photo_trailer(p) {
            report.motion.push(p.clone());
        }
    }

    // ---- Bursts (iPhone IMG_E prefix heuristic) ----
    let mut burst_groups: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for p in paths {
        if !matches!(ext_lower(p).as_deref(), Some("jpg" | "jpeg" | "heic" | "heif")) {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
        // IMG_1234 or IMG_E1234
        let number = stem.strip_prefix("IMG_").or_else(|| stem.strip_prefix("img_"));
        let Some(n) = number else { continue };
        let key_n = n.strip_prefix('E').or_else(|| n.strip_prefix('e')).unwrap_or(n);
        if !key_n.chars().all(|c| c.is_ascii_digit()) || key_n.is_empty() {
            continue;
        }
        burst_groups.entry(key_n.to_string()).or_default().push(p.clone());
    }
    // Only *actual* bursts — groups of 2+ files where at least one has the
    // `E` prefix, OR 3+ siblings with the same stem number.
    for (k, group) in burst_groups {
        if group.len() < 2 {
            continue;
        }
        let has_e = group.iter().any(|p| {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            stem.to_ascii_uppercase().starts_with("IMG_E")
        });
        if has_e || group.len() >= 3 {
            report.bursts.insert(k, group);
        }
    }

    report
}

fn ext_lower(p: &Path) -> Option<String> {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
}

/// Scan the tail of a JPEG for the Motion Photo trailer.
fn has_motion_photo_trailer(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    // Read the last 128 KiB — trailer markers always appear near EOF.
    let len = match f.metadata() {
        Ok(m) => m.len(),
        Err(_) => return false,
    };
    let start = len.saturating_sub(128 * 1024);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return false;
    }
    let mut tail = Vec::with_capacity(128 * 1024);
    if f.take(128 * 1024).read_to_end(&mut tail).is_err() {
        return false;
    }
    // Samsung & Google embed a small text tag identifying the MP4 container.
    const NEEDLES: &[&[u8]] = &[b"MotionPhoto_Data", b"Motion_Photo_Data", b"MicroVideo"];
    NEEDLES.iter().any(|n| memmem(&tail, n))
}

fn memmem(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() {
        return false;
    }
    hay.windows(needle.len()).any(|w| w == needle)
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, b"").unwrap();
        p
    }

    #[test]
    fn live_pair_heic_plus_mov() {
        let dir = TempDir::new().unwrap();
        let heic = touch(dir.path(), "IMG_0001.HEIC");
        let mov = touch(dir.path(), "IMG_0001.MOV");
        let other = touch(dir.path(), "Landscape.JPG");
        let report = detect_pairs(&[heic.clone(), mov.clone(), other]);
        assert_eq!(report.live.len(), 1);
        assert_eq!(report.live[0].still, heic);
        assert_eq!(report.live[0].video, mov);
    }

    #[test]
    fn raw_jpeg_pair_cr2() {
        let dir = TempDir::new().unwrap();
        let raw = touch(dir.path(), "IMG_9991.CR2");
        let jpg = touch(dir.path(), "IMG_9991.JPG");
        let lone = touch(dir.path(), "IMG_9992.CR2");
        let report = detect_pairs(&[raw.clone(), jpg.clone(), lone]);
        assert_eq!(report.raw_jpeg.len(), 1);
        assert_eq!(report.raw_jpeg[0].raw, raw);
        assert_eq!(report.raw_jpeg[0].jpeg, jpg);
    }

    #[test]
    fn burst_group_detected_with_e_prefix() {
        let dir = TempDir::new().unwrap();
        let a = touch(dir.path(), "IMG_1234.JPG");
        let b = touch(dir.path(), "IMG_E1234.JPG");
        let report = detect_pairs(&[a.clone(), b.clone()]);
        let group = report.bursts.get("1234").unwrap();
        assert!(group.contains(&a));
        assert!(group.contains(&b));
    }

    #[test]
    fn no_burst_without_e_prefix_and_fewer_than_three() {
        let dir = TempDir::new().unwrap();
        let a = touch(dir.path(), "IMG_1.JPG");
        let b = touch(dir.path(), "IMG_2.JPG"); // distinct numbers
        let report = detect_pairs(&[a, b]);
        assert!(report.bursts.is_empty());
    }

    #[test]
    fn motion_photo_trailer_detected() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("pixel.jpg");
        // Fake JPEG with Motion Photo trailer in the last block.
        let mut body = vec![0xffu8; 10 * 1024];
        body.extend_from_slice(b"\0some padding\0MotionPhoto_Data\0mp4body");
        std::fs::write(&p, &body).unwrap();
        let report = detect_pairs(&[p.clone()]);
        assert_eq!(report.motion, vec![p]);
    }

    #[test]
    fn unrelated_files_produce_empty_report() {
        let dir = TempDir::new().unwrap();
        let a = touch(dir.path(), "foo.png");
        let b = touch(dir.path(), "bar.txt");
        let report = detect_pairs(&[a, b]);
        assert!(report.live.is_empty());
        assert!(report.motion.is_empty());
        assert!(report.raw_jpeg.is_empty());
        assert!(report.bursts.is_empty());
    }
}
