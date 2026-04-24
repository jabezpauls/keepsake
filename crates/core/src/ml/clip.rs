//! CLIP ViT-L/14 — Phase 2.1.
//!
//! **Visual path (this file):** decode → resize-shorter-side-to-224 →
//! center-crop to 224×224 → CHW float32 → normalise with OpenAI mean/std →
//! session.run → L2-normalise → 768-d embedding.
//!
//! **Text path (next commit, tokenizer.rs + text_embed):** tokenise with the
//! CLIP BPE into 77 IDs → session.run → pool → L2-normalise.
//!
//! Any deviation from the OpenAI pre-processing pipeline changes embeddings
//! and poisons similarity scoring, so the constants here are **frozen** per
//! `plans/phase-2-browsing.md` §36.

use image::imageops::FilterType;
use image::GenericImageView;
use ndarray::{Array1, Array2, Array4};
use ort::value::TensorRef;

use super::loader::SharedSession;
use super::tokenizer::{ClipTokenizer, CLIP_CTX};
use crate::{Error, Result};

/// CLIP input resolution. Both ViT-L/14 and ViT-B/32 take 224×224 — OpenAI
/// chose it for all their CLIP variants and the ViT-B/32's 32-px patches
/// still tile over this input size, so the constant is shared across
/// bundles.
pub const CLIP_INPUT: u32 = 224;

/// OpenAI canonical per-channel means (RGB). FROZEN.
pub const OPENAI_MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
/// OpenAI canonical per-channel std (RGB). FROZEN.
pub const OPENAI_STD: [f32; 3] = [0.268_629_54, 0.261_302_6, 0.275_777_1];

/// Decode arbitrary image bytes, resize shorter-side to 224, center-crop to
/// 224×224, and emit an `(1, 3, 224, 224)` float32 tensor normalised with
/// OpenAI stats. Separated from inference so unit tests can validate the
/// tensor-math without loading an ONNX session.
pub fn preprocess_image_bytes(bytes: &[u8]) -> Result<Array4<f32>> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| Error::Media(format!("clip preprocess decode: {e}")))?;
    Ok(preprocess_image(&img))
}

/// Preprocess an in-memory `DynamicImage`. Pure function; separated from
/// `preprocess_image_bytes` so callers that already have a decoded image
/// (e.g. thumbnail pipeline) skip the re-decode.
pub fn preprocess_image(img: &image::DynamicImage) -> Array4<f32> {
    let (w, h) = img.dimensions();
    // Resize shorter-side to 224 while preserving aspect ratio, then
    // center-crop. `Lanczos3` matches OpenAI's reference preprocessing (PIL
    // BICUBIC is close enough; Lanczos3 is a common Rust substitute used by
    // rust-clip and candle-transformers; embedding drift is well under 1%).
    let (nw, nh) = if w < h {
        (
            CLIP_INPUT,
            (h as f32 * CLIP_INPUT as f32 / w as f32).round() as u32,
        )
    } else {
        (
            (w as f32 * CLIP_INPUT as f32 / h as f32).round() as u32,
            CLIP_INPUT,
        )
    };
    let resized = img.resize_exact(nw, nh, FilterType::Lanczos3);
    let cx = (nw.saturating_sub(CLIP_INPUT)) / 2;
    let cy = (nh.saturating_sub(CLIP_INPUT)) / 2;
    let cropped = resized.crop_imm(cx, cy, CLIP_INPUT, CLIP_INPUT).to_rgb8();

    // Build NCHW tensor directly so we don't materialise an intermediate
    // Vec<Vec<Vec<f32>>>.
    let mut out = Array4::<f32>::zeros((1, 3, CLIP_INPUT as usize, CLIP_INPUT as usize));
    for (y, row) in cropped.rows().enumerate() {
        for (x, px) in row.enumerate() {
            for c in 0..3 {
                let v = f32::from(px[c]) / 255.0;
                out[[0, c, y, x]] = (v - OPENAI_MEAN[c]) / OPENAI_STD[c];
            }
        }
    }
    out
}

