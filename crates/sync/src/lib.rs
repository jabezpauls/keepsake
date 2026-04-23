//! Peer sync crate — Phase 3.
//!
//! Phase 3.1 (C1–C5):
//! * [`ticket`] — `PairingTicket` wire format (architecture.md §6).
//! * [`peer`]   — iroh `Endpoint` + ticket issuance.
//! * [`accept`] — ticket parsing + `peer_accept` persistence.
//!
//! Phase 3.2 (C6–C14) — album sharing:
//! * [`blobs`]  — iroh-blobs FsStore bridge serving CAS ciphertext.
//! * (C7) `docs` — iroh-docs namespace wrapper.
//! * (C8) `share` — publish collection key + metadata.
//! * (C9) `receive` — receive loop for incoming shares.

pub mod accept;
pub mod blobs;
pub mod docs;
pub mod peer;
pub mod ticket;

pub use accept::{accept_base32, accept_ticket, forget, list_for_user};
pub use blobs::BlobsBridge;
pub use docs::{DocsBridge, DocsKey, SharedCollectionNamespace};
pub use peer::{Peer, PeerConfig};
pub use ticket::PairingTicket;
