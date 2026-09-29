use crate::{Error, Result};
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub corpus_dir: PathBuf,
    pub data_dir: PathBuf,
    #[serde(default = "manifest")]
    pub manifest: PathBuf,
    #[serde(default = "max_chars")]
    pub chunk_chars: usize,
    #[serde(default = "overlap")]
    pub chunk_overlap: usize,
    #[serde(default = "max_bytes")]
    pub max_document_bytes: u64,
    #[serde(default = "v1")]
    pub v1: String,
    #[serde(default = "v2")]
    pub v2: String,
}

fn manifest() -> PathBuf {
    "manifest.yaml".into()
}
fn max_chars() -> usize {
    6000
}
fn overlap() -> usize {
    500
}
fn max_bytes() -> u64 {
    128 * 1024 * 1024
}
fn v1() -> String {
    "nextgenpsd2-v1".into()
}
fn v2() -> String {
    "openfinance-v2".into()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let path = path.canonicalize()?;
        let base = path
            .parent()
            .ok_or_else(|| Error::Invalid("config has no parent".into()))?;
        let mut config: Self = toml::from_str(&std::fs::read_to_string(&path)?)
            .map_err(|e| Error::Invalid(format!("configuration: {e}")))?;
        config.corpus_dir = base.join(&config.corpus_dir).canonicalize()?;
        config.data_dir = base.join(&config.data_dir);
        if !(256..=100_000).contains(&config.chunk_chars)
            || config.chunk_overlap >= config.chunk_chars
            || config.max_document_bytes == 0
            || config.v1.trim().is_empty()
            || config.v2.trim().is_empty()
            || config.v1 == config.v2
        {
            return Err(Error::Invalid(
                "invalid chunk, size, or version configuration".into(),
            ));
        }
        safe_relative(&config.manifest)?;
        Ok(config)
    }
}

pub fn safe_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Error::Invalid("expected a confined relative path".into()));
    }
    Ok(())
}

pub fn corpus_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    safe_relative(relative)?;
    let root = root.canonicalize()?;
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(Error::Invalid(
            "source escapes corpus or is not a file".into(),
        ));
    }
    Ok(path)
}
