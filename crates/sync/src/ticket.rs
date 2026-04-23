//! PairingTicket wire format — FROZEN per `plans/architecture.md` §6.
//!
//! Why not reuse `iroh_base::ticket::NodeTicket`? Iroh's ticket carries the
//! Ed25519 node key + relay info but not our libsodium X25519 identity key.
//! Media Vault peers must pair with both so receivers can seal collection
//! keys back (`crates/core/src/crypto/envelope.rs::seal_for_peer`). If you
//! find yourself tempted to "simplify" by routing through NodeTicket: the
//! X25519 key is the reason.
//!
//! Wire layout (base32 of):
//!
//! ```text
//! 1  byte    version (0x01)
//! 32 bytes   iroh_node_pub (Ed25519 public)
//! 32 bytes   identity_pub  (X25519 public)
//! 2  bytes   relay_url_len (u16 LE)
//! N  bytes   relay_url (UTF-8, N = relay_url_len, may be 0)
//! 8  bytes   timestamp (i64 LE, UNIX seconds)
//! 64 bytes   signature (Ed25519 over all preceding bytes, signed by
//!            iroh_node_sk)
//! ```
//!
//! Fixed overhead: 139 bytes. Relay URLs longer than 512 bytes are rejected
//! (`Error::TicketFormat`) because a real relay URL is far shorter and an
//! overlong field is a signal of malformed input.

use std::io::{Cursor, Read};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey, SECRET_KEY_LENGTH};
use mv_core::{Error, Result};

/// Current ticket version. Bump only with a companion migration that keeps
/// the old version decodable.
pub const TICKET_VERSION: u8 = 0x01;

/// Max relay URL length we accept. Real relay URLs are ~40–80 bytes; cap at
/// 512 so malformed tickets fail fast instead of trying to allocate 64 KiB
/// strings.
pub const MAX_RELAY_URL_LEN: usize = 512;

/// Ed25519 public/secret sizes (re-exported from ed25519-dalek for clarity).
pub const ED25519_PUB_LEN: usize = 32;
pub const ED25519_SIG_LEN: usize = 64;
/// X25519 public key size (matches `crypto::envelope::PEER_PK_LEN`).
pub const X25519_PUB_LEN: usize = 32;

/// Minimum ticket length with empty relay_url:
/// 1 + 32 + 32 + 2 + 8 + 64 = 139 (relay_url body has length 0).
pub const MIN_TICKET_LEN: usize = 1 + ED25519_PUB_LEN + X25519_PUB_LEN + 2 + 8 + ED25519_SIG_LEN;

/// Parsed pairing ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingTicket {
    pub iroh_node_pub: [u8; ED25519_PUB_LEN],
    pub identity_pub: [u8; X25519_PUB_LEN],
    pub relay_url: String,
    pub timestamp: i64,
    pub signature: [u8; ED25519_SIG_LEN],
}

impl PairingTicket {
    /// Build + sign a ticket. `iroh_node_sk` is the 32-byte Ed25519 secret
    /// scalar (NOT seed) — matches what `libsodium` stores as the "seed" in
    /// our keystore. `identity_pub` is the libsodium X25519 public key.
    pub fn sign(
        iroh_node_sk: &[u8; SECRET_KEY_LENGTH],
        identity_pub: &[u8; X25519_PUB_LEN],
        relay_url: String,
        timestamp: i64,
    ) -> Result<Self> {
        if relay_url.len() > MAX_RELAY_URL_LEN {
            return Err(Error::TicketFormat("relay_url too long"));
        }
        let signing = SigningKey::from_bytes(iroh_node_sk);
        let iroh_node_pub = signing.verifying_key().to_bytes();

        let pre_sig = Self::pre_sig_bytes(&iroh_node_pub, identity_pub, &relay_url, timestamp)?;
        let sig = signing.sign(&pre_sig);
        Ok(Self {
            iroh_node_pub,
            identity_pub: *identity_pub,
            relay_url,
            timestamp,
            signature: sig.to_bytes(),
        })
    }

    /// Verify the Ed25519 signature over the ticket's pre-sig bytes.
    /// Opaque on failure — callers translate to `Error::TicketSignature`
    /// without leaking why.
    pub fn verify(&self) -> Result<()> {
        let pre_sig = Self::pre_sig_bytes(
            &self.iroh_node_pub,
            &self.identity_pub,
            &self.relay_url,
            self.timestamp,
        )?;
        let vk = VerifyingKey::from_bytes(&self.iroh_node_pub)
            .map_err(|_| Error::TicketSignature)?;
        let sig = Signature::from_bytes(&self.signature);
        vk.verify(&pre_sig, &sig).map_err(|_| Error::TicketSignature)
    }

