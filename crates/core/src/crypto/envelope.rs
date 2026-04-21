//! Crypto envelope — the only caller of libsodium in the workspace.
//!
//! FROZEN API: `plans/architecture.md` §2.6. Every primitive choice,
//! parameter, and on-wire format is locked for Phase 1 onward.

use std::io::{Read, Write};

use libsodium_sys as ls;
use secrecy::{ExposeSecret, SecretBox, SecretString};
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::format::{
    parse_blob_prelude, serialise_blob_prelude, MAGIC_LEN, SECRETSTREAM_HEADER_LEN,
};
use super::init as sodium_init;
use crate::{Error, Result};

// --------- FROZEN CONSTANTS ---------------------------------------------------

/// Argon2id operations limit — libsodium `crypto_pwhash_OPSLIMIT_SENSITIVE`.
pub const ARGON2_OPSLIMIT: u64 = 4;

/// Argon2id memory limit — libsodium `crypto_pwhash_MEMLIMIT_SENSITIVE` (1 GiB).
pub const ARGON2_MEMLIMIT: usize = 1_073_741_824;

/// Algorithm identifier for Argon2id v1.3 in libsodium.
pub const ARGON2_ALG: i32 = ls::crypto_pwhash_ALG_ARGON2ID13 as i32;

/// Argon2id salt length in bytes.
pub const SALT_LEN: usize = ls::crypto_pwhash_SALTBYTES as usize;

/// Nonce length for row AEAD (XChaCha20-Poly1305-IETF).
pub const ROW_NONCE_LEN: usize = ls::crypto_aead_xchacha20poly1305_ietf_NPUBBYTES as usize;

/// Tag/MAC length appended to row AEAD ciphertexts.
pub const ROW_TAG_LEN: usize = ls::crypto_aead_xchacha20poly1305_ietf_ABYTES as usize;

/// Wrapping constants for secret-box-based key wrapping.
const SECRETBOX_KEY_LEN: usize = ls::crypto_secretbox_KEYBYTES as usize;
const SECRETBOX_NONCE_LEN: usize = ls::crypto_secretbox_NONCEBYTES as usize;
const SECRETBOX_TAG_LEN: usize = ls::crypto_secretbox_MACBYTES as usize;

/// Plaintext chunk size for the secretstream blob writer — exactly 4 MiB.
pub const PLAINTEXT_CHUNK_SIZE: usize = 4 * 1024 * 1024;

const SECRETSTREAM_STATE_LEN: usize = std::mem::size_of::<ls::crypto_secretstream_xchacha20poly1305_state>();
const SECRETSTREAM_ABYTES: usize = ls::crypto_secretstream_xchacha20poly1305_ABYTES as usize;

/// X25519 sealed-box public-key length.
pub const PEER_PK_LEN: usize = ls::crypto_box_PUBLICKEYBYTES as usize;
/// X25519 sealed-box secret-key length.
pub const PEER_SK_LEN: usize = ls::crypto_box_SECRETKEYBYTES as usize;
/// Overhead of a sealed box.
pub const PEER_SEAL_OVERHEAD: usize = ls::crypto_box_SEALBYTES as usize;

// --------- KEY NEWTYPES -------------------------------------------------------

macro_rules! define_key {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Zeroize, ZeroizeOnDrop)]
        pub struct $name([u8; 32]);

        impl $name {
            #[must_use]
            pub fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            #[must_use]
            pub fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            /// Generate a new random 32-byte key via libsodium's CSPRNG.
            pub fn random() -> Result<Self> {
                sodium_init()?;
                let mut buf = [0u8; 32];
                // SAFETY: libsodium's randombytes_buf writes exactly len bytes.
                unsafe {
                    ls::randombytes_buf(buf.as_mut_ptr().cast(), buf.len());
                }
                Ok(Self(buf))
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name)).field("bytes", &"[redacted; 32]").finish()
            }
        }
    };
}

