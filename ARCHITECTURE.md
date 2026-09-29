# Architecture

```text
Official / curated, operator-owned corpus
                  |
            bg-spec index
   discover -> hash -> extract -> normalize -> validate citations
                  |
     immutable generation (catalog.db + tantivy/)
                  |
        atomically replace data/CURRENT
                  |
   read-only bg-spec-mcp processes, one snapshot per process
                  |
   Copilot CLI / Herdr Requirements, Architect, Tester agents
```

## Workspace boundaries

| Crate | Responsibility |
|---|---|
| `bg-spec-core` | Explicit domain types, errors, configuration, repository traits, `SpecificationService`, `RequirementService`, `CompatibilityService` |
| `bg-spec-indexer` | Manifest-driven discovery, SHA-256, `PdfExtractor` / `PdfOxideExtractor`, page/section chunking, `openapiv3` normalization, curated citation validation, publication |
| `bg-spec-store` | SQLx/SQLite catalog and migrations; Tantivy BM25 retrieval; snapshot opening |
| `bg-spec-mcp` | Official `rmcp` `#[tool]` / `#[tool_router]` adapters, typed schemas, input/output limits, stdio binary |
| `bg-spec-cli` | `bg-spec index`, `doctor`, `stats`, `sources` |

Repository traits are injected into application services using `Arc<dyn ...>`.
The shared arcs represent concurrent, immutable service/repository handles.
Parsing/search work uses Tokio `spawn_blocking`; SQLite operations use SQLx
asynchronously. The MCP crate does not depend on the indexer outside tests.
There is no HTTP transport, application-repository integration, shell tool,
LLM client, or network reference resolver.

## Domain and provenance

The core explicitly defines `Document`, `DocumentKind`, `DocumentAuthority`,
`SpecificationVersion`, `SourceLocator`, `EvidenceChunk`, `OpenApiOperation`,
`OpenApiSchema`, `Requirement`, `RequirementSource`, `RequirementTrace`,
`CompatibilityChange`, `Conflict`, and `SearchResult`.

Source-backed objects carry `source_id`, `document_id`, `version`, `kind`,
`locator`, and `sha256`. Document hashes cover original file bytes. Evidence
hashes cover UTF-8 extracted text or normalized compact JSON; operation/schema
hashes cover their contract/schema JSON. A requirement's own hash covers its
normalized curated mapping. These are different hash domains, not interchangeable.

Stable manifest IDs identify sources; document IDs include source ID, file hash,
and extraction/manifest fingerprint. Chunk IDs include document ID, locator, and
content hash. Changing a source or normalization algorithm intentionally changes
its derived identities. Citations must retain a generation for historical use.

## SQLite and search

`crates/bg-spec-store/migrations/0001_catalog.sql` defines `documents`, `chunks`,
`openapi_operations`, `openapi_schemas`, `requirements`, `requirement_sources`,
and generation metadata. Foreign keys and uniqueness constraints are enforced.
OpenAPI and domain records are stored as JSON with indexed lookup columns.
SQLite, not Tantivy stored text, supplies returned evidence.

Tantivy indexes `chunk_id`, `source_id`, `version`, `kind`, `locator`, `page`,
`title`, and `content`. Content/title queries use BM25. Exact version/kind/source
filters are mandatory clauses. Caller query strings are tokenized literally,
not parsed as field-selector syntax. Ranking uses score descending, then stable
chunk ID ascending **inside the top-K collector**, including boundary ties.
No network-dependent embeddings or probabilistic interpretation are involved.

## Publication and concurrency

The logical `data/catalog.db` and `data/tantivy/` pair is stored **inside an
immutable generation** rather than replaced independently:

```text
data/
  index.lock
  CURRENT                           # active 64-character generation ID
  generations/
    <generation>/
      catalog.db
      tantivy/
```

This deliberate physical layout refinement avoids a cross-store two-phase
commit problem. A single OS advisory writer lock serializes indexers. Each
build uses an unpublished staging directory; SQLite transactions, foreign-key
checks, text hashes, Tantivy commit metadata, and record counts are checked.
Files/directories are fsynced before generation rename; `CURRENT` is replaced
atomically and its directory fsynced afterward. Failed builds leave the old
pointer untouched. A crash can leave an unreferenced staging/generation directory,
never a partially published active pair.

Readers open SQLite with `read_only(true)` and `immutable(true)`, and Tantivy
with manual reload and no writer. They resolve `CURRENT` once. Existing
processes retain their generation after indexing; new processes get the new one.
Catalog/search generation IDs, schema, and searchable counts must agree.
Old generations are retained, not automatically garbage-collected, because a
reader may still hold them. Never alter a published generation in place.
Use a local filesystem with working locks, atomic rename, and fsync semantics;
network/object filesystems are not supported deployment targets.

## Deterministic interpretation boundaries

Operations incorporate inherited parameters, root/path/operation servers,
effective security, and referenced security-scheme definitions. Local
non-schema references are resolved. Named schema references remain references;
services traverse a source-scoped graph, visiting each schema once.

Comparison returns added/removed/modified/unchanged facts, recursive JSON-pointer
field differences, and reachable named schema differences. Inline schemas are
compared in operation contracts. Arrays are compared as ordered values; array
reordering, descriptions, defaults, and schema renaming can produce differences.
Missing and explicit false/null values remain distinct. This is structural
comparison, not a complete OpenAPI semantic equivalence prover.

Conflict diagnostics cover divergent contracts/schemas for identical
version/endpoint keys and explicit curator declarations. The implementation
never chooses a winner using authority/precedence and never interprets prose
as a machine-verifiable contradiction.
