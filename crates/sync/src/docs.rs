//! iroh-docs bridge — per-collection CRDT namespaces that carry sealed
//! album metadata and per-recipient collection-key envelopes.
//!
//! See `plans/phase-3-peers-smart.md` §2 for the frozen key schema. This
//! module codifies that schema behind a [`DocsKey`] builder so callers
//! cannot drift from the layout by accident. Every value a caller writes
//! to a shared namespace must be sealed under the collection key first
//! (via `crypto::envelope::seal_row`) — iroh-docs is transport, not
//! confidentiality.
//!
//! Persistence at `<vault_root>/iroh/docs/`. Mapping between our local
//! `collection.id` and the 32-byte `NamespaceId` lives in the
//! `shared_namespace` table (schema v5).

use std::path::Path;

use iroh::Endpoint;
use iroh_blobs::api::Store as BlobsStore;
use iroh_docs::protocol::Docs;
use iroh_docs::sync::Capability;
use iroh_docs::{AuthorId, NamespaceId};
use iroh_gossip::net::Gossip;
use mv_core::{Error, Result};
use rusqlite::{params, Connection};

/// Typed iroh-docs key builder. The byte layout is **frozen** per
/// `phase-3-peers-smart.md §2`; this type is the only supported way to
/// produce a key for a shared-collection namespace.
///
/// Convention: every i64 encodes big-endian so prefix scans over the
/// namespace land in id-ascending order. Public keys encode as their raw
/// 32-byte value — no hex — so the key length stays predictable.
pub struct DocsKey;

impl DocsKey {
    const PREFIX_META: &'static [u8] = b"c/meta/";
    const PREFIX_MEM: &'static [u8] = b"c/mem/";
    const PREFIX_KEY: &'static [u8] = b"c/key/";
    const PREFIX_REV: &'static [u8] = b"c/rev/";

    /// `c/meta/<collection_id:be64>` — sealed `CollectionMeta` blob.
    pub fn collection_meta(collection_id: i64) -> Vec<u8> {
        let mut v = Vec::with_capacity(Self::PREFIX_META.len() + 8);
        v.extend_from_slice(Self::PREFIX_META);
        v.extend_from_slice(&collection_id.to_be_bytes());
        v
    }

    /// `c/mem/<asset_id:be64>` — sealed per-asset `MemberEntry` blob.
    pub fn collection_member(asset_id: i64) -> Vec<u8> {
        let mut v = Vec::with_capacity(Self::PREFIX_MEM.len() + 8);
        v.extend_from_slice(Self::PREFIX_MEM);
        v.extend_from_slice(&asset_id.to_be_bytes());
        v
    }

    /// `c/key/<recipient_identity_pub:32>` — collection key sealed for a
    /// specific peer's X25519 identity. Only that peer can unwrap.
    pub fn collection_key(recipient_identity_pub: &[u8; 32]) -> Vec<u8> {
        let mut v = Vec::with_capacity(Self::PREFIX_KEY.len() + 32);
        v.extend_from_slice(Self::PREFIX_KEY);
        v.extend_from_slice(recipient_identity_pub);
        v
    }

    /// `c/rev/<recipient_identity_pub:32>` — tombstone removing a
    /// recipient. Written on revocation (C10).
    pub fn collection_revocation(recipient_identity_pub: &[u8; 32]) -> Vec<u8> {
        let mut v = Vec::with_capacity(Self::PREFIX_REV.len() + 32);
        v.extend_from_slice(Self::PREFIX_REV);
        v.extend_from_slice(recipient_identity_pub);
        v
    }

    /// Peel the 32-byte recipient identity pub out of a `c/key/…` key,
    /// or `None` if the key doesn't carry that prefix.
    pub fn extract_key_recipient(key: &[u8]) -> Option<[u8; 32]> {
        if key.len() == Self::PREFIX_KEY.len() + 32 && key.starts_with(Self::PREFIX_KEY) {
            let mut out = [0u8; 32];
            out.copy_from_slice(&key[Self::PREFIX_KEY.len()..]);
            Some(out)
        } else {
            None
        }
    }

    /// Peel a 32-byte recipient pub out of a `c/rev/…` tombstone.
    pub fn extract_rev_recipient(key: &[u8]) -> Option<[u8; 32]> {
        if key.len() == Self::PREFIX_REV.len() + 32 && key.starts_with(Self::PREFIX_REV) {
            let mut out = [0u8; 32];
            out.copy_from_slice(&key[Self::PREFIX_REV.len()..]);
            Some(out)
        } else {
            None
        }
    }

