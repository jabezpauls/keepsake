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

use ndarray::{Array1, Array4};
use ort::value::TensorRef;

use super::clip::l2_normalize;
use super::loader::SharedSession;
use crate::{Error, Result};

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

/// ArcFace 112×112 destination template (5 points: left-eye, right-eye,
/// nose, mouth-left, mouth-right). FROZEN — changing these shifts every
/// embedding and invalidates the stored face index. See InsightFace
/// reference for provenance.
pub const ARCFACE_TEMPLATE: [(f32, f32); 5] = [
    (38.2946, 51.6963),
    (73.5318, 51.5014),
    (56.0252, 71.7366),
    (41.5493, 92.3655),
    (70.7299, 92.2041),
];

/// ArcFace input crop size (pixels, square). FROZEN.
pub const ARCFACE_INPUT: u32 = 112;
/// ArcFace embedding dimension. FROZEN per architecture.md §4.2.
pub const ARCFACE_DIM: usize = 512;

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

/// 2D similarity transform (rotation + uniform scale + translation) from
/// source → destination point sets, in the Procrustes / closed-form variant
/// (Umeyama without the SVD — for similarity transforms this reduces to
/// elementary sums). Returns a 2×3 affine matrix stored row-major as
/// `[a, b, tx,  c, d, ty]` such that `dst = M * [src; 1]`.
///
/// When all source points are identical (degenerate), returns a pure
/// translation that places them at the destination centroid.
pub fn similarity_transform(src: &[(f32, f32); 5], dst: &[(f32, f32); 5]) -> [f32; 6] {
    let n = src.len() as f32;
    let sx: f32 = src.iter().map(|p| p.0).sum::<f32>() / n;
    let sy: f32 = src.iter().map(|p| p.1).sum::<f32>() / n;
    let dx: f32 = dst.iter().map(|p| p.0).sum::<f32>() / n;
    let dy: f32 = dst.iter().map(|p| p.1).sum::<f32>() / n;

    let mut num_cos = 0.0f32;
    let mut num_sin = 0.0f32;
    let mut denom = 0.0f32;
    for (s, d) in src.iter().zip(dst.iter()) {
        let (ax, ay) = (s.0 - sx, s.1 - sy);
        let (bx, by) = (d.0 - dx, d.1 - dy);
        num_cos += ax * bx + ay * by;
        num_sin += ax * by - ay * bx;
        denom += ax * ax + ay * ay;
    }
    let (a, b) = if denom < 1e-12 {
        (1.0, 0.0) // degenerate — skip rotation/scale, rely on translation below.
    } else {
        (num_cos / denom, num_sin / denom)
    };
    // a = s·cos θ, b = s·sin θ. So 2×3 is [[a, -b, tx], [b, a, ty]].
    let tx = dx - (a * sx - b * sy);
    let ty = dy - (b * sx + a * sy);
    [a, -b, tx, b, a, ty]
}

/// Apply a 2×3 similarity to a point.
pub fn apply_affine(m: &[f32; 6], p: (f32, f32)) -> (f32, f32) {
    (
        m[0] * p.0 + m[1] * p.1 + m[2],
        m[3] * p.0 + m[4] * p.1 + m[5],
    )
}

/// Invert a similarity matrix (rotation + uniform scale + translation).
/// Returns `None` when the transform collapses (zero scale).
pub fn invert_similarity(m: &[f32; 6]) -> Option<[f32; 6]> {
    // M = [[a, -b, tx], [b, a, ty]]; det = a² + b².
    let a = m[0];
    let neg_b = m[1];
    let b = m[3];
    // Consistency assertion: m[4] should equal a. If it doesn't we're past a
    // pure similarity and the cheap inverse isn't valid.
    debug_assert!((m[4] - a).abs() < 1e-4);
    debug_assert!((m[1] + b).abs() < 1e-4);
    let det = a * a + b * b;
    if det < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let ia = a * inv;
    let ib = -b * inv;
    // Inverse of a similarity: rotation^T / s² with translation (-R^T t / s²).
    // Written out in M-row form:
    let itx = -(ia * m[2] - ib * m[5]);
    let ity = -(ib * m[2] + ia * m[5]);
    let _ = neg_b; // kept for the debug_assert only
    Some([ia, -ib, itx, ib, ia, ity])
}

