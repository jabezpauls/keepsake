# Media Vault — Build Plan

Phase-by-phase build instructions for a **local-first, end-to-end encrypted, peer-to-peer personal media management app** that solves the "iPhone 512 GB gets full, I dump media to disk, now I can't find anything" problem — with ingest of arbitrary heterogeneous backup dumps, full iPhone-Photos feature surface, per-album passwords, and a hidden vault.

Originating research, rationale, and "goated feature" reasoning live in [`/home/jabe/.claude/plans/so-basically-i-am-jiggly-candy.md`](../../.claude/plans/so-basically-i-am-jiggly-candy.md). This folder is the execution plan.

---

## How to use this folder

Agents execute phases in order. Within a phase, independent tasks can run in parallel.

1. **Every session starts with `architecture.md`.** That document is the set of non-negotiable contracts. If anything in a phase doc contradicts it, architecture.md wins.
2. Open the phase doc you're assigned.
3. Check what's already built (`ls crates/`, read the tip of each module) — the phase doc is the *target* state.
4. Execute tasks in the order given unless you have a concrete reason to reorder.
5. A phase is "done" only when its **Acceptance criteria** all pass. No partial completes.

---

## Documents

| File | What it's for | Read when |
|---|---|---|
| [`architecture.md`](architecture.md) | **Frozen contracts.** Crypto envelope, CAS layout, DB schema, provenance, peer identity, code conventions, testing rules. Binding across all phases. | Every session. Before writing any code. |
| [`phase-1-foundation.md`](phase-1-foundation.md) | Scaffolding, crypto, CAS, schema, ingest adapters, XMP sidecar, minimal timeline UI. Single local user. | Phase 1. |
| [`phase-2-browsing.md`](phase-2-browsing.md) | Timeline polish, map, faces/people, CLIP natural-language search, near-dup + burst clustering, Live/Motion/RAW pair handling. | After Phase 1. |
| [`phase-3-peers-smart.md`](phase-3-peers-smart.md) | Iroh peer-to-peer sync, multi-user on same device, shared albums, offsite-as-peer, OCR, trips, memories, smart albums, pets, encrypted public share links. | After Phase 2. |
| [`phase-4-extras.md`](phase-4-extras.md) | Whisper transcripts, mobile companion (Tauri iOS/Android), non-destructive edits, iMazing/iTunes/WhatsApp/Telegram adapters, local LLM chat, RAW develop, physical-print scan. | After Phase 3. |

---

## Global invariants (cheat sheet — full detail in architecture.md)

These cannot be violated anywhere in any phase:

1. **Crypto primitives are limited to libsodium + BLAKE3 + Argon2id.** Never hand-rolled, never silently substituted.
2. **All subject-matter metadata is AEAD-encrypted on disk** — filenames, GPS, EXIF, embeddings, OCR text, face vectors, album names, person names. Only `blake3_plaintext`, `bytes`, `width`, `height`, `duration_ms`, `mime`, `taken_at_utc` (day granularity), `source_id`, `imported_at`, `is_video`, `is_raw` are in plaintext.
3. **All crypto goes through `crates/core/src/crypto/envelope.rs`.** No ad-hoc calls to libsodium from elsewhere.
4. **Encrypted CAS is the source of truth.** Originals-in-place is a staged optional mode, not the default.
5. **The DB is never a roach motel.** Every metadata edit round-trips to XMP sidecars on export.
6. **ML runs entirely in Rust via `ort`.** Never spawn a Python process.
7. **Every install is a full peer.** No daemon/client split. Multi-user family works via Iroh + X25519-wrapped collection keys.
8. **Phase 1's crypto envelope, CAS layout, schema split, provenance format, and peer-identity format are immutable.** No schema changes after Phase 1 ships without a migration release.

---

## Repo layout (target)

```
/home/jabe/Workspace/media-view/
├── Cargo.toml                     # workspace
├── plans/                         # this folder
├── crates/
│   ├── core/                      # library — crypto, CAS, db, ingest, media, ml, search
│   └── sync/                      # Phase 3 — Iroh peer node
├── app/
│   ├── src-tauri/                 # Tauri v2 Rust shell
│   └── src/                       # React + TypeScript UI
├── models/                        # ONNX model files (Phase 2+)
├── tests/
│   ├── fixtures/                  # golden iPhone/Takeout/near-dup dumps
│   └── integration/
└── scripts/                       # codegen, model export, dev tooling
```

---

## Phase verification summary

Each phase declares **Acceptance criteria** at the bottom of its doc. The phase is not complete until all pass. High-level:

- **Phase 1**: can ingest an iPhone folder + a Takeout zip, timeline renders, an album is created + unlocked by per-album password, hidden-vault unlock gesture works, XMP export round-trips, all crypto round-trip tests green, encrypted vault is indistinguishable from random without the password.
- **Phase 2**: CLIP natural-language search returns correct top-k on a 10k-asset fixture library, face clusters can be named/merged/split, map + timeline render at 60 fps on 500k assets, near-dup clusters collapse bursts correctly.
- **Phase 3**: two peers pair via QR ticket, shared album syncs without exposing plaintext to a third peer, OCR + FTS5 find a word in a screenshot, trips auto-group, memories surface "on this day" cards, encrypted public link works behind password + expires.
- **Phase 4**: feature flags for Whisper, mobile, edits, LLM chat are all shippable individually.

---

## If you are an agent taking on a phase

- You **don't** need to re-derive architectural decisions — they're in `architecture.md`. If you think one is wrong, flag it to the human instead of changing it.
- You **do** need to verify current repo state before editing. Run `ls crates/` / `ls app/src/` and read the top of any file you touch.
- Use `cargo check --workspace` + `cargo test --workspace` after each meaningful change.
- Prefer small commits that pass CI over big commits that don't.
- If a task in a phase doc is underspecified, make the smallest reasonable call, note it in the commit message, and flag to the human.
