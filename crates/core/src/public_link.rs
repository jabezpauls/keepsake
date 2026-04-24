//! Public share-link crypto (D7).
//!
//! A "public link" authorises a random, revocable URL to render one
//! collection through a browser viewer. URL shape:
//!
//!   `https://<host>/s/<pub_id_base32>[#<viewer_key_b64>]`
//!
//! - `pub_id` — 16 random bytes, base32-encoded in the URL path.
//!   The hosting peer looks up the `public_link` row by this id.
//! - `viewer_key` — 32 random bytes. The real decryption key for the
//!   manifest the hosting peer serves. Never written to the URL path
//!   and never seen by the relay (fragments stay client-side).
//!
//! Two modes:
//!
//! - **No password**: the `viewer_key` travels in the URL fragment.
//!   Anyone with the full URL can decrypt. `wrapped_key` is absent.
//!   URL fragment leak = full compromise, so this mode is for
//!   contexts where the sender can authenticate the channel (signal,
//!   keybase, etc.).
//!
//! - **Password-gated**: the URL fragment is empty; `wrapped_key`
//!   holds the `viewer_key` sealed under `KEK = Argon2id(password,
//!   salt)`. The viewer prompts, derives the KEK in-browser (slow —
//!   ~3s), unwraps `viewer_key`, then decrypts the manifest. URL
//!   alone is insufficient; URL + password are required.
//!
//! `expires_at` is enforced at fetch time — a row whose `expires_at`
//! is in the past is treated as revoked. Revoking outright deletes
//! the row, so the pub_id stops resolving immediately.
//!
//! The viewer SPA + hosting-peer HTTP gateway are out of scope for
//! this module; it stops at the crypto + DB-column format.

use secrecy::SecretString;

use crate::crypto::envelope::{
    derive_kek_from_password, random_bytes, unwrap_with_key, wrap_with_key, SALT_LEN,
};
use crate::{Error, Result};

/// Length of the `pub_id` — the public URL-path identifier. 16 bytes
/// (128 bits) is enough to be infeasible to enumerate while staying
/// short enough for a readable URL (26 chars in RFC4648 base32).
pub const PUB_ID_LEN: usize = 16;

/// Length of the viewer key. Same shape as a `CollectionKey` —
/// 32 bytes of CSPRNG output.
pub const VIEWER_KEY_LEN: usize = 32;

/// One freshly generated public link ready to persist + surface.
#[derive(Debug)]
pub struct NewPublicLink {
    /// Random path component (16 bytes). Insert into `public_link.pub_id`.
    pub pub_id: [u8; PUB_ID_LEN],
    /// Base32-encoded form of `pub_id`. The hosting peer puts this in
    /// the URL path; the viewer parses it back.
    pub pub_id_b32: String,
    /// The key the manifest will be encrypted with. Always stored
    /// somewhere — either in the URL fragment (no password) or
    /// wrapped in `wrapped_key` (password).
    pub viewer_key: [u8; VIEWER_KEY_LEN],
    /// Present when `has_password = true`. Argon2id-sealed
    /// `viewer_key`. Persist into `public_link.wrapped_key`.
    pub wrapped_key: Option<Vec<u8>>,
    /// Present when `has_password = true`. Random 16-byte salt used
    /// to derive the KEK from the password. Persist into
    /// `public_link.password_salt`.
    pub password_salt: Option<[u8; SALT_LEN]>,
    /// Convenience flag mirroring `wrapped_key.is_some()` — consumers
    /// often want this without an `Option::is_some` call.
    pub has_password: bool,
}

impl NewPublicLink {
    /// Build a URL fragment suitable for `#<fragment>` in the link.
    /// Empty string when password-gated (the fragment carries no
    /// secret in that mode).
    pub fn url_fragment(&self) -> String {
        if self.has_password {
            String::new()
        } else {
            base32_lower(&self.viewer_key)
        }
    }
}

/// Generate a fresh public-link record. `password = None` puts the
/// viewer key in the URL fragment; `password = Some(_)` wraps it under
/// Argon2id + a fresh salt.
pub fn generate(password: Option<&SecretString>) -> Result<NewPublicLink> {
    let mut pub_id = [0u8; PUB_ID_LEN];
    random_bytes(&mut pub_id)?;
    let mut viewer_key = [0u8; VIEWER_KEY_LEN];
    random_bytes(&mut viewer_key)?;

    let (wrapped_key, password_salt, has_password) = match password {
        Some(pw) => {
            let mut salt = [0u8; SALT_LEN];
            random_bytes(&mut salt)?;
            let kek = derive_kek_from_password(pw, &salt)?;
            let wrapped = wrap_with_key(&viewer_key, kek.as_bytes())?;
            (Some(wrapped), Some(salt), true)
        }
        None => (None, None, false),
    };

    Ok(NewPublicLink {
        pub_id,
        pub_id_b32: base32_lower(&pub_id),
        viewer_key,
        wrapped_key,
        password_salt,
        has_password,
    })
}