    /// Peel an i64 asset_id out of a `c/mem/…` key.
    pub fn extract_member_id(key: &[u8]) -> Option<i64> {
        if key.len() == Self::PREFIX_MEM.len() + 8 && key.starts_with(Self::PREFIX_MEM) {
            let mut be = [0u8; 8];
            be.copy_from_slice(&key[Self::PREFIX_MEM.len()..]);
            Some(i64::from_be_bytes(be))
        } else {
            None
        }
    }

    /// Peel an i64 collection_id out of a `c/meta/…` key.
    pub fn extract_meta_id(key: &[u8]) -> Option<i64> {
        if key.len() == Self::PREFIX_META.len() + 8 && key.starts_with(Self::PREFIX_META) {
            let mut be = [0u8; 8];
            be.copy_from_slice(&key[Self::PREFIX_META.len()..]);
            Some(i64::from_be_bytes(be))
        } else {
            None
        }
    }
}

/// A shared-collection namespace this peer is participating in — either
/// as the owner (`role='owner'`) or as a recipient (`role='peer'`).
pub struct SharedCollectionNamespace {
    pub collection_id: i64,
    pub doc: iroh_docs::api::Doc,
    pub author: AuthorId,
    pub role: String,
}

/// Bridge between our DB and iroh-docs. Thin wrapper: holds the `Docs`
/// protocol + DB mutex, exposes `create_shared` / `open_shared` /
/// `import_shared` and the key builder.
pub struct DocsBridge {
    docs: Docs,
}

impl DocsBridge {
    /// Boot a persistent iroh-docs instance at `<vault_root>/iroh/docs/`.
    /// Requires the already-configured `endpoint`, the shared
    /// `BlobsStore` (iroh-docs stores entry content bytes in the same
    /// store our BlobsBridge serves from), and a `Gossip` instance (the
    /// CRDT sync protocol runs over gossip).
    pub async fn start(
        vault_root: &Path,
        endpoint: Endpoint,
        blobs: BlobsStore,
        gossip: Gossip,
    ) -> Result<Self> {
        let docs_dir = vault_root.join("iroh").join("docs");
        std::fs::create_dir_all(&docs_dir)?;
        let docs = Docs::persistent(docs_dir.clone())
            .spawn(endpoint, blobs, gossip)
            .await
            .map_err(|e| {
                tracing::debug!(%e, ?docs_dir, "iroh-docs spawn failed");
                Error::PeerUnreachable
            })?;
        Ok(Self { docs })
    }

    /// Protocol handler — caller registers under `iroh_docs::ALPN`.
    pub fn protocol(&self) -> Docs {
        self.docs.clone()
    }

    /// Create a brand-new shared-collection namespace (we are owner +
    /// default author). Persists the mapping into `shared_namespace`
    /// (schema v5) so subsequent boots can recover via `open_shared`.
    pub async fn create_shared(
        &self,
        conn: &tokio::sync::Mutex<Connection>,
        collection_id: i64,
    ) -> Result<SharedCollectionNamespace> {
        let doc = self.docs.api().create().await.map_err(|e| {
            tracing::debug!(%e, "docs.create() failed");
            Error::PeerUnreachable
        })?;
        let author = self.docs.api().author_default().await.map_err(|e| {
            tracing::debug!(%e, "author_default failed");
            Error::PeerUnreachable
        })?;

        let ns_bytes: [u8; 32] = *AsRef::<[u8; 32]>::as_ref(&doc.id());
        let author_bytes = author.to_bytes();
        let now = chrono::Utc::now().timestamp();
        {
            let c = conn.lock().await;
            c.execute(
                r"INSERT OR REPLACE INTO shared_namespace
                  (collection_id, namespace_id, role, author_id, created_at)
                  VALUES (?1, ?2, 'owner', ?3, ?4)",
                params![collection_id, &ns_bytes[..], &author_bytes[..], now],
            )?;
        }

        Ok(SharedCollectionNamespace {
            collection_id,
            doc,
            author,
            role: "owner".into(),
        })
    }

