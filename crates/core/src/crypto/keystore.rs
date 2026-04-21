//! Keystore — user creation, unlock, hidden vault, album passwords.
//!
//! This module assembles the key material required at runtime; it does NOT
//! implement persistence (that lives in `db/queries.rs`). The shape of each
//! record matches the `user` / `collection` / `collection_key` DDL in
//! `plans/architecture.md` §4.2.
//!
//! Plausible-deniability contract (§9): a vault with a hidden sub-vault must
//! be byte-indistinguishable from a vault without one, except when the hidden
//! password is entered. We enforce this by:
//!
//! * storing the wrapped hidden master key inside a blob whose mere presence
//!   reveals nothing (any vault may carry it, populated with random bytes
//!   at user creation);
//! * never letting the `has_hidden_vault` flag be set persistently — it lives
//!   only in memory after a successful unlock and resets when the vault is
//!   locked.

use secrecy::SecretString;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::envelope::{
    derive_kek_from_password, random_bytes, unwrap_bytes, unwrap_master_key, unwrap_with_key,
    wrap_bytes, wrap_master_key, wrap_with_key, CollectionKey, MasterKey, PeerKeypair, SALT_LEN,
};
use crate::{Error, Result};

// --------- ON-DISK RECORDS ----------------------------------------------------

/// The material written into the `user` row at signup.
///
/// Column mapping (see §4.2):
///
/// * `username_ct`         = `username_ct`
/// * `argon2_salt`         = `main_salt`
/// * `wrapped_master_key`  = `wrapped_master_key`
/// * `identity_pub`        = `identity_pub`
/// * `wrapped_identity_sk` = `wrapped_identity_sk`
/// * `iroh_node_pub`       = `iroh_node_pub`
/// * `wrapped_iroh_sk`     = `wrapped_iroh_sk`
/// * `recovery_blob`       = `hidden_wrapped_master_key` (piggy-backs per §9
///   plausible-deniability: when the hidden vault is unused this is random
///   bytes of the same length, so vaults with and without a hidden sub-vault
///   are byte-indistinguishable).
#[derive(Debug)]
pub struct UserRecord {
    pub username_ct: Vec<u8>,
    pub main_salt: [u8; SALT_LEN],
    pub wrapped_master_key: Vec<u8>,
    pub identity_pub: [u8; 32],
    pub wrapped_identity_sk: Vec<u8>,
    pub iroh_node_pub: [u8; 32],
    pub wrapped_iroh_sk: Vec<u8>,
    pub hidden_wrapped_master_key: Vec<u8>,
    pub hidden_salt: [u8; SALT_LEN],
}

/// A vault that has been unlocked; holds secrets in RAM only.
///
/// Dropping this struct zeroes all key material.
#[derive(ZeroizeOnDrop)]
pub struct UnlockedUser {
    #[zeroize(skip)]
    pub user_id: i64,
    pub master_key: MasterKey,
    #[zeroize(skip)]
    pub identity: PeerKeypair,
    #[zeroize(skip)]
    pub iroh_node: PeerKeypair, // Ed25519 uses 32-byte secret with this shape
    /// Hidden-vault master key, populated only after a successful second
    /// unlock. Absence means "hidden not unlocked" — never exposed to callers
    /// that don't hold this struct.
    pub hidden_master_key: Option<MasterKey>,
}

// --------- USER CREATION ------------------------------------------------------