/// Viewer-side: recover the `viewer_key` from the wrapped form. Used
/// in tests + any Rust-side viewer (the browser viewer does the same
/// computation in WASM libsodium, not via this function).
pub fn open_with_password(
    wrapped_key: &[u8],
    salt: &[u8; SALT_LEN],
    password: &SecretString,
) -> Result<[u8; VIEWER_KEY_LEN]> {
    let kek = derive_kek_from_password(password, salt)?;
    let plain = unwrap_with_key(wrapped_key, kek.as_bytes())?;
    if plain.len() != VIEWER_KEY_LEN {
        return Err(Error::Crypto);
    }
    let mut out = [0u8; VIEWER_KEY_LEN];
    out.copy_from_slice(&plain);
    Ok(out)
}

/// Whether the link is currently serveable. `expires_at = None` means
/// no expiry; otherwise compare to `now_seconds`.
pub fn is_live(expires_at: Option<i64>, now_seconds: i64) -> bool {
    match expires_at {
        None => true,
        Some(ts) => now_seconds < ts,
    }
}

fn base32_lower(bytes: &[u8]) -> String {
    base32::encode(base32::Alphabet::Rfc4648 { padding: false }, bytes).to_ascii_lowercase()
}

/// Parse a base32-encoded `pub_id` back into its 16-byte form. Case
/// insensitive, padding optional — defends against operators pasting
/// copy-paste-adjusted variants.
pub fn parse_pub_id_b32(input: &str) -> Result<[u8; PUB_ID_LEN]> {
    let upper = input.to_ascii_uppercase();
    let bytes = base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &upper)
        .ok_or(Error::TicketFormat("pub_id base32"))?;
    if bytes.len() != PUB_ID_LEN {
        return Err(Error::TicketFormat("pub_id length"));
    }
    let mut out = [0u8; PUB_ID_LEN];
    out.copy_from_slice(&bytes);
    Ok(out)
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_password_link_carries_key_in_fragment() {
        let link = generate(None).unwrap();
        assert!(!link.has_password);
        assert!(link.wrapped_key.is_none());
        assert!(link.password_salt.is_none());
        let frag = link.url_fragment();
        assert!(!frag.is_empty());
        // Fragment is the viewer key in base32.
        let decoded = base32::decode(
            base32::Alphabet::Rfc4648 { padding: false },
            &frag.to_uppercase(),
        )
        .unwrap();
        assert_eq!(decoded, link.viewer_key);
    }

    #[test]
    fn password_link_wraps_viewer_key_and_empty_fragment() {
        let pw = SecretString::from("hunter2-for-the-family");
        let link = generate(Some(&pw)).unwrap();
        assert!(link.has_password);
        let wrapped = link.wrapped_key.as_ref().unwrap();
        let salt = link.password_salt.as_ref().unwrap();
        // URL fragment carries no secret in password mode.
        assert!(link.url_fragment().is_empty());

        // Correct password round-trips.
        let recovered = open_with_password(wrapped, salt, &pw).unwrap();
        assert_eq!(recovered, link.viewer_key);
    }

    #[test]
    fn wrong_password_fails_unwrap() {
        let pw = SecretString::from("correct-horse-staple");
        let link = generate(Some(&pw)).unwrap();
        let wrong = SecretString::from("wrong-horse-battery-staple");
        let wrapped = link.wrapped_key.as_ref().unwrap();
        let salt = link.password_salt.as_ref().unwrap();
        assert!(open_with_password(wrapped, salt, &wrong).is_err());
    }

    #[test]
    fn each_link_gets_independent_pub_id_and_viewer_key() {
        let a = generate(None).unwrap();
        let b = generate(None).unwrap();
        assert_ne!(a.pub_id, b.pub_id);
        assert_ne!(a.viewer_key, b.viewer_key);
    }

    #[test]
    fn pub_id_roundtrips_through_base32() {
        let link = generate(None).unwrap();
        let parsed = parse_pub_id_b32(&link.pub_id_b32).unwrap();
        assert_eq!(parsed, link.pub_id);
        // Case-insensitivity contract.
        let upper = link.pub_id_b32.to_ascii_uppercase();
        assert_eq!(parse_pub_id_b32(&upper).unwrap(), link.pub_id);
    }

    #[test]
    fn malformed_pub_id_rejected() {
        assert!(parse_pub_id_b32("not-valid-base32!").is_err());
        // Right alphabet, wrong length.
        let too_short = base32::encode(base32::Alphabet::Rfc4648 { padding: false }, b"short");
        assert!(parse_pub_id_b32(&too_short).is_err());
    }

    #[test]
    fn expiry_gate() {
        assert!(is_live(None, 1_700_000_000));
        assert!(is_live(Some(1_700_000_001), 1_700_000_000));
        assert!(!is_live(Some(1_700_000_000), 1_700_000_000));
        assert!(!is_live(Some(1_699_999_999), 1_700_000_000));
    }
}
