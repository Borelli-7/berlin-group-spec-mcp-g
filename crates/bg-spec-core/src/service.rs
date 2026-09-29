use crate::{comparison::structural_diff, *};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub fn validate_text(value: &str, name: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(Error::Invalid(format!(
            "{name} must be nonempty, control-free, and at most {max} bytes"
        )));
    }
    Ok(())
}

pub fn validate_endpoint(path: &str, method: &str) -> Result<String> {
    validate_text(path, "path", 2048)?;
    let method = method.to_ascii_uppercase();
    if !path.starts_with('/')
        || ![
            "GET", "PUT", "POST", "DELETE", "OPTIONS", "HEAD", "PATCH", "TRACE",
        ]
        .contains(&method.as_str())
    {
        return Err(Error::Invalid(
            "expected an OpenAPI path and HTTP method".into(),
        ));
    }
    Ok(method)
}

#[derive(Clone)]
pub struct SpecificationService {
    pub catalog: Arc<dyn CatalogRepository>,
    pub search: Arc<dyn SearchRepository>,
}

impl SpecificationService {
    pub async fn search_specification(
        &self,
        query: &str,
        filter: SearchFilter,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        validate_text(query, "query", 2048)?;
        if !(1..=100).contains(&limit) {
            return Err(Error::Invalid("limit must be 1..100".into()));
        }
        if let Some(version) = &filter.version {
            validate_text(version, "version", 128)?;
        }
        if let Some(source) = &filter.source {
            validate_text(source, "source", 256)?;
        }
        let hits = self.search.search(query, filter, limit).await?;
        let mut results = Vec::with_capacity(hits.len());
        for (id, relevance) in hits {
            results.push(SearchResult {
                evidence: self.catalog.chunk(&id).await?,
                relevance,
                classification: "discovered evidence; not an authoritative requirement".into(),
            });
        }
        Ok(results)
    }

    pub async fn read_source(&self, source: &str, locator: &str) -> Result<Value> {
        validate_text(source, "source_id", 256)?;
        validate_text(locator, "locator", 4096)?;
        let document = self.catalog.document(source).await?;
        let content = self.catalog.chunks(source, locator).await?;
        if content.is_empty() {
            if let Some(page) = locator
                .strip_prefix("page:")
                .and_then(|n| n.parse::<u32>().ok())
                && document.ocr_required_pages.contains(&page)
            {
                return Err(Error::Extraction(format!(
                    "{source} {locator}: ocr_required"
                )));
            }
            return Err(Error::NotFound(format!(
                "locator {locator} in source {source}"
            )));
        }
        Ok(
            json!({"source":document,"exact_location":locator,"content":content,"content_semantics":"indexed extracted text / normalized JSON, not original PDF bytes"}),
        )
    }

    pub async fn endpoint(
        &self,
        version: &str,
        path: &str,
        method: &str,
    ) -> Result<OpenApiOperation> {
        validate_text(version, "version", 128)?;
        let method = validate_endpoint(path, method)?;
        let mut operations = self.catalog.operations(version, path, &method).await?;
        match operations.len() {
            0 => Err(Error::NotFound(format!("{version} {method} {path}"))),
            1 => Ok(operations.remove(0)),
            _ => Err(Error::Ambiguous(format!(
                "{version} {method} {path}; use get_endpoint_requirements to inspect all sources"
            ))),
        }
    }

    pub async fn schema(&self, version: &str, name: &str) -> Result<OpenApiSchema> {
        validate_text(version, "version", 128)?;
        validate_text(name, "schema name", 512)?;
        let mut schemas = self.catalog.schemas(version, name).await?;
        match schemas.len() {
            0 => Err(Error::NotFound(format!("schema {version} {name}"))),
            1 => Ok(schemas.remove(0)),
            _ => Err(Error::Ambiguous(format!("schema {version} {name}"))),
        }
    }

