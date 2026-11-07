use crate::{
    chunking::evidence,
    openapi::parse_openapi,
    pdf::{PdfOxideExtractor, parse_pdf},
};
use bg_spec_core::{
    CatalogRepository, Config, Document, DocumentKind, EndpointRef, Error, IndexedDocument,
    IndexedStatus, Manifest, ManifestEntry, Provenance, Requirement, RequirementFile, Result,
    SourceLocator, config, config::corpus_path, sha256, validate_endpoint, validate_text,
};
use bg_spec_store::{Snapshot, SqliteCatalog, TantivySearch};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use walkdir::WalkDir;

// Bump whenever extraction/normalization/index semantics change.
const PIPELINE_VERSION: &str = "bg-spec-pipeline-3";

#[derive(Debug, Serialize)]
pub struct IndexReport {
    pub generation: String,
    pub indexed: usize,
    pub skipped: usize,
    pub ocr_required: Vec<String>,
    pub unlisted_files: Vec<String>,
}

pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest> {
    let text = std::str::from_utf8(bytes).map_err(|e| Error::Invalid(e.to_string()))?;
    let mut manifest: Manifest =
        serde_saphyr::from_str(text).map_err(|e| Error::Invalid(format!("manifest: {e}")))?;
    let mut ids = BTreeSet::new();
    for source in &manifest.sources {
        validate_text(&source.id, "source ID", 256)?;
        validate_text(&source.version.0, "version", 128)?;
        if !source
            .id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        {
            return Err(Error::Invalid(
                "source IDs may contain only ASCII letters, numbers, -, _, .".into(),
            ));
        }
        config::safe_relative(Path::new(&source.path))?;
        if !ids.insert(&source.id) {
            return Err(Error::Invalid(format!("duplicate source ID {}", source.id)));
        }
    }
    manifest.sources.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(manifest)
}

fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    if file.metadata()?.len() > max {
        return Err(Error::Invalid(
            "document exceeds configured byte limit".into(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(Error::Invalid("document grew beyond byte limit".into()));
    }
    Ok(bytes)
}

fn manifest_hash(entry: &ManifestEntry, config: &Config) -> Result<String> {
    Ok(sha256(
        serde_json::to_string(&(
            entry,
            PIPELINE_VERSION,
            config.chunk_chars,
            config.chunk_overlap,
        ))?
        .as_bytes(),
    ))
}

pub async fn verify_sources(config: &Config, catalog: &dyn CatalogRepository) -> Result<()> {
    let manifest_path = corpus_path(&config.corpus_dir, &config.manifest)?;
    let manifest = parse_manifest(&read_bounded(&manifest_path, config.max_document_bytes)?)?;
    let documents: BTreeMap<_, _> = catalog
        .documents()
        .await?
        .into_iter()
        .map(|d| (d.provenance.source_id.clone(), d))
        .collect();
    if manifest.sources.len() != documents.len() {
        return Err(Error::Integrity(
            "manifest source set changed; re-index".into(),
        ));
    }
    for entry in &manifest.sources {
        let document = documents.get(&entry.id).ok_or_else(|| {
            Error::Integrity(format!("new manifest source {}; re-index", entry.id))
        })?;
        let path = corpus_path(&config.corpus_dir, Path::new(&entry.path))?;
        let max = config.max_document_bytes;
        let hash =
            tokio::task::spawn_blocking(move || read_bounded(&path, max).map(|b| sha256(&b)))
                .await
                .map_err(Error::storage)??;
        if hash != document.provenance.sha256
            || manifest_hash(entry, config)? != document.manifest_sha256
        {
            return Err(Error::Integrity(format!(
                "source/manifest/pipeline drift for {}; re-index",
                entry.id
            )));
        }
    }
    Ok(())
}

pub async fn index(config: &Config) -> Result<IndexReport> {
    std::fs::create_dir_all(config.data_dir.join("generations"))?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(config.data_dir.join("index.lock"))?;
    lock.try_lock().map_err(Error::Locked)?;
    let manifest_path = corpus_path(&config.corpus_dir, &config.manifest)?;
    let manifest = parse_manifest(&read_bounded(&manifest_path, config.max_document_bytes)?)?;
    let listed: BTreeSet<_> = manifest
        .sources
        .iter()
        .map(|s| s.path.clone())
        .chain(std::iter::once(
            config.manifest.to_string_lossy().into_owned(),
        ))
        .collect();
    let mut unlisted_files = Vec::new();
    for entry in WalkDir::new(&config.corpus_dir).follow_links(false) {
        let entry = entry.map_err(|e| Error::Io(e.into()))?;
        if entry.file_type().is_file() {
            let relative = entry
                .path()
                .strip_prefix(&config.corpus_dir)
                .map_err(|e| Error::Invalid(e.to_string()))?
                .to_string_lossy()
                .into_owned();
            if !listed.contains(&relative) {
                unlisted_files.push(relative);
            }
        }
    }
    unlisted_files.sort();
    let previous = if config.data_dir.join("CURRENT").exists() {
        Some(Snapshot::open(config).await?)
    } else {
        None
    };
    let old_documents: BTreeMap<_, _> = if let Some(previous) = &previous {
        previous
            .catalog
            .documents()
            .await?
            .into_iter()
            .map(|d| (d.provenance.source_id.clone(), d))
            .collect()
    } else {
        BTreeMap::new()
    };
    let mut documents = Vec::new();
    let mut requirement_files = Vec::new();
    let mut indexed = 0;
    let mut skipped = 0;
    let mut fingerprints = Vec::new();
    for entry in &manifest.sources {
        let path = corpus_path(&config.corpus_dir, Path::new(&entry.path))?;
        let max = config.max_document_bytes;
        let bytes = tokio::task::spawn_blocking(move || read_bounded(&path, max))
            .await
            .map_err(|e| Error::Extraction(e.to_string()))??;
        let hash = sha256(&bytes);
        let manifest_hash = manifest_hash(entry, config)?;
        fingerprints.push(format!("{}:{hash}:{manifest_hash}", entry.id));
        let document = Document {
            provenance: Provenance {
                source_id: entry.id.clone(),
                document_id: sha256(format!("{}\0{hash}\0{manifest_hash}", entry.id).as_bytes()),
                version: entry.version.clone(),
                kind: entry.kind,
                locator: SourceLocator("document".into()),
                sha256: hash.clone(),
            },
            title: entry.title.clone().unwrap_or_else(|| entry.id.clone()),
            authority: entry.authority,
            precedence: entry.precedence,
            path: entry.path.clone(),
            indexed_status: IndexedStatus::Indexed,
            ocr_required_pages: Vec::new(),
            manifest_sha256: manifest_hash.clone(),
        };
        if entry.kind == DocumentKind::Requirements {
            let text = std::str::from_utf8(&bytes).map_err(|e| Error::Invalid(e.to_string()))?;
            let file: RequirementFile = serde_saphyr::from_str(text)
                .map_err(|e| Error::Invalid(format!("requirement mappings: {e}")))?;
            requirement_files.push((document.clone(), file));
        }
        if let (Some(previous), Some(old)) = (&previous, old_documents.get(&entry.id))
            && old.provenance.sha256 == hash
            && old.manifest_sha256 == manifest_hash
        {
            documents.push(previous.catalog.export_document(old.clone()).await?);
            skipped += 1;
            continue;
        }
        let config = config.clone();
        let parsed = tokio::task::spawn_blocking(move || match document.provenance.kind {
            DocumentKind::Openapi => parse_openapi(&document, &bytes, &config),
            DocumentKind::Pdf => parse_pdf(document, &bytes, &PdfOxideExtractor, &config),
            DocumentKind::Requirements => Ok(IndexedDocument {
                document: Some(document),
                ..Default::default()
            }),
        })
        .await
        .map_err(|e| Error::Extraction(e.to_string()))??;
        documents.push(parsed);
        indexed += 1;
    }
    let generation = sha256(fingerprints.join("\n").as_bytes());
    let requirements = resolve_requirements(&requirement_files, &documents)?;
    // Mapping chunks are rebuilt from validated mappings, even when their file was unchanged.
    for document in documents.iter_mut().filter(|d| {
        d.document
            .as_ref()
            .is_some_and(|d| d.provenance.kind == DocumentKind::Requirements)
    }) {
        document.chunks.clear();
        let metadata = document
            .document
            .as_ref()
            .ok_or_else(|| Error::Integrity("missing mapping document".into()))?;
        for requirement in requirements
            .iter()
            .filter(|r| r.provenance.source_id == metadata.provenance.source_id)
        {
            document.chunks.push(evidence(
                metadata,
                &serde_json::to_string(&requirement.mapping)?,
                &requirement.provenance.locator.0,
                None,
                true,
            ));
        }
    }
    let mut report = IndexReport {
        generation: generation.clone(),
        indexed,
        skipped,
        ocr_required: vec![],
        unlisted_files,
    };
    for document in &documents {
        if let Some(document) = &document.document
            && document.indexed_status == IndexedStatus::OcrRequired
        {
            report
                .ocr_required
                .push(document.provenance.source_id.clone());
        }
    }
    let final_path = config.data_dir.join("generations").join(&generation);
    if !final_path.exists() {
        let staging = tempfile::Builder::new()
            .prefix(".staging-")
            .tempdir_in(config.data_dir.join("generations"))?;
        let catalog =
            SqliteCatalog::create(&staging.path().join("catalog.db"), &generation).await?;
        for document in &documents {
            catalog.write_document(document).await?;
        }
        catalog.write_requirements(&requirements).await?;
        catalog.doctor().await?;
        catalog.close().await;
        let chunks: Vec<_> = documents
            .into_iter()
            .flat_map(|d| d.chunks)
            .filter(|c| c.searchable)
            .collect();
        let search_path = staging.path().join("tantivy");
        let id = generation.clone();
        tokio::task::spawn_blocking(move || TantivySearch::build(&search_path, &chunks, &id))
            .await
            .map_err(Error::storage)??;
        sync_tree(staging.path())?;
        std::fs::rename(staging.path(), &final_path)?;
        File::open(config.data_dir.join("generations"))?.sync_all()?;
    }
    let catalog = SqliteCatalog::open_read_only(&final_path.join("catalog.db")).await?;
    if catalog.generation().await? != generation {
        return Err(Error::Integrity("existing generation ID mismatch".into()));
    }
    let search = TantivySearch::open(&final_path.join("tantivy"), &generation)?;
    Snapshot {
        catalog: std::sync::Arc::new(catalog),
        search: std::sync::Arc::new(search),
    }
    .doctor()
    .await?;
    let mut pointer = tempfile::NamedTempFile::new_in(&config.data_dir)?;
    writeln!(pointer, "{generation}")?;
    pointer.as_file().sync_all()?;
    pointer
        .persist(config.data_dir.join("CURRENT"))
        .map_err(|e| Error::Io(e.error))?;
    File::open(&config.data_dir)?.sync_all()?;
    Ok(report)
}

fn sync_tree(path: &Path) -> Result<()> {
    for entry in WalkDir::new(path).contents_first(true) {
        let entry = entry.map_err(|e| Error::Io(e.into()))?;
        File::open(entry.path())?.sync_all()?;
    }
    Ok(())
}

fn resolve_requirements(
    files: &[(Document, RequirementFile)],
    documents: &[IndexedDocument],
) -> Result<Vec<Requirement>> {
    let mut requirements = Vec::new();
    let mut ids = BTreeSet::new();
    for (document, file) in files {
        for mapping in &file.requirements {
            validate_text(&mapping.id, "requirement ID", 256)?;
            validate_text(&mapping.title, "requirement title", 1024)?;
            if !ids.insert(mapping.id.clone()) {
                return Err(Error::Invalid(format!(
                    "duplicate requirement {}",
                    mapping.id
                )));
            }
            if mapping.version != document.provenance.version
                || mapping.sources.is_empty()
                || mapping.acceptance_criteria.is_empty()
            {
                return Err(Error::Invalid(format!(
                    "requirement {} needs matching version, sources and acceptance criteria",
                    mapping.id
                )));
            }
            for criterion in &mapping.acceptance_criteria {
                validate_text(criterion, "acceptance criterion", 16_384)?;
            }
            let mut mapping = mapping.clone();
            let infer_endpoints = mapping.endpoints.is_empty();
            let mut resolved_sources = Vec::new();
            let mut unique_sources = BTreeSet::new();
            for source in &mapping.sources {
                if !unique_sources.insert((&source.source_id, &source.locator.0)) {
                    return Err(Error::Invalid(
                        "duplicate requirement source locator".into(),
                    ));
                }
                let indexed = documents
                    .iter()
                    .find(|d| {
                        d.document
                            .as_ref()
                            .is_some_and(|d| d.provenance.source_id == source.source_id)
                    })
                    .ok_or_else(|| {
                        Error::Invalid(format!("unknown requirement source {}", source.source_id))
                    })?;
                let chunks: Vec<_> = indexed
                    .chunks
                    .iter()
                    .filter(|c| c.provenance.locator == source.locator)
                    .collect();
                if chunks.is_empty() {
                    return Err(Error::Invalid(format!(
                        "unresolved citation {} {} (possibly ocr_required)",
                        source.source_id, source.locator.0
                    )));
                }
                for chunk in chunks {
                    resolved_sources.push(chunk.provenance.clone());
                }
                for operation in &indexed.operations {
                    if infer_endpoints
                        && operation.provenance.version == mapping.version
                        && (source.locator == operation.provenance.locator
                            || source.locator.0 == format!("path:{}", operation.path))
                    {
                        let endpoint = EndpointRef {
                            path: operation.path.clone(),
                            method: operation.method.clone(),
                        };
                        if !mapping.endpoints.contains(&endpoint) {
                            mapping.endpoints.push(endpoint);
                        }
                    }
                }
            }
            for endpoint in &mut mapping.endpoints {
                endpoint.method = validate_endpoint(&endpoint.path, &endpoint.method)?;
                if !documents.iter().flat_map(|d| &d.operations).any(|o| {
                    o.provenance.version == mapping.version
                        && o.path == endpoint.path
                        && o.method == endpoint.method
                }) {
                    return Err(Error::Invalid(format!(
                        "requirement {} has unresolved endpoint",
                        mapping.id
                    )));
                }
            }
            mapping
                .endpoints
                .sort_by(|a, b| a.path.cmp(&b.path).then(a.method.cmp(&b.method)));
            mapping.endpoints.dedup();
            let mut provenance = document.provenance.clone();
            provenance.locator = SourceLocator(format!("requirement:{}", mapping.id));
            provenance.sha256 = sha256(serde_json::to_string(&mapping)?.as_bytes());
            requirements.push(Requirement {
                mapping,
                provenance,
                resolved_sources,
            });
        }
    }
    for requirement in &requirements {
        for id in &requirement.mapping.conflicts_with {
            if id == &requirement.mapping.id || !ids.contains(id) {
                return Err(Error::Invalid(format!(
                    "invalid conflicts_with reference {id}"
                )));
            }
        }
    }
    requirements.sort_by(|a, b| a.mapping.id.cmp(&b.mapping.id));
    Ok(requirements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bg_spec_core::DocumentAuthority;
    #[test]
    fn manifest_validation_is_explicit() {
        let valid = b"sources:\n  - id: guidelines\n    kind: pdf\n    version: v2\n    authority: normative\n    precedence: 100\n    path: pdf/guidelines.pdf\n";
        assert_eq!(
            parse_manifest(valid).unwrap().sources[0].authority,
            DocumentAuthority::Normative
        );
        assert!(parse_manifest(&[valid.as_slice(), valid.as_slice()].concat()).is_err());
        assert!(
            parse_manifest(
                String::from_utf8_lossy(valid)
                    .replace("pdf/guidelines.pdf", "../secret")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            parse_manifest(
                String::from_utf8_lossy(valid)
                    .replace("normative", "guessed")
                    .as_bytes()
            )
            .is_err()
        );
    }
}