/// Generate all material needed to create a new user.
///
/// The returned `UserRecord` is what the caller writes into the `user` table.
/// The returned `UnlockedUser` is what the app keeps in memory for the newly
/// created session. `username` is plaintext input only — the record stores
/// the ciphertext.
pub fn create_user(username: &str, password: &SecretString) -> Result<(UserRecord, UnlockedUser)> {
    super::init()?;

    // 1. Main password → KEK → master key.
    let mut main_salt = [0u8; SALT_LEN];
    random_bytes(&mut main_salt)?;
    let kek = derive_kek_from_password(password, &main_salt)?;
    let mk = MasterKey::random()?;
    let wrapped_master_key = wrap_master_key(&mk, &kek)?;

    // 2. Hidden sub-vault placeholder.
    //    At signup there is no hidden password; we store random bytes of the
    //    same length as a real wrapped master key so presence leaks nothing.
    //    The hidden salt is always present; the record format is fixed.
    let mut hidden_salt = [0u8; SALT_LEN];
    random_bytes(&mut hidden_salt)?;
    let mut hidden_wrapped_master_key = vec![0u8; wrapped_master_key.len()];
    random_bytes(&mut hidden_wrapped_master_key)?;

    // 3. X25519 identity keypair (sharing) — secret wrapped by master key.
    let identity = PeerKeypair::generate()?;
    let wrapped_identity_sk = wrap_bytes(identity.secret_bytes(), &mk)?;

    // 4. Iroh Ed25519 node keypair — the underlying libsodium API only offers
    //    `crypto_sign_keypair`, which is Ed25519. For Phase 1 we synthesise a
    //    32-byte seed via randomness and wrap it; Phase 3 will derive the
    //    actual Ed25519 key when iroh ties in. This seed is the SK we wrap.
    let mut iroh_seed = [0u8; 32];
    random_bytes(&mut iroh_seed)?;
    let iroh_node = PeerKeypair::from_iroh_seed(iroh_seed)?;
    let wrapped_iroh_sk = wrap_bytes(iroh_node.secret_bytes(), &mk)?;

    // 5. Encrypt the username using the master key as its own sub-key. This
    //    is row-AEAD shape (nonce || ct) so other row helpers can open it.
    let username_ct = super::envelope::seal_row(username.as_bytes(), 0, mk.as_bytes())?;

    let record = UserRecord {
        username_ct,
        main_salt,
        wrapped_master_key,
        identity_pub: identity.public.0,
        wrapped_identity_sk,
        iroh_node_pub: iroh_node.public.0,
        wrapped_iroh_sk,
        hidden_wrapped_master_key,
        hidden_salt,
    };

    let unlocked = UnlockedUser {
        user_id: 0, // assigned by the DB on insert
        master_key: mk,
        identity,
        iroh_node,
        hidden_master_key: None,
    };

    Ok((record, unlocked))
}

// --------- UNLOCK -------------------------------------------------------------

/// Attempt to unlock a vault.
///
/// The error path returns [`Error::KeyOrData`] regardless of the underlying
/// reason — wrong password, tampered wrapped key, or truncated record —
/// so callers cannot distinguish one from another.
pub fn unlock(record: &UserRecord, password: &SecretString, user_id: i64) -> Result<UnlockedUser> {
    super::init()?;
    let kek = derive_kek_from_password(password, &record.main_salt)?;
    let mk = unwrap_master_key(&record.wrapped_master_key, &kek)?;

    let identity_sk_box = unwrap_bytes(&record.wrapped_identity_sk, &mk)?;
    let identity_sk_arr: [u8; 32] = secrecy::ExposeSecret::expose_secret(&identity_sk_box)
        .as_slice()
        .try_into()
        .map_err(|_| Error::KeyOrData)?;
    let identity = PeerKeypair::from_bytes(record.identity_pub, identity_sk_arr);

    let iroh_sk_box = unwrap_bytes(&record.wrapped_iroh_sk, &mk)?;
    let iroh_sk_arr: [u8; 32] = secrecy::ExposeSecret::expose_secret(&iroh_sk_box)
        .as_slice()
        .try_into()
        .map_err(|_| Error::KeyOrData)?;
    let iroh_node = PeerKeypair::from_iroh_seed_and_public(iroh_sk_arr, record.iroh_node_pub)?;

    Ok(UnlockedUser {
        user_id,
        master_key: mk,
        identity,
        iroh_node,
        hidden_master_key: None,
    })
}

// --------- HIDDEN VAULT -------------------------------------------------------

