# Phase 3 — Peers & Smart (family, OCR, trips, memories, smart albums, share-links)

> Prerequisite: Phase 2 ships + acceptance criteria green. Single-user library is feature-complete for browse + search.

---

## Goal

Turn the app from "personal vault on one machine" into "family library, many peers, offsite backup, and the smart surfaces that make the library feel alive":

1. **Peer-to-peer sync via Iroh.** Every install is a full peer. No daemon/client split. Family members and an offsite replica are all just more peers.
2. **Multi-user on the same device** with independent key material + shared-album exchange.
3. **OCR on images** with blind-indexed FTS5 search.
4. **Trip detection** from geo + time clusters.
5. **Memories & on-this-day** auto-generated cards.
6. **Smart albums** — saved rule sets.
7. **Pet recognition**.
8. **Encrypted public share-links** with password + expiry.

**Phase 3 ends when two paired devices share an album end-to-end with zero plaintext leaking to a third unpaired peer, and all acceptance criteria in §10 pass.**

---

## Prerequisites

- Phase 2 acceptance criteria all green.
- Peer identity (Iroh Ed25519 + libsodium X25519) already generated at user creation in Phase 1 — just never networked. Phase 3 wires the network.
- `collection_key` table, `seal_for_peer` / `open_from_peer` already implemented. Reuse.

---

## Immutable contracts this phase adds

- **Iroh PairingTicket format** exactly as in `architecture.md` §6. Base32, versioned byte, fixed field order, 8-byte UNIX seconds, Ed25519 signature over all preceding bytes.
- **iroh-docs document schema**: one "per-user" namespace + one per shared collection. Keys and values listed in §2.4.
- **Share envelope format** — how a collection key gets wrapped for a recipient. Frozen in §3.3 below.
- **OCR token blind-indexing**: `token_blind = HMAC-SHA256(searchKey, lowercase(token))`, truncated to 16 bytes.
- **Trip cluster canonical id**: deterministic hash of sorted member asset ids — stable across re-runs.

---

## Tasks

### 1. `crates/sync/` — Iroh integration

1. `sync/src/peer.rs`:
   ```rust
   pub struct Peer {
       router:   iroh::protocol::Router,
       blobs:    iroh_blobs::store::fs::FsStore,
       docs:     iroh_docs::engine::Engine,
       identity: PeerIdentity, // Ed25519 node key + X25519 share key, both unwrapped
   }
   impl Peer {
       pub async fn start(vault: &VaultHandle) -> Result<Self>;
       pub async fn ticket(&self, mode: TicketMode) -> Result<PairingTicket>;
       pub async fn accept_ticket(&self, ticket: &PairingTicket) -> Result<PeerId>;
       pub async fn subscribe_shared(&self, collection_id: CollectionId) -> Result<()>;
       pub async fn shutdown(self);
   }
   ```
   Uses `iroh`, `iroh-blobs`, `iroh-docs` crates. Persistent state stored in `<vault>/iroh/` (separate from `cas/` and `index.db`).

2. Blob bridging: when the DB records a new `asset.cas_ref`, also announce that hash to iroh-blobs. When a local asset is requested by a peer, serve the ciphertext bytes. **Never decrypt for serving** — peers have their own key material.

3. Trust model: a peer sending you a blob must be an accepted peer (in `peer_accept` plaintext table). Unknown peers get a connection-closed reply.

### 2. `iroh-docs` schema

Per-vault setup creates **two kinds of namespaces**:

