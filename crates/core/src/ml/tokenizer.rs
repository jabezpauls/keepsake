//! CLIP BPE tokenizer wrapper — Phase 2.1.
//!
//! Loads the HuggingFace-format `clip_tokenizer.json` (post-processing
//! includes BOS/EOS) and enforces the 77-token context length used by every
//! OpenAI CLIP export.
//!
//! The actual tokenizer.json file lives alongside the ONNX weights and is
//! pinned by `ml::manifest`. Any drift in the file invalidates all text
//! embeddings cached in `asset_vec` so the sha256 pin is load-bearing.

use std::path::Path;

use tokenizers::Tokenizer;

use crate::{Error, Result};

/// FROZEN — CLIP ViT-L/14 context length.
pub const CLIP_CTX: usize = 77;
/// FROZEN — OpenAI CLIP BOS token id (`<|startoftext|>`).
pub const CLIP_BOS: u32 = 49_406;
/// FROZEN — OpenAI CLIP EOS token id (`<|endoftext|>`).
pub const CLIP_EOS: u32 = 49_407;

/// Wraps a loaded `Tokenizer` so callers don't depend on the raw crate shape.
pub struct ClipTokenizer {
    inner: Tokenizer,
}

impl ClipTokenizer {
    /// Load from `<model_dir>/clip_tokenizer.json`. Errors map to
    /// `Error::ModelsUnavailable` (missing) or `Error::Ingest` (malformed).
    pub fn load(model_dir: &Path) -> Result<Self> {
        let path = model_dir.join("clip_tokenizer.json");
        if !path.exists() {
            return Err(Error::ModelsUnavailable);
        }
        let inner = Tokenizer::from_file(&path).map_err(|e| {
            tracing::debug!(%e, "clip tokenizer load failed");
            Error::Ingest("clip_tokenizer.json malformed".into())
        })?;
        Ok(Self { inner })
    }

    /// Tokenise `text` into a dense length-77 `i64` vector suitable for feeding
    /// into CLIP's textual ONNX session.
    ///
    /// The returned vector is always exactly `CLIP_CTX` long, starts with
    /// `CLIP_BOS`, has `CLIP_EOS` at either position 76 (truncated) or
    /// immediately after the final token (padded), and zero-pads the tail.
    ///
    /// `Tokenizer::encode` already applies the serialized post-processor,
    /// which for the OpenAI CLIP tokenizer attaches BOS/EOS. We then do our
    /// own padding/truncation to 77 — not relying on the JSON's padding spec
    /// makes us robust to tokenizer-file drift and is what `open_clip`'s own
    /// Python tokenize() does.
    pub fn encode(&self, text: &str) -> Result<Vec<i64>> {
        let encoding = self
            .inner
            .encode(text, true)
            .map_err(|e| Error::Ingest(format!("clip tokenize: {e}")))?;
        let ids = encoding.get_ids();
        Ok(pad_or_truncate(ids))
    }
}

/// Shape the raw BPE-with-BOS/EOS IDs into the fixed 77-token layout. Public
/// for unit testing; `encode` is the normal entry point.
pub fn pad_or_truncate(ids: &[u32]) -> Vec<i64> {
    let mut out = vec![0_i64; CLIP_CTX];
    if ids.is_empty() {
        // Degenerate: still emit BOS/EOS so the session sees a valid prompt.
        out[0] = CLIP_BOS as i64;
        out[1] = CLIP_EOS as i64;
        return out;
    }
    if ids.len() <= CLIP_CTX {
        // Fits. Copy as-is; if the post-processor skipped EOS, nail the last
        // token ourselves so the session always sees a proper terminator.
        for (i, &id) in ids.iter().enumerate() {
            out[i] = id as i64;
        }
        if ids.last() != Some(&CLIP_EOS) && ids.len() < CLIP_CTX {
            out[ids.len()] = CLIP_EOS as i64;
        }
        return out;
    }
    // Overflow: keep the first 76 tokens + force EOS at position 76. Matches
    // OpenAI's `tokenize()` behavior on over-long inputs.
    for i in 0..CLIP_CTX - 1 {
        out[i] = ids[i] as i64;
    }
    out[CLIP_CTX - 1] = CLIP_EOS as i64;
    out
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_ids_get_padded_and_eos_forced() {
        // BOS=49406, "hi"=[1, 2], EOS=49407 — simulate a minimal post-processor.
        let ids = vec![CLIP_BOS, 1, 2, CLIP_EOS];
        let v = pad_or_truncate(&ids);
        assert_eq!(v.len(), CLIP_CTX);
        assert_eq!(v[0], CLIP_BOS as i64);
        assert_eq!(v[1], 1);
        assert_eq!(v[2], 2);
        assert_eq!(v[3], CLIP_EOS as i64);
        // Rest padded with zeros.
        for slot in v.iter().skip(4) {
            assert_eq!(*slot, 0);
        }
    }

    #[test]
    fn eos_added_if_post_processor_skipped_it() {
        // Simulate a tokenizer.json without EOS post-processing.
        let ids = vec![CLIP_BOS, 10, 11, 12];
        let v = pad_or_truncate(&ids);
        assert_eq!(v[0], CLIP_BOS as i64);
        assert_eq!(v[4], CLIP_EOS as i64);
    }

    #[test]
    fn long_sequence_truncates_and_forces_eos() {
        // 100 tokens → last slot must be EOS, length 77.
        let ids: Vec<u32> = (0..100).map(|i| (i + 1) as u32).collect();
        let v = pad_or_truncate(&ids);
        assert_eq!(v.len(), CLIP_CTX);
        assert_eq!(v[0], 1);
        assert_eq!(v[CLIP_CTX - 1], CLIP_EOS as i64);
    }

    #[test]
    fn empty_input_still_emits_bos_eos() {
        let v = pad_or_truncate(&[]);
        assert_eq!(v.len(), CLIP_CTX);
        assert_eq!(v[0], CLIP_BOS as i64);
        assert_eq!(v[1], CLIP_EOS as i64);
    }

    #[test]
    fn exact_length_input_fits_without_truncation() {
        let mut ids = vec![CLIP_BOS];
        ids.extend((0..75).map(|i| i as u32 + 1000));
        ids.push(CLIP_EOS);
        assert_eq!(ids.len(), CLIP_CTX);
        let v = pad_or_truncate(&ids);
        assert_eq!(v.len(), CLIP_CTX);
        assert_eq!(v[0], CLIP_BOS as i64);
        assert_eq!(v[CLIP_CTX - 1], CLIP_EOS as i64);
    }
}