define_key!(
    MasterKey,
    "Root symmetric key for a user. Wraps collection keys."
);
define_key!(
    CollectionKey,
    "Per-collection symmetric key. Wraps file keys of assets in the collection."
);
define_key!(FileKey, "Per-file symmetric key used with the secretstream blob AEAD.");
define_key!(SearchKey, "HMAC key used to blind-index FTS5 tokens (Phase 3).");
define_key!(
    KeyEncryptionKey,
    "Password-derived KEK. Wraps the master key in `user.wrapped_master_key`."
);

// --------- PEER IDENTITY ------------------------------------------------------

/// X25519 public key used for sealed-box sharing.
#[derive(Clone)]
pub struct PeerPublicKey(pub [u8; PEER_PK_LEN]);

impl std::fmt::Debug for PeerPublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerPublicKey").field("pk_hex", &hex::encode(self.0)).finish()
    }
}

/// X25519 keypair. Secret portion zeroized on drop.
#[derive(ZeroizeOnDrop)]
pub struct PeerKeypair {
    #[zeroize(skip)]
    pub public: PeerPublicKey,
    secret: [u8; PEER_SK_LEN],
}

impl PeerKeypair {
    pub fn generate() -> Result<Self> {
        sodium_init()?;
        let mut pk = [0u8; PEER_PK_LEN];
        let mut sk = [0u8; PEER_SK_LEN];
        // SAFETY: out-params sized per libsodium constants; return checked.
        let rc = unsafe { ls::crypto_box_keypair(pk.as_mut_ptr(), sk.as_mut_ptr()) };
        if rc != 0 {
            return Err(Error::Crypto);
        }
        Ok(Self { public: PeerPublicKey(pk), secret: sk })
    }

    /// Reconstruct from raw key material (e.g. after unwrapping).
    pub fn from_bytes(public: [u8; PEER_PK_LEN], secret: [u8; PEER_SK_LEN]) -> Self {
        Self { public: PeerPublicKey(public), secret }
    }

    pub fn secret_bytes(&self) -> &[u8; PEER_SK_LEN] {
        &self.secret
    }

    /// Create an Iroh-shaped keypair from a 32-byte seed. The public key is
    /// derived via libsodium's Ed25519 `crypto_sign_seed_keypair` so the
    /// node identity matches what Phase 3 will wire to Iroh's `NodeId`.
    pub fn from_iroh_seed(seed: [u8; 32]) -> Result<Self> {
        sodium_init()?;
        let mut pk = [0u8; PEER_PK_LEN];
        let mut sk_full = [0u8; ls::crypto_sign_SECRETKEYBYTES as usize];
        // SAFETY: sizes match libsodium constants; return value is checked.
        let rc = unsafe {
            ls::crypto_sign_seed_keypair(pk.as_mut_ptr(), sk_full.as_mut_ptr(), seed.as_ptr())
        };
        if rc != 0 {
            return Err(Error::Crypto);
        }
        // For persistence we keep only the 32-byte seed as the "secret".
        let _ = sk_full; // full SK is redundant with the seed.
        Ok(Self { public: PeerPublicKey(pk), secret: seed })
    }

    /// Reconstruct an Iroh-shaped keypair from an already-wrapped seed and its
    /// stored public key. Used during `unlock`, which has both pieces.
    pub fn from_iroh_seed_and_public(seed: [u8; 32], public: [u8; PEER_PK_LEN]) -> Result<Self> {
        // Verify the stored public matches the seed to catch corruption.
        let kp = Self::from_iroh_seed(seed)?;
        if kp.public.0 != public {
            return Err(Error::KeyOrData);
        }
        Ok(kp)
    }
}

impl std::fmt::Debug for PeerKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerKeypair")
            .field("public", &self.public)
            .field("secret", &"[redacted; 32]")
            .finish()
    }
}

// --------- ARGON2id KDF -------------------------------------------------------

