//! iroh-blobs bridge — serves our on-disk CAS ciphertext to peers by
//! BLAKE3(ciphertext), and exposes helpers to import + write blobs into
//! our local CAS on the receive side.
//!
//! Why not ride the existing CAS directly? iroh-blobs maintains its own
//! redb-backed index of (hash → on-disk data + outboard/bao tree) for
//! verified streaming; its storage format isn't our `MVV1 || chunks`
//! layout. So we operate a second persistent store at
//! `<vault>/iroh/blobs/` and `add_path` our CAS ciphertext files into it
//! on announce. Read-only from the CAS side — we never mutate the
//! original bytes.
//!
//! **The blobs are pure ciphertext.** The server-side peer serves the
//! `MVV1`-envelope bytes exactly as they sit in CAS. Decryption requires
//! the collection key, which travels separately through iroh-docs
//! (C8/C9). A peer who fetches a blob but lacks the wrapping key sees
//! only opaque AEAD output.

use std::path::Path;
use std::sync::Arc;

use iroh_blobs::api::blobs::{AddPathOptions, ImportMode};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, BlobsProtocol, Hash};
use mv_core::cas::CasStore;
use mv_core::{Error, Result};
use rusqlite::Connection;
use tokio::sync::Mutex;

/// Bridge between our plaintext-addressed CAS and iroh-blobs'
/// ciphertext-addressed BAO store. Constructed via [`BlobsBridge::start`]
/// and its `protocol()` handler is registered on an
/// [`iroh::protocol::Router`] by [`crate::peer::Peer::mount_router`].
pub struct BlobsBridge {
    store: FsStore,
    protocol: BlobsProtocol,
    cas: Arc<CasStore>,
    db: Arc<Mutex<Connection>>,
}

impl BlobsBridge {
    /// Boot (or recover) the persistent iroh-blobs store at
    /// `<vault_root>/iroh/blobs/` and wrap it with a `BlobsProtocol`
    /// handler. Idempotent — opening an existing store re-indexes it.
    pub async fn start(
        vault_root: &Path,
        cas: Arc<CasStore>,
        db: Arc<Mutex<Connection>>,
    ) -> Result<Self> {
        let blobs_dir = vault_root.join("iroh").join("blobs");
        std::fs::create_dir_all(&blobs_dir)?;
        let store = FsStore::load(&blobs_dir).await.map_err(|e| {
            tracing::debug!(%e, ?blobs_dir, "iroh-blobs FsStore::load failed");
            Error::PeerUnreachable
        })?;
        let protocol = BlobsProtocol::new(&store, None);
        Ok(Self {
            store,
            protocol,
            cas,
            db,
        })
    }

    /// The protocol handler this bridge serves. Registered by the Router
    /// under `iroh_blobs::protocol::ALPN`.
    pub fn protocol(&self) -> BlobsProtocol {
        self.protocol.clone()
    }

    /// Reference to the persistent store; chiefly useful in tests and the
    /// C9 receive loop's downloader path.
    pub fn store(&self) -> &FsStore {
        &self.store
    }

    /// Reference to our CAS. Receivers need it to write fetched
    /// ciphertexts into the plaintext-addressed layout.
    pub fn cas(&self) -> &Arc<CasStore> {
        &self.cas
    }

    /// Reference to the shared DB mutex. The C9 receive loop calls
    /// `set_ciphertext_blake3` + asset inserts through this.
    pub fn db(&self) -> &Arc<Mutex<Connection>> {
        &self.db
    }

    /// Import the on-disk ciphertext backing `asset_id` into the
    /// iroh-blobs store. Computes the ciphertext BLAKE3 if
    /// `asset.ciphertext_blake3` is NULL and persists it back. Returns
    /// the iroh `Hash` that peers will use to fetch.
    ///
    /// Idempotent — iroh-blobs dedupes by hash, and our DB cache of the
    /// hash survives restarts.
    pub async fn announce_asset(&self, asset_id: i64) -> Result<Hash> {
        // 1. Look up cas_ref + (maybe) cached ciphertext hash.
        let (cas_ref, cached_hash) = {
            let conn = self.db.lock().await;
            conn.query_row(
                "SELECT cas_ref, ciphertext_blake3 FROM asset WHERE id = ?1",
                [asset_id],
                |row| {
                    let cas_ref: String = row.get(0)?;
                    let ct_hash: Option<Vec<u8>> = row.get(1)?;
                    Ok((cas_ref, ct_hash))
                },
            )?
        };

        // 2. Compute ciphertext hash if not cached; persist.
        let hash_bytes: [u8; 32] = if let Some(h) = cached_hash {
            if h.len() != 32 {
                return Err(Error::BlobFormat);
            }
            let mut a = [0u8; 32];
            a.copy_from_slice(&h);
            a
        } else {
            let arr = self.cas.compute_ciphertext_blake3(&cas_ref)?;
            let conn = self.db.lock().await;
            mv_core::db::queries::set_ciphertext_blake3(&conn, asset_id, &arr)?;
            arr
        };
        let hash = Hash::from_bytes(hash_bytes);

        // 3. Short-circuit if already imported.
        if self.store.blobs().has(hash).await.map_err(|e| {
            tracing::debug!(%e, "iroh-blobs has() rpc failed");
            Error::PeerUnreachable
        })? {
            return Ok(hash);
        }

        // 4. Import the on-disk ciphertext file. iroh-blobs re-hashes on
        //    import and a mismatch here means the file changed under us.
        let (path, _size) = self.cas.open_ciphertext_path(&cas_ref)?;
        let tt = self
            .store
            .blobs()
            .add_path_with_opts(AddPathOptions {
                path,
                format: BlobFormat::Raw,
                mode: ImportMode::TryReference,
            })
            .temp_tag()
            .await
            .map_err(|e| {
                tracing::debug!(%e, asset_id, "iroh-blobs add_path failed");
                Error::PeerUnreachable
            })?;
        let imported = tt.hash();
        drop(tt);

        if imported != hash {
            tracing::error!(
                ?imported, expected = ?hash,
                "iroh-blobs BLAKE3 disagrees with compute_ciphertext_blake3"
            );
            return Err(Error::BlobFormat);
        }
        Ok(hash)
    }

