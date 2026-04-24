//! Wire-level DTOs exchanged with the TS/React frontend.
//!
//! Every struct derives `ts-rs::TS` so `cargo test` regenerates the matching
//! TypeScript definitions in `app/src/bindings/`. Keep this module the single
//! source of truth for the IPC surface — never hand-write TS types.
//!
//! `i64` fields use `#[ts(type = "number")]` because Tauri serialises numeric
//! primitives as JSON `number` (ts-rs otherwise emits `bigint`, which is not
//! interoperable with plain-number JSON).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct SessionHandle {
    #[ts(type = "number")]
    pub user_id: i64,
    pub username: String,
    #[ts(type = "number")]
    pub default_collection_id: i64,
    pub hidden_unlocked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct SourceView {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub root_path: String,
    pub adapter_kind: String,
    pub linked_only: bool,
    #[ts(type = "number")]
    pub bytes_total: i64,
    #[ts(type = "number")]
    pub file_count: i64,
    #[ts(type = "number")]
    pub imported_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum IngestState {
    Idle,
    Running {
        #[ts(type = "number")]
        files_processed: u64,
        #[ts(type = "number")]
        files_total: u64,
        current: Option<String>,
    },
    Done {
        #[ts(type = "number")]
        inserted: u64,
        #[ts(type = "number")]
        deduped: u64,
        #[ts(type = "number")]
        skipped: u64,
        #[ts(type = "number")]
        errors: u64,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct IngestStatus {
    #[ts(type = "number")]
    pub source_id: i64,
    pub state: IngestState,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct TimelineCursor {
    #[ts(type = "number")]
    pub day: i64,
    #[ts(type = "number")]
    pub id: i64,
}

impl TimelineCursor {
    pub fn start() -> Self {
        Self {
            day: i64::MAX,
            id: i64::MAX,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct TimelineEntryView {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number | null")]
    pub taken_at_utc_day: Option<i64>,
    pub mime: String,
    pub is_video: bool,
    pub is_live: bool,
    pub is_raw: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct TimelinePage {
    pub entries: Vec<TimelineEntryView>,
    pub next_cursor: Option<TimelineCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct AssetDetailView {
    #[ts(type = "number")]
    pub id: i64,
    pub mime: String,
    #[ts(type = "number")]
    pub bytes: i64,
    #[ts(type = "number | null")]
    pub width: Option<i64>,
    #[ts(type = "number | null")]
    pub height: Option<i64>,
    #[ts(type = "number | null")]
    pub duration_ms: Option<i64>,
    #[ts(type = "number | null")]
    pub taken_at_utc_day: Option<i64>,
    pub is_video: bool,
    pub is_live: bool,
    pub is_motion: bool,
    pub is_raw: bool,
    pub is_screenshot: bool,
    pub filename: String,
    pub taken_at_utc: Option<String>,
    pub gps: Option<GpsView>,
    pub device: Option<String>,
    pub lens: Option<String>,
    pub exif_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct GpsView {
    pub lat: f64,
    pub lon: f64,
    pub alt: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct AlbumView {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub kind: String,
    #[ts(type = "number")]
    pub member_count: i64,
    pub has_password: bool,
    pub unlocked: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct ExportOptions {
    pub include_xmp: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct ExportReport {
    #[ts(type = "number")]
    pub files_written: u64,
    #[ts(type = "number")]
    pub bytes_written: u64,
    #[ts(type = "number")]
    pub xmp_written: u64,
    #[ts(type = "number")]
    pub skipped: u64,
}

// =========== Phase 2 DTOs =====================================================

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct PersonView {
    #[ts(type = "number")]
    pub id: i64,
    pub name: Option<String>,
    pub hidden: bool,
    #[ts(type = "number")]
    pub face_count: i64,
    #[ts(type = "number | null")]
    pub cover_asset_id: Option<i64>,
}

/// One detected face within a single asset, for the viewer face overlay.
///
/// `bbox` is xywh in thumb1024 pixel space (same coord space the viewer
/// renders from), so the client scales to % for CSS positioning.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct AssetFaceView {
    #[ts(type = "number")]
    pub face_id: i64,
    #[ts(type = "number | null")]
    pub person_id: Option<i64>,
    pub person_name: Option<String>,
    pub bbox: [f32; 4],
    pub quality: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct MapPoint {
    #[ts(type = "number")]
    pub asset_id: i64,
    pub lat: f64,
    pub lon: f64,
    #[ts(type = "number | null")]
    pub taken_at_utc_day: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct SearchRequest {
    pub text: Option<String>,
    #[ts(type = "Array<number>")]
    pub person_ids: Vec<i64>,
    #[ts(type = "number | null")]
    pub after_day: Option<i64>,
    #[ts(type = "number | null")]
    pub before_day: Option<i64>,
    #[ts(type = "number | null")]
    pub source_id: Option<i64>,
    pub has_faces: Option<bool>,
    pub is_video: Option<bool>,
    pub is_raw: Option<bool>,
    pub is_screenshot: Option<bool>,
    pub is_live: Option<bool>,
    pub camera_make: Option<String>,
    pub lens: Option<String>,
    #[ts(type = "number")]
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct SearchHitView {
    #[ts(type = "number")]
    pub id: i64,
    pub score: Option<f32>,
    #[ts(type = "number | null")]
    pub taken_at_utc_day: Option<i64>,
    pub mime: String,
    pub is_video: bool,
    pub is_live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct NearDupCluster {
    #[ts(type = "number")]
    pub cluster_id: i64,
    pub members: Vec<NearDupMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct NearDupMember {
    #[ts(type = "number")]
    pub asset_id: i64,
    pub is_best: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct MlStatus {
    /// `true` when the app was built with `--features ml-models`. Says nothing
    /// about whether weights are actually loaded — see `runtime_loaded`.
    pub models_available: bool,
    /// `true` when the on-device ML runtime has loaded (weights + tokenizer
    /// live in memory, ort sessions ready). Implies `models_available`.
    pub runtime_loaded: bool,
    /// Human-readable execution provider the runtime picked ("Cpu", "Cuda",
    /// "CoreMl", or "disabled" when `runtime_loaded=false`). Shown in the
    /// banner so users can verify where inference is actually running.
    pub execution_provider: String,
    #[ts(type = "number")]
    pub pending: i64,
    #[ts(type = "number")]
    pub running: i64,
    #[ts(type = "number")]
    pub done: i64,
    #[ts(type = "number")]
    pub failed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct PairingTicketView {
    /// Base32-encoded ticket to copy/paste or render as a QR. Always
    /// lowercase RFC4648 without padding.
    pub base32: String,
    /// Hex of the signer's 32-byte Ed25519 node id. Useful for the UI to
    /// show "my node: abc12345…" next to the ticket.
    pub my_node_id_hex: String,
    /// UNIX seconds the ticket was signed at.
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct PeerAcceptedView {
    /// Hex of the remote node id. UI truncates for display.
    pub node_id_hex: String,
    /// Hex of the remote X25519 identity public key. Phase 3.2 consumers use
    /// this to seal collection keys back.
    pub identity_pub_hex: String,
    /// `None` = LAN-only; `Some` = relay the remote published.
    pub relay_url: Option<String>,
    #[ts(type = "number")]
    pub added_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct MemoryGroupView {
    /// Year the photos were taken in.
    #[ts(type = "number")]
    pub year: i32,
    /// Years ago relative to today ("1 year ago", "5 years ago").
    #[ts(type = "number")]
    pub years_ago: i32,
    /// Asset ids ordered; the UI pages/thumbnails from these.
    #[ts(type = "Array<number>")]
    pub asset_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct TripView {
    #[ts(type = "number")]
    pub id: i64,
    /// Decrypted trip name — e.g. "Trip · 12 photos · day 20050..20055".
    /// D2 (reverse geocoding) will replace this with "Tokyo, 2024".
    pub name: String,
    #[ts(type = "number")]
    pub member_count: i64,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct ShareRecipientView {
    /// Peer node id (Ed25519) in hex — matches what peer_list returns.
    pub peer_node_id_hex: String,
    /// Peer identity public key (X25519) in hex — the recipient's
    /// seal/open handle.
    pub peer_identity_pub_hex: String,
    /// Iroh relay URL the recipient published (None for LAN-only).
    pub relay_url: Option<String>,
    /// UNIX seconds when the wrapping landed on the sender side. Not
    /// when the recipient accepted it — the sender has no way to know
    /// that without an explicit ack.
    #[ts(type = "number")]
    pub shared_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct IncomingShareView {
    #[ts(type = "number")]
    pub collection_id: i64,
    pub namespace_id_hex: String,
    pub sender_identity_pub_hex: String,
    /// `"pending"` = namespace joined, no collection key yet.
    /// `"accepted"` = key unwrapped + album rendered.
    /// `"revoked"` = tombstone received.
    pub state: String,
    /// Populated once the first `c/meta/` event decrypts. Until then
    /// the UI shows "(incoming share)".
    pub album_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct ShareInviteView {
    /// Base32-encoded iroh-docs `DocTicket`. Paste into the recipient's
    /// "Accept invite" textarea to subscribe to the namespace.
    pub namespace_ticket_base32: String,
    #[ts(type = "number")]
    pub collection_id: i64,
    pub asset_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
/// D6 multi-user summary. Surfaced pre-unlock on the login screen and
/// post-unlock in the same-device share picker. Username stays sealed
/// — callers can render "User #{user_id}" until the typed-in username
/// unlocks the row.
pub struct UserSummaryView {
    #[ts(type = "number")]
    pub user_id: i64,
    /// X25519 public key in hex. Matches the bytes we hand remote peers
    /// when sealing a collection key — the same ID space is reused for
    /// same-device sharing.
    pub identity_pub_hex: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
/// D4 rule spec. Mirrors `mv_core::analytics::smart_albums::SmartRule`
/// 1:1 so the UI can author a rule without reaching into a separate
/// crate. Fields default to `null` (all) or `[]` (person ids) — the
/// `any` empty-rule check lives on the command side, not in types.
pub struct SmartRuleView {
    pub is_raw: Option<bool>,
    pub is_video: Option<bool>,
    pub is_screenshot: Option<bool>,
    pub is_live: Option<bool>,
    pub has_faces: Option<bool>,
    pub camera_make: Option<String>,
    pub lens: Option<String>,
    #[ts(type = "number | null")]
    pub source_id: Option<i64>,
    #[ts(type = "Array<number>")]
    pub person_ids: Vec<i64>,
    #[ts(type = "number | null")]
    pub after_day: Option<i64>,
    #[ts(type = "number | null")]
    pub before_day: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct SmartAlbumView {
    #[ts(type = "number")]
    pub id: i64,
    /// Decrypted display name.
    pub name: String,
    /// Rule that produced this album's snapshot.
    pub rule: SmartRuleView,
    /// Last materialised count. Stale until `refresh_smart_album` runs.
    #[ts(type = "number")]
    pub member_count: i64,
    /// UNIX seconds of the last refresh. `None` when the album was
    /// created but never materialised (shouldn't happen — create always
    /// refreshes — but kept nullable for UI safety).
    #[ts(type = "number | null")]
    pub snapshot_at: Option<i64>,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct MlReindexReport {
    /// Jobs newly inserted for CLIP embedding. Dedupes are excluded.
    #[ts(type = "number")]
    pub embed_queued: u32,
    /// Jobs newly inserted for face detection + embedding.
    #[ts(type = "number")]
    pub detect_queued: u32,
    /// Distinct assets that at least one sweep hit — useful for showing a
    /// single "N assets reindexed" number.
    #[ts(type = "number")]
    pub assets_touched: u32,
}
