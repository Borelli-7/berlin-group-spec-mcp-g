use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpecificationVersion(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Pdf,
    Openapi,
    Requirements,
}

impl DocumentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Openapi => "openapi",
            Self::Requirements => "requirements",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentAuthority {
    Normative,
    Technical,
    Informative,
    Project,
    Test,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceLocator(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_id: String,
    pub document_id: String,
    pub version: SpecificationVersion,
    pub kind: DocumentKind,
    pub locator: SourceLocator,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    pub id: String,
    pub kind: DocumentKind,
    pub version: SpecificationVersion,
    pub authority: DocumentAuthority,
    pub precedence: i32,
    pub path: String,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub sources: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexedStatus {
    Indexed,
    OcrRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    #[serde(flatten)]
    pub provenance: Provenance,
    pub title: String,
    pub authority: DocumentAuthority,
    pub precedence: i32,
    /// Corpus-relative, never an absolute host path.
    pub path: String,
    pub indexed_status: IndexedStatus,
    pub ocr_required_pages: Vec<u32>,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceChunk {
    pub chunk_id: String,
    #[serde(flatten)]
    pub provenance: Provenance,
    pub title: String,
    pub page: Option<u32>,
    pub section: Option<String>,
    pub text: String,
    /// Complete pages/semantic units are retrievable; only bounded chunks are searched.
    pub searchable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiOperation {
    #[serde(flatten)]
    pub provenance: Provenance,
    pub path: String,
    pub method: String,
    #[serde(rename = "operationId")]
    pub operation_id: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub parameters: Value,
    pub request_body: Value,
    pub responses: Value,
    pub security: Value,
    pub tags: Value,
    pub contract: Value,
    pub referenced_schemas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiSchema {
    #[serde(flatten)]
    pub provenance: Provenance,
    pub name: String,
    pub schema: Value,
    pub referenced_schemas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementSource {
    pub source_id: String,
    pub locator: SourceLocator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointRef {
    pub path: String,
    pub method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementMapping {
    pub id: String,
    pub version: SpecificationVersion,
    pub title: String,
    pub sources: Vec<RequirementSource>,
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub endpoints: Vec<EndpointRef>,
    /// Explicit curator assertions; never inferred from contradictory-looking prose.
    #[serde(default)]
    pub conflicts_with: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementFile {
    pub requirements: Vec<RequirementMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirement {
    #[serde(flatten)]
    pub mapping: RequirementMapping,
    pub provenance: Provenance,
    pub resolved_sources: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequirementTrace {
    pub requirement: Requirement,
    pub specification_sources: Vec<Provenance>,
    pub endpoints: Vec<OpenApiOperation>,
    pub schemas: Vec<OpenApiSchema>,
    pub acceptance_criteria: Vec<String>,
    pub related_evidence: Vec<EvidenceChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(flatten)]
    pub evidence: EvidenceChunk,
    pub relevance: f32,
    pub classification: String,
}

#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub version: Option<String>,
    pub kind: Option<DocumentKind>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
    Unchanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityChange {
    pub kind: ChangeKind,
    pub location: String,
    pub fact: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub sources: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub kind: String,
    pub description: String,
    pub sources: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub endpoint: EndpointRef,
    pub openapi: Vec<OpenApiOperation>,
    pub requirements: Vec<Requirement>,
    pub schemas: Vec<OpenApiSchema>,
    pub related_evidence: Vec<SearchResult>,
    pub related_evidence_query: String,
    pub conflicts: Vec<Conflict>,
    pub sources: Vec<Document>,
    pub conflict_detection: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexedDocument {
    pub document: Option<Document>,
    pub chunks: Vec<EvidenceChunk>,
    pub operations: Vec<OpenApiOperation>,
    pub schemas: Vec<OpenApiSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparison {
    pub v1_operation: Option<OpenApiOperation>,
    pub v2_operation: Option<OpenApiOperation>,
    pub changes: Vec<CompatibilityChange>,
    pub schema_changes: Vec<CompatibilityChange>,
    pub compatibility_relevant_facts: String,
    pub sources: Vec<Provenance>,
}
