//! Peer lifecycle — wraps an `iroh::Endpoint` built from the unlocked user's
//! Ed25519 seed.
//!
//! **LAN-only by default.** `iroh::Endpoint::builder(N0)` dials the public
//! iroh relay (`https://relay.iroh.network`). Architecture.md §9 commits the
//! project to "no network calls by default", so we build with the `Minimal`
//! preset + explicit `RelayMode::Disabled`. A future `MV_IROH_RELAY` env var
//! or Settings field will opt in to a user-chosen relay.
//!
//! The Phase 3.1 commits covered boot + ticket issuance. Phase 3.2 adds an
//! optional [`iroh::protocol::Router`] that accepts the blobs ALPN (iroh-
//! blobs) and — in C7 — the docs ALPN. `mount_router` is idempotent; the
//! first call transitions the peer from "endpoint-only" to "serving
//! requests", and subsequent calls are no-ops.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex as StdMutex};

use iroh::endpoint::{presets, Endpoint};
use iroh::protocol::Router;
use iroh::{RelayMode, SecretKey};
use mv_core::crypto::keystore::UnlockedUser;
use mv_core::{Error, Result};

use crate::blobs::BlobsBridge;
use crate::ticket::PairingTicket;

/// Config handed to [`Peer::start`]. All fields have sensible defaults so
/// callers can pass `PeerConfig::default()` for a LAN-only peer.
///
/// `bind_port = 0` → OS-assigned ephemeral UDP port (matches iroh's normal
/// behavior for tests). Long-running peers set an explicit port.
#[derive(Debug, Clone, Default)]
pub struct PeerConfig {
    /// Relay URL (if any) to publish in generated tickets + dial when
    /// accepting remote peers. `None` = LAN-only.
    pub relay_url: Option<String>,
    /// UDP port to bind. 0 = OS-assigned ephemeral.
    pub bind_port: u16,
}

/// Live peer handle. Owns the Iroh endpoint; drop shuts the transport down.
pub struct Peer {
    endpoint: Endpoint,
    identity_pub: [u8; 32],
    relay_url: Option<String>,
    /// Router is installed lazily by `mount_router`. Guarded by a
    /// `std::sync::Mutex` (not `tokio::sync`) because we only ever swap
    /// it — no await holds the guard.
    router: StdMutex<Option<Router>>,
    /// `BlobsBridge` we've mounted. Kept behind the mutex so we can
    /// expose it from handlers without re-building.
    blobs: StdMutex<Option<Arc<BlobsBridge>>>,
}

impl Peer {
    /// Boot the peer from an unlocked user's Ed25519 seed + X25519 identity.
    /// Network IO is lazy — `bind()` sets up the UDP socket but doesn't dial
    /// anything until `connect` is called (C5+).
    pub async fn start(user: &UnlockedUser, config: PeerConfig) -> Result<Self> {
        let seed = *user.iroh_node.secret_bytes();
        let secret_key = SecretKey::from_bytes(&seed);
        let identity_pub = user.identity.public.0;

        let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, config.bind_port);

        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(secret_key)
            .relay_mode(RelayMode::Disabled)
            .bind_addr(addr)
            .map_err(|e| {
                tracing::debug!(%e, "iroh bind_addr failed");
                Error::PeerUnreachable
            })?
            .bind()
            .await
            .map_err(|e| {
                tracing::debug!(%e, "iroh endpoint bind failed");
                Error::PeerUnreachable
            })?;

