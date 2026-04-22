//! Per-job-kind dispatch for `MlWorker::drain_one` — Phase 2.1.
//!
//! This module bridges the job queue (`ml_job` table) to the live ONNX
//! runtime. Each job kind gets one `run_*` function here; `drain_blocking`
//! (in `runtime.rs`) dispatches based on `MlJobKind`.
//!
//! All functions are feature-gated behind `ml-models` because they need
//! `ort::Session` access. Without the flag, `drain_blocking` resolves the
//! same jobs to `Error::ModelsUnavailable` as it did in Phase 2.

use std::collections::HashMap;

use crate::cas::CasStore;
use crate::crypto::{open_row, seal_row, unwrap_file_key, CollectionKey};
use crate::db;
use crate::{Error, Result};

use super::clip;
use super::faces::{
    self, cluster_centroids, dbscan_cosine, hungarian_reassign, ClusterId,
};
use super::runtime::MlRuntime;

/// Default DBSCAN cosine-distance threshold for face clustering
/// (plans/phase-2-browsing.md §3.iv).
const PERSON_DBSCAN_EPS: f32 = 0.4;
/// Minimum samples per DBSCAN cluster — Phase 2 pins 2 (singletons stay
/// noise; People tab suppresses them).
const PERSON_DBSCAN_MIN: usize = 2;
/// Hungarian stable-reassign threshold — below this similarity, the new
/// cluster gets a fresh person_id rather than inheriting an old one.
const PERSON_REASSIGN_SIM: f32 = 0.55;

/// Run an `EmbedAsset` job: decrypt the 1024px thumbnail (falling back to
/// 256px), embed via CLIP visual, upsert the 768-d vector into `asset_vec`.
///
/// The caller provides a `ck_for_asset` resolver that returns the right
/// collection key per asset (default for plain assets, album key for
/// password-protected albums). Mirrors `nearp::rebuild` so the mental model
/// stays the same.
pub fn run_embed_asset(
    conn: &rusqlite::Connection,
    cas: &CasStore,
    rt: &MlRuntime,
    asset_id: i64,
    ck_for_asset: &impl Fn(i64) -> Option<CollectionKey>,
) -> Result<()> {
    let Some(ck) = ck_for_asset(asset_id) else {
        return Err(Error::Locked);
    };
    let Some(asset) = db::get_asset(conn, asset_id)? else {
        return Err(Error::NotFound);
    };
    let fk = unwrap_file_key(&asset.wrapped_file_key, &ck)?;

    // Prefer the 1024px thumb because CLIP's 224×224 center-crop benefits
    // from more pixels; fall back to 256px if the larger one isn't there.
    let thumb_ref = db::get_derivative(conn, asset_id, "thumb1024")?
        .or(db::get_derivative(conn, asset_id, "thumb256")?)
        .ok_or_else(|| Error::Ingest(format!("no thumb for asset {asset_id}")))?;
    let bytes = cas.get(&thumb_ref, &fk)?;
    let embedding = clip::embed_image_bytes(&rt.sessions.clip_visual, &bytes)?;
    db::upsert_asset_vec(conn, asset_id, &embedding)?;
    Ok(())
}

/// Run a `DetectFaces` job: decrypt the 1024px thumbnail, run SCRFD detection
/// on the full frame, align + embed every face, insert one `face` row each.
///
/// **Idempotency:** callers are expected to have deleted existing faces for
/// this asset before enqueueing a re-run (via a cascaded person delete or an
/// explicit SQL DELETE). For Phase 2.1 we don't enforce that here — the job
/// queue's dedupe on `(kind, asset_id)` is enough to prevent accidental
/// double-runs under normal flow.
pub fn run_detect_faces(
    conn: &rusqlite::Connection,
    cas: &CasStore,
    rt: &MlRuntime,
    asset_id: i64,
    ck_for_asset: &impl Fn(i64) -> Option<CollectionKey>,
) -> Result<()> {
    let Some(ck) = ck_for_asset(asset_id) else {
        return Err(Error::Locked);
    };
    let Some(asset) = db::get_asset(conn, asset_id)? else {
        return Err(Error::NotFound);
    };
    let fk = unwrap_file_key(&asset.wrapped_file_key, &ck)?;
    let thumb_ref = db::get_derivative(conn, asset_id, "thumb1024")?
        .or(db::get_derivative(conn, asset_id, "thumb256")?)
        .ok_or_else(|| Error::Ingest(format!("no thumb for asset {asset_id}")))?;

    let bytes = cas.get(&thumb_ref, &fk)?;
    let img = image::load_from_memory(&bytes).map_err(|e| Error::Media(format!("face decode: {e}")))?;
    let rgb = img.to_rgb8();

    let detections = faces::detect_faces(&rt.sessions.scrfd, &rgb)?;
    for det in &detections {
        let embedding = faces::embed_face(&rt.sessions.arcface, &rgb, &det.landmarks)?;
        // Serialise bbox as 4 × f32 little-endian (matches ingest-time
        // convention in ml::faces).
        let mut bbox_bytes = Vec::with_capacity(16);
        for v in det.bbox {
            bbox_bytes.extend_from_slice(&v.to_le_bytes());
        }
        let mut emb_bytes = Vec::with_capacity(embedding.len() * 4);
        for v in &embedding {
            emb_bytes.extend_from_slice(&v.to_le_bytes());
        }
        let bbox_ct = seal_row(&bbox_bytes, asset_id as u64, ck.as_bytes())?;
        let emb_ct = seal_row(&emb_bytes, asset_id as u64, ck.as_bytes())?;
        db::insert_face(conn, asset_id, f64::from(det.score), &bbox_ct, &emb_ct)?;
    }
    Ok(())
}