- **User namespace** (one per local user) — holds:
  - `user/profile/<user_id>` → `UserPublicProfile` (display name encrypted by user, but the writer's X25519 public key plaintext so peers can seal back).
  - `peer/accept/<peer_node_id>` → `AcceptedPeer` (plaintext: the approving user's id and an optional note).
- **Shared-collection namespace** (one per shared album, generated at share time) — holds:
  - `collection/meta/<collection_id>` → `CollectionMeta` (encrypted by the collection key; contains name, kind, cover asset).
  - `collection/members/<asset_id>` → `MemberEntry` (asset metadata, see §3).
  - `collection/key/<recipient_user_id>` → the collection key sealed for that recipient (sealed box).
  - `collection/revocation/<recipient_user_id>` → tombstone when access is revoked.

Namespaces are Iroh "Docs" (CRDT-style key-value); writes are authenticated by a namespace key held by the owner.

### 3. Sharing flow (`sync/src/share.rs`)

1. **Share an album with Alice** (local user → local user on the same device, or → remote peer Alice):
   - Create or reuse the shared-collection namespace for this album.
   - Seal the `collectionKey` with Alice's X25519 public key → write into `collection/key/<alice_user_id>`.
   - Write the `collection/meta/...` and `collection/members/...` entries.
   - Announce the relevant CAS blobs (hashes of ciphertext) to iroh-blobs.

2. **Receive an album** (Alice's device):
   - Subscribe to the namespace (share acceptance UI).
   - On seeing a `collection/key/<my_user_id>` row, unwrap with own X25519 private key.
   - With the collection key, decrypt `collection/meta` + all `collection/members/...` into Alice's local DB.
   - Iroh-blobs pulls the asset ciphertexts on demand when Alice scrolls the album.

3. **Revoke**: write revocation tombstone → rotate the collection key → re-seal to remaining recipients. Iroh-blobs garbage-collect the old blobs on remaining peers only when they locally re-encrypt with the new key (expensive; make this a batch operation).

### 4. Multi-user on the same device

The `user` table already supports many rows (Phase 1). Wire up:

1. Login screen lists registered users. Select + password.
2. Hidden-vault gesture works *per user*.
3. At rest, the vault dir is single — both users' key material + both users' data coexist there. **A user can only see their own collections** (owner_id filter) + anything shared to them.
4. Sharing same-device: pick a user from a dropdown, the system uses their X25519 public key locally to seal.
5. Each user has their own Iroh node identity (different Ed25519 keypair).

### 5. Offsite replica peer

A minimal "headless" mode for the app:

1. `--headless --replica` CLI flag skips UI entirely.
2. App generates a node identity on first run, prints its pairing ticket once, then waits.
3. When another peer pairs with it, the headless node accepts replication of any namespace the pairing peer authorises.
4. It pulls ciphertext blobs opportunistically. It never holds any user's password — it is purely a ciphertext cache.
5. Ship a systemd unit template at `scripts/replica.service`.

### 6. OCR (`crates/core/src/ml/ocr.rs`)

1. PaddleOCR ONNX (det + rec + cls). Three sessions on `MlRuntime`.
2. `fn ocr_image(rt: &MlRuntime, bytes: &[u8]) -> Vec<OcrLine>` — returns line-level text with bounding boxes + confidence.
3. Store lines in `ocr_text_ct` as JSON `[{text, bbox, conf}]`.
4. Build `asset_text` FTS5 entries:
   - Tokenise text (unicode61 tokenizer, lowercase, diacritics removed).
   - HMAC each token with the user's `searchKey` → hex → insert `INSERT INTO asset_text(asset_id, token_blind) VALUES(?, ?)` per token.
5. Search UI `text:"word"` filter:
   - HMAC the query word with current user's `searchKey`.
   - FTS5 match against `token_blind`.
6. Constraints:
   - **Exact-word match only.** Substring and prefix search impossible under blind indexing.
   - Multi-word queries are AND over HMAC'd tokens.
   - Document this clearly in the UI ("whole words only").
7. OCR worker jobs enqueued by `MlWorker` at ingest time for images (not videos; not RAW).

### 7. Trip detection (`analytics/trips.rs`)

1. Pull all assets with GPS + taken-at (decrypt necessary columns at detect time under a session key kept in memory).
2. DBSCAN on (lat, lon, time-as-days) with eps tuned per user (default: 50 km × 3 days).
3. Every cluster → write a `collection` row with `kind='trip'`, `name_ct` initially an inferred name from the most common reverse-geocoded place.
4. Re-run incrementally: new assets only.
5. UI surface: "Trips" tab in the sidebar; each trip card shows cover + place + date range.
6. Expose a `trip_detect_preview(params)` command so the user can tweak eps and see a dry-run before committing.

### 8. Memories / on-this-day (`analytics/memories.rs`)

1. **"On this day"** — for each day-of-year, collect assets from prior years within ±3 days. Surface on home screen.
2. **Themed compilations** — generate candidate memory collections:
   - *Person × Year*: "Ami in 2024"
   - *Trip retrospective*: re-use `kind='trip'`
   - *Recent weeks*: 7-day rolling cards
   - *Year in photos*: end-of-year montage
3. A memory is a `collection` with `kind='memory'` and a `memory_spec_ct` JSON column (add to schema as an additive migration).
4. **Video slideshow renderer**: `crates/core/src/analytics/slideshow.rs` uses `ffmpeg-next` to stitch 10–20 best-shot assets into a 30–60s MP4 with cross-fades and a looped ambient audio track (bundled royalty-free). Rendered on-demand; cached CAS entry linked via `derivative(kind='memory_mp4')`.
5. UI surface: Home tab shows memory cards with tap-to-expand + slideshow play.

### 9. Smart albums (`analytics/smart_albums.rs`)

1. Smart-album spec is a JSON rule doc:
   ```json
   {
     "all_of": [
       {"is_raw": true},
       {"device_make": "SONY"},
       {"iso_min": 800},
       {"date_after": "2024-01-01"}
     ]
   }
   ```
2. `collection` with `kind='smart_album'` stores the spec in a new `smart_spec_ct` column (additive migration).
3. At query time, compile spec → `SearchQuery` → execute.
4. Materialised view cached in `collection_member_smart(collection_id, asset_id, snapshot_at)` plaintext table for fast listing; invalidated on `asset` inserts/deletes.
5. UI surface: smart-album builder — chip-based rule editor mirroring the search chips.

### 10. Pets classifier (`analytics/pets.rs`)

1. A small ONNX classifier (dog/cat/horse/bird/none) — model ~10 MB. Trained on iNaturalist + PetFinder subset; bundle.
2. Run on every asset at ingest. Store `is_pet` plaintext flag (additive schema change — fine) + encrypted `pet_species_ct`.
3. Cluster within the "dog"/"cat" subsets using CLIP embeddings + DBSCAN to group "your dog" vs. "a random dog". Use the same UI as People: a "Pets" tab, name each cluster.

### 11. Reverse geocoding (`crates/core/src/geocode/`)

Fully offline.

1. Bundle either:
   - a pruned Nominatim snapshot (~2-4 GB) — higher quality but heavy.
   - a compact `cities500` + admin-polygon dataset (~100 MB) — named place at city granularity.
2. Default: ship `cities500`; offer the Nominatim dump as an opt-in download in Settings.
3. API: `fn reverse(lat: f64, lon: f64) -> Option<NamedPlace>` returning `{city, region, country}`.
4. Cache results per `(rounded_lat, rounded_lon)`.

### 12. Encrypted public share-links (`crates/core/src/share/public_link.rs`)

1. Link format:
   ```
   https://<peer-relay>/s/<collection_pub_id>#<secret_b64>
   ```
   - `collection_pub_id` is a random 128-bit identifier distinct from `collection.id`.
   - `#<secret_b64>` is the viewer key, never sent to the server (fragment identifier).
2. Optional password: Argon2id(viewer_pwd) wraps the viewer key server-side.
3. Expiry: enforced by the serving peer.
4. Serving peer: any online peer that has the namespace + blobs. No special server.
5. Viewer is a lightweight web page in `app/src-viewer/` (a second Vite project) that:
   - Fetches encrypted manifest from `<peer>/s/<pub_id>/manifest`.
   - Unwraps with the viewer key (from URL fragment + optional password).
   - Streams encrypted blobs, decrypts in-browser via WASM libsodium.
6. Screenshot / download toggle: client-side heuristic (disable right-click, overlay watermark). Weak by nature; document this.

### 13. UI additions

- `ui/share/` — share dialog (pick users or generate public link), active-shares list, revoke.
- `ui/trips/` — list + detail.
- `ui/memories/` — home screen cards.
- `ui/smart_albums/` — builder + detail.
- `ui/pets/` — same shape as People.
- Settings → Peers — pairing QR/ticket, list of accepted peers, revoke.
- Settings → Users — add user, set hidden password, recovery phrase.

### 14. Tests

- `tests/integration/peer_pairing.rs` — two in-process Iroh nodes, pair, share an album, verify the receiver sees metadata + can fetch a blob. Spawn a third unpaired node, verify it cannot read the namespace.
- `tests/integration/ocr_roundtrip.rs` — synthetic receipt images → OCR → search for a known word → expected asset returned.
- `tests/integration/trips.rs` — synthetic geo+time point set → expected trip clusters.
- `tests/integration/smart_album.rs` — rule → expected membership on fixture library.
- `tests/integration/public_link.rs` — generate link, fetch manifest, decrypt with key, verify one asset renders; wrong viewer key → failure.
- `tests/integration/multi_user.rs` — create two users on one vault; user A cannot see user B's albums; share A→B works; B sees it under their login.

---

## File-by-file checklist

| Path | New/Mod | Notes |
|---|---|---|
| `crates/sync/Cargo.toml` | mod | add iroh, iroh-blobs, iroh-docs |
| `crates/sync/src/lib.rs` | mod | re-exports |
| `crates/sync/src/peer.rs` | new | node lifecycle |
| `crates/sync/src/share.rs` | new | album share flow |
| `crates/sync/src/blobs.rs` | new | CAS ↔ iroh-blobs bridge |
| `crates/core/src/db/schema.rs` | mod (additive) | `ml_job` (from P2), `nd_cluster` (P2), `peer_accept`, `shared_namespace`, `public_link`, `pet_cluster`, `trip_cache`, `memory` columns |
| `crates/core/src/ml/ocr.rs` | new | PaddleOCR |
| `crates/core/src/analytics/trips.rs` | new | DBSCAN on geo+time |
| `crates/core/src/analytics/memories.rs` | new | on-this-day + themed |
| `crates/core/src/analytics/slideshow.rs` | new | ffmpeg slideshow renderer |
| `crates/core/src/analytics/smart_albums.rs` | new | rule compile + exec |
| `crates/core/src/analytics/pets.rs` | new | classifier + cluster |
| `crates/core/src/geocode/mod.rs` | new | reverse geocode |
| `crates/core/src/share/public_link.rs` | new | encrypted links |
| `crates/core/src/search/mod.rs` | mod | add text:, place:, person:, trip: |
| `app/src-tauri/src/commands/peer.rs` | new | pair, ticket, list peers |
| `app/src-tauri/src/commands/share.rs` | new | share album + public link |
| `app/src-tauri/src/commands/analytics.rs` | new | trips, memories, smart |
| `app/src/ui/share/*` | new | |
| `app/src/ui/trips/*` | new | |
| `app/src/ui/memories/*` | new | |
| `app/src/ui/smart_albums/*` | new | |
| `app/src/ui/pets/*` | new | |
| `app/src-viewer/` | new | lightweight public-link viewer |
| `scripts/replica.service` | new | systemd template |
| `models/paddleocr-{det,rec,cls}.onnx` | new | bundled |
| `models/pet-classifier.onnx` | new | bundled |

Rough size: +6k Rust, +4k TS, +1k viewer.

---

## Execution order

1. `crates/sync/` + Iroh peer boot (no sharing yet — just run a node).
2. iroh-docs namespace creation per user.
3. Pairing ticket issue + accept (UI).
4. Share flow end-to-end on localhost: two vaults on the same box → pair via localhost ticket → share album → receive.
5. CAS ↔ iroh-blobs bridge (so the received side can fetch ciphertext).
6. Multi-user on same device UI.
7. OCR pipeline + blind FTS5 index.
8. Trip detection.
9. Reverse geocoding integration.
10. Smart albums.
11. Memories + slideshow renderer.
12. Pets classifier.
13. Public share-links + viewer SPA.
14. Offsite replica headless mode.
15. Tests.

Parallelisable: OCR, trips, memories, pets, smart albums, public links — once §5 is done, the rest are independent.

---

## Acceptance criteria (Phase 3 is DONE when all pass)

1. **Pairing**: two Iroh nodes on the same machine exchange a QR ticket, accept, and appear as each other's peers.
2. **Album sharing**: user A shares an album with user B; B sees it in their app after login; a third unpaired node (C) given the same namespace ticket sees encrypted bytes but cannot decrypt the collection key (fails with `KeyOrData`).
3. **Blob fetch**: B opens an asset in the shared album; iroh-blobs fetches the ciphertext from A on demand; decryption in B's client succeeds.
4. **Revocation**: A revokes B; B's next fetch attempts fail; A's remaining members can still read after key rotation.
5. **Multi-user**: two local users created on one vault; each unlocked separately; neither sees the other's private collections. Hidden-vault flag is per-user.
6. **OCR**: fixture of 20 images including receipts / whiteboard photos / text screenshots → FTS5 index returns the correct asset for at least 17/20 target queries.
7. **Trip detection**: fixture with 400 geo-time points produces the 8 ground-truth trip clusters (± 1 cluster allowed).
8. **Memories**: for a fixture library spanning 3 years, "on this day" surfaces the correct set; themed memories produce at least 3 valid cards.
9. **Smart albums**: rule "is_raw AND device_make=SONY AND iso>800" yields the 12 ground-truth assets in the fixture.
10. **Pets**: fixture with 50 pet photos across 4 individual animals (2 dogs, 2 cats) clusters into 4 people-like entries with pairwise F1 ≥ 0.85.
11. **Public link**: generate link, paste in fresh browser, password-gate works, expired link rejected, asset decrypts + renders; link without password ≠ works; wrong viewer key ≠ works.
12. **Replica mode**: a headless peer set up on a second machine (localhost for test) successfully receives ciphertext blobs for a shared namespace and cannot decrypt anything.
13. **Crypto invariants**: Phase 1 + Phase 2 adversarial tests still pass. No plaintext leaks in Iroh-docs entries (use `tracing` with redaction tests).
14. **No regressions**: all Phase 1 + Phase 2 acceptance criteria remain green.

---

## Out of scope for Phase 3

- Mobile apps (Phase 4).
- Video transcript / Whisper (Phase 4).
- Non-destructive edits (Phase 4).
- LLM chat (Phase 4).
- iMazing / WhatsApp / Telegram adapters (Phase 4).
- RAW develop (Phase 4).
- Physical-prints scan (Phase 4).

---

## Security notes & trade-offs to document

- **Offsite replica is trust-minimized** — the replica holds only ciphertext, but it *does* learn which blob hashes exist and when they were written. That leaks presence/timestamps. Document in SECURITY.md.
- **Blind FTS5 index leaks token presence** — an attacker with the vault dir can see *how many distinct tokens* appear in OCR'd images and some per-token frequencies. They cannot read tokens. Document this in SECURITY.md.
- **Public link viewer uses WASM libsodium**. Ship subresource integrity hashes for the WASM. Any peer serving the viewer can swap the HTML — that's a known risk. Mitigation: sign the viewer bundle with the owner's Ed25519; viewer auto-verifies.
- **iroh-docs** doesn't authenticate writes beyond the namespace key. Anyone with the namespace write key can scribble. Only the owner shares the write key; recipients get a read-only "export" of the namespace. Verify this behavior at implementation time.
