//! On-device ML bundle catalog.
//!
//! Keepsake ships with two model bundles users can choose between at first
//! run. The trade-off is **inference speed × download size × embedding
//! quality**:
//!
//! | Bundle | CLIP | Face | Download | Reindex 2k on 8-core CPU |
//! |--------|------|------|----------|--------------------------|
//! | Full   | ViT-L/14 (768-d) | buffalo_l (SCRFD 10g + ArcFace R100, 512-d) | ~1.5 GB | 30–45 min |
//! | Lite   | ViT-B/32 (512-d) | buffalo_m (SCRFD 2.5g + ArcFace R50, 512-d)  | ~790 MB | 5–8 min |
//!
//! The Full bundle is the GPU-recommended default — on a modern NVIDIA card
//! it reindexes in 1–2 minutes and gives the best semantic-search recall.
//! The Lite bundle trades ~15% search quality for a 5× CPU speedup and a
//! 2× smaller download.
//!
//! Face-embedding dim is held at 512-d across both bundles so a bundle
//! switch doesn't invalidate every face cluster in the DB — only CLIP
//! embeddings must be regenerated (same column, different length). Search
//! cosine-similarity is already dim-agnostic, so the `asset_vec` BLOBs
//! decoded post-switch just won't match the new query embeddings until
//! reindex runs.

use serde::{Deserialize, Serialize};

use super::manifest::ModelEntry;

/// Stable identifier for a bundle. Persisted to `<vault>/models/bundle.json`
/// so the loader knows which SHA-256 pins to verify against on startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash, ts_rs::TS)]
#[ts(export, export_to = "../../../app/src/bindings/")]
#[serde(rename_all = "snake_case")]
pub enum BundleId {
    /// Heavy bundle (ViT-L/14 + buffalo_l). GPU-recommended.
    Full,
    /// Lightweight bundle (ViT-B/32 + buffalo_m). CPU-friendly.
    Lite,
}

impl BundleId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Lite => "lite",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "full" => Self::Full,
            "lite" => Self::Lite,
            _ => return None,
        })
    }
}

/// One file in a bundle. Mirrors [`ModelEntry`] but with the download URL
/// and env override colocated — the catalog is the single source of truth
/// both the verifier and the downloader consult.
#[derive(Debug, Clone, Copy)]
pub struct BundleFile {
    /// Filename in the models directory. Matches the hardcoded names the
    /// loader opens (`clip_visual.onnx` / `arcface.onnx` / ...).
    pub name: &'static str,
    pub sha256_hex: &'static str,
    pub default_url: &'static str,
    /// `MV_MODEL_URL_<KEY>` env var that wins over `default_url` per file.
    pub env_var: &'static str,
    /// Approximate download size in bytes. For wizard UI totals; the real
    /// size comes from the HTTP `Content-Length`.
    pub approx_bytes: u64,
}

/// User-facing description of a bundle. Everything the wizard needs to
/// render the choice screen.
#[derive(Debug, Clone, Copy)]
pub struct BundleSpec {
    pub id: BundleId,
    pub display_name: &'static str,
    pub description: &'static str,
    pub recommended_on: Recommendation,
    /// Expected CLIP embedding dim. Loader asserts the session's actual
    /// output shape matches this; a mismatch is a signal the wrong file
    /// was placed in the directory.
    pub clip_dim: usize,
    /// Expected face-embedding dim. Same deal.
    pub face_dim: usize,
    pub files: &'static [BundleFile],
}

/// Hardware classes the wizard uses to recommend a bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recommendation {
    /// Recommended when the probe reports an accelerator (CUDA / CoreML).
    Gpu,
    /// Recommended when no accelerator is available.
    Cpu,
}

