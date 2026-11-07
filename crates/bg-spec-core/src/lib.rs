pub mod comparison;
pub mod config;
pub mod model;
pub mod repository;
pub mod service;

pub use config::Config;
pub use model::*;
pub use repository::*;
pub use service::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("ambiguous source: {0}")]
    Ambiguous(String),
    #[error("index integrity failure: {0}")]
    Integrity(String),
    /// The underlying store/search/task failure is kept as the error source.
    #[error("storage operation failed")]
    Storage(#[source] BoxError),
    #[error("indexer lock is held by another process")]
    Locked(#[source] std::fs::TryLockError),
    #[error("document extraction failed: {0}")]
    Extraction(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

impl Error {
    /// Wraps a storage-layer failure while preserving it as [`std::error::Error::source`].
    pub fn storage(error: impl Into<BoxError>) -> Self {
        Self::Storage(error.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[must_use]
pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}
