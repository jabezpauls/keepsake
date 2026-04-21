# Phase 2 — Browsing (Apple-Photos surface + search)

> Prerequisite: Phase 1 shipped and acceptance criteria all pass. `cargo test --workspace` green at tip.

---

## Goal

Make the library *feel* like Apple Photos:

1. Timeline is smooth at 500k+ assets.
2. Map view shows where photos were taken.
3. Faces get detected, clustered, named, merged/split.
4. Natural-language search works ("dog on beach at sunset").
5. Near-duplicates and bursts collapse into cleanable clusters with a best-shot pick.
6. Live Photos, Motion Photos, and RAW+JPEG pairs are first-class in the UI.

All of this uses **only on-device Rust ML via `ort` (ONNX Runtime)**. No Python sidecar. No network calls.

**This phase ends when a user can import a Source, wait for ML to finish, and browse/search/people exactly like iPhone Photos — single user, no peers yet.**

---

## Prerequisites

- Phase 1 acceptance criteria all green.
- `models/` directory populated by `scripts/download_models.sh` (or checked in via Git LFS if offline — decide at implementation time). Models required:
  - CLIP ViT-L/14: visual encoder + text encoder (OpenCLIP's ONNX export).
  - SCRFD 10G BNKPS (face detector).
  - ArcFace R100 (face embedding).
  - Lightweight pHash routine — no model, pure code.
- `sqlite-vec` extension available and loaded at DB open (stub already there from Phase 1).

---

## Immutable contracts this phase adds (do not change later)

- **CLIP embedding dimension = 768** (ViT-L/14). Table `asset_vec` is created with `FLOAT[768]`. Changing this later requires re-embedding the entire library.
- **ArcFace embedding dimension = 512**. `face.embedding_ct` stores 512 f32 values (2 KiB after AEAD).
- **pHash = 64-bit `dhash`**. Hamming-distance threshold for near-dup default: 6 bits. Configurable in UI, but the stored hash format is fixed.
- **CLIP image pre-processing** follows OpenAI's canonical pipeline: resize shorter side to 224, center-crop, normalise with OpenAI stats. Any deviation changes embeddings and poisons search.
- **Face bounding-box convention**: `(x, y, w, h)` in image-pixel space, stored in `face.bbox_ct` as 4 × f32.

---

## Tasks

### 1. Model pipeline infrastructure

1. `crates/core/src/ml/mod.rs` — one place to load/warm all ONNX models behind a `MlRuntime` struct.
   ```rust
   pub struct MlRuntime {
       env:            Arc<ort::Environment>,
       clip_visual:    ort::Session,
       clip_textual:   ort::Session,
       face_detector:  ort::Session,
       face_embedder:  ort::Session,
       // phase 3 later: ocr_det, ocr_rec, ocr_cls, pet_classifier
   }
   impl MlRuntime {
       pub fn load(model_dir: &Path, config: MlConfig) -> Result<Self>;
       pub fn shutdown(self);
   }
   ```
   Loaded once at app start after unlock. Execution providers: CUDA if present, CoreML on macOS, CPU (MLAS) fallback; auto-select.

2. `scripts/download_models.sh` — fetches checksummed ONNX files into `models/`. Checksums baked into the script. SHA-256 verification is mandatory; mismatched download aborts.

3. `scripts/export_clip.py` (once, not in the built app) — if the canonical OpenCLIP ONNX export is unsuitable, this script re-exports. Not shipped to users; lives in the repo for reproducibility. Output only; never called at runtime.

4. Add `ml-models` feature flag to `crates/core/Cargo.toml` — when off, `MlRuntime::load` returns `Err(Error::ModelsUnavailable)` and UI shows "Install models" prompt. Default on.

5. Background worker pool: `crates/core/src/ml/worker.rs`
   ```rust
   pub struct MlWorker { /* tokio tasks on a rayon pool */ }
   impl MlWorker {
       pub fn spawn(rt: Arc<MlRuntime>, db: Db, cas: CasStore) -> Self;
       pub fn enqueue(&self, job: MlJob);
       pub fn progress(&self) -> watch::Receiver<MlProgress>;
       pub fn shutdown(self);
   }
   enum MlJob { EmbedAsset(AssetId), DetectFaces(AssetId), RebuildPersonClusters, ... }
   ```
   Concurrency: `min(num_cpus, 4)` inference workers + 1 face-clustering task. Jobs persisted in a `ml_job` plaintext table so interrupted runs resume.

### 2. CLIP embedding pipeline (`ml/clip.rs`)

1. `fn embed_image(rt: &MlRuntime, bytes: &[u8]) -> Result<[f32; 768]>`
   - Decode to RGB via `image` crate.
   - Resize shorter side to 224, center-crop to 224×224.
   - Normalise with OpenAI stats: mean `(0.48145466, 0.4578275, 0.40821073)`, std `(0.26862954, 0.26130258, 0.27577711)`.
   - Run through `clip_visual` session.
   - Unit-normalise (L2) the output.
2. `fn embed_text(rt: &MlRuntime, query: &str) -> Result<[f32; 768]>`
   - Tokenise via `tokenizers` crate using the CLIP BPE.
   - Pad/truncate to 77 tokens.
   - Run through `clip_textual`.
   - Unit-normalise.
3. Batch variant: `fn embed_images_batch(..., batch: u32 = 16) -> Vec<[f32; 768]>` for throughput.
4. `MlJob::EmbedAsset(id)` handler: decrypt a thumbnail (1024px if available else 256) from CAS, run `embed_image`, store into `asset.clip_embedding_ct` AND `asset_vec` plaintext (see architecture.md §4.3 trade-off).

### 3. Face pipeline (`ml/faces.rs`)

1. `fn detect_faces(rt: &MlRuntime, bytes: &[u8]) -> Vec<FaceDetection>` via SCRFD. Returns bbox + detection confidence + 5 landmarks.
2. For each detection, align-crop to 112×112 (ArcFace convention) using the landmarks → feed through `face_embedder` → 512-d unit vector.
3. Store `face` rows: plaintext `quality` = SCRFD confidence × face-pose heuristic; encrypted bbox + embedding; `person_id` NULL initially.
4. Clustering (`fn rebuild_person_clusters`):
   - Pull all face embeddings (decrypted), run DBSCAN with `eps=0.4` cosine distance, `min_samples=2`.
   - Every cluster without an existing `person_id` gets a new `person` row with `name_ct=None`.
   - Existing clusters: stable-assign by matching the new centroid to the old cluster's centroid with Hungarian algorithm above threshold 0.55; below threshold = new person.
   - Exclusive-singletons are skipped (single face of one person is fine but we don't create a People entry until 2+ faces).
5. Merge/split operations:
   - `merge_persons(src, dst)` — UPDATE face.person_id; DELETE src.
   - `split_person(person_id, face_ids)` — create new person, move those faces.
   - `hide_person(person_id)` — sets `person.hidden=1`; UI filters them out.

### 4. Perceptual hash & near-dup clustering (`ml/phash.rs`)

1. Implement 64-bit **dhash** (difference-hash): resize to 9×8 greyscale, compare each pixel to its right neighbour → 64 bits. Pure Rust, no ML.
2. Store into `phash` table at ingest time (retrofit Phase 1's ingest adapter to call into this — OK: `phash.rs` exists in Phase 1 stubs, Phase 2 fills it in; the `phash` table was already declared).
3. Near-dup clusters computed by:
   - Partitioned LSH — bucket hashes by high-4-bits, hamming-compare within buckets.
   - Pairs with hamming distance ≤ threshold grouped via union-find.
   - Results cached in a new plaintext `nd_cluster(cluster_id, asset_id)` table with a dirty flag.
4. Best-shot picker:
   - Score each asset = 0.6 × sharpness (Laplacian variance on thumbnail) + 0.3 × exposure quality + 0.1 × face-quality sum.
   - Cluster's best-shot = highest-scoring member.
5. UI integration: a "Review Duplicates" screen lists clusters, shows the best-shot pre-selected, user clicks "Keep only this" to move others to trash.

### 5. Media-pair first-class UI

Already grouped in Phase 1 (`media/pairs.rs`); Phase 2 exposes them in UI:

1. **Live Photos** — in the asset grid, a small "LIVE" badge in the corner; long-press or hold-to-play plays the paired MOV.
2. **Motion Photos** — same badge + playback path. Extract the MP4 portion at display time (don't re-encode).
3. **RAW+JPEG** — show the JPEG preview with a "RAW" badge. Detail view has a "Show RAW" toggle that requests the RAW via `asset_original`. RAW assets without a JPEG sibling still appear but show a placeholder (no thumbnail available until Phase 2's RAW preview extractor lands — see Task 9).
4. **Bursts** — collapse in the timeline grid to the first image with a stack badge and count. Tap → expanded burst view. Best-shot (§4) drives which image is the cover.

### 6. Timeline polish (`ui/timeline/`)

1. Year / Month / Day zoom levels. Scroll between with two-finger pinch / scroll-wheel + cmd. Each level has its own layout density.
2. Virtualization: `@tanstack/react-virtual` on the row level, custom intra-row item virtualization for days with hundreds of items.
3. Sticky month + year headers.
4. 60 fps scroll target on 500k assets (test fixture: Task 10).
5. Keyboard nav: arrows, `j/k` page, `/` focuses search, `cmd+f` opens filter bar.
6. Jump-to-date control (year scrubber on the right edge à la iPhone Photos).

### 7. Map view (`ui/map/`)

1. MapLibre GL JS with a self-hosted tile style. Phase 2 ships a minimal open style (OpenStreetMap raster via your own relay tile source, or bundle [protomaps PMTiles](https://protomaps.com/) locally for fully offline).
2. Supercluster-style marker clustering client-side; on click of cluster, show a grid sheet of its photos.
3. Heatmap toggle (density-only, no data fetch).
4. Trip overlay placeholder — Phase 3 wires it; Phase 2 reserves the UI surface.
5. Date-range slider at the bottom filters what's mapped.
6. Privacy: map view honors hidden-vault state; hidden-vault assets never appear unless hidden is unlocked. Same for password-protected albums (members hidden until album unlocked).

### 8. Search (`ui/search/` + `crates/core/src/search/`)

1. Rust side:
   ```rust
   pub struct SearchQuery {
       pub text:          Option<String>,       // natural-language
       pub people:        Vec<PersonId>,
       pub date_range:    Option<(DateTime<Utc>, DateTime<Utc>)>,
       pub source:        Option<SourceId>,
       pub has_faces:     Option<bool>,
       pub has_text:      Option<bool>,         // Phase 3
       pub is_video:      Option<bool>,
       pub is_raw:        Option<bool>,
       pub is_screenshot: Option<bool>,
       pub is_live:       Option<bool>,
       pub camera_make:   Option<String>,       // decrypts rows to match
       pub lens:          Option<String>,
       pub limit:         u32,
   }
   pub fn search(q: &SearchQuery) -> Result<Vec<AssetView>>;
   ```
   Execution:
   - Start from plaintext filters (date, is_video, etc.) → candidate set.
   - If `text` present: embed text via CLIP, vec-search the candidate set (use `asset_vec` pre-filtered by primary-key set).
   - If `camera_make`/`lens`: pull candidates' `exif_all_ct`, decrypt in-memory, filter.
   - Merge & rank; return top-`limit`.
2. Search bar UI: single text field + chip-based filter row (Date, People, Source, Video/Photo, Screenshot, Live, RAW). Chips map 1:1 to `SearchQuery` fields.
3. Saved queries become **Smart Album seeds** for Phase 3.

### 9. RAW preview extraction (if skipped in Phase 1)

If Phase 1 only flagged RAWs without thumbnails:

1. Use `libraw-sys` FFI or `rawloader` to extract embedded JPEG preview from each RAW file on ingest.
2. If no embedded preview, demosaic via `rawloader` and resize. Slow but one-time per asset.
3. Store as standard `derivative(kind='thumb1024', ...)` entries.

### 10. Performance + load tests

1. `tests/integration/perf_timeline.rs` — synthesise 500k `asset` rows with random plaintext metadata; measure cold timeline first-page latency (< 200 ms), scroll frame budget via a headless Playwright trace (< 16 ms/frame on dev box). Both SLOs must pass.
2. `tests/integration/perf_search.rs` — 50k assets with random CLIP vectors; text query top-50 under 200 ms cold, < 50 ms warm.
3. `tests/integration/face_pipeline.rs` — fixture of 40 faces across 10 identities in varied lighting. Assert clusters = 10 ± 1, and that a merge followed by a re-cluster is stable.
4. `tests/integration/near_dup.rs` — the `near_dup_set` fixture from Phase 1: clusters should match ground truth exactly (5 clusters, correct membership).

---

## File-by-file checklist (new or modified)

| Path | New/Mod | Notes |
|---|---|---|
| `crates/core/src/ml/mod.rs` | new | MlRuntime, MlConfig, MlJob enum |
| `crates/core/src/ml/worker.rs` | new | persistent job queue + tokio/rayon |
| `crates/core/src/ml/clip.rs` | new | embed image/text |
| `crates/core/src/ml/faces.rs` | new | detect + embed + cluster |
| `crates/core/src/ml/phash.rs` | new (was stub Phase 1) | dhash compute + cluster |
| `crates/core/src/search/mod.rs` | new | query struct + executor |
| `crates/core/src/media/raw_preview.rs` | new (if Phase 1 deferred) | libraw/rawloader |
| `crates/core/src/db/queries.rs` | mod | add face/phash/vec helpers, `ml_job` table |
| `crates/core/src/db/schema.rs` | mod (additive only) | `ml_job`, `nd_cluster` plaintext tables |
| `app/src-tauri/src/commands/search.rs` | new | expose search API |
| `app/src-tauri/src/commands/ml.rs` | new | progress, re-index, person ops |
| `app/src-tauri/src/commands/map.rs` | new | geo points for date-range |
| `app/src/ui/timeline/*` | mod | zoom levels, perf |
| `app/src/ui/map/*` | new | MapLibre integration |
| `app/src/ui/people/*` | new | cluster review, naming, merge/split |
| `app/src/ui/search/*` | new | bar + chips |
| `app/src/ui/duplicates/*` | new | near-dup cluster review |
| `models/*` | new | ONNX weights |
| `scripts/download_models.sh` | new | checksummed fetch |
| `tests/integration/*` | new | perf, face, near-dup |

Rough size: +4k Rust, +3k TS.

---

## Execution order

1. `MlRuntime` + model download + sanity CLIP embed test.
2. pHash pass → retrofit into Phase 1's ingest pipeline.
3. CLIP pipeline + `asset_vec` population.
4. Face detect + embed.
5. Face clustering + person CRUD + merge/split.
6. Search executor + UI.
7. Map view.
8. Timeline polish (zoom levels, perf).
9. Media-pair UI surfaces (Live/Motion/Burst/RAW).
10. Near-dup clustering UI.
11. Perf tests → tune.
12. Acceptance pass.

Parallelisable: map view ↔ people UI ↔ search UI (three UI agents), timeline polish ↔ face pipeline.

---

## Acceptance criteria (Phase 2 is DONE when all pass)

1. **ML pipeline runs end-to-end on the Phase 1 iPhone golden fixture**: every asset ends up with `clip_embedding_ct` populated, `phash` row written; faces detected on fixture images and clustered into the expected number of people.
2. **Natural-language search**: on a 1k-asset fixture dataset with known captions, top-5 CLIP search for each caption hits the ground-truth image in ≥ 80% of cases.
3. **Face cluster quality**: on the 40-face fixture, pairwise-F1 of produced clusters vs. ground truth ≥ 0.9.
4. **Merge/split stability**: merge two persons, re-cluster, no faces flip person; split a person into two via face selection, re-cluster, both halves stay put.
5. **Timeline SLOs**: 500k-asset synthetic library renders first page ≤ 200 ms cold, sustained ≥ 60 fps scroll on a mid-range laptop (Intel i5/M1). Measured in Playwright.
6. **Search SLOs**: cold 50k-asset top-50 CLIP search ≤ 200 ms; warm ≤ 50 ms.
7. **Near-dup**: `near_dup_set` fixture yields 5 clusters with correct membership; no false positives.
8. **Map**: on a fixture with 200 geo-tagged assets across 3 continents, clusters form at zoom-1, individual markers at zoom-15; hidden-vault assets DO NOT appear unless hidden is unlocked.
9. **Media pairs**: a Live Photo plays; a Motion Photo plays; a RAW+JPEG pair shows RAW badge and RAW preview is accessible.
10. **Crypto invariants still hold**: Phase 1's adversarial dump test still passes. `clip_embedding_ct`, `bbox_ct`, `embedding_ct`, and every other `_ct` column are indistinguishable from random without the password. `asset_vec` (plaintext vectors) is allowed to leak similarity structure — that's the accepted trade-off in architecture.md §4.3 and must be documented in the `SECURITY.md` file (update it here).
11. **All of Phase 1's acceptance criteria remain green.**
12. **No Python runtime on the host machine is needed to run the app.** Only ONNX Runtime + Rust.

---

## Explicitly out of scope for Phase 2

- Peer-to-peer, multi-user (Phase 3).
- OCR, transcripts (Phase 3 / 4).
- Memories, trips, smart-album rules (Phase 3).
- Pets classifier (Phase 3).
- Mobile app (Phase 4).

---

## Known decisions to record in commits

- CLIP variant = ViT-L/14 (not ViT-B/32). Chose L for quality; document compute cost.
- Face detector = SCRFD 10G BNKPS (heavier, higher recall on occluded faces than RetinaFace).
- dhash over pHash for near-dup (dhash is the fastest and empirically robust enough — see imagededup comparative study). Hamming threshold 6 chosen after eyeballing fixture.
- MapLibre with PMTiles offline. Do NOT call out to any remote tile service by default; add it only behind an opt-in toggle later.
