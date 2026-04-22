# Security model

Media Vault is local-first and end-to-end encrypted. This document enumerates
what's protected, what leaks, and the decisions behind the trade-offs.

## Threat model

Trusted:
- The user's unlocked session (master key + per-collection keys in RAM).
- The host OS process running Claude Code / Tauri with the user's privileges.

Untrusted:
- Anyone with file-system-level access to the vault directory.
- Peers on the sync network (Phase 3).
- The database file (`index.db`) read by a cold-storage forensic tool.

## Primary invariants

1. **No plaintext asset bytes on disk.** Every asset in the CAS is
   XChaCha20-Poly1305 sealed with a per-asset `FileKey`. Thumbnails and
   derivatives share the asset's `FileKey` and live in the same CAS.
2. **No plaintext row content for sensitive fields.** `filename_ct`,
   `taken_at_utc_ct`, `gps_ct`, `device_ct`, `lens_ct`, `exif_all_ct`,
   `clip_embedding_ct`, `bbox_ct`, `embedding_ct` are all AEAD-sealed under
   the owning collection key.
3. **Wrong-password responses are indistinguishable** from:
   - "no user" for initial unlock,
   - "no hidden vault" for the hidden-vault gesture,
   - "album exists but has no password" for password albums (returns empty).

## Accepted leakage

The following are **plaintext by design** in `index.db`. The architecture
trade-off is documented in `plans/architecture.md §4.3`.

| Plaintext | Why | Risk |
|---|---|---|
| `asset.blake3_plaintext` | Dedupe key | An attacker with DB access can confirm they have a file you have by hashing theirs. |
| `asset.mime`, `bytes`, `width`, `height`, `duration_ms` | Timeline/grid layout | Media format mix is known. |
| `asset.taken_at_utc_day` | Sticky month headers + timeline paging | Day-granularity date of every asset leaks (not time-of-day). |
| `asset.is_video`, `is_raw`, `is_screenshot`, `is_live`, `is_motion` | Fast flag filters | Flags leak. |
| `asset.source_id`, `imported_at`, `cas_ref` | Provenance + CAS lookup | Provenance links leak. |
| `phash.hash` | Near-dup LSH bucketing | dhash leaks perceptual similarity. |
| `asset_vec.embedding` | CLIP vector search | **Whole embedding leaks.** An attacker can reconstruct approximate scene content (if they control a CLIP text encoder and the same ONNX weights). This is Phase 2's most load-bearing leak. |
| `asset_location.path_hash` | Re-ingest dedupe | Keyed BLAKE3 prefix; without the collection key it's indistinguishable from random. |
| `nd_cluster` | Near-dup review UI | Cluster shape leaks. |
| `ml_job` | Worker queue | Pending work counts + per-asset kind leak. |

Phase 3 moves `asset_vec` to encrypted-search-index (searchable-encryption or
private-information-retrieval) and `phash` to a bucketed-hash scheme; those
reductions are explicitly deferred until the Phase-2 search latencies are
profiled against the plaintext baseline.

## Crypto primitives (frozen)

See `plans/architecture.md §2.6`. Changing any of these requires a major
migration release.

- KDF: Argon2id, `SENSITIVE` parameters (1 GiB RAM, 4 passes).
- Row AEAD: XChaCha20-Poly1305.
- Blob AEAD: libsodium secretstream, 4 MiB chunks, `TAG_FINAL` on last.
- Peer seal: X25519 → HChaCha20-Poly1305.
- Identity: Ed25519 (Iroh) + X25519 (app-layer sealing).
- Hashes: BLAKE3 for plaintext dedupe; SHA-256 (embedded) for CAS object
  integrity check under `verify`.

## ML-model safety

- Model files are verified by SHA-256 before loading. Mismatches abort
  `scripts/download_models.sh` — a compromised download can't silently ship
  a different CLIP or face embedding.
- ONNX Runtime is a system-level dependency, not bundled; updates flow
  through the user's package manager.
- Off-flag builds (default) have no network code in the ML path — it simply
  doesn't exist in the binary.

## Reporting

Open an issue on the repo for non-urgent findings. For anything that
concerns a leak beyond what's listed above, email
`auspicestech@gmail.com` rather than filing publicly.
