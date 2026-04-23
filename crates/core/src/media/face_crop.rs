//! Crop + resize + encode a face thumbnail from an already-decoded asset
//! thumb (e.g. thumb1024 WebP bytes).
//!
//! Used by the Tauri `person_face_thumbnail` command to produce the Apple-
//! Photos-style square face avatar the People grid displays. The bbox is in
//! the source thumb's pixel space (xywh), matching how `worker_exec` writes
//! it post-SCRFD-unletterbox.

use image::{imageops::FilterType, ImageFormat};

use crate::{Error, Result};

/// Crop a face out of `thumb_bytes` with a symmetric padding margin, resize
/// to a square `out_size × out_size`, and re-encode as WebP.
///
/// - `bbox_xywh`: face bounding box in thumb-pixel space as `[x, y, w, h]`.
/// - `padding_frac`: extra margin on each side expressed as a fraction of
///   the longer bbox dimension. `0.15` → roughly Immich's 120% / 20% total
///   padding; `0.3` → Apple Photos's slightly more generous framing.
///
/// Returns `Err` if the input bytes don't decode, the bbox is degenerate, or
/// WebP encoding fails. Callers typically fall back to the raw thumb.
pub fn crop_face_webp(
    thumb_bytes: &[u8],
    bbox_xywh: [f32; 4],
    out_size: u32,
    padding_frac: f32,
) -> Result<Vec<u8>> {
    let img = image::load_from_memory(thumb_bytes)
        .map_err(|e| Error::Media(format!("face thumb decode: {e}")))?;
    let iw = img.width() as f32;
    let ih = img.height() as f32;
    let [bx, by, bw, bh] = bbox_xywh;
    if !(bw > 0.0 && bh > 0.0 && iw > 0.0 && ih > 0.0) {
        return Err(Error::Media("face thumb: degenerate bbox/image".into()));
    }

    // Square crop around the face centre, sized by the longer bbox dimension
    // plus padding_frac on each side. Clamp against the thumb's bounds so the
    // crop rect stays inside the image — edge-on faces just get less padding
    // on the clipped side.
    let cx = bx + bw * 0.5;
    let cy = by + bh * 0.5;
    let side = bw.max(bh) * (1.0 + padding_frac * 2.0);
    let x0 = (cx - side * 0.5).clamp(0.0, iw);
    let y0 = (cy - side * 0.5).clamp(0.0, ih);
    let x1 = (cx + side * 0.5).clamp(0.0, iw);
    let y1 = (cy + side * 0.5).clamp(0.0, ih);
    let cw = (x1 - x0).round().max(1.0) as u32;
    let ch = (y1 - y0).round().max(1.0) as u32;
    let cx0 = x0.round() as u32;
    let cy0 = y0.round() as u32;

    let cropped = img.crop_imm(cx0, cy0, cw, ch);
    let resized = cropped.resize_exact(out_size, out_size, FilterType::Lanczos3);

    let mut out = Vec::with_capacity(16 * 1024);
    let mut cur = std::io::Cursor::new(&mut out);
    resized
        .write_to(&mut cur, ImageFormat::WebP)
        .map_err(|e| Error::Media(format!("face thumb webp encode: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn source_webp(w: u32, h: u32) -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(w, h, |x, y| {
            // Distinct stripes so the crop rect is visually verifiable.
            Rgb([(x % 255) as u8, (y % 255) as u8, ((x + y) % 255) as u8])
        });
        let mut out = Vec::new();
        let mut cur = std::io::Cursor::new(&mut out);
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut cur, ImageFormat::WebP)
            .unwrap();
        out
    }

    #[test]
    fn crop_produces_square_at_requested_size() {
        let src = source_webp(400, 300);
        let bytes = crop_face_webp(&src, [100.0, 50.0, 80.0, 80.0], 128, 0.2).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.width(), 128);
        assert_eq!(decoded.height(), 128);
    }

    #[test]
    fn crop_clamps_bbox_near_edge() {
        // bbox partly off the right edge — crop must clamp, not panic.
        let src = source_webp(200, 200);
        let bytes = crop_face_webp(&src, [180.0, 90.0, 40.0, 40.0], 64, 0.3).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.width(), 64);
        assert_eq!(decoded.height(), 64);
    }

    #[test]
    fn crop_rejects_degenerate_bbox() {
        let src = source_webp(100, 100);
        let err = crop_face_webp(&src, [0.0, 0.0, 0.0, 0.0], 64, 0.1);
        assert!(err.is_err());
    }
}
