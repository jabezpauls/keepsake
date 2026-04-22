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
    pub models_available: bool,
    #[ts(type = "number")]
    pub pending: i64,
    #[ts(type = "number")]
    pub running: i64,
    #[ts(type = "number")]
    pub done: i64,
    #[ts(type = "number")]
    pub failed: i64,
}
