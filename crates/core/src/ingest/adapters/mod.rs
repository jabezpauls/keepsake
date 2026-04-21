//! Device-specific ingest adapters.
//!
//! All three share the heavy lifting in `generic.rs`; iPhone and Takeout
//! layer pre-processing on top of it.

pub mod generic;
pub mod google_takeout;
pub mod iphone_folder;

pub use google_takeout::GoogleTakeoutAdapter;
pub use iphone_folder::IPhoneFolderAdapter;
