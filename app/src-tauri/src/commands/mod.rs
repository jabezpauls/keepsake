//! Tauri IPC surface.
//!
//! Commands are split by area; each module exposes free functions that become
//! the `#[tauri::command]`s registered in `lib.rs`. Returning `Result<_, String>`
//! is the convention so TS sees plain-string errors (sensitive detail is
//! stripped in `errors::AppError::from(...)`).

pub mod albums;
pub mod auth;
pub mod export;
pub mod map;
pub mod ml;
pub mod nearp;
pub mod peer;
pub mod people;
pub mod search;
pub mod sources;
pub mod timeline;