/// File shared by both bundles: the CLIP tokenizer JSON. OpenAI's CLIP BPE
/// is identical for ViT-L/14 and ViT-B/32, so we source it from the
/// ViT-L-14__openai mirror in both cases — same SHA, one less cache miss
/// when switching.
const SHARED_TOKENIZER: BundleFile = BundleFile {
    name: "clip_tokenizer.json",
    sha256_hex: "6d9109cc838977f3ca94a379eec36aecc7c807e1785cd729660ca2fc0171fb35",
    default_url:
        "https://huggingface.co/immich-app/ViT-L-14__openai/resolve/main/textual/tokenizer.json",
    env_var: "MV_MODEL_URL_CLIP_TOKENIZER",
    approx_bytes: 3_640_000,
};

/// ArcFace R50 from `immich-app/buffalo_m/recognition`. Identical bytes to
/// the R50 in buffalo_l, so Full and Lite share this pin — downloading the
/// other bundle after the first doesn't re-fetch it.
const SHARED_ARCFACE: BundleFile = BundleFile {
    name: "arcface.onnx",
    sha256_hex: "4c06341c33c2ca1f86781dab0e829f88ad5b64be9fba56e56bc9ebdefc619e43",
    default_url: "https://huggingface.co/immich-app/buffalo_m/resolve/main/recognition/model.onnx",
    env_var: "MV_MODEL_URL_ARCFACE",
    approx_bytes: 174_383_860,
};

/// The Full bundle: ViT-L/14 + buffalo_l.
pub const FULL: BundleSpec = BundleSpec {
    id: BundleId::Full,
    display_name: "Standard (recommended for GPU)",
    description: "OpenCLIP ViT-L/14 + InsightFace buffalo_l. Highest search recall; \
        best when you have an NVIDIA GPU or Apple Silicon.",
    recommended_on: Recommendation::Gpu,
    clip_dim: 768,
    face_dim: 512,
    files: &[
        BundleFile {
            name: "clip_visual.onnx",
            sha256_hex: "2b02d572f59c509f4b97b9c54a868453cca1a652cd5d60e1d51d0052f055cb8c",
            default_url:
                "https://huggingface.co/immich-app/ViT-L-14__openai/resolve/main/visual/model.onnx",
            env_var: "MV_MODEL_URL_CLIP_VISUAL",
            approx_bytes: 1_237_000_000,
        },
        BundleFile {
            name: "clip_textual.onnx",
            sha256_hex: "9fbe72ea8d36c2effaccedcf7249e3729ad0d9b4af6604b433ecdd0105663c9c",
            default_url:
                "https://huggingface.co/immich-app/ViT-L-14__openai/resolve/main/textual/model.onnx",
            env_var: "MV_MODEL_URL_CLIP_TEXTUAL",
            approx_bytes: 256_000_000,
        },
        SHARED_TOKENIZER,
        BundleFile {
            name: "scrfd.onnx",
            sha256_hex: "5838f7fe053675b1c7a08b633df49e7af5495cee0493c7dcf6697200b85b5b91",
            default_url:
                "https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx",
            env_var: "MV_MODEL_URL_SCRFD",
            approx_bytes: 16_923_827,
        },
        SHARED_ARCFACE,
    ],
};

