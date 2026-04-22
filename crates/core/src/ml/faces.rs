//! Face detection (SCRFD) + embedding (ArcFace) — Phase 2.1.
//!
//! This file ships the pieces that can be unit-tested without model weights:
//! IoU, NMS, anchor-grid decoding, and the bounding-box data model. The
//! session-calling surfaces (`detect_faces`, `embed_face`) are Tier-B: they
//! need a real `ort::Session`, so their round-trip tests live in
//! `tests/integration/faces.rs` under `#[ignore]`.
//!
//! Contracts (plans/phase-2-browsing.md §36):
//! - **SCRFD** is the 10G BNKPS export with 9 output tensors: three strides
//!   (8, 16, 32) × {score, bbox, kps}. 2 anchors per spatial location.
//! - **Face bbox** storage: `(x, y, w, h)` in original-image pixel space.
//! - **ArcFace** is 112×112 NCHW f32, BGR, centered at (127.5, 127.5, 127.5)
//!   divided by 128, returns 512-d unit vector.

use std::cmp::Ordering;

/// One detected face in the original-image coordinate system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceDetection {
    /// xywh in image-pixel space.
    pub bbox: [f32; 4],
    /// Five-point landmarks (left-eye, right-eye, nose, mouth-left, mouth-right).
    pub landmarks: [(f32, f32); 5],
    /// SCRFD confidence 0..1.
    pub score: f32,
}

/// SCRFD strides the 10G export emits heads for. FROZEN.
pub const SCRFD_STRIDES: [u32; 3] = [8, 16, 32];
/// SCRFD anchors per spatial location. FROZEN.
pub const SCRFD_NUM_ANCHORS: usize = 2;
/// SCRFD input size. FROZEN per the 10G BNKPS export.
pub const SCRFD_INPUT: u32 = 640;

/// Default SCRFD score threshold. Callers override per-scene if needed.
pub const DEFAULT_SCORE_THR: f32 = 0.5;
/// Default NMS IoU threshold.
pub const DEFAULT_NMS_IOU: f32 = 0.4;

/// One SCRFD detection candidate in network-input coordinates (the 640×640
/// letterboxed frame). `decode_stride` emits these before NMS + inverse-
/// letterbox; `detect_faces` hands the surviving ones back in original
/// image space.
#[derive(Debug, Clone, Copy)]
pub struct RawDetection {
    pub bbox: [f32; 4], // x1, y1, x2, y2 (corners, not xywh)
    pub landmarks: [(f32, f32); 5],
    pub score: f32,
}

