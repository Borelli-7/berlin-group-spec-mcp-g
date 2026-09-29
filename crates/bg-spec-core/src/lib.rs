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
    #[error("storage error: {0}")]
    Storage(String),
    #[error("document extraction failed: {0}")]
    Extraction(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
