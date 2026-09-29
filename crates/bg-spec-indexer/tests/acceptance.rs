mod support;

use bg_spec_core::*;
use bg_spec_indexer::{
    index,
    openapi::parse_openapi,
    pdf::{PdfExtractor, PdfOxideExtractor, parse_pdf},
};
use bg_spec_store::Snapshot;
use serde_json::json;
use std::{fs, path::Path};

const ENDPOINT: &str = "/accounts/{accountId}/transactions";
const REQUIREMENT: &str = "FIXTURE-OFV2-TRANSACTIONS-001";

#[tokio::test]
async fn full_acceptance_scenario_and_incremental_reuse() {
    let (_temp, config) = support::fixture();
    let first = index(&config).await.unwrap();
    assert_eq!(first.indexed, 5);
    assert!(first.ocr_required.is_empty());
    let snapshot = Snapshot::open(&config).await.unwrap();
    snapshot.catalog.assert_read_only().await.unwrap();
    let specifications = snapshot.specifications();
    let requirements = RequirementService {
        specifications: specifications.clone(),
    };
    let bundle = requirements
        .get_endpoint_requirements(&config.v2, ENDPOINT, "GET")
        .await
        .unwrap();
    assert_eq!(bundle.openapi.len(), 1);
    assert_eq!(bundle.requirements[0].mapping.id, REQUIREMENT);
    assert_eq!(bundle.schemas.len(), 2);
    assert!(
        bundle
            .related_evidence
            .iter()
            .any(|h| h.evidence.text.contains("MUST include transactionId"))
    );
    assert!(bundle.conflicts.is_empty());
    assert_eq!(bundle.sources.len(), 3);
    let endpoint = specifications
        .endpoint(&config.v2, ENDPOINT, "get")
        .await
        .unwrap();
    assert_eq!(endpoint.parameters.as_array().unwrap().len(), 2);
    assert_eq!(endpoint.parameters[0]["name"], "accountId");
    assert_eq!(endpoint.parameters[0]["required"], true);
    assert_eq!(endpoint.security, json!([{"bearerAuth":[]}]));
    assert_eq!(
        endpoint.contract["securitySchemes"]["bearerAuth"]["scheme"],
        "bearer"
    );
    assert_eq!(
        endpoint.responses["200"]["content"]["application/json"]["schema"]["items"]["$ref"],
        "#/components/schemas/Transaction"
    );
    let trace = requirements.trace_requirement(REQUIREMENT).await.unwrap();
    assert_eq!(trace.endpoints.len(), 1);
    assert_eq!(trace.schemas.len(), 2);
    assert_eq!(trace.acceptance_criteria.len(), 2);
    for source in &trace.specification_sources {
        let read = specifications
            .read_source(&source.source_id, &source.locator.0)
            .await
            .unwrap();
        assert_eq!(read["content"][0]["sha256"], source.sha256);
    }
    let comparison = CompatibilityService {
        specifications: specifications.clone(),
        v1: config.v1.clone(),
        v2: config.v2.clone(),
    };
    let diff = comparison
        .compare(ENDPOINT, "GET", None, None)
        .await
        .unwrap();
    assert!(
        diff.changes
            .iter()
            .any(|c| c.location == "operation/responses/400" && c.kind == ChangeKind::Added)
    );
    for fact in [
        "required changed",
        "enum changed",
        "format changed",
        "nullable changed",
        "property added",
        "property type changed",
    ] {
        assert!(diff.schema_changes.iter().any(|c| c.fact == fact), "{fact}");
    }
    assert!(diff.schema_changes.iter().all(|c| !c.sources.is_empty()));
    let schema = specifications
        .schema(&config.v2, "Transaction")
        .await
        .unwrap();
    assert_eq!(schema.referenced_schemas, ["Amount"]);
    let filter = SearchFilter {
        version: Some(config.v2.clone()),
        kind: Some(DocumentKind::Pdf),
        source: Some("fixture-v2-guidelines".into()),
    };
    let hits = specifications
        .search_specification("transactions", filter.clone(), 10)
        .await
        .unwrap();
    assert!(!hits.is_empty());
    for hit in &hits {
        assert_eq!(hit.evidence.provenance.source_id, "fixture-v2-guidelines");
        assert_eq!(
            hit.evidence.provenance.sha256,
            sha256(hit.evidence.text.as_bytes())
        );
        assert!(hit.evidence.page.is_some());
        specifications
            .read_source(
                &hit.evidence.provenance.source_id,
                &hit.evidence.provenance.locator.0,
            )
            .await
            .unwrap();
    }
    let find = requirements
        .find_requirement(REQUIREMENT, Some(config.v2.clone()), 10)
        .await
        .unwrap();
    assert_eq!(find["curated_requirements"][0]["id"], REQUIREMENT);
    let discovered = requirements
        .find_requirement("bookingStatus", Some(config.v2.clone()), 10)
        .await
        .unwrap();
    assert!(
        !discovered["discovered_evidence"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let second = index(&config).await.unwrap();
    assert_eq!(second.skipped, 5);
    assert_eq!(second.indexed, 0);
    assert_eq!(first.generation, second.generation);
    let reopened = Snapshot::open(&config).await.unwrap();
    let hits2 = reopened
        .specifications()
        .search_specification("transactions", filter, 10)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(hits).unwrap(),
        serde_json::to_value(hits2).unwrap()
    );
    assert!(
        specifications
            .read_source("../../secret", "page:1")
            .await
            .is_err()
    );
    assert!(
        specifications
            .search_specification("transactions", SearchFilter::default(), 101)
            .await
            .is_err()
    );
    assert!(
        specifications
            .search_specification("!!!", SearchFilter::default(), 10)
            .await
            .is_err()
    );
    assert!(
        comparison
            .compare(ENDPOINT, "GET", Some("typo"), None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn publication_is_atomic_and_old_readers_remain_valid() {
    let (_temp, config) = support::fixture();
    let original = index(&config).await.unwrap();
    let old = Snapshot::open(&config).await.unwrap();
    let file = config.corpus_dir.join("openapi/v2/transactions.yaml");
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace("getTransactionsV2", "getTransactionsV2Updated");
    fs::write(&file, &text).unwrap();
    let updated = index(&config).await.unwrap();
    assert_eq!((updated.indexed, updated.skipped), (1, 4));
    assert_ne!(original.generation, updated.generation);
    assert_eq!(
        old.specifications()
            .endpoint(&config.v2, ENDPOINT, "GET")
            .await
            .unwrap()
            .operation_id
            .as_deref(),
        Some("getTransactionsV2")
    );
    let new = Snapshot::open(&config).await.unwrap();
    assert_eq!(
        new.specifications()
            .endpoint(&config.v2, ENDPOINT, "GET")
            .await
            .unwrap()
            .operation_id
            .as_deref(),
        Some("getTransactionsV2Updated")
    );
    fs::write(&file, "malformed: [").unwrap();
    assert!(index(&config).await.is_err());
    assert_eq!(
        Snapshot::open(&config)
            .await
            .unwrap()
            .catalog
            .generation()
            .await
            .unwrap(),
        updated.generation
    );
}

#[tokio::test]
async fn metadata_changes_and_removals_do_not_leave_stale_records() {
    let (_temp, config) = support::fixture();
    index(&config).await.unwrap();
    let manifest_path = config.corpus_dir.join("manifest.yaml");
    let mut manifest: Manifest =
        serde_saphyr::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    manifest.sources.retain(|s| s.id != "fixture-v1-guidelines");
    manifest
        .sources
        .iter_mut()
        .find(|s| s.id == "fixture-v2-guidelines")
        .unwrap()
        .authority = DocumentAuthority::Informative;
    fs::write(&manifest_path, serde_saphyr::to_string(&manifest).unwrap()).unwrap();
    let result = index(&config).await.unwrap();
    assert_eq!((result.indexed, result.skipped), (1, 3));
    let snapshot = Snapshot::open(&config).await.unwrap();
    assert!(
        snapshot
            .catalog
            .document("fixture-v1-guidelines")
            .await
            .is_err()
    );
    assert_eq!(
        snapshot
            .catalog
            .document("fixture-v2-guidelines")
            .await
            .unwrap()
            .authority,
        DocumentAuthority::Informative
    );
}

#[tokio::test]
async fn ocr_required_is_explicit_and_cannot_back_a_curated_requirement() {
    let (_temp, config) = support::fixture();
    let blank = include_bytes!("fixtures/blank.pdf");
    fs::write(config.corpus_dir.join("pdf/v1/guidelines.pdf"), blank).unwrap();
    let report = index(&config).await.unwrap();
    assert_eq!(report.ocr_required, ["fixture-v1-guidelines"]);
    let snapshot = Snapshot::open(&config).await.unwrap();
    assert_eq!(
        snapshot.catalog.health().await.unwrap()["status"],
        "degraded"
    );
    let error = snapshot
        .specifications()
        .read_source("fixture-v1-guidelines", "page:1")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ocr_required"));
    fs::write(config.corpus_dir.join("pdf/v2/guidelines.pdf"), blank).unwrap();
    assert!(
        index(&config)
            .await
            .unwrap_err()
            .to_string()
            .contains("unresolved citation")
    );
}

#[tokio::test]
async fn divergent_contracts_schemas_and_curated_conflicts_are_not_resolved() {
    let (_temp, config) = support::fixture();
    let manifest_path = config.corpus_dir.join("manifest.yaml");
    let mut manifest: Manifest =
        serde_saphyr::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let mut alternate = manifest
        .sources
        .iter()
        .find(|s| s.id == "fixture-v2-openapi")
        .unwrap()
        .clone();
    alternate.id = "fixture-v2-alternate".into();
    alternate.path = "openapi/v2/alternate.yaml".into();
    alternate.precedence = 1000;
    let text = fs::read_to_string(config.corpus_dir.join("openapi/v2/transactions.yaml"))
        .unwrap()
        .replace("getTransactionsV2", "otherOperation")
        .replace("nullable: false", "nullable: true");
    fs::write(config.corpus_dir.join(&alternate.path), text).unwrap();
    manifest.sources.push(alternate);
    fs::write(manifest_path, serde_saphyr::to_string(&manifest).unwrap()).unwrap();
    let requirement_path = config.corpus_dir.join("requirements/requirements.yaml");
    let mut file: RequirementFile =
        serde_saphyr::from_str(&fs::read_to_string(&requirement_path).unwrap()).unwrap();
    let mut other = file.requirements[0].clone();
    other.id = "FIXTURE-CONFLICT".into();
    file.requirements[0].conflicts_with.push(other.id.clone());
    file.requirements.push(other);
    fs::write(requirement_path, serde_saphyr::to_string(&file).unwrap()).unwrap();
    index(&config).await.unwrap();
    let snapshot = Snapshot::open(&config).await.unwrap();
    assert!(matches!(
        snapshot
            .specifications()
            .endpoint(&config.v2, ENDPOINT, "GET")
            .await,
        Err(Error::Ambiguous(_))
    ));
    let bundle = RequirementService {
        specifications: snapshot.specifications(),
    }
    .get_endpoint_requirements(&config.v2, ENDPOINT, "GET")
    .await
    .unwrap();
    assert_eq!(bundle.openapi.len(), 2);
    for kind in [
        "divergent_openapi_contracts",
        "divergent_openapi_schemas",
        "curated_conflict",
    ] {
        assert_eq!(
            bundle.conflicts.iter().filter(|c| c.kind == kind).count(),
            1,
            "{kind}"
        );
    }
    assert!(bundle.conflicts.iter().all(|c| c.sources.len() >= 2));
}

#[tokio::test]
async fn multiline_summaries_and_explicit_endpoint_mappings() {
    let (_temp, config) = support::fixture();
    let path = config.corpus_dir.join("openapi/v2/transactions.yaml");
    let mut api: serde_json::Value =
        serde_saphyr::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    api["paths"][ENDPOINT]["get"]["summary"] =
        json!("Read account\ntransactions\twith unicode \u{20ac}");
    api["paths"][ENDPOINT]["post"] =
        json!({"responses":{"200":{"description":"Synthetic additional method"}}});
    api["paths"][ENDPOINT]["get"]["parameters"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "name":"accountId","in":"path","required":true,"schema":{"type":"integer"}
        }));
    fs::write(path, serde_saphyr::to_string(&api).unwrap()).unwrap();
    index(&config).await.unwrap();
    let snapshot = Snapshot::open(&config).await.unwrap();
    let service = RequirementService {
        specifications: snapshot.specifications(),
    };
    let bundle = service
        .get_endpoint_requirements(&config.v2, ENDPOINT, "GET")
        .await
        .unwrap();
    assert!(bundle.related_evidence_query.len() <= 1024);
    assert!(!bundle.related_evidence_query.chars().any(char::is_control));
    assert_eq!(bundle.openapi[0].parameters[0]["schema"]["type"], "integer");
    assert_eq!(bundle.openapi[0].parameters.as_array().unwrap().len(), 2);
    assert_eq!(bundle.requirements[0].mapping.endpoints.len(), 1);
    assert!(
        service
            .get_endpoint_requirements(&config.v2, ENDPOINT, "POST")
            .await
            .unwrap()
            .requirements
            .is_empty()
    );
}

#[tokio::test]
async fn incoming_curated_conflicts_are_detected_without_false_endpoint_links() {
    let (_temp, config) = support::fixture();
    let path = config.corpus_dir.join("requirements/requirements.yaml");
    let mut mappings: RequirementFile =
        serde_saphyr::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let mut incoming = mappings.requirements[0].clone();
    incoming.id = "FIXTURE-INCOMING-CONFLICT".into();
    incoming.sources = vec![RequirementSource {
        source_id: "fixture-v2-guidelines".into(),
        locator: SourceLocator("page:2".into()),
    }];
    incoming.endpoints.clear();
    incoming.conflicts_with = vec![REQUIREMENT.into()];
    mappings.requirements.push(incoming);
    fs::write(path, serde_saphyr::to_string(&mappings).unwrap()).unwrap();
    index(&config).await.unwrap();
    let snapshot = Snapshot::open(&config).await.unwrap();
    let bundle = RequirementService {
        specifications: snapshot.specifications(),
    }
    .get_endpoint_requirements(&config.v2, ENDPOINT, "GET")
    .await
    .unwrap();
    assert_eq!(bundle.requirements.len(), 1);
    assert_eq!(bundle.conflicts.len(), 1);
    assert_eq!(bundle.conflicts[0].kind, "curated_conflict");
}

#[tokio::test]
async fn doctor_checks_full_pages_manifest_drift_and_writer_lock() {
    use bg_spec_indexer::pipeline::verify_sources;
    use sqlx::Connection;
    let (_temp, config) = support::fixture();
    index(&config).await.unwrap();
    let snapshot = Snapshot::open(&config).await.unwrap();
    snapshot.doctor().await.unwrap();
    verify_sources(&config, snapshot.catalog.as_ref())
        .await
        .unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(config.data_dir.join("index.lock"))
        .unwrap();
    fs2::FileExt::try_lock_exclusive(&lock).unwrap();
    assert!(
        index(&config)
            .await
            .unwrap_err()
            .to_string()
            .contains("indexer lock")
    );
    drop(lock);
    let manifest = config.corpus_dir.join("manifest.yaml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(&manifest, text.replace("precedence: 0", "precedence: 7")).unwrap();
    assert!(
        verify_sources(&config, snapshot.catalog.as_ref())
            .await
            .unwrap_err()
            .to_string()
            .contains("drift")
    );
    fs::write(&manifest, text).unwrap();
    snapshot.catalog.close().await;
    drop(snapshot);
    let path = bg_spec_store::active_snapshot(&config.data_dir)
        .unwrap()
        .join("catalog.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(path);
    let mut writer = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    sqlx::query("UPDATE chunks SET data=json_set(data,'$.text','tampered exact page') WHERE source_id='fixture-v2-guidelines' AND locator='page:1'")
            .execute(&mut writer).await.unwrap();
    writer.close().await.unwrap();
    let snapshot = Snapshot::open(&config).await.unwrap();
    assert!(
        snapshot
            .doctor()
            .await
            .unwrap_err()
            .to_string()
            .contains("content hash")
    );
}

#[tokio::test]
async fn concurrent_snapshots_return_identical_evidence() {
    let (_temp, config) = support::fixture();
    index(&config).await.unwrap();
    let mut readers = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let config = config.clone();
        readers.spawn(async move {
            let snapshot = Snapshot::open(&config).await.unwrap();
            let hits = snapshot
                .specifications()
                .search_specification("transactions", SearchFilter::default(), 10)
                .await
                .unwrap();
            serde_json::to_value(hits).unwrap()
        });
    }
    let first = readers.join_next().await.unwrap().unwrap();
    while let Some(next) = readers.join_next().await {
        assert_eq!(first, next.unwrap());
    }
}
#[tokio::test]
async fn dangling_requirements_external_refs_and_path_escapes_fail_closed() {
    let (_temp, config) = support::fixture();
    let mapping = config.corpus_dir.join("requirements/requirements.yaml");
    fs::write(
        &mapping,
        fs::read_to_string(&mapping)
            .unwrap()
            .replace("page:1", "page:999"),
    )
    .unwrap();
    assert!(
        index(&config)
            .await
            .unwrap_err()
            .to_string()
            .contains("unresolved citation")
    );
    assert!(!config.data_dir.join("CURRENT").exists());
    assert!(config::corpus_path(&config.corpus_dir, Path::new("../secret")).is_err());
    assert!(config::corpus_path(&config.corpus_dir, Path::new("/etc/passwd")).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", config.corpus_dir.join("escape")).unwrap();
        assert!(config::corpus_path(&config.corpus_dir, Path::new("escape")).is_err());
    }
}

fn metadata(config: &Config) -> Document {
    let manifest = bg_spec_indexer::pipeline::parse_manifest(
        &fs::read(config.corpus_dir.join("manifest.yaml")).unwrap(),
    )
    .unwrap();
    let entry = &manifest.sources[0];
    Document {
        provenance: Provenance {
            source_id: entry.id.clone(),
            document_id: "test".into(),
            version: entry.version.clone(),
            kind: DocumentKind::Openapi,
            locator: SourceLocator("document".into()),
            sha256: "test".into(),
        },
        title: "test".into(),
        authority: DocumentAuthority::Test,
        precedence: 0,
        path: entry.path.clone(),
        indexed_status: IndexedStatus::Indexed,
        ocr_required_pages: vec![],
        manifest_sha256: "test".into(),
    }
}

#[test]
fn pdf_extraction_and_injected_extractor() {
    let (_temp, config) = support::fixture();
    let bytes = fs::read(config.corpus_dir.join("pdf/v2/guidelines.pdf")).unwrap();
    let pages = PdfOxideExtractor.extract(&bytes).unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].number, 1);
    assert!(pages[0].text.contains("MUST include transactionId"));
    assert!(PdfOxideExtractor.extract(b"not a pdf").is_err());
    struct Fake;
    impl PdfExtractor for Fake {
        fn extract(&self, _: &[u8]) -> Result<Vec<bg_spec_indexer::pdf::PdfPage>> {
            Ok(vec![bg_spec_indexer::pdf::PdfPage {
                number: 1,
                text: "Exact normative paragraph.".into(),
            }])
        }
    }
    let indexed = parse_pdf(metadata(&config), b"", &Fake, &config).unwrap();
    assert_eq!(indexed.chunks[0].text, "Exact normative paragraph.");
    assert_eq!(indexed.chunks[0].provenance.locator.0, "page:1");
}

#[test]
fn openapi_normalization_references_and_inheritance() {
    let (_temp, config) = support::fixture();
    let yaml = fs::read_to_string(config.corpus_dir.join("openapi/v2/transactions.yaml")).unwrap();
    let document = metadata(&config);
    let indexed = parse_openapi(document.clone(), yaml.as_bytes(), &config).unwrap();
    assert_eq!(indexed.operations.len(), 1);
    assert_eq!(indexed.schemas.len(), 2);
    let changed = yaml.replace(
        "      operationId:",
        "      security: []\n      operationId:",
    );
    let parsed = parse_openapi(document.clone(), changed.as_bytes(), &config).unwrap();
    assert_eq!(parsed.operations[0].security, json!([]));
    assert!(
        parsed.operations[0].contract["securitySchemes"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    let external = yaml.replace(
        "#/components/schemas/Amount",
        "https://example.invalid/schema.yaml",
    );
    assert!(
        parse_openapi(document.clone(), external.as_bytes(), &config)
            .unwrap_err()
            .to_string()
            .contains("external")
    );
    let dangling = yaml.replace("#/components/schemas/Amount", "#/components/schemas/Absent");
    assert!(parse_openapi(document.clone(), dangling.as_bytes(), &config).is_err());
    let unsupported = yaml.replace("3.0.3", "3.1.0");
    assert!(
        parse_openapi(document, unsupported.as_bytes(), &config)
            .unwrap_err()
            .to_string()
            .contains("3.0.x")
    );
}

#[tokio::test]
async fn recursive_schema_closure_terminates_and_source_hashes_match() {
    let (_temp, config) = support::fixture();
    let path = config.corpus_dir.join("openapi/v2/transactions.yaml");
    let text = fs::read_to_string(&path).unwrap().replace(
        "        creditorName:\n          type: string",
        "        creditorName:\n          type: string\n        previous:\n          $ref: '#/components/schemas/Transaction'"
    );
    fs::write(path, text).unwrap();
    index(&config).await.unwrap();
    let snapshot = Snapshot::open(&config).await.unwrap();
    let operation = snapshot
        .specifications()
        .endpoint(&config.v2, ENDPOINT, "GET")
        .await
        .unwrap();
    let schemas = snapshot
        .specifications()
        .referenced_schemas(&operation)
        .await
        .unwrap();
    assert_eq!(schemas.len(), 2);
    for schema in schemas {
        assert_eq!(
            schema.provenance.sha256,
            sha256(serde_json::to_string(&schema.schema).unwrap().as_bytes())
        );
    }
}
