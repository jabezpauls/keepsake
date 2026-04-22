//! Model manifest — pins exactly which ONNX files we expect on disk and the
//! SHA-256 of each. Loader uses this to reject stale/mismatched weights before
//! they reach an `ort::Session`.
//!
//! URLs deliberately live in `scripts/download_models.sh`, not here: the repo
//! does not redistribute weights (see architecture.md §9 and the Phase 2.1
//! plan). Users source their own URLs at download time and pin to the
//! checksums below.
//!
//! **Updating a checksum** requires:
//! 1. A re-run of Tier-B model-gated tests with the new weights.
//! 2. An equal update to `scripts/download_models.sh`.
//! 3. A commit message documenting why the pin moved.

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::{Error, Result};

/// One entry per model file we expect inside `model_dir`.
#[derive(Debug, Clone, Copy)]
pub struct ModelEntry {
    /// Logical name — matches the expected filename inside `model_dir`.
    pub name: &'static str,
    /// Hex-encoded SHA-256 of the expected file. Lowercase, 64 chars.
    pub sha256_hex: &'static str,
}

/// The five artefacts Phase 2.1 needs. Order is load order.
///
/// Checksums are **placeholders** until a real pin is chosen by running
/// Tier-C evaluations and recording the result here + in
/// `scripts/download_models.sh`. Placeholders are all zeroes so any real file
/// trips `MlModelChecksum` on load until they are updated — this is intended:
/// we fail closed rather than accept an unknown weight blob.
pub const MODELS: &[ModelEntry] = &[
    ModelEntry {
        name: "clip_visual.onnx",
        sha256_hex: "0000000000000000000000000000000000000000000000000000000000000000",
    },
    ModelEntry {
        name: "clip_textual.onnx",
        sha256_hex: "0000000000000000000000000000000000000000000000000000000000000000",
    },
    ModelEntry {
        name: "clip_tokenizer.json",
        sha256_hex: "0000000000000000000000000000000000000000000000000000000000000000",
    },
    ModelEntry {
        name: "scrfd.onnx",
        sha256_hex: "0000000000000000000000000000000000000000000000000000000000000000",
    },
    ModelEntry {
        name: "arcface.onnx",
        sha256_hex: "0000000000000000000000000000000000000000000000000000000000000000",
    },
];

/// Compute the hex SHA-256 of a file on disk. Streams; does not allocate the
/// whole file. Returns `Error::Io` on read failure.
pub fn sha256_file(path: &Path) -> Result<String> {
    let f = File::open(path)?;
    let mut reader = BufReader::new(f);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
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
