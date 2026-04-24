//! Model manifest — pins exactly which ONNX files we expect on disk and the
//! SHA-256 of each. Loader uses this to reject stale/mismatched weights before
//! they reach an `ort::Session`.
//!
//! The catalog of expected files now lives in [`super::bundles`]; this
//! module is the streaming-SHA verifier the loader + downloader share.
//! `MODELS` remains as a legacy alias for the Full bundle so call sites
//! that predate the bundle split keep compiling.
//!
//! **Updating a checksum** requires:
//! 1. A re-run of Tier-B model-gated tests with the new weights.
//! 2. An equal update to `scripts/download_models.sh`.
//! 3. A commit message documenting why the pin moved.

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use super::bundles::{BundleId, BundleSpec};
use crate::{Error, Result};

/// One entry per model file we expect inside `model_dir`.
#[derive(Debug, Clone, Copy)]
pub struct ModelEntry {
    /// Logical name — matches the expected filename inside `model_dir`.
    pub name: &'static str,
    /// Hex-encoded SHA-256 of the expected file. Lowercase, 64 chars.
    pub sha256_hex: &'static str,
}

/// The Full bundle's file set — kept as a `pub const` so the (now small
/// number of) call sites that predate the bundle split still compile. New
/// callers should query the selected bundle via
/// [`super::bundles::by_id`] + [`BundleSpec::manifest_entries`] instead.
pub const MODELS: &[ModelEntry] = &[
    ModelEntry {
        name: "clip_visual.onnx",
        sha256_hex: "2b02d572f59c509f4b97b9c54a868453cca1a652cd5d60e1d51d0052f055cb8c",
    },
    ModelEntry {
        name: "clip_textual.onnx",
        sha256_hex: "9fbe72ea8d36c2effaccedcf7249e3729ad0d9b4af6604b433ecdd0105663c9c",
    },
    ModelEntry {
        name: "clip_tokenizer.json",
        sha256_hex: "6d9109cc838977f3ca94a379eec36aecc7c807e1785cd729660ca2fc0171fb35",
    },
    ModelEntry {
        name: "scrfd.onnx",
        sha256_hex: "5838f7fe053675b1c7a08b633df49e7af5495cee0493c7dcf6697200b85b5b91",
    },
    ModelEntry {
        name: "arcface.onnx",
        sha256_hex: "4c06341c33c2ca1f86781dab0e829f88ad5b64be9fba56e56bc9ebdefc619e43",
    },
];

/// Compute the hex SHA-256 of a file on disk. Streams; does not allocate the
/// whole file. Returns `Error::Io` on read failure.
pub fn sha256_file(path: &Path) -> Result<String> {
    let f = File::open(path)?;
    let mut reader = BufReader::new(f);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Verify one manifest entry against a file inside `model_dir`. `Ok(())` means
/// the file exists and matches; `Err(MlModelChecksum)` means checksum
/// mismatch; `Err(ModelsUnavailable)` means the file isn't present at all.
pub fn verify_entry(model_dir: &Path, entry: &ModelEntry) -> Result<()> {
    let path = model_dir.join(entry.name);
    if !path.exists() {
        return Err(Error::ModelsUnavailable);
    }
    let got = sha256_file(&path)?;
    if got.eq_ignore_ascii_case(entry.sha256_hex) {
        Ok(())
    } else {
        Err(Error::MlModelChecksum(entry.name))
    }
}

/// Verify every entry in `MODELS`. Stops at the first failure. Callers should
/// propagate the error as-is — variant identity carries the exact cause.
pub fn verify_all(model_dir: &Path) -> Result<()> {
    for entry in MODELS {
        verify_entry(model_dir, entry)?;
    }
    Ok(())
}

/// Verify every entry declared by `bundle`. Semantically identical to
/// [`verify_all`] but uses the bundle's pinned SHAs — the right path for
/// Lite-bundle deployments where `MODELS` doesn't describe what's on disk.
pub fn verify_bundle(model_dir: &Path, bundle: &BundleSpec) -> Result<()> {
    for entry in bundle.manifest_entries() {
        verify_entry(model_dir, &entry)?;
    }
    Ok(())
}

/// Filename (relative to `model_dir`) where the wizard records the bundle
/// the user picked. Small JSON, written once per switch.
pub const BUNDLE_FILE: &str = "bundle.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct BundlePersisted {
    id: String,
}