    pub async fn referenced_schemas(
        &self,
        operation: &OpenApiOperation,
    ) -> Result<Vec<OpenApiSchema>> {
        let mut pending = operation.referenced_schemas.clone();
        let mut visited = BTreeSet::new();
        let mut result = Vec::new();
        while let Some(name) = pending.pop() {
            if !visited.insert(name.clone()) {
                continue;
            }
            if visited.len() > 10_000 {
                return Err(Error::Invalid("schema graph exceeds 10000 nodes".into()));
            }
            let schema = self
                .catalog
                .schema(&operation.provenance.source_id, &name)
                .await?;
            pending.extend(schema.referenced_schemas.iter().cloned());
            result.push(schema);
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }
}

#[derive(Clone)]
pub struct RequirementService {
    pub specifications: SpecificationService,
}

impl RequirementService {
    pub async fn find_requirement(
        &self,
        query: &str,
        version: Option<String>,
        limit: usize,
    ) -> Result<Value> {
        let evidence = self
            .specifications
            .search_specification(
                query,
                SearchFilter {
                    version: version.clone(),
                    ..Default::default()
                },
                limit,
            )
            .await?;
        let query = query.to_lowercase();
        let requirements = self
            .specifications
            .catalog
            .requirements(version.as_deref())
            .await?;
        let mut curated = Vec::new();
        for requirement in requirements {
            let text = format!(
                "{} {} {}",
                requirement.mapping.id,
                requirement.mapping.title,
                requirement.mapping.acceptance_criteria.join(" ")
            )
            .to_lowercase();
            if query.split_whitespace().all(|word| text.contains(word)) {
                curated.push(requirement);
            }
        }
        let total_curated_matches = curated.len();
        curated.truncate(limit);
        Ok(
            json!({"curated_requirements":curated,"total_curated_matches":total_curated_matches,"discovered_evidence":evidence}),
        )
    }

    pub async fn trace_requirement(&self, id: &str) -> Result<RequirementTrace> {
        validate_text(id, "requirement_id", 256)?;
        let requirement = self.specifications.catalog.requirement(id).await?;
        let mut endpoints = Vec::new();
        let mut schemas = BTreeMap::new();
        let mut evidence = BTreeMap::new();
        for endpoint in &requirement.mapping.endpoints {
            let operations = self
                .specifications
                .catalog
                .operations(
                    &requirement.mapping.version.0,
                    &endpoint.path,
                    &endpoint.method,
                )
                .await?;
            if operations.is_empty() {
                return Err(Error::Integrity(format!(
                    "missing curated endpoint for {id}"
                )));
            }
            for operation in operations {
                for schema in self.specifications.referenced_schemas(&operation).await? {
                    schemas.insert(
                        (schema.provenance.source_id.clone(), schema.name.clone()),
                        schema,
                    );
                }
                endpoints.push(operation);
            }
        }
        for source in &requirement.resolved_sources {
            for chunk in self
                .specifications
                .catalog
                .chunks(&source.source_id, &source.locator.0)
                .await?
            {
                evidence.insert(chunk.chunk_id.clone(), chunk);
            }
        }
        Ok(RequirementTrace {
            specification_sources: requirement.resolved_sources.clone(),
            acceptance_criteria: requirement.mapping.acceptance_criteria.clone(),
            requirement,
            endpoints,
            schemas: schemas.into_values().collect(),
            related_evidence: evidence.into_values().collect(),
        })
    }

