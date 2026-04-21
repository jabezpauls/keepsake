//! Crypto layer — all primitive calls to libsodium flow through this module.
//!
//! Public contract (`plans/architecture.md` §2.6):
//!
//! * Argon2id with `sensitive` limits for password-derived KEKs.
//! * XChaCha20-Poly1305-IETF for row AEAD.
//! * XChaCha20-Poly1305 secretstream for file-blob encryption, with a
//!   `MVV1` magic header and 4 MiB plaintext chunks.
//! * X25519 sealed boxes for peer sharing.
//!
//! No other module in the workspace may call libsodium directly.
//!
//! # Safety
//!
//! This module is the only consumer of `libsodium-sys-stable`. Every FFI call
//! is wrapped here; return codes are always checked.

// Selectively re-enable `unsafe` for FFI wrappers. The rest of the crate
// inherits `#![deny(unsafe_code)]` from the workspace lints.
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod envelope;
pub mod format;
pub mod keystore;

pub use envelope::{
    derive_kek_from_password, open_blob_reader, open_from_peer, open_row, seal_blob_writer,
    seal_for_peer, seal_row, unwrap_collection_key, unwrap_file_key, unwrap_master_key,
    wrap_collection_key, wrap_file_key, wrap_master_key, BlobReader, BlobWriter, CollectionKey,
    FileKey, KeyEncryptionKey, MasterKey, PeerKeypair, PeerPublicKey, SearchKey, ARGON2_ALG,
    ARGON2_MEMLIMIT, ARGON2_OPSLIMIT, PLAINTEXT_CHUNK_SIZE, ROW_NONCE_LEN, SALT_LEN,
};
pub use format::{MAGIC, MAGIC_LEN, SECRETSTREAM_HEADER_LEN};

/// One-time libsodium initialisation. Safe to call from many threads.
pub fn init() -> crate::Result<()> {
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, Ordering};

    static INIT: Once = Once::new();
    static OK: AtomicBool = AtomicBool::new(false);
    INIT.call_once(|| {
        // SAFETY: `sodium_init` is thread-safe and re-entrant per libsodium docs.
        let rc = unsafe { libsodium_sys::sodium_init() };
        if rc >= 0 {
            OK.store(true, Ordering::Release);
        }
    });
    if OK.load(Ordering::Acquire) {
        Ok(())
    } else {
        Err(crate::Error::Crypto)
    }
}