/// Rebuild the entire person clustering from scratch: read every face's
/// embedding_ct, decrypt with its asset's key, run DBSCAN, match new cluster
/// centroids back to existing `person` rows via Hungarian assignment, and
/// update each `face.person_id` so IDs stay stable across re-runs.
///
/// Runs synchronously — the caller (MlWorker) wraps it in spawn_blocking.
pub fn run_rebuild_person_clusters(
    conn: &rusqlite::Connection,
    ck_for_asset: &impl Fn(i64) -> Option<CollectionKey>,
) -> Result<()> {
    let face_rows = db::list_all_faces(conn)?;
    let mut face_ids: Vec<i64> = Vec::with_capacity(face_rows.len());
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(face_rows.len());
    let mut old_person_by_face: Vec<Option<i64>> = Vec::with_capacity(face_rows.len());
    let mut skipped = 0usize;

    for row in &face_rows {
        let Some(ck) = ck_for_asset(row.asset_id) else {
            skipped += 1;
            continue;
        };
        let Ok(raw) = open_row(&row.embedding_ct, row.asset_id as u64, ck.as_bytes()) else {
            skipped += 1;
            continue;
        };
        let mut v = Vec::with_capacity(raw.len() / 4);
        for chunk in raw.chunks_exact(4) {
            v.push(f32::from_le_bytes(chunk.try_into().expect("chunk_exact")));
        }
        if v.len() != faces::ARCFACE_DIM {
            skipped += 1;
            continue;
        }
        face_ids.push(row.id);
        vectors.push(v);
        old_person_by_face.push(row.person_id);
    }

    if skipped > 0 {
        tracing::debug!(
            skipped,
            total = face_rows.len(),
            "face rows skipped during rebuild (locked or malformed)"
        );
    }
    if vectors.is_empty() {
        return Ok(());
    }

    let new_labels = dbscan_cosine(&vectors, PERSON_DBSCAN_EPS, PERSON_DBSCAN_MIN);
    let new_centroids = cluster_centroids(&vectors, &new_labels);

    // Build old centroids per existing person using the same face vectors we
    // just decrypted. Faces whose old person_id is None contribute nothing.
    let old_centroids = old_centroids_from_vectors(&vectors, &old_person_by_face);

    // Determine fresh-id allocation base (max existing person id + 1).
    let base_person_id = std::cmp::max(
        old_centroids.iter().map(|(id, _, _)| *id).max().unwrap_or(-1),
        db::list_persons(conn, /* owner_id */ 1, true)?
            .iter()
            .map(|p| p.id as ClusterId)
            .max()
            .unwrap_or(-1),
    );
    // Feed `hungarian_reassign` centroids whose IDs start past existing
    // persons so allocated fresh IDs don't collide.
    let new_centroids_offset: Vec<_> = new_centroids
        .iter()
        .map(|(id, v, n)| (*id + base_person_id + 1, v.clone(), *n))
        .collect();
    let map = hungarian_reassign(&new_centroids_offset, &old_centroids, PERSON_REASSIGN_SIM);

    // Two-pass update: for each new label, resolve the target person_id.
    // If the target matches an existing person, reuse; otherwise materialise
    // a new person row.
    let mut label_to_person: HashMap<ClusterId, i64> = HashMap::new();
    for (shifted_id, target_id) in map {
        // `target_id` may be either an existing person or a freshly-allocated
        // integer past base. Only `insert_person` if it's the latter.
        let exists = old_centroids.iter().any(|(id, _, _)| *id == target_id);
        let person_id = if exists {
            target_id as i64
        } else {
            db::insert_person(conn, /* owner_id */ 1, None)?
        };
        label_to_person.insert(shifted_id, person_id);
    }

    // Rewrite face.person_id in one pass.
    for (idx, &lbl) in new_labels.iter().enumerate() {
        if lbl < 0 {
            // Noise — clear person_id if it had one.
            db::reassign_faces_to_person(conn, &[face_ids[idx]], None)?;
            continue;
        }
        let shifted_id = lbl + base_person_id + 1;
        if let Some(&pid) = label_to_person.get(&shifted_id) {
            db::reassign_faces_to_person(conn, &[face_ids[idx]], Some(pid))?;
        }
    }
    Ok(())
}

/// Build per-old-person centroids straight from the decrypted vectors we
/// already have in memory. Cheaper than round-tripping through the DB + a
/// second decrypt pass.
fn old_centroids_from_vectors(
    vectors: &[Vec<f32>],
    old_ids: &[Option<i64>],
) -> Vec<(ClusterId, Vec<f32>, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<i64, (Vec<f32>, usize)> = BTreeMap::new();
    for (v, pid) in vectors.iter().zip(old_ids.iter()) {
        let Some(pid) = pid else { continue };
        let entry = acc.entry(*pid).or_insert_with(|| (vec![0.0; v.len()], 0));
        for (a, b) in entry.0.iter_mut().zip(v.iter()) {
            *a += b;
        }
        entry.1 += 1;
    }
    let mut out = Vec::with_capacity(acc.len());
    for (pid, (mut sum, count)) in acc {
        let inv = 1.0 / count as f32;
        for x in &mut sum {
            *x *= inv;
        }
        super::clip::l2_normalize(&mut sum);
        out.push((pid as ClusterId, sum, count));
    }
    out
}
