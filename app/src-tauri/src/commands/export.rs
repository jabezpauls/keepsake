//! Album export — writes decrypted originals + XMP sidecars to a destination.

use std::path::PathBuf;

use mv_core::crypto::{open_row, unwrap_file_key};
use mv_core::db;
use mv_core::ingest::sidecar::{write_xmp_sidecar, XmpFields};
use tauri::State;

use crate::commands::albums::collection_key_for;
use crate::dto::{ExportOptions, ExportReport};
use crate::errors::{wire, AppError, AppResult};
use crate::state::AppState;

#[tauri::command]
pub async fn export_album(
    state: State<'_, AppState>,
    id: i64,
    dest: String,
    options: ExportOptions,
) -> Result<ExportReport, String> {
    wire(export_album_impl(&state, id, dest, options).await)
}

async fn export_album_impl(
    state: &AppState,
    id: i64,
    dest: String,
    options: ExportOptions,
) -> AppResult<ExportReport> {
    let dest_dir = PathBuf::from(dest);
    if !dest_dir.is_dir() {
        std::fs::create_dir_all(&dest_dir)?;
    }

    let (db_handle, cas, master_bytes, user_id, default_cid, default_ck_bytes, unlocked_ck_bytes) = {
        let guard = state.inner.lock().await;
        let s = guard.session.as_ref().ok_or(AppError::Locked)?;
        let override_bytes = s.unlocked_albums.get(&id).map(|ck| *ck.as_bytes());
        (
            s.db.clone(),
            s.cas.clone(),
            *s.user.master_key.as_bytes(),
            s.user.user_id,
            s.default_collection_id,
            *s.default_collection_key.as_bytes(),
            override_bytes,
        )
    };

    tokio::task::spawn_blocking(move || -> AppResult<ExportReport> {
        let guard = db_handle.blocking_lock();
        let override_ck = unlocked_ck_bytes.map(mv_core::crypto::CollectionKey::from_bytes);
        let ck = collection_key_for(
            master_bytes,
            &guard,
            user_id,
            id,
            default_cid,
            default_ck_bytes,
            override_ck.as_ref(),
        )?;
        let members = db::list_collection_member_ids(&guard, id)?;

        let mut report = ExportReport::default();
        for aid in members {
            let asset = match db::get_asset(&guard, aid)? {
                Some(a) => a,
                None => {
                    report.skipped += 1;
                    continue;
                }
            };
            let fk = match unwrap_file_key(&asset.wrapped_file_key, &ck) {
                Ok(k) => k,
                Err(_) => {
                    report.skipped += 1;
                    continue;
                }
            };
            let bytes = match cas.get(&asset.cas_ref, &fk) {
                Ok(b) => b,
                Err(_) => {
                    report.skipped += 1;
                    continue;
                }
            };
            let filename = open_row(&asset.filename_ct, 0, ck.as_bytes())
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_else(|| format!("asset_{}.bin", asset.id));
            let mut out_path = dest_dir.join(&filename);
            let mut i = 1;
            while out_path.exists() {
                let stem = out_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("asset");
                let ext = out_path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("bin");
                out_path = dest_dir.join(format!("{stem}-{i}.{ext}"));
                i += 1;
            }
            std::fs::write(&out_path, &bytes)?;
            report.files_written += 1;
            report.bytes_written += bytes.len() as u64;

            if options.include_xmp {
                let fields = build_xmp_fields(&asset, &ck);
                if write_xmp_sidecar(&out_path, &fields).is_ok() {
                    report.xmp_written += 1;
                }
            }
        }
        Ok(report)
    })
    .await
    .map_err(AppError::from)?
}

fn build_xmp_fields(asset: &db::AssetRow, ck: &mv_core::crypto::CollectionKey) -> XmpFields {
    let mut f = XmpFields::default();
    f.title = open_row(&asset.filename_ct, 0, ck.as_bytes())
        .ok()
        .and_then(|b| String::from_utf8(b).ok());
    f.date_created = asset
        .taken_at_utc_ct
        .as_deref()
        .and_then(|b| open_row(b, 0, ck.as_bytes()).ok())
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|t| t.with_timezone(&chrono::Utc));
    f.gps = asset
        .gps_ct
        .as_deref()
        .and_then(|b| open_row(b, 0, ck.as_bytes()).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| {
            let lat = v.get("lat")?.as_f64()?;
            let lon = v.get("lon")?.as_f64()?;
            Some((lat, lon))
        });
    f
}
