//! Peer sync crate — Phase 3.
//!
//! Phase 3.1 (this commit range):
//! * [`ticket`] — `PairingTicket` wire format (architecture.md §6).
//!
//! Later in Phase 3.1: `peer`, `accept`. Phase 3.2 adds iroh-docs namespaces
//! and the CAS ↔ iroh-blobs bridge.

pub mod ticket;

pub use ticket::PairingTicket;
