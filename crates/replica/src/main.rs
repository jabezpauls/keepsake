//! `mv-replica` — headless ciphertext-only iroh peer (Phase 3 D8).
//!
//! The replica is an offsite backup node. It holds no plaintext and no
//! user password. Concretely:
//!
//! 1. On first run it bootstraps a single-user keystore at
//!    `$MV_REPLICA_ROOT` (default: `$HOME/.local/share/mv-replica`).
//!    The bootstrap password is a fixed sentinel — anyone with disk
//!    access can already read everything the replica holds (nothing
//!    sensitive), and locking against that adversary gains nothing.
//! 2. It prints its pairing ticket once per boot. Operators copy it
//!    into a normal Media Vault installation's "Peers > Accept" flow
//!    to authorise the replica for future shares.
//! 3. It brings up the full iroh stack (blobs + gossip + docs) and
//!    spawns the receive loop so any namespace tickets it already
//!    knows about resume. New namespace tickets can be fed in via
//!    `--accept-namespace <ticket>` on boot.
//! 4. It waits on Ctrl+C (systemd sends SIGTERM, which maps to the same
//!    graceful shutdown path).
//!
//! Blobs are pulled opportunistically by iroh-docs as CRDT events land
//! — the replica never holds, derives, or serves plaintext.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mv_core::cas::CasStore;
use mv_core::crypto::keystore::{self, UnlockedUser, UserRecord};
use mv_core::db;
use mv_sync::{
    accept_namespace_ticket, spawn_receive_loop, BlobsBridge, DocsBridge, Peer, PeerConfig,
    ReceiveContext,
};
use secrecy::SecretString;

/// Fixed bootstrap password for the replica's degenerate single-user
/// keystore. See module docs — nothing secret lives inside, so this is
/// a plaintext sentinel rather than an operator-supplied secret.
const REPLICA_PASSWORD: &str = "mv-replica-bootstrap-v1-no-secrets";
const REPLICA_USERNAME: &str = "__replica__";

#[derive(Debug, Default)]
struct Args {
    vault_root: Option<PathBuf>,
    relay_url: Option<String>,
    bind_port: u16,
    accept_namespace_tickets: Vec<String>,
    print_ticket_only: bool,
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--vault-root" => {
                i += 1;
                args.vault_root = Some(PathBuf::from(&raw[i]));
            }
            "--relay" => {
                i += 1;
                args.relay_url = Some(raw[i].clone());
            }
            "--bind-port" => {
                i += 1;
                args.bind_port = raw[i].parse().unwrap_or(0);
            }
            "--accept-namespace" => {
                i += 1;
                args.accept_namespace_tickets.push(raw[i].clone());
            }
            "--print-ticket" => {
                args.print_ticket_only = true;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("mv-replica: unknown arg {other:?}");
                print_help();
                std::process::exit(2);
            }
        }
        i += 1;
    }
    args
}

fn print_help() {
    println!("mv-replica — Media Vault offsite replica (headless)");
    println!();
    println!("Usage: mv-replica [options]");
    println!("  --vault-root <dir>        data dir (default: $MV_REPLICA_ROOT or $HOME/.local/share/mv-replica)");
    println!("  --relay <url>             optional iroh relay URL (default: LAN-only)");
    println!("  --bind-port <port>        UDP port (default: OS-assigned ephemeral)");
    println!("  --accept-namespace <b32>  accept a namespace ticket at boot (repeatable)");
    println!("  --print-ticket            print pairing ticket + exit");
    println!("  -h, --help                this help");
}