/// Sample an RGB8 image at sub-pixel coordinates using bilinear
/// interpolation. Returns black for out-of-bounds reads (matches ArcFace's
/// reference warpAffine behaviour with BORDER_CONSTANT=0).
pub fn bilinear_sample(img: &image::RgbImage, x: f32, y: f32) -> [u8; 3] {
    let w = img.width() as i32;
    let h = img.height() as i32;
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let x1 = x0 + 1;
    let y1 = y0 + 1;
    let dx = x - x0 as f32;
    let dy = y - y0 as f32;

    let get = |gx: i32, gy: i32| -> [f32; 3] {
        if gx < 0 || gy < 0 || gx >= w || gy >= h {
            [0.0, 0.0, 0.0]
        } else {
            let p = img.get_pixel(gx as u32, gy as u32);
            [f32::from(p[0]), f32::from(p[1]), f32::from(p[2])]
        }
    };
    let p00 = get(x0, y0);
    let p10 = get(x1, y0);
    let p01 = get(x0, y1);
    let p11 = get(x1, y1);
    let mut out = [0u8; 3];
    for c in 0..3 {
        let v = p00[c] * (1.0 - dx) * (1.0 - dy)
            + p10[c] * dx * (1.0 - dy)
            + p01[c] * (1.0 - dx) * dy
            + p11[c] * dx * dy;
        out[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Align a detected face into a 112×112 ArcFace-ready crop using the
/// detected 5-point landmarks and the canonical destination template.
///
/// Returns the aligned crop as an `RgbImage` — callers decide whether to
/// convert to the ArcFace input tensor (`arcface_preprocess`) or save
/// for debugging.
pub fn align_face(img: &image::RgbImage, landmarks: &[(f32, f32); 5]) -> image::RgbImage {
    let m = similarity_transform(landmarks, &ARCFACE_TEMPLATE);
    // Warp by applying M^-1 to each output pixel and sampling source.
    let inv = invert_similarity(&m).unwrap_or([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    let mut out = image::RgbImage::new(ARCFACE_INPUT, ARCFACE_INPUT);
    for y in 0..ARCFACE_INPUT {
        for x in 0..ARCFACE_INPUT {
            let (sx, sy) = apply_affine(&inv, (x as f32, y as f32));
            let px = bilinear_sample(img, sx, sy);
            out.put_pixel(x, y, image::Rgb(px));
        }
    }
    out
}

/// Build the ArcFace input tensor from an aligned 112×112 RGB crop.
/// InsightFace convention: BGR channel order, normalised by (x - 127.5) / 128
/// per channel, shape (1, 3, 112, 112).
pub fn arcface_preprocess(aligned: &image::RgbImage) -> Array4<f32> {
    debug_assert_eq!(aligned.width(), ARCFACE_INPUT);
    debug_assert_eq!(aligned.height(), ARCFACE_INPUT);
    let mut t = Array4::<f32>::zeros((1, 3, ARCFACE_INPUT as usize, ARCFACE_INPUT as usize));
    for (y, row) in aligned.rows().enumerate() {
        for (x, px) in row.enumerate() {
            // RGB → BGR reorder on the channel axis.
            t[[0, 0, y, x]] = (f32::from(px[2]) - 127.5) / 128.0;
            t[[0, 1, y, x]] = (f32::from(px[1]) - 127.5) / 128.0;
            t[[0, 2, y, x]] = (f32::from(px[0]) - 127.5) / 128.0;
        }
    }
    t
}

/// Build the SCRFD input tensor from an `RgbImage` using letterbox
/// preprocessing: isotropic scale to fit in `SCRFD_INPUT × SCRFD_INPUT`, then
/// pad with black to center the content. Returns the tensor plus the
/// `(scale, pad_x, pad_y)` needed to invert the transform on output boxes.
///
/// Channel order: BGR (InsightFace convention). Per-channel normalise
/// `(x - 127.5) / 128`.
pub fn scrfd_preprocess(img: &image::RgbImage) -> (Array4<f32>, f32, f32, f32) {
    let (scale, pad_x, pad_y) = letterbox_params(img.width(), img.height(), SCRFD_INPUT);
    let new_w = (img.width() as f32 * scale).round() as u32;
    let new_h = (img.height() as f32 * scale).round() as u32;
    let resized = image::imageops::resize(img, new_w, new_h, image::imageops::FilterType::Triangle);

    let side = SCRFD_INPUT as usize;
    let mut t = Array4::<f32>::zeros((1, 3, side, side));
    // Fill the padded area with the normalised equivalent of 0 (black): each
    // channel becomes (0 - 127.5) / 128 = -0.9961...
    let pad_norm = (0.0 - 127.5) / 128.0;
    t.fill(pad_norm);

    let ox = pad_x.round() as i32;
    let oy = pad_y.round() as i32;
    for (y, row) in resized.rows().enumerate() {
        for (x, px) in row.enumerate() {
            let dst_x = ox + x as i32;
            let dst_y = oy + y as i32;
            if dst_x < 0 || dst_y < 0 || dst_x >= side as i32 || dst_y >= side as i32 {
                continue;
            }
            t[[0, 0, dst_y as usize, dst_x as usize]] = (f32::from(px[2]) - 127.5) / 128.0;
            t[[0, 1, dst_y as usize, dst_x as usize]] = (f32::from(px[1]) - 127.5) / 128.0;
            t[[0, 2, dst_y as usize, dst_x as usize]] = (f32::from(px[0]) - 127.5) / 128.0;
        }
    }
    (t, scale, pad_x, pad_y)
}

/// SCRFD end-to-end on an `RgbImage` — preprocess, run, decode every stride
/// head, NMS, inverse-letterbox. Returns surviving detections in the input
/// image's pixel space.
///
/// `session` must be the 10G BNKPS export (9 output tensors). The loader
/// asserts this at load time; if you're here with anything else we've
/// already failed with `Error::MlModelShape`.
pub fn detect_faces(session: &SharedSession, img: &image::RgbImage) -> Result<Vec<FaceDetection>> {
    let (tensor, scale, pad_x, pad_y) = scrfd_preprocess(img);
    let view = tensor.view();
    let input =
        TensorRef::from_array_view(view).map_err(|e| Error::Media(format!("scrfd input: {e}")))?;
    let mut sess = session
        .lock()
        .map_err(|_| Error::Ingest("scrfd session mutex poisoned".into()))?;
    let outputs = sess
        .run(ort::inputs![input])
        .map_err(|e| Error::Media(format!("scrfd run: {e}")))?;

    // SCRFD 10G BNKPS emits heads in canonical order:
    // [score_8, score_16, score_32, bbox_8, bbox_16, bbox_32, kps_8, kps_16, kps_32].
    // We extract each as a flat f32 slice.
    let mut heads: Vec<(Vec<i64>, Vec<f32>)> = Vec::with_capacity(9);
    for (_, value) in &outputs {
        let (shape, data) = value
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::Media(format!("scrfd extract: {e}")))?;
        heads.push((shape.as_ref().to_vec(), data.to_vec()));
    }
    // Drop SessionOutputs before releasing the session lock.
    drop(outputs);
    drop(sess);
    if heads.len() != 9 {
        return Err(Error::MlModelShape(
            "scrfd.onnx: expected 9 output heads at run time",
        ));
    }

    let mut candidates: Vec<RawDetection> = Vec::new();
    for (i, &stride) in SCRFD_STRIDES.iter().enumerate() {
        let feat = SCRFD_INPUT / stride;
        let scores = &heads[i].1;
        let bboxes = &heads[i + 3].1;
        let kps = &heads[i + 6].1;
        candidates.extend(decode_stride(
            scores,
            bboxes,
            kps,
            feat,
            feat,
            stride,
            SCRFD_NUM_ANCHORS,
            DEFAULT_SCORE_THR,
        ));
    }
    let kept = nms(candidates, DEFAULT_NMS_IOU);

    Ok(kept
        .into_iter()
        .map(|d| unletterbox(&d, scale, pad_x, pad_y))
        .collect())
}

/// Full align + embed. Returns a 512-d unit vector.
pub fn embed_face(
    session: &SharedSession,
    full_img: &image::RgbImage,
    landmarks: &[(f32, f32); 5],
) -> Result<Vec<f32>> {
    let aligned = align_face(full_img, landmarks);
    let tensor = arcface_preprocess(&aligned);
    let view = tensor.view();
    let input = TensorRef::from_array_view(view)
        .map_err(|e| Error::Media(format!("arcface input: {e}")))?;
    let mut sess = session
        .lock()
        .map_err(|_| Error::Ingest("arcface session mutex poisoned".into()))?;
    let outputs = sess
        .run(ort::inputs![input])
        .map_err(|e| Error::Media(format!("arcface run: {e}")))?;
    let (_, first) = outputs.iter().next().ok_or(Error::MlModelShape(
        "arcface.onnx: no output tensor at run time",
    ))?;
    let (shape, data) = first
        .try_extract_tensor::<f32>()
        .map_err(|e| Error::Media(format!("arcface extract: {e}")))?;
    let last = *shape
        .as_ref()
        .last()
        .ok_or(Error::MlModelShape("arcface.onnx: empty output shape"))?;
    if last != ARCFACE_DIM as i64 || data.len() != ARCFACE_DIM {
        return Err(Error::MlModelShape(
            "arcface.onnx: expected 512-d pooled output",
        ));
    }
    let mut arr = Array1::from_vec(data.to_vec());
    l2_normalize(arr.as_slice_mut().expect("contiguous"));
    Ok(arr.to_vec())
}

// =========== CLUSTERING =======================================================

/// DBSCAN cluster label. `-1` conventionally means noise (below min_samples
/// in any reachable neighbourhood). Stored as i32 for easy round-trip with
/// sqlite.
pub type ClusterId = i32;

/// Cosine similarity between two unit vectors. Because embeddings coming out
/// of `clip::l2_normalize` / `embed_face` are already L2-normalised this is
/// just a dot product; we still divide by norms defensively.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = (na * nb).sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        dot / denom
    }
}

/// DBSCAN on cosine distance (`1 - cosine_similarity`). Inputs are assumed
/// already L2-normalised (we don't force-normalise to let callers share with
/// other callers that need raw vectors).
///
/// `eps` is in cosine-distance units — e.g. 0.4 for the face-clustering
/// default. `min_samples` is the minimum reachable-neighbourhood size for
/// a point to be considered core (2 by default).
///
/// Returns one label per input point. Clusters are numbered from 0 in
/// discovery order; noise points are `-1`.
pub fn dbscan_cosine(vectors: &[Vec<f32>], eps: f32, min_samples: usize) -> Vec<ClusterId> {
    let n = vectors.len();
    let mut labels: Vec<ClusterId> = vec![-1; n];
    let mut visited = vec![false; n];
    let mut cluster_id: ClusterId = 0;

    // Precompute neighbourhoods. Face libraries are O(K) points (tens of
    // thousands), so O(N²) distance is fine; we avoid redoing it per seed.
    let distance = |i: usize, j: usize| -> f32 { 1.0 - cosine(&vectors[i], &vectors[j]) };

    let region_query = |i: usize| -> Vec<usize> {
        (0..n)
            .filter(|&j| j != i && distance(i, j) <= eps)
            .collect()
    };

    for i in 0..n {
        if visited[i] {
            continue;
        }
        visited[i] = true;
        let neighbours = region_query(i);
        if neighbours.len() + 1 < min_samples {
            // Noise (for now; may get picked up as a border point later).
            continue;
        }
        // Expand cluster.
        labels[i] = cluster_id;
        let mut stack = neighbours;
        while let Some(j) = stack.pop() {
            if !visited[j] {
                visited[j] = true;
                let nb = region_query(j);
                if nb.len() + 1 >= min_samples {
                    stack.extend(nb);
                }
            }
            if labels[j] == -1 {
                labels[j] = cluster_id;
            }
        }
        cluster_id += 1;
    }
    labels
}

/// Compute the mean vector of each cluster. Returns a map from cluster_id to
/// (centroid, member count). Ignores `-1` (noise).
pub fn cluster_centroids(
    vectors: &[Vec<f32>],
    labels: &[ClusterId],
) -> Vec<(ClusterId, Vec<f32>, usize)> {
    debug_assert_eq!(vectors.len(), labels.len());
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<ClusterId, (Vec<f32>, usize)> = BTreeMap::new();
    for (v, &lbl) in vectors.iter().zip(labels.iter()) {
        if lbl < 0 {
            continue;
        }
        let entry = acc.entry(lbl).or_insert_with(|| (vec![0.0; v.len()], 0));
        for (a, b) in entry.0.iter_mut().zip(v.iter()) {
            *a += b;
        }
        entry.1 += 1;
    }
    let mut out: Vec<(ClusterId, Vec<f32>, usize)> = Vec::with_capacity(acc.len());
    for (id, (mut sum, count)) in acc {
        let inv = 1.0 / count as f32;
        for x in &mut sum {
            *x *= inv;
        }
        // Re-normalise centroid so downstream cosine scoring stays cheap.
        super::clip::l2_normalize(&mut sum);
        out.push((id, sum, count));
    }
    out
}

/// Stable-label re-assignment: match new cluster centroids back to old ones
/// using the Hungarian algorithm on cosine distance, subject to a similarity
/// threshold (0.55 per plans/phase-2-browsing.md §3.iv). Unmatched new
/// clusters get fresh IDs.
///
/// Returns a map from new_cluster_id → assigned_cluster_id (either an old id
/// or a fresh allocation). Callers use this to rewrite face.person_id with
/// minimal churn across re-cluster runs.
pub fn hungarian_reassign(
    new_centroids: &[(ClusterId, Vec<f32>, usize)],
    old_centroids: &[(ClusterId, Vec<f32>, usize)],
    sim_threshold: f32,
) -> std::collections::HashMap<ClusterId, ClusterId> {
    use std::collections::{HashMap, HashSet};

    let mut out: HashMap<ClusterId, ClusterId> = HashMap::new();
    if new_centroids.is_empty() {
        return out;
    }

    // Build a dense cost matrix (scaled to integers — pathfinding's
    // kuhn_munkres works on i64 *benefit* rather than float cost).
    let n_new = new_centroids.len();
    let n_old = old_centroids.len();
    let dim = n_new.max(n_old);
    // kuhn_munkres expects a benefit matrix with finite values. We don't use
    // `i64::MIN` for padding because the algorithm subtracts row-mins at
    // initialisation and that overflows. Instead pad with 0 (cosine of
    // orthogonal vectors) — the sim_threshold check below strips out any
    // accidental "low-benefit" assignments the solver might pick anyway.
    let mut weights: Vec<Vec<i64>> = vec![vec![0; dim]; dim];
    for (i, (_, nc, _)) in new_centroids.iter().enumerate() {
        for (j, (_, oc, _)) in old_centroids.iter().enumerate() {
            let sim = cosine(nc, oc);
            // Scale by 1e6 for sub-milli-bit resolution. Add 1e6 so the
            // minimum value is 0 (cosine ∈ [-1, 1] → benefit ∈ [0, 2e6]).
            weights[i][j] = ((sim + 1.0) * 1_000_000.0) as i64;
        }
    }
    let mat = pathfinding::matrix::Matrix::from_rows(weights).expect("square");
    let (_score, assignment) = pathfinding::kuhn_munkres::kuhn_munkres(&mat);
    // `assignment[i] = j` means new_centroids[i] → old_centroids[j] if in range
    // and above similarity threshold; otherwise unmatched.
    let mut used_old: HashSet<ClusterId> = HashSet::new();
    let mut max_old_id: ClusterId = old_centroids
        .iter()
        .map(|(id, _, _)| *id)
        .max()
        .unwrap_or(-1);
    for (i, j) in assignment.iter().enumerate() {
        if i >= n_new {
            break;
        }
        let (new_id, new_c, _) = &new_centroids[i];
        if *j < n_old {
            let (old_id, old_c, _) = &old_centroids[*j];
            let sim = cosine(new_c, old_c);
            if sim >= sim_threshold {
                out.insert(*new_id, *old_id);
                used_old.insert(*old_id);
                continue;
            }
        }
        // Unmatched new cluster: allocate a fresh id past the old max.
        max_old_id += 1;
        out.insert(*new_id, max_old_id);
    }
    out
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
    fn similarity_transform_template_to_template_is_identity() {
        // Mapping the ArcFace template to itself should return the identity
        // matrix (modulo float noise).
        let m = similarity_transform(&ARCFACE_TEMPLATE, &ARCFACE_TEMPLATE);
        assert!((m[0] - 1.0).abs() < 1e-4, "a={}", m[0]);
        assert!(m[1].abs() < 1e-4, "b_neg={}", m[1]);
        assert!(m[2].abs() < 1e-3, "tx={}", m[2]);
        assert!(m[3].abs() < 1e-4, "b={}", m[3]);
        assert!((m[4] - 1.0).abs() < 1e-4, "a2={}", m[4]);
        assert!(m[5].abs() < 1e-3, "ty={}", m[5]);
    }

    #[test]
    fn similarity_transform_maps_scaled_translated_points_correctly() {
        // src is template scaled 2x and shifted by (100, 50).
        let src: [(f32, f32); 5] = std::array::from_fn(|i| {
            let (x, y) = ARCFACE_TEMPLATE[i];
            (x * 2.0 + 100.0, y * 2.0 + 50.0)
        });
        let m = similarity_transform(&src, &ARCFACE_TEMPLATE);
        // Forward-mapping each src point should land on the template (within
        // numeric noise).
        for (i, s) in src.iter().enumerate() {
            let (dx, dy) = apply_affine(&m, *s);
            let (tx, ty) = ARCFACE_TEMPLATE[i];
            assert!((dx - tx).abs() < 1e-2, "x off: got {dx} expected {tx}");
            assert!((dy - ty).abs() < 1e-2, "y off: got {dy} expected {ty}");
        }
    }

    #[test]
    fn invert_similarity_composes_to_identity() {
        let src: [(f32, f32); 5] =
            std::array::from_fn(|i| (ARCFACE_TEMPLATE[i].0 * 3.0, ARCFACE_TEMPLATE[i].1 * 3.0));
        let m = similarity_transform(&src, &ARCFACE_TEMPLATE);
        let inv = invert_similarity(&m).expect("non-degenerate");
        // M ∘ M^-1 applied to a test point should return the point.
        let p = (42.0, 99.0);
        let m_p = apply_affine(&m, p);
        let round = apply_affine(&inv, m_p);
        assert!((round.0 - p.0).abs() < 1e-2);
        assert!((round.1 - p.1).abs() < 1e-2);
    }

    #[test]
    fn bilinear_sample_at_integer_coord_matches_pixel() {
        let mut img = image::RgbImage::new(4, 4);
        img.put_pixel(2, 1, image::Rgb([200, 100, 50]));
        let got = bilinear_sample(&img, 2.0, 1.0);
        assert_eq!(got, [200, 100, 50]);
    }

    #[test]
    fn align_face_with_template_landmarks_is_near_identity_crop() {
        // Synthesise a 112×112 image whose "landmarks" already sit at the
        // template. Alignment should return effectively the same image.
        let mut img =
            image::RgbImage::from_pixel(ARCFACE_INPUT, ARCFACE_INPUT, image::Rgb([40, 80, 160]));
        // Mark each template position with a distinctive pixel.
        for (x, y) in &ARCFACE_TEMPLATE {
            img.put_pixel(*x as u32, *y as u32, image::Rgb([255, 0, 0]));
        }
        let aligned = align_face(&img, &ARCFACE_TEMPLATE);
        // The marker pixels (or their neighbours post-bilinear) should still
        // have a strongly red dominant channel at the expected positions.
        for (x, y) in &ARCFACE_TEMPLATE {
            let px = aligned.get_pixel(*x as u32, *y as u32);
            assert!(
                px[0] as i32 > px[1] as i32 && px[0] as i32 > px[2] as i32,
                "red dominance lost at ({x}, {y}): {:?}",
                px.0
            );
        }
    }

    #[test]
    fn cosine_of_identical_unit_vectors_is_one() {
        let a = vec![1.0, 0.0, 0.0];
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_of_opposite_vectors_is_minus_one() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((cosine(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_safe() {
        let a = vec![0.0_f32; 5];
        let b = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert!(cosine(&a, &b).abs() < 1e-12);
    }

    fn unit(v: &[f32]) -> Vec<f32> {
        let mut v = v.to_vec();
        super::super::clip::l2_normalize(&mut v);
        v
    }

    #[test]
    fn dbscan_finds_two_clusters_in_distinct_directions() {
        // Two well-separated directions in 4D space, 3 points each.
        let a1 = unit(&[1.0, 0.05, 0.0, 0.0]);
        let a2 = unit(&[0.98, 0.0, 0.05, 0.0]);
        let a3 = unit(&[0.99, 0.02, -0.02, 0.0]);
        let b1 = unit(&[0.0, 0.0, 1.0, 0.05]);
        let b2 = unit(&[0.0, 0.05, 0.98, 0.0]);
        let b3 = unit(&[-0.02, 0.0, 0.99, 0.02]);
        let vs = vec![a1, a2, a3, b1, b2, b3];
        let labels = dbscan_cosine(&vs, 0.4, 2);
        // Clusters 0, 1 respectively; no noise.
        assert_eq!(labels[0], labels[1]);
        assert_eq!(labels[0], labels[2]);
        assert_eq!(labels[3], labels[4]);
        assert_eq!(labels[3], labels[5]);
        assert_ne!(labels[0], labels[3]);
        assert!(labels.iter().all(|&l| l >= 0));
    }

    #[test]
    fn dbscan_isolated_point_becomes_noise() {
        let a = unit(&[1.0, 0.0, 0.0]);
        let b = unit(&[0.99, 0.05, 0.0]);
        let c = unit(&[0.0, 0.0, 1.0]); // far from a/b, alone
        let labels = dbscan_cosine(&[a, b, c], 0.2, 2);
        assert_eq!(labels[0], labels[1]);
        assert_eq!(labels[2], -1);
    }

    #[test]
    fn hungarian_reassign_is_stable_when_centroids_match() {
        // Two old clusters, two new clusters that are effectively the same.
        let old = vec![
            (10, unit(&[1.0, 0.0, 0.0]), 5),
            (11, unit(&[0.0, 1.0, 0.0]), 4),
        ];
        let new = vec![
            (0, unit(&[0.99, 0.05, 0.0]), 5),
            (1, unit(&[0.05, 0.99, 0.0]), 4),
        ];
        let map = hungarian_reassign(&new, &old, 0.55);
        assert_eq!(map.get(&0), Some(&10));
        assert_eq!(map.get(&1), Some(&11));
    }

    #[test]
    fn hungarian_reassign_allocates_fresh_id_for_new_cluster() {
        // One old, two new — second new has no match; gets id past old-max.
        let old = vec![(10, unit(&[1.0, 0.0, 0.0]), 5)];
        let new = vec![
            (0, unit(&[0.99, 0.05, 0.0]), 5),
            (1, unit(&[0.0, 1.0, 0.0]), 3),
        ];
        let map = hungarian_reassign(&new, &old, 0.55);
        assert_eq!(map.get(&0), Some(&10));
        let assigned = *map.get(&1).expect("second cluster gets an id");
        assert!(
            assigned > 10,
            "fresh id should be past old-max, got {assigned}"
        );
    }

    #[test]
    fn cluster_centroids_ignores_noise_and_renormalises() {
        let v0 = unit(&[1.0, 0.0, 0.0]);
        let v1 = unit(&[0.99, 0.05, 0.0]);
        let v2 = unit(&[0.0, 0.0, 1.0]);
        let labels = vec![0, 0, -1];
        let cents = cluster_centroids(&[v0, v1, v2], &labels);
        assert_eq!(cents.len(), 1);
        let (id, c, n) = &cents[0];
        assert_eq!(*id, 0);
        assert_eq!(*n, 2);
        // Unit-norm.
        let norm: f32 = c.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn arcface_preprocess_shape_and_normalisation() {
        let img =
            image::RgbImage::from_pixel(ARCFACE_INPUT, ARCFACE_INPUT, image::Rgb([255, 127, 0]));
        let t = arcface_preprocess(&img);
        assert_eq!(t.shape(), &[1, 3, 112, 112]);
        // BGR order: channel 0 = B = 0, channel 1 = G = 127, channel 2 = R = 255.
        let b = t[[0, 0, 0, 0]];
        let g = t[[0, 1, 0, 0]];
        let r = t[[0, 2, 0, 0]];
        assert!((b - ((0.0 - 127.5) / 128.0)).abs() < 1e-5);
        assert!((g - ((127.0 - 127.5) / 128.0)).abs() < 1e-5);
        assert!((r - ((255.0 - 127.5) / 128.0)).abs() < 1e-5);
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
