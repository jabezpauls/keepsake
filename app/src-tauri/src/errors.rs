//! Error → wire-string mapping for Tauri commands.
//!
//! Every public command returns `Result<T, String>` so error text reaches
//! the TS side as plain data. We deliberately scrub anything that could
//! reveal internals — the crypto layer already produces opaque variants.

use mv_core::Error as CoreError;

/// Wire-safe error type. `Display`s to a short, user-safe string.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("locked")]
    Locked,
    #[error("not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("already exists")]
    AlreadyExists,
    #[error("crypto")]
    Crypto,
    #[error("io: {0}")]
    Io(String),
    #[error("db: {0}")]
    Db(String),
    #[error("ingest: {0}")]
    Ingest(String),
    #[error("internal")]
    Internal,
}

impl From<CoreError> for AppError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::Crypto | CoreError::BlobFormat | CoreError::KeyOrData => Self::Crypto,
            CoreError::Io(err) => Self::Io(err.kind().to_string()),
            CoreError::Db(err) => Self::Db(err.to_string()),
            CoreError::Media(m) => Self::Ingest(m),
            CoreError::Ingest(m) => Self::Ingest(m),
            CoreError::NotFound => Self::NotFound,
            CoreError::Locked => Self::Locked,
            CoreError::RateLimited => Self::BadRequest("rate limited".into()),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.kind().to_string())
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(_: tokio::task::JoinError) -> Self {
        Self::Internal
    }
}

pub type AppResult<T> = std::result::Result<T, AppError>;

/// Convenience: convert an AppResult<T> to the `Result<T, String>` shape Tauri expects.
pub fn wire<T>(r: AppResult<T>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}