/// Intersection-over-union on xyxy (corner-form) boxes.
pub fn iou_xyxy(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let ix1 = a[0].max(b[0]);
    let iy1 = a[1].max(b[1]);
    let ix2 = a[2].min(b[2]);
    let iy2 = a[3].min(b[3]);
    let iw = (ix2 - ix1).max(0.0);
    let ih = (iy2 - iy1).max(0.0);
    let inter = iw * ih;
    let area_a = (a[2] - a[0]).max(0.0) * (a[3] - a[1]).max(0.0);
    let area_b = (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
    let union = area_a + area_b - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Standard greedy NMS over a slice of candidates. Returns surviving
/// detections in score-descending order.
///
/// Threshold is cosine-independent: purely spatial on bounding boxes.
pub fn nms(mut candidates: Vec<RawDetection>, iou_thr: f32) -> Vec<RawDetection> {
    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    let mut keep: Vec<RawDetection> = Vec::with_capacity(candidates.len());
    'outer: for cand in candidates {
        for kept in &keep {
            if iou_xyxy(&cand.bbox, &kept.bbox) > iou_thr {
                continue 'outer;
            }
        }
        keep.push(cand);
    }
    keep
}

/// Decode one SCRFD head triplet into candidate detections *before* NMS.
///
/// `scores` has shape `(H*W*num_anchors,)` or `(H*W*num_anchors, 1)`.
/// `bbox_preds` is 4-column flat: `(H*W*num_anchors, 4)` offsets as
/// `(left, top, right, bottom)` distances from anchor center, in stride
/// units. `kps_preds` is 10-column: `(H*W*num_anchors, 10)`.
///
/// All three must be the same row-count. `stride` scales offsets into
/// network-input pixels, and `(feat_h, feat_w)` give the grid shape.
///
/// Returned boxes live in 640×640 network-input coordinates; the caller is
/// responsible for inverse-letterboxing into the original image.
#[allow(clippy::too_many_arguments)]
pub fn decode_stride(
    scores: &[f32],
    bbox_preds: &[f32],
    kps_preds: &[f32],
    feat_h: u32,
    feat_w: u32,
    stride: u32,
    num_anchors: usize,
    score_thr: f32,
) -> Vec<RawDetection> {
    let cells = (feat_h * feat_w) as usize;
    let n = cells * num_anchors;
    debug_assert_eq!(scores.len(), n);
    debug_assert_eq!(bbox_preds.len(), n * 4);
    debug_assert_eq!(kps_preds.len(), n * 10);

    let mut out = Vec::with_capacity(32);
    let s = stride as f32;
    for y in 0..feat_h {
        for x in 0..feat_w {
            for a in 0..num_anchors {
                let idx = ((y * feat_w + x) as usize) * num_anchors + a;
                let score = scores[idx];
                if score < score_thr {
                    continue;
                }
                // Anchor center — SCRFD pre-expands by stride so the center
                // sits at the pixel grid, not the cell corner.
                let cx = x as f32 * s;
                let cy = y as f32 * s;

                let bl = bbox_preds[idx * 4] * s;
                let bt = bbox_preds[idx * 4 + 1] * s;
                let br = bbox_preds[idx * 4 + 2] * s;
                let bb = bbox_preds[idx * 4 + 3] * s;
                let bbox = [cx - bl, cy - bt, cx + br, cy + bb];

                let mut lms = [(0.0f32, 0.0f32); 5];
                for (k, lm) in lms.iter_mut().enumerate() {
                    let kx = kps_preds[idx * 10 + k * 2] * s + cx;
                    let ky = kps_preds[idx * 10 + k * 2 + 1] * s + cy;
                    *lm = (kx, ky);
                }

                out.push(RawDetection {
                    bbox,
                    landmarks: lms,
                    score,
                });
            }
        }
    }
    out
}

/// Invert the letterbox (isotropic resize + center-pad) applied during
/// preprocessing. `scale` is `network_input / max(orig_w, orig_h)`.
/// `(pad_x, pad_y)` are the left/top margins in network-input pixels.
///
/// Converts raw network-space detections into final xywh bounding boxes in
/// original-image coordinates plus original-space landmarks.
pub fn unletterbox(raw: &RawDetection, scale: f32, pad_x: f32, pad_y: f32) -> FaceDetection {
    let inv = 1.0 / scale;
    let x1 = (raw.bbox[0] - pad_x) * inv;
    let y1 = (raw.bbox[1] - pad_y) * inv;
    let x2 = (raw.bbox[2] - pad_x) * inv;
    let y2 = (raw.bbox[3] - pad_y) * inv;
    let mut lms = [(0.0f32, 0.0f32); 5];
    for (i, (kx, ky)) in raw.landmarks.iter().enumerate() {
        lms[i] = ((*kx - pad_x) * inv, (*ky - pad_y) * inv);
    }
    FaceDetection {
        bbox: [x1, y1, (x2 - x1).max(0.0), (y2 - y1).max(0.0)],
        landmarks: lms,
        score: raw.score,
    }
}

/// Compute `(scale, pad_x, pad_y)` for a letterboxed input of
/// `input_size × input_size` from an original `(orig_w, orig_h)`.
pub fn letterbox_params(orig_w: u32, orig_h: u32, input_size: u32) -> (f32, f32, f32) {
    let s = input_size as f32 / orig_w.max(orig_h) as f32;
    let new_w = orig_w as f32 * s;
    let new_h = orig_h as f32 * s;
    let pad_x = (input_size as f32 - new_w) / 2.0;
    let pad_y = (input_size as f32 - new_h) / 2.0;
    (s, pad_x, pad_y)
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(bbox: [f32; 4], score: f32) -> RawDetection {
        RawDetection {
            bbox,
            landmarks: [(0.0, 0.0); 5],
            score,
        }
    }

    #[test]
    fn iou_identical_is_one_disjoint_is_zero() {
        let a = [0.0, 0.0, 10.0, 10.0];
        assert!((iou_xyxy(&a, &a) - 1.0).abs() < 1e-6);

        let b = [100.0, 100.0, 110.0, 110.0];
        assert!(iou_xyxy(&a, &b) < 1e-6);
    }

    #[test]
    fn iou_half_overlap_is_one_third() {
        // Two 10x10 boxes overlapping in a 10x5 strip → intersection 50,
        // union 10*10 + 10*10 - 50 = 150, IoU = 1/3.
        let a = [0.0, 0.0, 10.0, 10.0];
        let b = [0.0, 5.0, 10.0, 15.0];
        assert!((iou_xyxy(&a, &b) - (1.0 / 3.0)).abs() < 1e-5);
    }

    #[test]
    fn nms_keeps_best_and_suppresses_overlap() {
        let cands = vec![
            raw([0.0, 0.0, 10.0, 10.0], 0.95),
            raw([1.0, 1.0, 11.0, 11.0], 0.90), // high-overlap with #1
            raw([100.0, 100.0, 110.0, 110.0], 0.85), // disjoint, keep
            raw([2.0, 2.0, 12.0, 12.0], 0.60), // also overlaps with #1
        ];
        let kept = nms(cands, DEFAULT_NMS_IOU);
        assert_eq!(kept.len(), 2);
        // Sorted by score.
        assert!((kept[0].score - 0.95).abs() < 1e-6);
        assert!((kept[1].score - 0.85).abs() < 1e-6);
    }

    #[test]
    fn nms_leaves_non_overlapping_alone() {
        let cands = vec![
            raw([0.0, 0.0, 5.0, 5.0], 0.9),
            raw([10.0, 10.0, 15.0, 15.0], 0.8),
            raw([20.0, 20.0, 25.0, 25.0], 0.7),
        ];
        assert_eq!(nms(cands, 0.4).len(), 3);
    }

    #[test]
    fn decode_stride_emits_expected_box_for_single_hot_anchor() {
        // 2×2 feature map, 1 anchor, stride 8. Hot cell at (y=1, x=1) with
        // score 0.9 and offsets (l=1, t=1, r=2, b=2) → anchor center
        // (x=8, y=8), box = (8 - 8, 8 - 8, 8 + 16, 8 + 16) = (0, 0, 24, 24).
        let feat_h = 2;
        let feat_w = 2;
        let mut scores = vec![0.0; 4];
        let mut bboxes = vec![0.0; 16];
        let kps = vec![0.0; 40];
        scores[3] = 0.9;
        bboxes[12] = 1.0;
        bboxes[13] = 1.0;
        bboxes[14] = 2.0;
        bboxes[15] = 2.0;

        let dets = decode_stride(&scores, &bboxes, &kps, feat_h, feat_w, 8, 1, 0.5);
        assert_eq!(dets.len(), 1);
        let d = dets[0];
        assert!((d.bbox[0] - 0.0).abs() < 1e-4);
        assert!((d.bbox[1] - 0.0).abs() < 1e-4);
        assert!((d.bbox[2] - 24.0).abs() < 1e-4);
        assert!((d.bbox[3] - 24.0).abs() < 1e-4);
        assert!((d.score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn decode_stride_respects_score_threshold() {
        let scores = vec![0.4, 0.6, 0.3, 0.9];
        let bboxes = vec![0.0; 16];
        let kps = vec![0.0; 40];
        let dets = decode_stride(&scores, &bboxes, &kps, 2, 2, 8, 1, 0.5);
        assert_eq!(dets.len(), 2); // 0.6 and 0.9 survive
    }

    #[test]
    fn letterbox_params_square_input_is_identity() {
        let (s, px, py) = letterbox_params(640, 640, 640);
        assert!((s - 1.0).abs() < 1e-6);
        assert!((px - 0.0).abs() < 1e-6);
        assert!((py - 0.0).abs() < 1e-6);
    }

    #[test]
    fn letterbox_params_landscape_pads_top_and_bottom() {
        // 1280×640 → scale 0.5 → fits exactly, no padding.
        let (s, px, py) = letterbox_params(1280, 640, 640);
        assert!((s - 0.5).abs() < 1e-6);
        assert!((px - 0.0).abs() < 1e-6);
        assert!((py - 160.0).abs() < 1e-6);
    }

    #[test]
    fn unletterbox_round_trips_a_known_box() {
        // Original image 1280×640 → scale 0.5 → pad (0, 160). A raw 640-space
        // box at (160, 320, 480, 420) unletterboxes back to a 1280-space
        // xywh box at (320, 320, 640, 200).
        let raw = super::RawDetection {
            bbox: [160.0, 320.0, 480.0, 420.0],
            landmarks: [(0.0, 0.0); 5],
            score: 0.99,
        };
        let f = unletterbox(&raw, 0.5, 0.0, 160.0);
        assert!((f.bbox[0] - 320.0).abs() < 1e-4);
        assert!((f.bbox[1] - 320.0).abs() < 1e-4);
        assert!((f.bbox[2] - 640.0).abs() < 1e-4);
        assert!((f.bbox[3] - 200.0).abs() < 1e-4);
    }
}
