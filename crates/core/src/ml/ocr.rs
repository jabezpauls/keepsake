//! PaddleOCR pipeline (D5 follow-up).
//!
//! Three-stage pipeline:
//! 1. **Detection** (`ocr_det.onnx`, DBNet-style) — produces per-line
//!    axis-aligned bounding boxes from a full-res image.
//! 2. **Classification** (`ocr_cls.onnx`) — optional 0°/180° orientation
//!    gate. Rotates crops so recognition gets upright glyphs.
//! 3. **Recognition** (`ocr_rec.onnx`, CRNN-style) — per-crop CTC decode
//!    to UTF-8 strings.
//!
//! The full runner isn't implemented in this slice — weight-dependent
//! image-to-text pipelines need validated fixtures + a pinned CTC
//! decoder. What this module DOES ship today:
//!
//! - Job kind wiring (`MlJobKind::Ocr` queued at ingest for image
//!   assets).
//! - `OcrLine` shape + JSON storage format under `asset.ocr_text_ct`.
//! - `index_asset_tokens_from_ocr` — given already-recognised text,
//!   tokenise + HMAC + push into the `asset_text` FTS5 table via
//!   `crate::blind_text::index_asset_tokens`. Callers (tests, a future
//!   runner) hand it recognised text; the blind-FTS path is identical
//!   to what user-typed captions already exercise.
//!
//! A follow-up slice will add `fn ocr_image(runtime, bytes) ->
//! Vec<OcrLine>` once weights + fixtures are in place.

use serde::{Deserialize, Serialize};

use crate::blind_text;
use crate::crypto::MasterKey;
use crate::Result;

/// One recognised line of text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrLine {
    /// Recognised UTF-8 text.
    pub text: String,
    /// Axis-aligned bbox in original-image pixels `[x0, y0, x1, y1]`.
    pub bbox: [u32; 4],
    /// CTC confidence, in [0, 1]. Callers may drop low-confidence lines.
    pub conf: f32,
}

/// Serialise a set of lines into the JSON layout we persist in
/// `asset.ocr_text_ct`. Returns bytes ready for `seal_row`.
pub fn serialize_lines(lines: &[OcrLine]) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(lines).unwrap_or_default())
}

/// Tokenise recognised text + push blind tokens into `asset_text` for
/// the given asset. Same contract as the manual-caption path —
/// whichever source wrote most recently wins (index is one-row-per-
/// asset; a future scope column would let OCR + user captions
/// coexist).
pub fn index_asset_tokens_from_ocr(
    conn: &rusqlite::Connection,
    asset_id: i64,
    master_key: &MasterKey,
    lines: &[OcrLine],
) -> Result<usize> {
    let sk = blind_text::derive_search_key(master_key);
    let mut bag: Vec<String> = Vec::new();
    for line in lines {
        bag.extend(blind_text::tokenize(&line.text));
    }
    // Dedup across lines — blind_text::index_asset_tokens already
    // dedupes, but it's cheap to do it here so the output byte size
    // is minimal.
    bag.sort();
    bag.dedup();
    blind_text::index_asset_tokens(conn, asset_id, &sk, &bag)
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn open() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate::apply(&conn).unwrap();
        conn
    }

    #[test]
    fn serialize_roundtrips() {
        let lines = vec![
            OcrLine {
                text: "Latte".into(),
                bbox: [10, 20, 80, 40],
                conf: 0.97,
            },
            OcrLine {
                text: "with oat milk".into(),
                bbox: [10, 45, 160, 65],
                conf: 0.91,
            },
        ];
        let bytes = serialize_lines(&lines).unwrap();
        let parsed: Vec<OcrLine> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].text, "Latte");
    }

    #[test]
    fn indexes_ocr_tokens_so_search_hits() {
        let conn = open();
        let mk = MasterKey::from_bytes([5u8; 32]);
        let lines = vec![
            OcrLine {
                text: "BOARDING PASS — GATE 42".into(),
                bbox: [0; 4],
                conf: 0.95,
            },
            OcrLine {
                text: "Flight SQ22 non-stop".into(),
                bbox: [0; 4],
                conf: 0.88,
            },
        ];
        let n = index_asset_tokens_from_ocr(&conn, 1, &mk, &lines).unwrap();
        assert!(n > 0);

        let sk = blind_text::derive_search_key(&mk);
        // Exact words from the OCR'd text match.
        assert_eq!(
            blind_text::search_assets(&conn, &sk, "boarding", 10).unwrap(),
            vec![1]
        );
        assert_eq!(
            blind_text::search_assets(&conn, &sk, "flight sq22", 10).unwrap(),
            vec![1]
        );
        // Unrelated word misses.
        assert!(blind_text::search_assets(&conn, &sk, "menu", 10)
            .unwrap()
            .is_empty());
    }
}