    pub async fn get_endpoint_requirements(
        &self,
        version: &str,
        path: &str,
        method: &str,
    ) -> Result<EvidenceBundle> {
        validate_text(version, "version", 128)?;
        let method = validate_endpoint(path, method)?;
        let openapi = self
            .specifications
            .catalog
            .operations(version, path, &method)
            .await?;
        if openapi.is_empty() {
            return Err(Error::NotFound(format!("{version} {method} {path}")));
        }
        let mut schemas = BTreeMap::new();
        for operation in &openapi {
            for schema in self.specifications.referenced_schemas(operation).await? {
                schemas.insert(
                    (schema.provenance.source_id.clone(), schema.name.clone()),
                    schema,
                );
            }
        }
        let all_requirements = self.specifications.catalog.requirements(None).await?;
        let requirements: Vec<_> = all_requirements
            .iter()
            .filter(|r| {
                r.mapping.version.0 == version
                    && r.mapping
                        .endpoints
                        .iter()
                        .any(|e| e.path == path && e.method == method)
            })
            .cloned()
            .collect();
        let terms = [method.as_str(), path]
            .into_iter()
            .chain(openapi.iter().flat_map(|o| {
                [
                    o.operation_id.as_deref().unwrap_or(""),
                    o.summary.as_deref().unwrap_or(""),
                ]
            }));
        let mut query = String::new();
        for word in terms
            .flat_map(|text| text.split(|c: char| !c.is_alphanumeric()))
            .filter(|word| !word.is_empty() && word.len() <= 64)
            .take(64)
        {
            if query.len() + word.len() + 1 > 1024 {
                break;
            }
            if !query.is_empty() {
                query.push(' ');
            }
            query.push_str(word);
        }
        let mut related = self
            .specifications
            .search_specification(
                &query,
                SearchFilter {
                    version: Some(version.into()),
                    kind: Some(DocumentKind::Pdf),
                    source: None,
                },
                20,
            )
            .await?;
        // Exact curated citations are included even when BM25 does not retrieve them.
        for requirement in &requirements {
            for source in &requirement.resolved_sources {
                for chunk in self
                    .specifications
                    .catalog
                    .chunks(&source.source_id, &source.locator.0)
                    .await?
                {
                    if chunk.provenance.kind == DocumentKind::Pdf
                        && !related
                            .iter()
                            .any(|r| r.evidence.chunk_id == chunk.chunk_id)
                    {
                        related.push(SearchResult {
                            evidence: chunk,
                            relevance: 0.0,
                            classification:
                                "curated source citation; relevance score not applicable".into(),
                        });
                    }
                }
            }
        }
        let mut conflicts = Vec::new();
        for (index, first) in openapi.iter().enumerate() {
            for second in &openapi[index + 1..] {
                if first.contract != second.contract {
                    conflicts.push(Conflict {
                        kind: "divergent_openapi_contracts".into(),
                        description: "Different contracts for the same version/path/method. Authority and precedence are metadata, not automatic resolution.".into(),
                        sources: vec![first.provenance.clone(), second.provenance.clone()],
                    });
                }
            }
        }
        let schema_values: Vec<_> = schemas.values().collect();
        for (index, first) in schema_values.iter().enumerate() {
            for second in &schema_values[index + 1..] {
                if first.name == second.name && first.schema != second.schema {
                    conflicts.push(Conflict {
                        kind: "divergent_openapi_schemas".into(),
                        description: format!(
                            "Referenced schema {} differs between sources for this endpoint.",
                            first.name
                        ),
                        sources: vec![first.provenance.clone(), second.provenance.clone()],
                    });
                }
            }
        }
        let mut pairs = BTreeSet::new();
        let relevant_ids: BTreeSet<_> =
            requirements.iter().map(|r| r.mapping.id.as_str()).collect();
        for requirement in &all_requirements {
            for other_id in &requirement.mapping.conflicts_with {
                if !relevant_ids.contains(requirement.mapping.id.as_str())
                    && !relevant_ids.contains(other_id.as_str())
                {
                    continue;
                }
                let other = self.specifications.catalog.requirement(other_id).await?;
                let mut pair = [requirement.mapping.id.clone(), other_id.clone()];
                pair.sort();
                if pairs.insert(pair) {
                    conflicts.push(Conflict {
                        kind: "curated_conflict".into(),
                        description: format!(
                            "{} conflicts with {other_id}, as declared by the curator",
                            requirement.mapping.id
                        ),
                        sources: [
                            vec![requirement.provenance.clone(), other.provenance.clone()],
                            requirement.resolved_sources.clone(),
                            other.resolved_sources,
                        ]
                        .concat(),
                    });
                }
            }
        }
        let mut source_ids = BTreeSet::new();
        for operation in &openapi {
            source_ids.insert(operation.provenance.source_id.clone());
        }
        for requirement in &requirements {
            source_ids.insert(requirement.provenance.source_id.clone());
            source_ids.extend(
                requirement
                    .resolved_sources
                    .iter()
                    .map(|s| s.source_id.clone()),
            );
        }
        for hit in &related {
            source_ids.insert(hit.evidence.provenance.source_id.clone());
        }
        for conflict in &conflicts {
            source_ids.extend(conflict.sources.iter().map(|s| s.source_id.clone()));
        }
        let mut sources = Vec::new();
        for id in source_ids {
            sources.push(self.specifications.catalog.document(&id).await?);
        }
        Ok(EvidenceBundle {
            endpoint: EndpointRef { path: path.into(), method },
            openapi, requirements, schemas: schemas.into_values().collect(),
            related_evidence: related, related_evidence_query: query, conflicts, sources,
            conflict_detection: "Only divergent OpenAPI contracts/schemas and explicit curated conflicts are detected. Prose contradictions are not inferred; an empty conflicts array is not proof of consistency.".into(),
        })
    }
}

#[derive(Clone)]
pub struct CompatibilityService {
    pub specifications: SpecificationService,
    pub v1: String,
    pub v2: String,
}

impl CompatibilityService {
    pub async fn compare(
        &self,
        path: &str,
        method: &str,
        version1: Option<&str>,
        version2: Option<&str>,
    ) -> Result<Comparison> {
        let version1 = version1.unwrap_or(&self.v1);
        let version2 = version2.unwrap_or(&self.v2);
        if version1 == version2 {
            return Err(Error::Invalid("comparison versions must differ".into()));
        }
        let documents = self.specifications.catalog.documents().await?;
        for version in [version1, version2] {
            validate_text(version, "comparison version", 128)?;
            if !documents.iter().any(|d| {
                d.provenance.kind == DocumentKind::Openapi && d.provenance.version.0 == version
            }) {
                return Err(Error::NotFound(format!(
                    "OpenAPI corpus for version {version}"
                )));
            }
        }
        async fn optional(
            service: &SpecificationService,
            version: &str,
            path: &str,
            method: &str,
        ) -> Result<Option<OpenApiOperation>> {
            match service.endpoint(version, path, method).await {
                Ok(operation) => Ok(Some(operation)),
                Err(Error::NotFound(_)) => Ok(None),
                Err(error) => Err(error),
            }
        }
        let before = optional(&self.specifications, version1, path, method).await?;
        let after = optional(&self.specifications, version2, path, method).await?;
        if before.is_none() && after.is_none() {
            return Err(Error::NotFound("endpoint absent in both versions".into()));
        }
        let mut sources: Vec<_> = before
            .iter()
            .chain(after.iter())
            .map(|o| o.provenance.clone())
            .collect();
        for (version, operation) in [(version1, &before), (version2, &after)] {
            if operation.is_none() {
                sources.extend(
                    documents
                        .iter()
                        .filter(|d| {
                            d.provenance.kind == DocumentKind::Openapi
                                && d.provenance.version.0 == version
                        })
                        .map(|d| d.provenance.clone()),
                );
            }
        }
        let changes = structural_diff(
            before.as_ref().map(|o| &o.contract),
            after.as_ref().map(|o| &o.contract),
            "operation",
            &sources,
        );
        let mut old = BTreeMap::new();
        let mut new = BTreeMap::new();
        for (operation, schemas) in [(before.as_ref(), &mut old), (after.as_ref(), &mut new)] {
            if let Some(operation) = operation {
                for schema in self.specifications.referenced_schemas(operation).await? {
                    sources.push(schema.provenance.clone());
                    schemas.insert(schema.name.clone(), schema);
                }
            }
        }
        let mut schema_changes = Vec::new();
        for name in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
            let citations: Vec<_> = old
                .get(name)
                .into_iter()
                .chain(new.get(name))
                .map(|s| s.provenance.clone())
                .collect();
            schema_changes.extend(structural_diff(
                old.get(name).map(|s| &s.schema),
                new.get(name).map(|s| &s.schema),
                &format!("schemas/{}", comparison::pointer_escape(name)),
                &citations,
            ));
        }
        Ok(Comparison {
            v1_operation: before, v2_operation: after, changes, schema_changes, sources,
            compatibility_relevant_facts: "Structural facts only. No overall backward-compatibility verdict; array order, descriptions, and schema-name changes remain observable.".into(),
        })
    }
}