/// Read the persisted bundle choice from `<model_dir>/bundle.json`. Returns
/// `None` when the file is absent (fresh install) or malformed — the caller
/// treats both as "wizard hasn't run yet".
#[must_use]
pub fn read_selected_bundle(model_dir: &Path) -> Option<BundleId> {
    let path = model_dir.join(BUNDLE_FILE);
    let raw = std::fs::read(&path).ok()?;
    let doc: BundlePersisted = serde_json::from_slice(&raw).ok()?;
    BundleId::from_str(&doc.id)
}

/// Persist the bundle choice so the next startup skips the wizard and
/// boots straight into the right runtime. Creates `model_dir` if missing.
pub fn write_selected_bundle(model_dir: &Path, id: BundleId) -> Result<()> {
    std::fs::create_dir_all(model_dir)?;
    let doc = BundlePersisted {
        id: id.as_str().to_string(),
    };
    let raw = serde_json::to_vec_pretty(&doc)
        .map_err(|e| Error::Ingest(format!("bundle.json serialize: {e}")))?;
    std::fs::write(model_dir.join(BUNDLE_FILE), raw)?;
    Ok(())
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn hex_sha256(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        hex::encode(h.finalize())
    }

    #[test]
    fn sha256_matches_reference_vector() {
        // Standard test vector: SHA-256("abc").
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("abc.bin");
        std::fs::write(&p, b"abc").unwrap();
        let got = sha256_file(&p).unwrap();
        assert_eq!(
            got,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verify_entry_detects_missing_then_match_then_mismatch() {
        let tmp = TempDir::new().unwrap();
        let name = "fake.onnx";
        let path = tmp.path().join(name);

        let content = b"synthetic weights";
        let entry_ok = ModelEntry {
            name,
            sha256_hex: Box::leak(hex_sha256(content).into_boxed_str()),
        };
        let entry_bad = ModelEntry {
            name,
            sha256_hex: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        };

        // Missing → ModelsUnavailable.
        let r = verify_entry(tmp.path(), &entry_ok);
        assert!(matches!(r, Err(Error::ModelsUnavailable)));

        // Write it and verify match.
        let mut f = File::create(&path).unwrap();
        f.write_all(content).unwrap();
        drop(f);
        verify_entry(tmp.path(), &entry_ok).expect("checksum should match");

        // Wrong expected checksum → MlModelChecksum.
        let r = verify_entry(tmp.path(), &entry_bad);
        assert!(matches!(r, Err(Error::MlModelChecksum(n)) if n == name));
    }

    #[test]
    fn verify_all_short_circuits_on_first_missing() {
        let tmp = TempDir::new().unwrap();
        let r = verify_all(tmp.path());
        assert!(matches!(r, Err(Error::ModelsUnavailable)));
    }

    #[test]
    fn manifest_table_is_well_formed() {
        // Every entry must have a 64-char lowercase hex sha256, even the
        // placeholder zeroes — a wrong shape here would mask real drift.
        for e in MODELS {
            assert_eq!(e.sha256_hex.len(), 64, "bad length for {}", e.name);
            assert!(
                e.sha256_hex.chars().all(|c| c.is_ascii_hexdigit()),
                "non-hex in {}",
                e.name
            );
            assert!(!e.name.is_empty());
        }
        // Expected set of names (exact order: CLIP visual, textual, tokenizer,
        // SCRFD, ArcFace).
        let names: Vec<&str> = MODELS.iter().map(|m| m.name).collect();
        assert_eq!(
            names,
            vec![
                "clip_visual.onnx",
                "clip_textual.onnx",
                "clip_tokenizer.json",
                "scrfd.onnx",
                "arcface.onnx",
            ]
        );
    }
}