        Ok(Self {
            endpoint,
            identity_pub,
            relay_url: config.relay_url,
            router: StdMutex::new(None),
            blobs: StdMutex::new(None),
        })
    }

    /// The Ed25519 node id in its 32-byte form. Matches
    /// `user.iroh_node_pub` the DB already stores. `Endpoint::id` in iroh
    /// 0.98 returns the public-key-based `EndpointId` (aliased `PublicKey`).
    pub fn node_id_bytes(&self) -> [u8; 32] {
        *self.endpoint.id().as_bytes()
    }

    /// The X25519 identity public key. Recipients need this to seal back
    /// (collection keys, profile updates) via `seal_for_peer`.
    pub fn identity_pub(&self) -> [u8; 32] {
        self.identity_pub
    }

    /// Publish relay URL (if any). `None` = LAN-only.
    pub fn relay_url(&self) -> Option<&str> {
        self.relay_url.as_deref()
    }

    /// Issue a fresh pairing ticket, signed by the user's iroh secret key.
    /// Timestamp is current UNIX seconds; callers can inspect it before
    /// presenting to a peer.
    pub fn ticket(&self, iroh_secret_seed: &[u8; 32]) -> Result<PairingTicket> {
        let ts = chrono::Utc::now().timestamp();
        PairingTicket::sign(
            iroh_secret_seed,
            &self.identity_pub,
            self.relay_url.clone().unwrap_or_default(),
            ts,
        )
    }

    /// Reference into the underlying endpoint for connect / downloader logic.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Install the iroh `Router` wrapping the endpoint. Registers the
    /// `iroh-blobs` ALPN so peers can fetch ciphertext by BLAKE3. The C7
    /// commit extends this with the `iroh-docs` ALPN.
    ///
    /// Idempotent — calling twice reuses the existing router + bridge.
    /// Boots lazily so users who never share can skip the cost.
    pub fn mount_router(&self, blobs: Arc<BlobsBridge>) -> Result<()> {
        let mut slot = self.router.lock().expect("router mutex poisoned");
        if slot.is_some() {
            return Ok(());
        }
        let router = Router::builder(self.endpoint.clone())
            .accept(iroh_blobs::protocol::ALPN, blobs.protocol())
            .spawn();
        *slot = Some(router);
        *self.blobs.lock().expect("blobs mutex poisoned") = Some(blobs);
        Ok(())
    }

    /// The mounted `BlobsBridge` if any — `None` until `mount_router` has
    /// been called.
    pub fn blobs(&self) -> Option<Arc<BlobsBridge>> {
        self.blobs.lock().expect("blobs mutex poisoned").clone()
    }

    /// Graceful shutdown. Drops the router (which aborts the accept task)
    /// and closes the endpoint.
    pub async fn shutdown(self) {
        let router = self.router.lock().expect("router mutex poisoned").take();
        if let Some(r) = router {
            let _ = r.shutdown().await;
        }
        self.endpoint.close().await;
    }
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use mv_core::crypto::keystore;
    use secrecy::SecretString;

    fn fresh_user() -> UnlockedUser {
        mv_core::crypto::init().unwrap();
        let (_record, unlocked) =
            keystore::create_user("peer-test", &SecretString::from("very-long-pw-xyz")).unwrap();
        unlocked
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn peer_boots_with_lan_only_default() {
        let user = fresh_user();
        let peer = Peer::start(&user, PeerConfig::default())
            .await
            .expect("peer should bind");
        // node id must match the user's stored Iroh public.
        assert_eq!(peer.node_id_bytes(), user.iroh_node.public.0);
        assert_eq!(peer.identity_pub(), user.identity.public.0);
        assert!(peer.relay_url().is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_peers_have_distinct_node_ids() {
        let a = fresh_user();
        let b = fresh_user();
        let pa = Peer::start(&a, PeerConfig::default()).await.unwrap();
        let pb = Peer::start(&b, PeerConfig::default()).await.unwrap();
        assert_ne!(
            pa.node_id_bytes(),
            pb.node_id_bytes(),
            "two independently-seeded peers must have distinct node ids"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ticket_roundtrips_and_verifies() {
        let user = fresh_user();
        let peer = Peer::start(&user, PeerConfig::default()).await.unwrap();
        let seed = *user.iroh_node.secret_bytes();
        let ticket = peer.ticket(&seed).unwrap();
        ticket.verify().expect("own ticket must verify");
        assert_eq!(ticket.iroh_node_pub, user.iroh_node.public.0);
        assert_eq!(ticket.identity_pub, user.identity.public.0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mount_router_is_idempotent() {
        use mv_core::cas::CasStore;
        use std::sync::Arc as StdArc;
        use tokio::sync::Mutex;

        let user = fresh_user();
        let peer = Peer::start(&user, PeerConfig::default()).await.unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let cas = StdArc::new(CasStore::open(tmp.path()).unwrap());
        let db_path = tmp.path().join("index.db");
        let conn = mv_core::db::schema::open(&db_path).unwrap();
        let db = StdArc::new(Mutex::new(conn));
        let bridge = StdArc::new(
            BlobsBridge::start(tmp.path(), cas, db)
                .await
                .expect("bridge"),
        );

        peer.mount_router(bridge.clone()).unwrap();
        peer.mount_router(bridge.clone()).unwrap(); // second call no-op
        assert!(peer.blobs().is_some());
        peer.shutdown().await;
    }
}