    /// Local-only probe — does this bridge already have bytes for `hash`?
    /// Used by the receive loop to skip re-fetching and by tests.
    pub async fn has_local(&self, hash: Hash) -> Result<bool> {
        self.store.blobs().has(hash).await.map_err(|e| {
            tracing::debug!(%e, "iroh-blobs has() failed");
            Error::PeerUnreachable
        })
    }

    /// Read all ciphertext bytes for `hash` from the local iroh-blobs
    /// store into a `Vec`. Intended for small-ish blobs (sidecars,
    /// thumbnails) and for the C9 receive path that sinks fetched bytes
    /// into CAS. Large media should stream via the downloader directly.
    pub async fn read_bytes(&self, hash: Hash) -> Result<Vec<u8>> {
        let bytes = self.store.blobs().get_bytes(hash).await.map_err(|e| {
            tracing::debug!(%e, %hash, "iroh-blobs get_bytes failed");
            Error::PeerUnreachable
        })?;
        Ok(bytes.to_vec())
    }

    /// Write `bytes` into the CAS at the plaintext-hash path for
    /// `target_cas_ref`. Used by the C9 receive loop after a download
    /// completes to materialise the ciphertext under its
    /// plaintext-BLAKE3-keyed path (the canonical CAS layout).
    ///
    /// Fails if the file already exists (callers check
    /// `CasStore::open_ciphertext_path` first).
    pub fn write_ciphertext_to_cas(&self, target_cas_ref: &str, bytes: &[u8]) -> Result<()> {
        if target_cas_ref.len() < 2 {
            return Err(Error::BlobFormat);
        }
        let aa = &target_cas_ref[..2];
        let cas_path = self
            .cas
            .root_path()
            .join("cas")
            .join(aa)
            .join(target_cas_ref);
        if let Some(parent) = cas_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if cas_path.exists() {
            tracing::debug!(
                ?cas_path,
                "ciphertext already present in CAS; skipping write"
            );
            return Ok(());
        }
        std::fs::write(&cas_path, bytes)?;
        Ok(())
    }
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bridge_starts_and_round_trips_add_path() {
        mv_core::crypto::init().unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let cas = Arc::new(CasStore::open(tmp.path()).unwrap());
        let db_path = tmp.path().join("index.db");
        let conn = mv_core::db::schema::open(&db_path).unwrap();
        let db = Arc::new(Mutex::new(conn));

        let bridge = BlobsBridge::start(tmp.path(), cas.clone(), db.clone())
            .await
            .expect("bridge start");

        // Write a random file and add it by path directly (bypassing
        // announce_asset so this test doesn't depend on a fully populated
        // asset row — that path is exercised in two_peer_share Tier-B).
        let payload_path = tmp.path().join("scratch.bin");
        let payload = b"ciphertext-like-bytes-aaaaaaaaaaaaaaaa";
        std::fs::write(&payload_path, payload).unwrap();
        let tt = bridge
            .store
            .blobs()
            .add_path_with_opts(AddPathOptions {
                path: payload_path.clone(),
                format: BlobFormat::Raw,
                mode: ImportMode::Copy,
            })
            .temp_tag()
            .await
            .expect("add_path");
        let hash = tt.hash();
        drop(tt);

        assert!(bridge.has_local(hash).await.unwrap());
        let round = bridge.read_bytes(hash).await.unwrap();
        assert_eq!(round.as_slice(), payload);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn write_ciphertext_to_cas_honors_aa_layout() {
        mv_core::crypto::init().unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let cas = Arc::new(CasStore::open(tmp.path()).unwrap());
        let db_path = tmp.path().join("index.db");
        let conn = mv_core::db::schema::open(&db_path).unwrap();
        let db = Arc::new(Mutex::new(conn));
        let bridge = BlobsBridge::start(tmp.path(), cas.clone(), db.clone())
            .await
            .unwrap();

        let fake_cas_ref = "ab".to_string() + &"cd".repeat(31);
        bridge
            .write_ciphertext_to_cas(&fake_cas_ref, b"dummy-ciphertext")
            .unwrap();
        let expected = tmp.path().join("cas").join("ab").join(&fake_cas_ref);
        assert!(expected.exists(), "blob must land at <root>/cas/<aa>/<hex>");
        // Idempotent: calling again doesn't error.
        bridge
            .write_ciphertext_to_cas(&fake_cas_ref, b"dummy-ciphertext")
            .unwrap();
    }
}