/// Derive a 32-byte key-encryption key from a password + salt.
///
/// Uses `sensitive` limits (`OPSLIMIT=4`, `MEMLIMIT=1 GiB`) per §2.2.
pub fn derive_kek_from_password(pw: &SecretString, salt: &[u8; SALT_LEN]) -> Result<KeyEncryptionKey> {
    sodium_init()?;
    let mut out = [0u8; 32];
    let pw_bytes = pw.expose_secret().as_bytes();
    if pw_bytes.is_empty() {
        return Err(Error::Crypto);
    }
    // SAFETY: out is 32 bytes; pw pointer + len valid; salt fixed-size 16.
    let rc = unsafe {
        ls::crypto_pwhash(
            out.as_mut_ptr(),
            out.len() as u64,
            pw_bytes.as_ptr().cast(),
            pw_bytes.len() as u64,
            salt.as_ptr(),
            ARGON2_OPSLIMIT,
            ARGON2_MEMLIMIT,
            ARGON2_ALG,
        )
    };
    if rc != 0 {
        return Err(Error::Crypto);
    }
    Ok(KeyEncryptionKey::from_bytes(out))
}

// --------- KEY WRAPPING (secretbox XSalsa20-Poly1305) -------------------------
//
// Key-wrapping layer is separate from row AEAD: it uses a random 24-byte
// XSalsa20-Poly1305 nonce and a 32-byte key. Serialised as `nonce || ct`.

fn secretbox_seal(plain: &[u8], key: &[u8; SECRETBOX_KEY_LEN]) -> Result<Vec<u8>> {
    sodium_init()?;
    let mut nonce = [0u8; SECRETBOX_NONCE_LEN];
    // SAFETY: writes exactly nonce.len() bytes.
    unsafe { ls::randombytes_buf(nonce.as_mut_ptr().cast(), nonce.len()) };

    let mut out = vec![0u8; SECRETBOX_NONCE_LEN + plain.len() + SECRETBOX_TAG_LEN];
    out[..SECRETBOX_NONCE_LEN].copy_from_slice(&nonce);
    // SAFETY: out ciphertext region is plain.len()+tag bytes.
    let rc = unsafe {
        ls::crypto_secretbox_easy(
            out.as_mut_ptr().add(SECRETBOX_NONCE_LEN),
            plain.as_ptr(),
            plain.len() as u64,
            nonce.as_ptr(),
            key.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(Error::Crypto);
    }
    Ok(out)
}

fn secretbox_open(wrapped: &[u8], key: &[u8; SECRETBOX_KEY_LEN]) -> Result<Vec<u8>> {
    sodium_init()?;
    if wrapped.len() < SECRETBOX_NONCE_LEN + SECRETBOX_TAG_LEN {
        return Err(Error::KeyOrData);
    }
    let (nonce, ct) = wrapped.split_at(SECRETBOX_NONCE_LEN);
    let mut out = vec![0u8; ct.len() - SECRETBOX_TAG_LEN];
    // SAFETY: sizes derived from input slices; return value checked.
    let rc = unsafe {
        ls::crypto_secretbox_open_easy(
            out.as_mut_ptr(),
            ct.as_ptr(),
            ct.len() as u64,
            nonce.as_ptr(),
            key.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(Error::KeyOrData);
    }
    Ok(out)
}

pub fn wrap_master_key(mk: &MasterKey, kek: &KeyEncryptionKey) -> Result<Vec<u8>> {
    secretbox_seal(mk.as_bytes(), kek.as_bytes())
}

pub fn unwrap_master_key(wrapped: &[u8], kek: &KeyEncryptionKey) -> Result<MasterKey> {
    let bytes = secretbox_open(wrapped, kek.as_bytes())?;
    let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| Error::KeyOrData)?;
    Ok(MasterKey::from_bytes(arr))
}

pub fn wrap_collection_key(ck: &CollectionKey, mk: &MasterKey) -> Result<Vec<u8>> {
    secretbox_seal(ck.as_bytes(), mk.as_bytes())
}

pub fn unwrap_collection_key(wrapped: &[u8], mk: &MasterKey) -> Result<CollectionKey> {
    let bytes = secretbox_open(wrapped, mk.as_bytes())?;
    let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| Error::KeyOrData)?;
    Ok(CollectionKey::from_bytes(arr))
}

