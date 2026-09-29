# Corpus ingestion and indexing

## Source manifest

The manifest is the authority for source discovery and authority metadata.
`walkdir` reports unlisted files, but unlisted files are never indexed.

```yaml
sources:
  - id: bg-openfinance-v2-implementation-guidelines
    kind: pdf
    version: openfinance-v2
    authority: normative
    precedence: 100
    title: Operator-supplied official implementation guidelines
    path: pdf/v2/implementation-guidelines.pdf
  - id: bg-openfinance-v2-openapi
    kind: openapi
    version: openfinance-v2
    authority: technical
    precedence: 90
    path: openapi/v2/openfinance.yaml
  - id: project-v2-requirements
    kind: requirements
    version: openfinance-v2
    authority: project
    precedence: 0
    path: requirements/requirements.yaml
```

Kinds: `pdf`, `openapi`, `requirements`. Authorities: `normative`, `technical`,
`informative`, `project`, `test`, `unknown`. Version labels are explicit,
opaque strings. Source IDs are unique ASCII letters/numbers/`-_.`; no
authority or version is inferred from filenames. Precedence is exposed
metadata, not an automatic conflict-resolution rule. Requirement files must
also be manifest entries, ensuring their own provenance is retained.

Use additional PDF entries for introduction documents, operational rules,
implementation guidelines, clarifications, and errata. Preserve distinct
source IDs for distinct releases/publications. Corrections are not silently
merged into an earlier document.

## Incremental behavior

For every listed file: confine path -> read bounded bytes -> SHA-256 -> compare
with the prior catalog. Unchanged byte hashes **and** manifest/extractor/chunking
fingerprints reuse previous extraction/normalization. Metadata-only changes
also invalidate the appropriate document. Missing sources and dangling
requirement citations fail the build.

Requirements are parsed/revalidated each run, including when the mapping file
is unchanged, because its referenced documents may have changed. Removed
manifest entries disappear from the new generation. Existing processes keep
their historical generation until restarted.

When any input changes, SQLite/Tantivy are assembled into a fresh generation
from both new and reused records. Incremental indexing skips expensive
PDF/OpenAPI parsing, **not** generation-wide search reconstruction. A fully
unchanged corpus reuses the existing published generation. Indexing currently
holds normalized corpus records in memory; size offline workers accordingly.
Do not run the indexer in every agent's startup command.

Generation identity is a SHA-256 of sorted source IDs, file hashes, and
manifest/pipeline fingerprints. Bump the pipeline revision whenever extraction,
normalization, tokenizer, or schema semantics change.

## PDF pipeline

`PdfExtractor::extract` returns numbered page text. `PdfOxideExtractor` uses
`pdf_oxide::PdfDocument::from_bytes`, `page_count`, and per-page `extract_text`.
The pipeline runs offline in `spawn_blocking` and never rewrites the PDF.

Pages are stored intact at `page:N` (one-based physical page, not the printed
page label). Search chunks stay within pages and recognizable numbered or
uppercase headings. Within a section, paragraph boundaries are preferred,
then sentence and whitespace boundaries. A single oversized statement with
no usable boundary must be split at Unicode character boundaries to meet the
hard limit. Overlap is approximately 500 characters, aligned to a word
boundary when possible; it never crosses a section/page.

Chunk locator: `page:N:chars:START-END`, with zero-based, half-open Unicode
scalar offsets into the exact extracted page string. This is not a byte offset,
a PDF object reference, or a claim of original typesetting. `section` is a
best-effort heading label. Tables, columns, reading order, and ligatures need
operator review against the original publication.

Pages with fewer than three alphanumeric extracted characters are flagged
`ocr_required`; no empty chunks are produced. This conservative diagnostic
also flags truly blank pages. Corrupt-document/extraction errors fail indexing.
An OCR-required page cannot serve as a curated evidence citation. Replacing
the extractor with reviewed Pdfium/OCR code does not require changing services
or MCP tools; bump the pipeline fingerprint after doing so.

## OpenAPI pipeline

YAML is deserialized with `serde-saphyr` into `openapiv3::OpenAPI`; raw YAML
search is not the contract model. Only OpenAPI 3.0.x is supported. The typed
parser is not a full OpenAPI conformance validator.

Semantic records include API info, paths, operations, reusable schemas, and
all object-valued component groups (parameters, responses, request bodies,
headers, examples, links, callbacks, security schemes). Operations contain
path/method, operationId, summary/description, tags, effective parameters,
request body, responses, security, server definitions, and security schemes.
Operation parameters override inherited parameters with the same `(in,name)`.
An explicit operation `security: []` overrides global security.

Local component references are validated. Non-schema references are expanded
with depth/cycle checks; schema references retain their named graph identity.
External/network/file references and nested pointers beneath a component schema
are explicitly rejected. No URLs, callbacks, example links, or servers in a
specification are fetched or executed.

Locators:

```text
openapi:info
path:/accounts/{accountId}/transactions
operation:GET:/accounts/{accountId}/transactions
json:#/components/schemas/Transaction
json:#/components/responses/Transactions
```

OpenAPI locators designate normalized semantic JSON, not YAML line numbers.
Whole semantic units are retrievable. Oversized units have additional bounded
search chunks. Named schemas are individually indexed, and services resolve
their transitive graph within the originating source to avoid cross-document
name collisions.

## Diagnostics and maintenance

`index` reports the generation, indexed/skipped counts, OCR-required sources,
and unlisted files. `stats`/`health` report record counts and degraded extraction
status. `doctor` checks SQLite integrity, foreign keys, all page/chunk/contract/
schema/mapping hashes, record provenance, requirement citations, catalog/search
chunk identities, current source-file hashes, and manifest/pipeline drift.

Published generations must remain immutable. Back up `CURRENT` and the
referenced generation as one unit. Retention cleanup is an operator action:
stop all readers first and remove only explicitly identified unused generation
directories. No automatic deletion command or MCP maintenance tool exists.

Regenerate original fixture PDFs with `python3 scripts/generate_fixture_pdfs.py`.
The generator includes proper PDF object offsets and deterministic content;
it downloads nothing and needs only Python's standard library.
