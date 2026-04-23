//! Crate-wide error hierarchy.
//!
//! Rules (see `plans/phase-1-foundation.md` §2 and `plans/architecture.md` §9):
//!
//! 1. Crypto failures are deliberately vague. `Crypto` and `KeyOrData` do not
//!    distinguish "wrong key" from "tampered ciphertext" from "truncated input".
//!    Callers MUST NOT leak which is which to the UI or logs above `debug` level.
//! 2. Library crates use this enum; the app layer may wrap it in `anyhow::Error`
//!    at the Tauri boundary, where errors are normalised to strings for the
//!    frontend.
//! 3. Variants intentionally do not carry inner error payloads for crypto/data
//!    cases — the cost of leaking internals via `Display` is not worth the
//!    debugging convenience. Use `tracing::debug!` for FFI-level detail.

#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Any failure from the crypto envelope layer that isn't a clear
    /// "key wrong or data corrupt" signal — FFI errors, missing randomness,
    /// invalid parameter sizes, allocation failures.
    #[error("crypto failure")]
    Crypto,

    /// The bytes on disk or in a row don't parse as a valid vault blob/row.
    /// Distinct from `KeyOrData` so ingest code can tell "this file isn't
    /// ours" from "this file is ours but unreadable".
    #[error("invalid blob format")]
    BlobFormat,

    /// Decryption failed. Intentionally ambiguous between wrong key, tampered
    /// ciphertext, truncated input, or authentication-tag mismatch.
    #[error("key wrong or data corrupt")]
    KeyOrData,

    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),

    #[error("db: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("media: {0}")]
    Media(String),

    #[error("ingest: {0}")]
    Ingest(String),

    #[error("not found")]
    NotFound,

    /// Operation requires an unlocked key material that isn't loaded.
    /// The UI should prompt for unlock; callers must not reveal *which* key
    /// (main vs. hidden vs. album).
    #[error("locked")]
    Locked,

    /// Returned when password unlock attempts exceed the backoff threshold.
    /// The UI should show a generic "try again in N seconds" message and
    /// never reveal the counter's granularity.
    #[error("rate limited")]
    RateLimited,

    /// The on-device ML runtime isn't available — either the build lacks the
    /// `ml-models` feature flag or the model files aren't present on disk.
    /// UI surfaces this as a prompt to run `scripts/download_models.sh`.
    #[error("models unavailable")]
    ModelsUnavailable,

    /// Model file present but its SHA-256 doesn't match the manifest. Ships a
    /// specific variant so the UI can direct the user to re-download rather
    /// than hitting the generic `ModelsUnavailable` path.
    #[error("model checksum mismatch: {0}")]
    MlModelChecksum(&'static str),

    /// Model loaded but its ONNX output tensor layout doesn't match what our
    /// post-processor expects (e.g. SCRFD export with a different head count).
    /// See `ml::manifest` for pinned expectations.
    #[error("model shape mismatch: {0}")]
    MlModelShape(&'static str),

    /// A pairing-ticket byte layout doesn't match architecture.md §6 — too
    /// short, wrong version byte, overlong relay_url, etc. String hint names
    /// the exact mismatch so a operator staring at an imported ticket knows
    /// whether to re-copy or re-generate.
    #[error("pairing ticket format: {0}")]
    TicketFormat(&'static str),

    /// Pairing ticket parsed but its Ed25519 signature is invalid for the
    /// claimed iroh_node_pub. Intentionally opaque — cannot distinguish
    /// wrong-key from tampered-bytes, matching the Crypto / KeyOrData
    /// convention in §1.
    #[error("pairing ticket signature invalid")]
    TicketSignature,

    /// The remote peer couldn't be reached (network unreachable, relay
    /// timeout, refused). Distinct from the crypto variants so the UI can
    /// surface a retry prompt.
    #[error("peer unreachable")]
    PeerUnreachable,
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_io_error() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let err: Error = io.into();
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn crypto_display_is_opaque() {
        // The crypto variant must not leak implementation detail. Anything
        // more descriptive than "crypto failure" is a regression.
        assert_eq!(Error::Crypto.to_string(), "crypto failure");
        assert_eq!(Error::KeyOrData.to_string(), "key wrong or data corrupt");
    }

    #[test]
    fn locked_and_rate_limited_are_generic() {
        // These must not mention "hidden", "album", or "master" — the UI must
        // not be able to distinguish a hidden-vault lockout from a main one.
        assert_eq!(Error::Locked.to_string(), "locked");
        assert_eq!(Error::RateLimited.to_string(), "rate limited");
    }

    #[test]
    fn media_and_ingest_carry_context() {
        let m = Error::Media("unsupported codec".into());
        assert_eq!(m.to_string(), "media: unsupported codec");
        let i = Error::Ingest("source path vanished".into());
        assert_eq!(i.to_string(), "ingest: source path vanished");
    }
}
