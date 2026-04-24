//! Pet classifier (D9 follow-up).
//!
//! A small softmax classifier head over CLIP-ish image features (or a
//! standalone lightweight MobileNet variant). Labels are coarse:
//! `Dog | Cat | Bird | None`. Runs on every ingested image; when the
//! top label is not `None`, we write `asset.is_pet = 1` plus a sealed
//! species string into `pet_species_ct`.
//!
//! This slice ships the shape (labels, storage format, helpers) so
//! the `MlJobKind::ClassifyPet` variant and the ingest queue wiring
//! can land without weights. A follow-up drops in the ONNX runner
//! once model weights are chosen and pinned.
//!
//! Manual flagging via the `set_asset_pet` Tauri command works today
//! and shares the same DB columns, so the Pets tab has content before
//! any classifier runs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PetSpecies {
    Dog,
    Cat,
    Bird,
}

impl PetSpecies {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dog => "dog",
            Self::Cat => "cat",
            Self::Bird => "bird",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "dog" => Self::Dog,
            "cat" => Self::Cat,
            "bird" => Self::Bird,
            _ => return None,
        })
    }
}

/// Top-1 classifier result. `None` species means "the top label was
/// the no-pet bucket" — callers clear the `is_pet` flag in that case.
#[derive(Debug, Clone)]
pub struct PetDecision {
    pub species: Option<PetSpecies>,
    /// Softmax probability of the winning label, in `[0, 1]`.
    pub confidence: f32,
}

impl PetDecision {
    /// Whether this decision should flip `asset.is_pet = 1`. A
    /// conservative default (`confidence >= 0.6`) avoids false
    /// positives from ambiguous photos; callers may pass a higher
    /// threshold.
    pub fn is_pet(&self, threshold: f32) -> bool {
        self.species.is_some() && self.confidence >= threshold
    }
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn species_roundtrips() {
        for s in [PetSpecies::Dog, PetSpecies::Cat, PetSpecies::Bird] {
            assert_eq!(PetSpecies::from_str(s.as_str()), Some(s));
        }
        assert!(PetSpecies::from_str("hamster").is_none());
    }

    #[test]
    fn is_pet_threshold_gate() {
        let d = PetDecision {
            species: Some(PetSpecies::Dog),
            confidence: 0.72,
        };
        assert!(d.is_pet(0.6));
        assert!(!d.is_pet(0.8));

        let none = PetDecision {
            species: None,
            confidence: 0.99,
        };
        assert!(!none.is_pet(0.0));
    }
}