pub fn wrap_file_key(fk: &FileKey, ck: &CollectionKey) -> Result<Vec<u8>> {
    secretbox_seal(fk.as_bytes(), ck.as_bytes())
}

pub fn unwrap_file_key(wrapped: &[u8], ck: &CollectionKey) -> Result<FileKey> {
    let bytes = secretbox_open(wrapped, ck.as_bytes())?;
    let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| Error::KeyOrData)?;
    Ok(FileKey::from_bytes(arr))
}

/// Wrap raw bytes (e.g. a peer secret key) under the master key. Useful for
/// the Iroh and X25519 identity secrets stored alongside the user row.
pub fn wrap_bytes(plain: &[u8], mk: &MasterKey) -> Result<Vec<u8>> {
    secretbox_seal(plain, mk.as_bytes())
}

pub fn unwrap_bytes(wrapped: &[u8], mk: &MasterKey) -> Result<SecretBox<Vec<u8>>> {
    let out = secretbox_open(wrapped, mk.as_bytes())?;
    Ok(SecretBox::new(Box::new(out)))
}

/// Wrap bytes under an arbitrary 32-byte key. Used by the album-password
/// path where the key isn't a `MasterKey`.
pub fn wrap_with_key(plain: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    secretbox_seal(plain, key)
}

/// Open bytes wrapped under an arbitrary 32-byte key.
pub fn unwrap_with_key(wrapped: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    secretbox_open(wrapped, key)
}

// --------- ROW AEAD (XChaCha20-Poly1305-IETF) ---------------------------------

