//! Media Vault Tauri shell.
//!
//! Responsibilities:
//!
//! 1. Register IPC commands defined in `commands/`.
//! 2. Own the `AppState` (session + key material) for the lifetime of the window.
//! 3. Initialise tracing so mv-core logs flow to stderr.

pub mod commands;
pub mod dto;
pub mod errors;
pub mod state;

use state::{default_vault_root, AppState};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let app_state = AppState::new(default_vault_root());

    tauri::Builder::default()
        .manage(app_state)
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // Point ORT at the libonnxruntime shipped inside the app bundle.
            // Done in the setup hook (rather than `main`) because Tauri's
            // path resolver knows the canonical resource directory per
            // bundle format — `Contents/Resources/` on macOS, alongside the
            // exe on Windows, `/usr/lib/<identifier>/` on Linux DEB, the
            // mounted root on AppImage. A manual override via
            // `$ORT_DYLIB_PATH` always wins.
            if std::env::var_os("ORT_DYLIB_PATH").is_none() {
                if let Ok(resource_dir) = app.path().resource_dir() {
                    // Array-form `bundle.resources` preserves the source-tree
                    // path inside the bundle, so a workspace file at
                    // `app/src-tauri/resources/libonnxruntime.so` lands at
                    // `<resource_dir>/resources/libonnxruntime.so`.
                    let candidates: &[&str] = &[
                        "resources/libonnxruntime.so",
                        "resources/libonnxruntime.dylib",
                        "resources/onnxruntime.dll",
                        // Some Tauri layouts (e.g. installer .exe variants)
                        // flatten the `resources/` segment.
                        "libonnxruntime.so",
                        "libonnxruntime.dylib",
                        "onnxruntime.dll",
                    ];
                    for name in candidates {
                        let p = resource_dir.join(name);
                        if p.is_file() {
                            std::env::set_var("ORT_DYLIB_PATH", &p);
                            tracing::info!(path = %p.display(), "bundled ORT dylib selected");
                            break;
                        }
                    }
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::user_exists,
            commands::auth::create_user,
            commands::auth::unlock,
            commands::auth::lock,
            commands::auth::unlock_hidden,
            commands::auth::list_users,
            commands::auth::list_local_peers,
            commands::sources::add_source,
            commands::sources::list_sources,
            commands::sources::ingest_status,
            commands::timeline::timeline_page,
            commands::timeline::asset_detail,
            commands::timeline::asset_thumbnail,
            commands::timeline::asset_original,
            commands::albums::create_album,
            commands::albums::list_albums,
            commands::albums::unlock_album,
            commands::albums::album_page,
            commands::albums::add_to_album,
            commands::export::export_album,
            commands::people::list_people,
            commands::people::rename_person,
            commands::people::hide_person,
            commands::people::merge_people,
            commands::people::split_person,
            commands::people::person_face_thumbnail,
            commands::people::asset_faces,
            commands::search::search_assets,
            commands::map::map_points,
            commands::nearp::near_dup_rebuild,
            commands::nearp::near_dup_list,
            commands::ml::ml_status,
            commands::ml::ml_models_enabled,
            commands::ml::ml_bundle_options,
            commands::ml::ml_bundle_selected,
            commands::ml::ml_models_status,
            commands::ml::ml_models_download,
            commands::ml::ml_runtime_reload,
            commands::ml::ml_reindex,
            commands::peer::peer_my_ticket,
            commands::peer::peer_accept_ticket,
            commands::peer::peer_list,
            commands::peer::peer_forget,
            commands::share::share_album_with_peer,
            commands::share::list_album_shares,
            commands::share::revoke_album_share,
            commands::share::list_incoming_shares,
            commands::share::accept_incoming_share,
            commands::share::create_public_link,
            commands::share::list_public_links,
            commands::share::revoke_public_link,
            commands::share::export_public_link_bundle,
            commands::pets::set_asset_pet,
            commands::pets::list_pet_assets,
            commands::analytics::detect_trips_run,
            commands::analytics::list_trips,
            commands::analytics::memories_on_this_day,
            commands::analytics::memories_year_in_photos,
            commands::analytics::memories_person_year,
            commands::analytics::create_smart_album,
            commands::analytics::list_smart_albums,
            commands::analytics::refresh_smart_album,
            commands::analytics::delete_smart_album,
            commands::analytics::smart_album_page,
            commands::text_search::index_asset_text,
            commands::text_search::search_text_exact,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
