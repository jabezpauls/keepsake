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
        .invoke_handler(tauri::generate_handler![
            commands::auth::user_exists,
            commands::auth::create_user,
            commands::auth::unlock,
            commands::auth::lock,
            commands::auth::unlock_hidden,
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
            commands::search::search_assets,
            commands::map::map_points,
            commands::nearp::near_dup_rebuild,
            commands::nearp::near_dup_list,
            commands::ml::ml_status,
            commands::ml::ml_models_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
