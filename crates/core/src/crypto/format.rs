//! Wire-format constants and parsing for Media Vault blobs and rows.
//!
//! FROZEN per `plans/architecture.md` §2.4 and §2.5. Any change to the
//! on-disk layout requires a full migration release — do not edit without
//! a migration plan.

use crate::{Error, Result};

/// Magic bytes at the start of every vault blob. Readers reject any file that
/// doesn't begin with these four bytes before doing any crypto work.
pub const MAGIC: [u8; 4] = *b"MVV1";

pub const MAGIC_LEN: usize = MAGIC.len();

/// Length of the secretstream header written immediately after the magic.
/// libsodium exposes this as `crypto_secretstream_xchacha20poly1305_HEADERBYTES == 24`.
pub const SECRETSTREAM_HEADER_LEN: usize = 24;

/// Parse the blob prelude (magic + secretstream header) from `buf`.
///
/// Returns the secretstream header on success, or [`Error::BlobFormat`] if
/// the magic doesn't match, so the caller can distinguish "not our file" from
/// "crypto failure during streaming".
pub fn parse_blob_prelude(buf: &[u8]) -> Result<&[u8; SECRETSTREAM_HEADER_LEN]> {
    if buf.len() < MAGIC_LEN + SECRETSTREAM_HEADER_LEN {
        return Err(Error::BlobFormat);
    }
    if buf[..MAGIC_LEN] != MAGIC {
        return Err(Error::BlobFormat);
    }
    let header_slice: &[u8] = &buf[MAGIC_LEN..MAGIC_LEN + SECRETSTREAM_HEADER_LEN];
    let header: &[u8; SECRETSTREAM_HEADER_LEN] = header_slice
        .try_into()
        .map_err(|_| Error::BlobFormat)?;
    Ok(header)
}

/// Serialise `magic || header` into a contiguous buffer for writing.
pub fn serialise_blob_prelude(header: &[u8; SECRETSTREAM_HEADER_LEN]) -> Vec<u8> {
    let mut out = Vec::with_capacity(MAGIC_LEN + SECRETSTREAM_HEADER_LEN);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(header);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_is_four_bytes() {
        assert_eq!(MAGIC.len(), 4);
        assert_eq!(&MAGIC, b"MVV1");
    }

    #[test]
    fn prelude_round_trip() {
        let header = [7u8; SECRETSTREAM_HEADER_LEN];
        let buf = serialise_blob_prelude(&header);
        assert_eq!(buf.len(), MAGIC_LEN + SECRETSTREAM_HEADER_LEN);
        let parsed = parse_blob_prelude(&buf).unwrap();
        assert_eq!(parsed, &header);
    }

    #[test]
    fn prelude_rejects_bad_magic() {
        let mut buf = vec![0u8; MAGIC_LEN + SECRETSTREAM_HEADER_LEN];
        buf[..MAGIC_LEN].copy_from_slice(b"XXXX");
        assert!(matches!(parse_blob_prelude(&buf), Err(Error::BlobFormat)));
    }

    #[test]
    fn prelude_rejects_short_input() {
        let buf = vec![0u8; MAGIC_LEN + SECRETSTREAM_HEADER_LEN - 1];
        assert!(matches!(parse_blob_prelude(&buf), Err(Error::BlobFormat)));
    }
}