/// Set (or rotate) the hidden-vault password on an unlocked user.
///
/// Creates a *second* master key, wraps it under Argon2id(hidden_salt, pw),
/// and replaces the placeholder blob in `record.hidden_wrapped_master_key`
/// in-place. The caller is expected to persist the updated record.
///
/// The returned value is the new hidden master key, already loaded into
/// `UnlockedUser.hidden_master_key` for the current session.
pub fn set_hidden_vault_password(
    record: &mut UserRecord,
    user: &mut UnlockedUser,
    password: &SecretString,
) -> Result<()> {
    super::init()?;
    // Re-roll the hidden salt on each rotation.
    random_bytes(&mut record.hidden_salt)?;
    let kek = derive_kek_from_password(password, &record.hidden_salt)?;
    let hidden_mk = MasterKey::random()?;
    record.hidden_wrapped_master_key = wrap_master_key(&hidden_mk, &kek)?;
    user.hidden_master_key = Some(hidden_mk);
    Ok(())
}

/// Attempt to unlock the hidden sub-vault from an already-unlocked user.
///
/// Returns `Ok(())` on success (and populates `user.hidden_master_key`), or
/// [`Error::KeyOrData`] on any failure. The error is intentionally identical
/// to the "no hidden vault exists" case, so callers cannot distinguish the
/// two — an observer who tries random passwords cannot learn whether a
/// hidden sub-vault is present.
pub fn unlock_hidden_vault(
    record: &UserRecord,
    user: &mut UnlockedUser,
    password: &SecretString,
) -> Result<()> {
    super::init()?;
    let kek = derive_kek_from_password(password, &record.hidden_salt)?;
    let hidden_mk = unwrap_master_key(&record.hidden_wrapped_master_key, &kek)?;
    user.hidden_master_key = Some(hidden_mk);
    Ok(())
}

// --------- ALBUM PASSWORDS ----------------------------------------------------

/// An Argon2id-derived key that a collection has been additionally wrapped under.
/// Held only for the duration of an unlocked-album session.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct AlbumKey([u8; 32]);

