use crate::{SqliteCatalog, TantivySearch};
use bg_spec_core::*;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub fn active_snapshot(data: &Path) -> Result<PathBuf> {
    let id = std::fs::read_to_string(data.join("CURRENT"))?;
    let id = id.trim();
    if id.len() != 64 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Integrity(
            "invalid active generation identifier".into(),
        ));
    }
    let root = data.join("generations").canonicalize()?;
    let path = root.join(id).canonicalize()?;
    if !path.starts_with(root) {
        return Err(Error::Integrity("generation escapes data directory".into()));
    }
    Ok(path)
}

pub struct Snapshot {
    pub catalog: Arc<SqliteCatalog>,
    pub search: Arc<TantivySearch>,
}

impl Snapshot {
    pub async fn open(config: &Config) -> Result<Self> {
        let path = active_snapshot(&config.data_dir)?;
        let catalog = Arc::new(SqliteCatalog::open_read_only(&path.join("catalog.db")).await?);
        let generation = catalog.generation().await?;
        if path.file_name().and_then(|n| n.to_str()) != Some(&generation) {
            return Err(Error::Integrity("catalog generation mismatch".into()));
        }
        let search = Arc::new(TantivySearch::open(&path.join("tantivy"), &generation)?);
        let health = catalog.health().await?;
        if health["counts"]["chunks"].as_u64() != Some(search.count()) {
            return Err(Error::Integrity(
                "search/catalog chunk count mismatch".into(),
            ));
        }
        Ok(Self { catalog, search })
    }

    pub fn specifications(&self) -> SpecificationService {
        SpecificationService {
            catalog: self.catalog.clone(),
            search: self.search.clone(),
        }
    }

    pub async fn doctor(&self) -> Result<serde_json::Value> {
        let health = self.catalog.doctor().await?;
        let expected: Vec<_> = self
            .catalog
            .all_searchable_chunks()
            .await?
            .into_iter()
            .map(|c| c.chunk_id)
            .collect();
        let search = self.search.clone();
        let actual = tokio::task::spawn_blocking(move || search.chunk_ids())
            .await
            .map_err(|e| Error::Storage(e.to_string()))??;
        if actual != expected {
            return Err(Error::Integrity(
                "search/catalog chunk identities differ".into(),
            ));
        }
        Ok(health)
    }
}