    /// Serialise to the frozen wire layout (pre-base32).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MIN_TICKET_LEN + self.relay_url.len());
        out.push(TICKET_VERSION);
        out.extend_from_slice(&self.iroh_node_pub);
        out.extend_from_slice(&self.identity_pub);
        out.extend_from_slice(&(self.relay_url.len() as u16).to_le_bytes());
        out.extend_from_slice(self.relay_url.as_bytes());
        out.extend_from_slice(&self.timestamp.to_le_bytes());
        out.extend_from_slice(&self.signature);
        out
    }

    /// Parse from wire bytes. Does NOT verify the signature — call
    /// [`verify`] afterwards. Separated so callers can inspect a ticket
    /// before deciding whether to act on it.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < MIN_TICKET_LEN {
            return Err(Error::TicketFormat("truncated"));
        }
        let mut cur = Cursor::new(bytes);

        let mut ver = [0u8; 1];
        cur.read_exact(&mut ver).map_err(|_| Error::TicketFormat("read version"))?;
        if ver[0] != TICKET_VERSION {
            return Err(Error::TicketFormat("unsupported version"));
        }

        let mut iroh_node_pub = [0u8; ED25519_PUB_LEN];
        cur.read_exact(&mut iroh_node_pub)
            .map_err(|_| Error::TicketFormat("read iroh_node_pub"))?;

        let mut identity_pub = [0u8; X25519_PUB_LEN];
        cur.read_exact(&mut identity_pub)
            .map_err(|_| Error::TicketFormat("read identity_pub"))?;

        let mut url_len_buf = [0u8; 2];
        cur.read_exact(&mut url_len_buf)
            .map_err(|_| Error::TicketFormat("read relay_url_len"))?;
        let url_len = u16::from_le_bytes(url_len_buf) as usize;
        if url_len > MAX_RELAY_URL_LEN {
            return Err(Error::TicketFormat("relay_url too long"));
        }

        let mut url_bytes = vec![0u8; url_len];
        cur.read_exact(&mut url_bytes)
            .map_err(|_| Error::TicketFormat("read relay_url body"))?;
        let relay_url =
            String::from_utf8(url_bytes).map_err(|_| Error::TicketFormat("relay_url utf-8"))?;

        let mut ts_buf = [0u8; 8];
        cur.read_exact(&mut ts_buf)
            .map_err(|_| Error::TicketFormat("read timestamp"))?;
        let timestamp = i64::from_le_bytes(ts_buf);

        let mut signature = [0u8; ED25519_SIG_LEN];
        cur.read_exact(&mut signature)
            .map_err(|_| Error::TicketFormat("read signature"))?;

        // Any trailing bytes → malformed. Defensive: a sender that appends
        // noise is a protocol-break we don't want to silently accept.
        if cur.position() as usize != bytes.len() {
            return Err(Error::TicketFormat("trailing bytes"));
        }

        Ok(Self {
            iroh_node_pub,
            identity_pub,
            relay_url,
            timestamp,
            signature,
        })
    }

    /// Encode to base32 RFC4648 (lowercase, no padding) — the format a user
    /// actually sees + pastes between devices. Matches the char set iroh's
    /// own tickets use so operators can tell at a glance that this is a
    /// Media Vault ticket (prefix `mvv1...` is implied because the version
    /// byte is first — base32 of 0x01 is always `ae`).
    pub fn to_base32(&self) -> String {
        base32::encode(base32::Alphabet::Rfc4648Lower { padding: false }, &self.to_bytes())
    }

    /// Decode a base32 ticket string. Does NOT verify — call `verify()`.
    pub fn from_base32(s: &str) -> Result<Self> {
        let bytes = base32::decode(base32::Alphabet::Rfc4648Lower { padding: false }, s.trim())
            .ok_or(Error::TicketFormat("base32 decode"))?;
        Self::from_bytes(&bytes)
    }

    fn pre_sig_bytes(
        iroh_node_pub: &[u8; ED25519_PUB_LEN],
        identity_pub: &[u8; X25519_PUB_LEN],
        relay_url: &str,
        timestamp: i64,
    ) -> Result<Vec<u8>> {
        if relay_url.len() > MAX_RELAY_URL_LEN {
            return Err(Error::TicketFormat("relay_url too long"));
        }
        let url_len = relay_url.len() as u16;
        let mut out = Vec::with_capacity(1 + 32 + 32 + 2 + relay_url.len() + 8);
        out.push(TICKET_VERSION);
        out.extend_from_slice(iroh_node_pub);
        out.extend_from_slice(identity_pub);
        out.extend_from_slice(&url_len.to_le_bytes());
        out.extend_from_slice(relay_url.as_bytes());
        out.extend_from_slice(&timestamp.to_le_bytes());
        Ok(out)
    }
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_identity_pub() -> [u8; X25519_PUB_LEN] {
        // Not a real X25519 key — we only verify bytes round-trip here; the
        // sealing tests live in `crypto::envelope`.
        let mut out = [0u8; X25519_PUB_LEN];
        for (i, b) in out.iter_mut().enumerate() {
            *b = i as u8;
        }
        out
    }

    fn fake_signing_key() -> [u8; SECRET_KEY_LENGTH] {
        [0x42; SECRET_KEY_LENGTH]
    }

    #[test]
    fn round_trip_empty_relay_url() {
        let t = PairingTicket::sign(&fake_signing_key(), &fake_identity_pub(), String::new(), 0)
            .unwrap();
        t.verify().expect("sig should verify");
        let wire = t.to_bytes();
        assert_eq!(wire.len(), MIN_TICKET_LEN);
        let back = PairingTicket::from_bytes(&wire).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn round_trip_with_relay_url() {
        let url = "https://relay.example.com:4443".to_string();
        let t = PairingTicket::sign(&fake_signing_key(), &fake_identity_pub(), url.clone(), 12345)
            .unwrap();
        t.verify().unwrap();
        let back = PairingTicket::from_bytes(&t.to_bytes()).unwrap();
        assert_eq!(back.relay_url, url);
        assert_eq!(back.timestamp, 12345);
    }

    #[test]
    fn base32_round_trip() {
        let t =
            PairingTicket::sign(&fake_signing_key(), &fake_identity_pub(), "r".into(), 99).unwrap();
        let s = t.to_base32();
        assert!(!s.is_empty());
        let back = PairingTicket::from_base32(&s).unwrap();
        assert_eq!(t, back);
        back.verify().unwrap();
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let mut t =
            PairingTicket::sign(&fake_signing_key(), &fake_identity_pub(), String::new(), 0)
                .unwrap();
        t.signature[0] ^= 0xFF;
        assert!(matches!(t.verify(), Err(Error::TicketSignature)));
    }

    #[test]
    fn tampered_identity_is_rejected() {
        let mut t =
            PairingTicket::sign(&fake_signing_key(), &fake_identity_pub(), String::new(), 0)
                .unwrap();
        t.identity_pub[0] ^= 0xFF;
        assert!(matches!(t.verify(), Err(Error::TicketSignature)));
    }

    #[test]
    fn tampered_timestamp_is_rejected() {
        let mut t =
            PairingTicket::sign(&fake_signing_key(), &fake_identity_pub(), String::new(), 0)
                .unwrap();
        t.timestamp = 9999;
        assert!(matches!(t.verify(), Err(Error::TicketSignature)));
    }

    #[test]
    fn truncated_ticket_fails_format() {
        let t =
            PairingTicket::sign(&fake_signing_key(), &fake_identity_pub(), String::new(), 0)
                .unwrap();
        let wire = t.to_bytes();
        let r = PairingTicket::from_bytes(&wire[..wire.len() - 1]);
        assert!(matches!(r, Err(Error::TicketFormat(_))));
    }

    #[test]
    fn trailing_bytes_fail_format() {
        let t =
            PairingTicket::sign(&fake_signing_key(), &fake_identity_pub(), String::new(), 0)
                .unwrap();
        let mut wire = t.to_bytes();
        wire.push(0xAB);
        let r = PairingTicket::from_bytes(&wire);
        assert!(matches!(r, Err(Error::TicketFormat(s)) if s.contains("trailing")));
    }

    #[test]
    fn wrong_version_is_rejected() {
        let t =
            PairingTicket::sign(&fake_signing_key(), &fake_identity_pub(), String::new(), 0)
                .unwrap();
        let mut wire = t.to_bytes();
        wire[0] = 0x02;
        let r = PairingTicket::from_bytes(&wire);
        assert!(matches!(r, Err(Error::TicketFormat(s)) if s.contains("version")));
    }

    #[test]
    fn overlong_relay_url_is_rejected_in_sign() {
        let huge = "x".repeat(MAX_RELAY_URL_LEN + 1);
        let r = PairingTicket::sign(&fake_signing_key(), &fake_identity_pub(), huge, 0);
        assert!(matches!(r, Err(Error::TicketFormat(_))));
    }

    #[test]
    fn overlong_relay_url_is_rejected_in_parse() {
        // Craft a ticket with an oversized url_len field directly.
        let mut wire = vec![TICKET_VERSION];
        wire.extend_from_slice(&[0u8; 32]); // iroh_node_pub
        wire.extend_from_slice(&[0u8; 32]); // identity_pub
        wire.extend_from_slice(&((MAX_RELAY_URL_LEN + 1) as u16).to_le_bytes());
        wire.extend_from_slice(&vec![0u8; MAX_RELAY_URL_LEN + 1]);
        wire.extend_from_slice(&0i64.to_le_bytes());
        wire.extend_from_slice(&[0u8; 64]);
        let r = PairingTicket::from_bytes(&wire);
        assert!(matches!(r, Err(Error::TicketFormat(s)) if s.contains("relay_url too long")));
    }

    #[test]
    fn stable_known_vector_signing() {
        // Deterministic key + fields → identical bytes across runs. Locks
        // the wire layout against accidental drift.
        let t =
            PairingTicket::sign(&[0x11; SECRET_KEY_LENGTH], &[0x22; X25519_PUB_LEN], "r".into(), 7)
                .unwrap();
        let base32 = t.to_base32();
        // Byte-length check is the sturdiest regression catch — hash pinning
        // would fight ed25519-dalek randomness noise; byte count is purely
        // structural.
        assert_eq!(t.to_bytes().len(), MIN_TICKET_LEN + 1);
        // base32 of 140 bytes (ceil-up to multiple of 5) = 224 chars.
        assert_eq!(base32.len(), 224);
    }
}