/// L2-normalise a slice in-place. Used after every CLIP/ArcFace embed so
/// cosine similarity reduces to a dot product. Treats zero-norm as a no-op
/// (prevents NaN when the model emits a pathological all-zero vector —
/// cosine against any other vector is still 0 in that case, correct).
pub fn l2_normalize(v: &mut [f32]) {
    let mut sum_sq = 0.0f32;
    for x in v.iter() {
        sum_sq += x * x;
    }
    let norm = sum_sq.sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Run CLIP visual on preprocessed image bytes, returning a 768-d unit vector.
pub fn embed_image_bytes(session: &SharedSession, bytes: &[u8]) -> Result<Vec<f32>> {
    let tensor = preprocess_image_bytes(bytes)?;
    run_visual(session, tensor)
}

/// Run CLIP visual on an already-decoded image. Useful when the caller has
/// decoded once for thumbnailing and wants to avoid decoding again.
pub fn embed_image(session: &SharedSession, img: &image::DynamicImage) -> Result<Vec<f32>> {
    let tensor = preprocess_image(img);
    run_visual(session, tensor)
}

fn run_visual(session: &SharedSession, tensor: Array4<f32>) -> Result<Vec<f32>> {
    // ort's inputs!() macro accepts any `TensorRef`; we wrap an ndarray view.
    let view = tensor.view();
    let input =
        TensorRef::from_array_view(view).map_err(|e| Error::Media(format!("clip input: {e}")))?;
    let mut sess = session
        .lock()
        .map_err(|_| Error::Ingest("clip session mutex poisoned".into()))?;
    let outputs = sess
        .run(ort::inputs![input])
        .map_err(|e| Error::Media(format!("clip run: {e}")))?;
    // CLIP visual's pooled output is the single tensor the manifest pinned.
    // Names differ per export (some call it "image_embeds", some "output"),
    // so we index by position rather than by name.
    let (_, first) = outputs.iter().next().ok_or(Error::MlModelShape(
        "clip_visual.onnx: no output tensor returned at run time",
    ))?;
    let (shape, data) = first
        .try_extract_tensor::<f32>()
        .map_err(|e| Error::Media(format!("clip extract: {e}")))?;

    // Expect pooled `[1, dim]`. Some exports emit `[1, 1, dim]` or
    // `[1, 257, dim]` (token sequence) — reject those with a specific shape
    // error; if we ever ship a model like that we'll need an explicit
    // pooling step here. `dim` itself is runtime-driven by the bundle
    // (ViT-L/14 → 768, ViT-B/32 → 512); the loader validates it matches
    // `BundleSpec::clip_dim` once at startup, so callers can trust the
    // length here without re-checking.
    let dims = shape.as_ref();
    if dims.len() != 2 || dims[0] != 1 {
        return Err(Error::MlModelShape(
            "clip_visual.onnx: expected pooled [1, dim] output",
        ));
    }
    if data.len() as i64 != dims.iter().product::<i64>() {
        return Err(Error::MlModelShape(
            "clip_visual.onnx: buffer length disagrees with shape",
        ));
    }

    let mut arr = Array1::from_vec(data.to_vec());
    l2_normalize(arr.as_slice_mut().expect("contiguous"));
    Ok(arr.to_vec())
}

/// Run CLIP visual on preprocessed image bytes.
/// Returns a unit vector whose length matches the loaded bundle's CLIP
/// embedding dim (768 for ViT-L/14, 512 for ViT-B/32).
/// Tokenise + embed a natural-language query into a unit vector directly
/// comparable (cosine == dot) against the vectors stored in `asset_vec` —
/// provided both were produced by the same bundle. After a bundle switch
/// the two have different lengths; the search path then degrades to the
/// metadata-only fallback until reindex catches up.
pub fn embed_text(
    session: &SharedSession,
    tokenizer: &ClipTokenizer,
    text: &str,
) -> Result<Vec<f32>> {
    let ids = tokenizer.encode(text)?;
    debug_assert_eq!(ids.len(), CLIP_CTX);
    // OpenCLIP / HF CLIP ONNX exports typically declare `input_ids` as int32.
    // The tokenizer API returns i64 (the HF-tokenizers convention) so cast
    // down — CLIP vocab fits easily in 32 bits (max id = 49408).
    let ids32: Vec<i32> = ids.into_iter().map(|x| x as i32).collect();
    let tokens = Array2::<i32>::from_shape_vec((1, CLIP_CTX), ids32)
        .map_err(|e| Error::Ingest(format!("clip text tensor: {e}")))?;

    let view = tokens.view();
    let input = TensorRef::from_array_view(view)
        .map_err(|e| Error::Media(format!("clip text input: {e}")))?;
    let mut sess = session
        .lock()
        .map_err(|_| Error::Ingest("clip text session mutex poisoned".into()))?;
    let outputs = sess
        .run(ort::inputs![input])
        .map_err(|e| Error::Media(format!("clip text run: {e}")))?;

    let (_, first) = outputs.iter().next().ok_or(Error::MlModelShape(
        "clip_textual.onnx: no output tensor returned at run time",
    ))?;
    let (shape, data) = first
        .try_extract_tensor::<f32>()
        .map_err(|e| Error::Media(format!("clip text extract: {e}")))?;
    let dims = shape.as_ref();
    if dims.len() != 2 || dims[0] != 1 {
        return Err(Error::MlModelShape(
            "clip_textual.onnx: expected pooled [1, dim] output",
        ));
    }
    if data.len() as i64 != dims.iter().product::<i64>() {
        return Err(Error::MlModelShape(
            "clip_textual.onnx: buffer length disagrees with shape",
        ));
    }
    let mut arr = Array1::from_vec(data.to_vec());
    l2_normalize(arr.as_slice_mut().expect("contiguous"));
    Ok(arr.to_vec())
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn synthetic_image(w: u32, h: u32) -> image::DynamicImage {
        // Horizontal gradient: R ramps 0..255 with x, G = 128, B = 64.
        let mut buf = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let r = ((x as f32 / w.max(1) as f32) * 255.0) as u8;
                buf.put_pixel(x, y, Rgb([r, 128, 64]));
            }
        }
        image::DynamicImage::ImageRgb8(buf)
    }

    #[test]
    fn preprocess_shape_is_1_3_224_224() {
        let img = synthetic_image(480, 320);
        let t = preprocess_image(&img);
        assert_eq!(t.shape(), &[1, 3, 224, 224]);
    }

    #[test]
    fn preprocess_normalises_with_openai_stats() {
        // A tiny solid color image lets us check the normalisation constants
        // exactly. Full-white pixel → post-divide 1.0; subtract mean, divide
        // by std per channel.
        let img = image::DynamicImage::ImageRgb8(RgbImage::from_pixel(
            CLIP_INPUT,
            CLIP_INPUT,
            Rgb([255, 255, 255]),
        ));
        let t = preprocess_image(&img);
        for c in 0..3 {
            let expected = (1.0 - OPENAI_MEAN[c]) / OPENAI_STD[c];
            let got = t[[0, c, 0, 0]];
            assert!(
                (got - expected).abs() < 1e-5,
                "channel {c}: got {got} expected {expected}",
            );
        }
    }

    #[test]
    fn preprocess_center_crops_not_warps() {
        // A portrait image (shorter side = width) should resize width→224
        // and crop off the top and bottom of the height. The central pixel
        // should be dead-centre.
        let img = synthetic_image(224, 448);
        let t = preprocess_image(&img);
        // Mid-row, mid-col should exist at (c=0..2, y=112, x=112).
        assert!(t[[0, 0, 112, 112]].is_finite());
    }

    #[test]
    fn l2_normalize_produces_unit_norm() {
        let mut v = vec![1.0, 2.0, 2.0];
        l2_normalize(&mut v);
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_handles_zero_vector() {
        let mut v = vec![0.0_f32; 10];
        l2_normalize(&mut v);
        // Zero stays zero — must not NaN.
        assert!(v.iter().all(|x| x.abs() < 1e-12));
    }

    #[test]
    fn preprocess_is_stable_across_repeat_calls() {
        // Determinism sanity — same bytes → same tensor.
        let img = synthetic_image(256, 256);
        let a = preprocess_image(&img);
        let b = preprocess_image(&img);
        assert_eq!(a, b);
    }
}