    /// Re-open a namespace we already participate in. `role` is
    /// `"owner"` or `"peer"`.
    pub async fn open_shared(
        &self,
        conn: &tokio::sync::Mutex<Connection>,
        collection_id: i64,
        role: &str,
    ) -> Result<SharedCollectionNamespace> {
        let (ns_bytes, author_bytes): (Vec<u8>, Vec<u8>) = {
            let c = conn.lock().await;
            c.query_row(
                r"SELECT namespace_id, author_id FROM shared_namespace
                  WHERE collection_id = ?1 AND role = ?2",
                params![collection_id, role],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
        };
        if ns_bytes.len() != 32 || author_bytes.len() != 32 {
            return Err(Error::BlobFormat);
        }
        let mut ns_arr = [0u8; 32];
        ns_arr.copy_from_slice(&ns_bytes);
        let mut author_arr = [0u8; 32];
        author_arr.copy_from_slice(&author_bytes);

        let namespace_id: NamespaceId = ns_arr.into();
        let doc = self
            .docs
            .api()
            .open(namespace_id)
            .await
            .map_err(|e| {
                tracing::debug!(%e, ?namespace_id, "docs.open() failed");
                Error::PeerUnreachable
            })?
            .ok_or(Error::NotFound)?;
        let author: AuthorId = author_arr.into();

        Ok(SharedCollectionNamespace {
            collection_id,
            doc,
            author,
            role: role.to_string(),
        })
    }

    /// Import a capability (received from the sender out-of-band or via
    /// an invite message in C9) and persist it as a peer-side
    /// `shared_namespace` row. Used by the receive-loop ticket accept
    /// path.
    pub async fn import_shared(
        &self,
        conn: &tokio::sync::Mutex<Connection>,
        collection_id: i64,
        capability: Capability,
    ) -> Result<SharedCollectionNamespace> {
        let doc = self
            .docs
            .api()
            .import_namespace(capability)
            .await
            .map_err(|e| {
                tracing::debug!(%e, "docs.import_namespace failed");
                Error::PeerUnreachable
            })?;
        let author = self.docs.api().author_default().await.map_err(|e| {
            tracing::debug!(%e, "author_default failed");
            Error::PeerUnreachable
        })?;

        let ns_bytes: [u8; 32] = *AsRef::<[u8; 32]>::as_ref(&doc.id());
        let author_bytes = author.to_bytes();
        let now = chrono::Utc::now().timestamp();
        {
            let c = conn.lock().await;
            c.execute(
                r"INSERT OR REPLACE INTO shared_namespace
                  (collection_id, namespace_id, role, author_id, created_at)
                  VALUES (?1, ?2, 'peer', ?3, ?4)",
                params![collection_id, &ns_bytes[..], &author_bytes[..], now],
            )?;
        }

        Ok(SharedCollectionNamespace {
            collection_id,
            doc,
            author,
            role: "peer".into(),
        })
    }
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_builders_are_stable() {
        // Spot-check: prefix bytes + fixed length shape.
        let k_meta = DocsKey::collection_meta(42);
        assert_eq!(&k_meta[..7], b"c/meta/");
        assert_eq!(k_meta.len(), 7 + 8);

        let k_mem = DocsKey::collection_member(12345);
        assert_eq!(&k_mem[..6], b"c/mem/");

        let pk = [7u8; 32];
        let k_key = DocsKey::collection_key(&pk);
        assert_eq!(&k_key[..6], b"c/key/");
        assert_eq!(k_key.len(), 6 + 32);

        let k_rev = DocsKey::collection_revocation(&pk);
        assert_eq!(&k_rev[..6], b"c/rev/");
    }

    #[test]
    fn roundtrip_extractors() {
        let pk = [0xABu8; 32];
        let k = DocsKey::collection_key(&pk);
        assert_eq!(DocsKey::extract_key_recipient(&k), Some(pk));

        let r = DocsKey::collection_revocation(&pk);
        assert_eq!(DocsKey::extract_rev_recipient(&r), Some(pk));

        let m = DocsKey::collection_member(987_654);
        assert_eq!(DocsKey::extract_member_id(&m), Some(987_654));

        let meta = DocsKey::collection_meta(-17);
        assert_eq!(DocsKey::extract_meta_id(&meta), Some(-17));
    }

    #[test]
    fn extractors_reject_wrong_prefix() {
        assert!(
            DocsKey::extract_key_recipient(b"c/mem/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx").is_none()
        );
        assert!(DocsKey::extract_member_id(b"wrong").is_none());
    }
}