/// Seal a plaintext for row-level storage. Serialised as `nonce || ct`.
pub fn seal_row(plaintext: &[u8], row_id: u64, key: &[u8; 32]) -> Result<Vec<u8>> {
    sodium_init()?;
    let mut nonce = [0u8; ROW_NONCE_LEN];
    // SAFETY: writes exactly nonce.len() bytes.
    unsafe { ls::randombytes_buf(nonce.as_mut_ptr().cast(), nonce.len()) };
    let ad = row_id.to_le_bytes();

    let mut out = vec![0u8; ROW_NONCE_LEN + plaintext.len() + ROW_TAG_LEN];
    out[..ROW_NONCE_LEN].copy_from_slice(&nonce);

    let mut clen: std::os::raw::c_ulonglong = 0;
    // SAFETY: sizes derived from allocations; return checked.
    let rc = unsafe {
        ls::crypto_aead_xchacha20poly1305_ietf_encrypt(
            out.as_mut_ptr().add(ROW_NONCE_LEN),
            &mut clen,
            plaintext.as_ptr(),
            plaintext.len() as u64,
            ad.as_ptr(),
            ad.len() as u64,
            std::ptr::null(),
            nonce.as_ptr(),
            key.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(Error::Crypto);
    }
    debug_assert_eq!(clen as usize, plaintext.len() + ROW_TAG_LEN);
    Ok(out)
}

pub fn open_row(ciphertext: &[u8], row_id: u64, key: &[u8; 32]) -> Result<Vec<u8>> {
    sodium_init()?;
    if ciphertext.len() < ROW_NONCE_LEN + ROW_TAG_LEN {
        return Err(Error::KeyOrData);
    }
    let (nonce, ct) = ciphertext.split_at(ROW_NONCE_LEN);
    let ad = row_id.to_le_bytes();
    let mut out = vec![0u8; ct.len() - ROW_TAG_LEN];
    let mut mlen: std::os::raw::c_ulonglong = 0;
    // SAFETY: out sized to ct.len() - tag.
    let rc = unsafe {
        ls::crypto_aead_xchacha20poly1305_ietf_decrypt(
            out.as_mut_ptr(),
            &mut mlen,
            std::ptr::null_mut(),
            ct.as_ptr(),
            ct.len() as u64,
            ad.as_ptr(),
            ad.len() as u64,
            nonce.as_ptr(),
            key.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(Error::KeyOrData);
    }
    out.truncate(mlen as usize);
    Ok(out)
}

// --------- BLOB AEAD (secretstream XChaCha20-Poly1305) ------------------------

/// Streaming writer: `magic || header || chunks...`. Drop without calling
/// [`BlobWriter::finish`] leaves the output truncated.
pub struct BlobWriter<W: Write> {
    state: Box<[u8; SECRETSTREAM_STATE_LEN]>,
    inner: Option<W>,
    buffer: Vec<u8>,
    finished: bool,
}

impl<W: Write> BlobWriter<W> {
    fn new(key: &FileKey, mut inner: W) -> Result<Self> {
        sodium_init()?;
        let mut state = Box::new([0u8; SECRETSTREAM_STATE_LEN]);
        let mut header = [0u8; SECRETSTREAM_HEADER_LEN];
        // SAFETY: state and header sized per libsodium constants.
        let rc = unsafe {
            ls::crypto_secretstream_xchacha20poly1305_init_push(
                state.as_mut_ptr().cast(),
                header.as_mut_ptr(),
                key.as_bytes().as_ptr(),
            )
        };
        if rc != 0 {
            return Err(Error::Crypto);
        }
        inner.write_all(&serialise_blob_prelude(&header))?;
        Ok(Self {
            state,
            inner: Some(inner),
            buffer: Vec::with_capacity(PLAINTEXT_CHUNK_SIZE),
            finished: false,
        })
    }

    fn emit_chunk(&mut self, tag: u8) -> Result<()> {
        let Some(inner) = self.inner.as_mut() else {
            return Err(Error::Crypto);
        };
        let mut ct = vec![0u8; self.buffer.len() + SECRETSTREAM_ABYTES];
        let mut clen: std::os::raw::c_ulonglong = 0;
        // SAFETY: buffer and ct sized correctly; state pointer non-null.
        let rc = unsafe {
            ls::crypto_secretstream_xchacha20poly1305_push(
                self.state.as_mut_ptr().cast(),
                ct.as_mut_ptr(),
                &mut clen,
                self.buffer.as_ptr(),
                self.buffer.len() as u64,
                std::ptr::null(),
                0,
                tag,
            )
        };
        if rc != 0 {
            return Err(Error::Crypto);
        }
        ct.truncate(clen as usize);
        inner.write_all(&ct)?;
        self.buffer.clear();
        Ok(())
    }

    /// Append `data` to the blob, emitting full 4 MiB chunks as they fill up.
    pub fn write_all(&mut self, data: &[u8]) -> Result<()> {
        let mut remaining = data;
        while !remaining.is_empty() {
            let free = PLAINTEXT_CHUNK_SIZE - self.buffer.len();
            let take = free.min(remaining.len());
            self.buffer.extend_from_slice(&remaining[..take]);
            remaining = &remaining[take..];
            if self.buffer.len() == PLAINTEXT_CHUNK_SIZE {
                self.emit_chunk(ls::crypto_secretstream_xchacha20poly1305_TAG_MESSAGE as u8)?;
            }
        }
        Ok(())
    }

    /// Flush the final chunk with `TAG_FINAL` and return the wrapped writer.
    pub fn finish(mut self) -> Result<W> {
        self.emit_chunk(ls::crypto_secretstream_xchacha20poly1305_TAG_FINAL as u8)?;
        self.finished = true;
        self.inner.take().ok_or(Error::Crypto)
    }
}

impl<W: Write> Drop for BlobWriter<W> {
    fn drop(&mut self) {
        // State bytes are zeroed on drop — belt-and-braces beyond libsodium's
        // own wipe, since we don't call `*_rekey` or similar.
        self.state.zeroize();
    }
}

/// Streaming reader. Repeated calls to [`BlobReader::read_chunk`] yield
/// plaintext chunks until the reader reports `None`, which means the
/// `TAG_FINAL` chunk has been consumed.
pub struct BlobReader<R: Read> {
    state: Box<[u8; SECRETSTREAM_STATE_LEN]>,
    inner: R,
    finished: bool,
}

impl<R: Read> BlobReader<R> {
    fn new(key: &FileKey, mut inner: R) -> Result<Self> {
        sodium_init()?;
        let mut prelude = [0u8; MAGIC_LEN + SECRETSTREAM_HEADER_LEN];
        inner.read_exact(&mut prelude)?;
        let header = parse_blob_prelude(&prelude)?;

        let mut state = Box::new([0u8; SECRETSTREAM_STATE_LEN]);
        // SAFETY: state and header sized per constants; return checked.
        let rc = unsafe {
            ls::crypto_secretstream_xchacha20poly1305_init_pull(
                state.as_mut_ptr().cast(),
                header.as_ptr(),
                key.as_bytes().as_ptr(),
            )
        };
        if rc != 0 {
            return Err(Error::KeyOrData);
        }
        Ok(Self { state, inner, finished: false })
    }

    /// Read one encrypted frame + authenticate. Returns `None` at end-of-stream.
    ///
    /// Phase-1 callers use `read_to_end` only for derivatives/small plaintexts;
    /// large media goes through this low-level chunk loop.
    pub fn read_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        if self.finished {
            return Ok(None);
        }
        // Read a full encrypted frame: length-prefix-free, so we use
        // chunk_size + ABYTES as the upper bound and let the stream fail
        // if the bytes disagree. On-disk format always writes full 4 MiB + ABYTES
        // frames except the last, so we read up to that length and rely on
        // EOF detection for the shorter tail.
        let mut buf = vec![0u8; PLAINTEXT_CHUNK_SIZE + SECRETSTREAM_ABYTES];
        let mut filled = 0usize;
        loop {
            match self.inner.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(Error::Io(e)),
            }
            if filled == buf.len() {
                break;
            }
        }
        if filled == 0 {
            // Unexpected EOF before TAG_FINAL — treat as corruption/truncation.
            return Err(Error::KeyOrData);
        }
        buf.truncate(filled);

        let mut plain = vec![0u8; filled.saturating_sub(SECRETSTREAM_ABYTES)];
        let mut mlen: std::os::raw::c_ulonglong = 0;
        let mut tag: u8 = 0;
        // SAFETY: sizes derived from buf; state pointer non-null.
        let rc = unsafe {
            ls::crypto_secretstream_xchacha20poly1305_pull(
                self.state.as_mut_ptr().cast(),
                plain.as_mut_ptr(),
                &mut mlen,
                &mut tag,
                buf.as_ptr(),
                buf.len() as u64,
                std::ptr::null(),
                0,
            )
        };
        if rc != 0 {
            return Err(Error::KeyOrData);
        }
        plain.truncate(mlen as usize);
        if u32::from(tag) == ls::crypto_secretstream_xchacha20poly1305_TAG_FINAL {
            self.finished = true;
        }
        Ok(Some(plain))
    }

    /// Convenience: drain the entire stream into a single Vec. For tests and
    /// small derivatives only — media-sized blobs must iterate chunks.
    pub fn read_to_end(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        while let Some(chunk) = self.read_chunk()? {
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }
}

impl<R: Read> Drop for BlobReader<R> {
    fn drop(&mut self) {
        self.state.zeroize();
    }
}

pub fn seal_blob_writer<W: Write>(fk: &FileKey, w: W) -> Result<BlobWriter<W>> {
    BlobWriter::new(fk, w)
}

pub fn open_blob_reader<R: Read>(fk: &FileKey, r: R) -> Result<BlobReader<R>> {
    BlobReader::new(fk, r)
}

// --------- PEER SEALED BOX ----------------------------------------------------

pub fn seal_for_peer(plaintext: &[u8], recipient_pk: &PeerPublicKey) -> Result<Vec<u8>> {
    sodium_init()?;
    let mut out = vec![0u8; plaintext.len() + PEER_SEAL_OVERHEAD];
    // SAFETY: out sized to plaintext + seal overhead.
    let rc = unsafe {
        ls::crypto_box_seal(
            out.as_mut_ptr(),
            plaintext.as_ptr(),
            plaintext.len() as u64,
            recipient_pk.0.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(Error::Crypto);
    }
    Ok(out)
}

pub fn open_from_peer(ct: &[u8], kp: &PeerKeypair) -> Result<Vec<u8>> {
    sodium_init()?;
    if ct.len() < PEER_SEAL_OVERHEAD {
        return Err(Error::KeyOrData);
    }
    let mut out = vec![0u8; ct.len() - PEER_SEAL_OVERHEAD];
    // SAFETY: sizes derived from ct; return checked.
    let rc = unsafe {
        ls::crypto_box_seal_open(
            out.as_mut_ptr(),
            ct.as_ptr(),
            ct.len() as u64,
            kp.public.0.as_ptr(),
            kp.secret.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(Error::KeyOrData);
    }
    Ok(out)
}

// --------- RANDOM HELPERS -----------------------------------------------------

/// Generate `len` bytes of cryptographic randomness. Useful for salts.
pub fn random_bytes(buf: &mut [u8]) -> Result<()> {
    sodium_init()?;
    // SAFETY: writes exactly buf.len() bytes.
    unsafe { ls::randombytes_buf(buf.as_mut_ptr().cast(), buf.len()) };
    Ok(())
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn k32(seed: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = seed.wrapping_add(i as u8);
        }
        k
    }

    #[test]
    fn row_seal_open_identity() {
        let key = k32(1);
        for size in [0usize, 1, 16, 1024, 1 << 20] {
            let plaintext: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let ct = seal_row(&plaintext, 42, &key).unwrap();
            let pt = open_row(&ct, 42, &key).unwrap();
            assert_eq!(pt, plaintext, "size={size}");
        }
    }

    #[test]
    fn row_aead_rejects_wrong_row_id() {
        let key = k32(2);
        let ct = seal_row(b"secret", 1, &key).unwrap();
        assert!(matches!(open_row(&ct, 2, &key), Err(Error::KeyOrData)));
    }

    #[test]
    fn row_aead_rejects_wrong_key() {
        let ct = seal_row(b"secret", 1, &k32(3)).unwrap();
        assert!(matches!(open_row(&ct, 1, &k32(4)), Err(Error::KeyOrData)));
    }

    #[test]
    fn row_aead_rejects_tamper() {
        let key = k32(5);
        let mut ct = seal_row(b"secret-value", 7, &key).unwrap();
        // Flip the last ciphertext byte (inside the MAC region).
        let last = ct.len() - 1;
        ct[last] ^= 0x01;
        assert!(matches!(open_row(&ct, 7, &key), Err(Error::KeyOrData)));
    }

    #[test]
    fn master_collection_file_key_wrap_round_trip() {
        let pw = SecretString::from("hunter2-hunter2-hunter2");
        let salt = [9u8; SALT_LEN];
        let kek = derive_kek_from_password(&pw, &salt).unwrap();

        let mk = MasterKey::random().unwrap();
        let wrapped_mk = wrap_master_key(&mk, &kek).unwrap();
        let mk2 = unwrap_master_key(&wrapped_mk, &kek).unwrap();
        assert_eq!(mk.as_bytes(), mk2.as_bytes());

        let ck = CollectionKey::random().unwrap();
        let wrapped_ck = wrap_collection_key(&ck, &mk).unwrap();
        let ck2 = unwrap_collection_key(&wrapped_ck, &mk).unwrap();
        assert_eq!(ck.as_bytes(), ck2.as_bytes());

        let fk = FileKey::random().unwrap();
        let wrapped_fk = wrap_file_key(&fk, &ck).unwrap();
        let fk2 = unwrap_file_key(&wrapped_fk, &ck).unwrap();
        assert_eq!(fk.as_bytes(), fk2.as_bytes());
    }

    #[test]
    fn wrong_password_fails_to_unwrap_master_key() {
        let salt = [1u8; SALT_LEN];
        let kek_right = derive_kek_from_password(&SecretString::from("right-password-long-xxx"), &salt).unwrap();
        let kek_wrong = derive_kek_from_password(&SecretString::from("wrong-password-long-xxx"), &salt).unwrap();
        let mk = MasterKey::random().unwrap();
        let wrapped = wrap_master_key(&mk, &kek_right).unwrap();
        assert!(matches!(unwrap_master_key(&wrapped, &kek_wrong), Err(Error::KeyOrData)));
    }

    #[test]
    fn blob_round_trip_small_sizes() {
        let fk = FileKey::random().unwrap();
        for size in [0usize, 1, 100, 4096, PLAINTEXT_CHUNK_SIZE, PLAINTEXT_CHUNK_SIZE + 1] {
            let plaintext: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let mut out = Vec::new();
            let mut w = seal_blob_writer(&fk, &mut out).unwrap();
            w.write_all(&plaintext).unwrap();
            w.finish().unwrap();

            let mut r = open_blob_reader(&fk, std::io::Cursor::new(&out)).unwrap();
            let round = r.read_to_end().unwrap();
            assert_eq!(round, plaintext, "blob round-trip failed at size={size}");
        }
    }

    #[test]
    fn blob_rejects_wrong_key() {
        let fk = FileKey::random().unwrap();
        let other = FileKey::random().unwrap();
        let mut out = Vec::new();
        let mut w = seal_blob_writer(&fk, &mut out).unwrap();
        w.write_all(b"hello world").unwrap();
        w.finish().unwrap();
        let mut r = open_blob_reader(&other, std::io::Cursor::new(&out)).unwrap();
        assert!(matches!(r.read_chunk(), Err(Error::KeyOrData)));
    }

    #[test]
    fn blob_rejects_bad_magic() {
        let fk = FileKey::random().unwrap();
        let mut out = Vec::new();
        let mut w = seal_blob_writer(&fk, &mut out).unwrap();
        w.write_all(b"hello").unwrap();
        w.finish().unwrap();
        // Corrupt the magic bytes.
        out[0] ^= 0xff;
        let err = open_blob_reader(&fk, std::io::Cursor::new(&out)).err().unwrap();
        assert!(matches!(err, Error::BlobFormat));
    }

    #[test]
    fn blob_rejects_tamper_in_body() {
        let fk = FileKey::random().unwrap();
        let mut out = Vec::new();
        let mut w = seal_blob_writer(&fk, &mut out).unwrap();
        w.write_all(b"important message body").unwrap();
        w.finish().unwrap();
        // Flip a byte inside the ciphertext body (past the magic+header prelude).
        let idx = MAGIC_LEN + SECRETSTREAM_HEADER_LEN + 2;
        out[idx] ^= 0x01;
        let mut r = open_blob_reader(&fk, std::io::Cursor::new(&out)).unwrap();
        assert!(matches!(r.read_chunk(), Err(Error::KeyOrData)));
    }

    #[test]
    fn peer_sealed_box_round_trip() {
        let kp = PeerKeypair::generate().unwrap();
        let ct = seal_for_peer(b"hello from peer", &kp.public).unwrap();
        let pt = open_from_peer(&ct, &kp).unwrap();
        assert_eq!(pt, b"hello from peer");
    }

    #[test]
    fn peer_sealed_box_wrong_recipient_fails() {
        let kp = PeerKeypair::generate().unwrap();
        let other = PeerKeypair::generate().unwrap();
        let ct = seal_for_peer(b"for me only", &kp.public).unwrap();
        assert!(matches!(open_from_peer(&ct, &other), Err(Error::KeyOrData)));
    }

    #[test]
    fn debug_impls_redact() {
        let mk = MasterKey::random().unwrap();
        let s = format!("{mk:?}");
        assert!(s.contains("[redacted"), "Debug leaked bytes: {s}");
        let kp = PeerKeypair::generate().unwrap();
        let s = format!("{kp:?}");
        assert!(s.contains("[redacted"), "PeerKeypair Debug leaked: {s}");
    }
}