impl AlbumKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Derive the album key from a password + salt (caller persists the salt in
/// `collection.password_salt`).
pub fn derive_album_key(password: &SecretString, salt: &[u8; SALT_LEN]) -> Result<AlbumKey> {
    let kek = derive_kek_from_password(password, salt)?;
    Ok(AlbumKey(*kek.as_bytes()))
}

/// Wrap an unlocked collection key under an album password.
///
/// Call this after the user sets a password on an album. Produces the
/// `collection_key` row with `wrapping='album_password'` (plus the existing
/// `wrapping='master'` row stays in place so the owning user can always
/// decrypt via the main path).
pub fn wrap_collection_key_for_album(
    ck: &CollectionKey,
    password: &SecretString,
) -> Result<(Vec<u8>, [u8; SALT_LEN])> {
    super::init()?;
    let mut salt = [0u8; SALT_LEN];
    random_bytes(&mut salt)?;
    let key = derive_album_key(password, &salt)?;
    let wrapped = wrap_with_key(ck.as_bytes(), key.as_bytes())?;
    Ok((wrapped, salt))
}

/// Unlock the collection key given the album password + stored salt.
pub fn unlock_album(
    wrapped: &[u8],
    password: &SecretString,
    salt: &[u8; SALT_LEN],
) -> Result<CollectionKey> {
    super::init()?;
    let key = derive_album_key(password, salt)?;
    let bytes = unwrap_with_key(wrapped, key.as_bytes())?;
    let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| Error::KeyOrData)?;
    Ok(CollectionKey::from_bytes(arr))
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn pw(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    #[test]
    fn create_and_unlock_round_trip() {
        let (record, created) = create_user("alice", &pw("hunter2-hunter2-hunter2")).unwrap();
        let unlocked = unlock(&record, &pw("hunter2-hunter2-hunter2"), 7).unwrap();
        assert_eq!(created.master_key.as_bytes(), unlocked.master_key.as_bytes());
        assert_eq!(created.identity.public.0, unlocked.identity.public.0);
        assert_eq!(created.iroh_node.public.0, unlocked.iroh_node.public.0);
        assert_eq!(unlocked.user_id, 7);
        // Username decrypts to the input.
        let pt = super::super::envelope::open_row(&record.username_ct, 0, unlocked.master_key.as_bytes()).unwrap();
        assert_eq!(pt, b"alice");
    }

    #[test]
    fn wrong_password_is_opaque() {
        let (record, _) = create_user("alice", &pw("correct-horse-battery-staple")).unwrap();
        let err = unlock(&record, &pw("wrong-horse-battery-staple"), 1).err().unwrap();
        assert!(matches!(err, Error::KeyOrData));
    }

    #[test]
    fn hidden_vault_plausible_deniability_shape() {
        // The record carries a hidden placeholder that is bytewise
        // indistinguishable from a real wrapped key. Two fresh vaults must
        // have hidden_wrapped_master_key of the same length.
        let (a, _) = create_user("a", &pw("password-a-password-a")).unwrap();
        let (b, _) = create_user("b", &pw("password-b-password-b")).unwrap();
        assert_eq!(a.hidden_wrapped_master_key.len(), b.hidden_wrapped_master_key.len());
        // And the same length as a real wrapped master key.
        assert_eq!(a.hidden_wrapped_master_key.len(), a.wrapped_master_key.len());
    }

    #[test]
    fn hidden_vault_set_and_unlock() {
        let (mut record, mut unlocked) =
            create_user("alice", &pw("main-main-main-main")).unwrap();
        // Initially no hidden vault.
        assert!(unlocked.hidden_master_key.is_none());
        // Setting it populates the in-memory field and the record.
        set_hidden_vault_password(&mut record, &mut unlocked, &pw("hidden-hidden-hidden")).unwrap();
        let hidden_before = unlocked.hidden_master_key.as_ref().unwrap().as_bytes().to_vec();

        // Simulate lock + re-unlock cycle: main unlock does NOT carry hidden.
        let mut re_unlocked = unlock(&record, &pw("main-main-main-main"), 1).unwrap();
        assert!(re_unlocked.hidden_master_key.is_none());

        // Correct hidden password recovers the same key bytes.
        unlock_hidden_vault(&record, &mut re_unlocked, &pw("hidden-hidden-hidden")).unwrap();
        assert_eq!(re_unlocked.hidden_master_key.as_ref().unwrap().as_bytes(), hidden_before.as_slice());
    }

    #[test]
    fn wrong_hidden_password_fails_opaquely() {
        let (mut record, mut unlocked) =
            create_user("alice", &pw("main-main-main-main")).unwrap();
        set_hidden_vault_password(&mut record, &mut unlocked, &pw("hidden-hidden-hidden")).unwrap();
        let mut re = unlock(&record, &pw("main-main-main-main"), 1).unwrap();
        let err = unlock_hidden_vault(&record, &mut re, &pw("not-the-hidden-pw")).err().unwrap();
        assert!(matches!(err, Error::KeyOrData));
        assert!(re.hidden_master_key.is_none());
    }

    #[test]
    fn unlock_hidden_on_vault_without_one_fails_with_same_error() {
        // Vault with NO hidden vault set. The placeholder is random bytes that
        // will never authenticate. Any password must fail with KeyOrData —
        // not a different error — so an adversary can't detect absence.
        let (record, mut unlocked) = create_user("a", &pw("aaaaaaaaaaaaaaaa")).unwrap();
        let err = unlock_hidden_vault(&record, &mut unlocked, &pw("try-this-pw-try-this")).err().unwrap();
        assert!(matches!(err, Error::KeyOrData));
    }

    #[test]
    fn album_password_round_trip() {
        let ck = CollectionKey::random().unwrap();
        let (wrapped, salt) = wrap_collection_key_for_album(&ck, &pw("beach-2024-pictures")).unwrap();
        let ck2 = unlock_album(&wrapped, &pw("beach-2024-pictures"), &salt).unwrap();
        assert_eq!(ck.as_bytes(), ck2.as_bytes());
    }

    #[test]
    fn album_password_wrong_fails() {
        let ck = CollectionKey::random().unwrap();
        let (wrapped, salt) = wrap_collection_key_for_album(&ck, &pw("right-password-xx")).unwrap();
        let err = unlock_album(&wrapped, &pw("wrong-password-xx"), &salt).err().unwrap();
        assert!(matches!(err, Error::KeyOrData));
    }
}