fn default_vault_root() -> PathBuf {
    if let Ok(p) = std::env::var("MV_REPLICA_ROOT") {
        return PathBuf::from(p);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/mv-replica");
    }
    PathBuf::from("./mv-replica-data")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = parse_args();
    let vault_root = args.vault_root.clone().unwrap_or_else(default_vault_root);
    std::fs::create_dir_all(&vault_root)?;

    let index_path = vault_root.join("index.db");
    let (conn, user) =
        tokio::task::spawn_blocking(move || load_or_bootstrap(&index_path)).await??;
    let conn = Arc::new(tokio::sync::Mutex::new(conn));
    let cas = Arc::new(CasStore::open(&vault_root)?);

    // Bring up the peer.
    let peer_cfg = PeerConfig {
        relay_url: args.relay_url.clone(),
        bind_port: args.bind_port,
    };
    let peer = Peer::start(&user, peer_cfg).await?;

    // Print pairing ticket. Operators copy this into a normal app's
    // "Peers > Accept" flow to authorise future shares to the replica.
    let iroh_seed = *user.iroh_node.secret_bytes();
    let ticket = peer.ticket(&iroh_seed)?;
    println!("mv-replica ready");
    println!("  vault-root:       {}", vault_root.display());
    println!("  node id (hex):    {}", hex::encode(peer.node_id_bytes()));
    println!("  identity pub:     {}", hex::encode(peer.identity_pub()));
    println!(
        "  relay:            {}",
        peer.relay_url().unwrap_or("LAN-only")
    );
    println!("  pairing ticket:");
    println!("    {}", ticket.to_base32());

    if args.print_ticket_only {
        peer.shutdown().await;
        return Ok(());
    }

    // Mount the full share router (blobs + gossip + docs) so inbound
    // peers can talk namespace CRDT + pull ciphertext.
    let blobs = Arc::new(BlobsBridge::start(&vault_root, cas.clone(), conn.clone()).await?);
    let gossip = iroh_gossip::net::Gossip::builder().spawn(peer.endpoint().clone());
    let blobs_store: iroh_blobs::api::Store = blobs.store().clone().into();
    let docs = Arc::new(
        DocsBridge::start(
            &vault_root,
            peer.endpoint().clone(),
            blobs_store,
            gossip.clone(),
        )
        .await?,
    );
    peer.mount_router(blobs.clone(), Some(gossip), Some(docs.clone()))?;

    let ctx = ReceiveContext {
        docs: docs.clone(),
        blobs: blobs.clone(),
        conn: conn.clone(),
        user: Arc::new(user),
    };

    for (idx, ticket) in args.accept_namespace_tickets.iter().enumerate() {
        match accept_namespace_ticket(&ctx, ticket).await {
            Ok(cid) => {
                println!("accepted namespace #{idx} → collection_id = {cid}");
            }
            Err(e) => {
                tracing::error!(%e, idx, "accept_namespace failed");
            }
        }
    }

    let _handles = spawn_receive_loop(ctx).await?;
    println!("listening for events — send SIGINT/SIGTERM to exit");

    // Tokio's `ctrl_c` future also resolves on SIGTERM on unix when the
    // signal handler is installed by default, which `tokio::main` does.
    tokio::signal::ctrl_c().await?;
    println!("shutting down…");
    peer.shutdown().await;
    Ok(())
}

/// Open `index.db` and return a ready-to-use connection plus an
/// `UnlockedUser`. If the vault has no user yet, bootstrap it with the
/// `REPLICA_USERNAME` / `REPLICA_PASSWORD` sentinel pair.
fn load_or_bootstrap(index_path: &Path) -> anyhow::Result<(rusqlite::Connection, UnlockedUser)> {
    let conn = db::schema::open(index_path)?;
    let ids = db::list_user_ids(&conn)?;
    let pw = SecretString::from(REPLICA_PASSWORD.to_string());

    if ids.is_empty() {
        let (record, mut unlocked) = keystore::create_user(REPLICA_USERNAME, &pw)?;
        let now = chrono::Utc::now().timestamp();
        let user_id = db::insert_user(&conn, &record, now)?;
        unlocked.user_id = user_id;
        tracing::info!(user_id, "replica bootstrap: created sentinel keystore");
        return Ok((conn, unlocked));
    }

    // Existing keystore: take the first (and in the replica's
    // single-user model, only) user row. If a human ran mv-replica
    // against a full user vault by mistake, the sentinel password will
    // fail to open the wrapped master key — we surface that as an
    // error rather than iterating through users.
    let (user_id, _ipk, _ca) = ids[0].clone();
    let record: UserRecord = db::get_user_record(&conn, user_id)?;
    let unlocked = keystore::unlock(&record, &pw, user_id).map_err(|_| {
        anyhow::anyhow!(
            "cannot unlock replica vault at {} — password mismatch or non-replica data",
            index_path.display()
        )
    })?;
    tracing::info!(user_id, "replica resumed existing keystore");
    Ok((conn, unlocked))
}