/// The Lite bundle: ViT-B/32 + buffalo_m. Tuned for CPU-only hosts.
pub const LITE: BundleSpec = BundleSpec {
    id: BundleId::Lite,
    display_name: "Lite (recommended for CPU)",
    description: "OpenCLIP ViT-B/32 + InsightFace buffalo_m. ~5× faster on CPU \
        than Standard, about 15% less semantic-search recall. Smaller download.",
    recommended_on: Recommendation::Cpu,
    clip_dim: 512,
    face_dim: 512,
    files: &[
        BundleFile {
            name: "clip_visual.onnx",
            sha256_hex: "33a3df41ceef21acdf371af00f6dd0456ec1f9eba24d03a7720f9c3734e40859",
            default_url:
                "https://huggingface.co/immich-app/ViT-B-32__openai/resolve/main/visual/model.onnx",
            env_var: "MV_MODEL_URL_CLIP_VISUAL",
            approx_bytes: 351_613_724,
        },
        BundleFile {
            name: "clip_textual.onnx",
            sha256_hex: "b80cf0af751533a6712d92247f0ddc0c95208748bc59a1a27f33e67be6864e3b",
            default_url:
                "https://huggingface.co/immich-app/ViT-B-32__openai/resolve/main/textual/model.onnx",
            env_var: "MV_MODEL_URL_CLIP_TEXTUAL",
            approx_bytes: 254_193_396,
        },
        SHARED_TOKENIZER,
        BundleFile {
            name: "scrfd.onnx",
            sha256_hex: "041f73f47371333d1d17a6fee6c8ab4e6aecabefe398ff32cca4e2d5eaee0af9",
            default_url:
                "https://huggingface.co/immich-app/buffalo_m/resolve/main/detection/model.onnx",
            env_var: "MV_MODEL_URL_SCRFD",
            approx_bytes: 3_292_009,
        },
        SHARED_ARCFACE,
    ],
};

pub const ALL: &[BundleSpec] = &[FULL, LITE];

impl BundleSpec {
    /// Look up one file within the bundle by logical name. Returns `None`
    /// when the manifest drifts from the loader (e.g. the loader asks for
    /// `clip_visual.onnx` but the bundle renamed it) — that should be a
    /// hard build error caught in a unit test.
    #[must_use]
    pub fn file(&self, name: &str) -> Option<&'static BundleFile> {
        self.files.iter().find(|f| f.name == name)
    }

    /// [`ModelEntry`] projection used by `manifest::verify_all`.
    pub fn manifest_entries(&self) -> impl Iterator<Item = ModelEntry> + '_ {
        self.files.iter().map(|f| ModelEntry {
            name: f.name,
            sha256_hex: f.sha256_hex,
        })
    }
}

/// Resolve a bundle by id. Panics are impossible here — every variant of
/// [`BundleId`] has a corresponding constant in [`ALL`].
#[must_use]
pub fn by_id(id: BundleId) -> &'static BundleSpec {
    ALL.iter()
        .find(|b| b.id == id)
        .expect("ALL covers every BundleId")
}

/// Return the bundle recommended for the given hardware class. Used by the
/// wizard to pre-select an option on the choice screen.
#[must_use]
pub fn recommended(have_gpu: bool) -> BundleId {
    if have_gpu {
        BundleId::Full
    } else {
        BundleId::Lite
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_covers_every_id() {
        for id in [BundleId::Full, BundleId::Lite] {
            assert_eq!(by_id(id).id, id, "by_id({:?}) must round-trip", id);
        }
    }

    #[test]
    fn every_bundle_has_the_five_expected_files() {
        let expected = [
            "clip_visual.onnx",
            "clip_textual.onnx",
            "clip_tokenizer.json",
            "scrfd.onnx",
            "arcface.onnx",
        ];
        for b in ALL {
            for name in expected {
                assert!(b.file(name).is_some(), "bundle {:?} missing {name}", b.id);
            }
            assert_eq!(b.files.len(), expected.len(), "extra file in {:?}", b.id);
        }
    }

    #[test]
    fn bundle_id_strings_round_trip() {
        for id in [BundleId::Full, BundleId::Lite] {
            assert_eq!(BundleId::from_str(id.as_str()), Some(id));
        }
        assert!(BundleId::from_str("garbage").is_none());
    }

    #[test]
    fn recommended_matches_hardware_hint() {
        assert_eq!(recommended(true), BundleId::Full);
        assert_eq!(recommended(false), BundleId::Lite);
    }

    #[test]
    fn shas_are_well_formed_hex() {
        for b in ALL {
            for f in b.files {
                assert_eq!(f.sha256_hex.len(), 64, "bad sha len for {}", f.name);
                assert!(
                    f.sha256_hex.chars().all(|c| c.is_ascii_hexdigit()),
                    "non-hex in {}",
                    f.name
                );
            }
        }
    }
}
