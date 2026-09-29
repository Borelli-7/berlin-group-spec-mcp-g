# MCP tools

The server exposes exactly nine domain tools and one operational tool. All are
read-only, idempotent within a snapshot, non-destructive, and closed-world.
The official Rust MCP SDK generates JSON input schemas and handles protocol
initialization and tool dispatch.

Results use `structuredContent` and SDK-generated text content for clients
that do not consume structured output. Domain failures return `isError: true`
with `{ "code": "...", "message": "..." }`. Malformed invocations may instead
produce SDK-level protocol errors; neither error form is evidence.

## Tool reference

| Tool | Input | Structured result |
|---|---|---|
| `search_specification` | `query`, optional `version`, `kind`, `source`, `limit` | `{results:[...]}` with chunk text, source ID, document ID, version/kind, title, locator, page, hash, relevance, classification |
| `find_requirement` | `query`, optional `version`, `limit` | `curated_requirements`, `total_curated_matches`, separately labeled `discovered_evidence` |
| `read_source` | `source_id`, `locator` | `source` metadata, `exact_location`, exact indexed `content` records and their hashes, `content_semantics` |
| `read_openapi_endpoint` | `version`, `path`, `method` | Normalized operation, contract, effective parameters/security, responses, schema references, source provenance |
| `read_openapi_schema` | `version`, `name` | Schema JSON, name, source provenance, directly referenced schema names |
| `get_endpoint_requirements` | `version`, `path`, `method` | `endpoint`, `openapi`, `requirements`, transitive `schemas`, `related_evidence`, `related_evidence_query`, `conflicts`, `sources`, `conflict_detection` |
| `trace_requirement` | `requirement_id` | Curated requirement/mapping provenance, specification citations, endpoint contracts, schemas, acceptance criteria, exact related evidence |
| `compare_v1_v2` | `path`, `method`, optional `version1`, `version2` | Both operations, `changes`, `schema_changes`, interpretation warning, source citations |
| `list_sources` | `{}` | `{sources:[...]}` with authority/precedence, corpus-relative paths, hashes, extraction status |
| `health` | `{}` | Generation, counts, `status: ok/degraded`, OCR-required sources/pages |

Optional limits default to 10 and must be 1..100. Kinds are `pdf`, `openapi`,
and `requirements`. Source filtering uses `source` in search, but direct
retrieval uses `source_id`.

### Search is not requirement interpretation

`search_specification` explicitly states in its MCP tool description:
**results are evidence and MUST NOT automatically be interpreted as
authoritative requirements**. Result `classification` reiterates this boundary.
BM25 scores measure relevance, not normativity, confidence, or precedence.
Search uses OR matching among literal tokens with exact AND filters.
No query DSL or arbitrary filesystem path is accepted.

`SearchResult` flattens its evidence record: the evidence string is `text`;
the citation is `source_id` + `locator` + `sha256`. `page` is nullable for
non-PDF evidence. `section` is an extraction hint, not a normative assertion.

```json
{
  "name": "search_specification",
  "arguments": {
    "query": "transactions",
    "version": "openfinance-v2",
    "kind": "pdf",
    "source": "fixture-v2-guidelines",
    "limit": 10
  }
}
```

The full responses in `examples/responses.json` were produced by the real
stdio binary against the synthetic fixture corpus, not manually fabricated.

### Primary evidence bundle

`get_endpoint_requirements` returns **an array** in `openapi` so conflicting
sources remain visible. It gathers source-scoped schema closures, curated
endpoint mappings, and the top 20 related PDF search hits. Exact curated PDF
citations are added even if search did not retrieve them; those citations have
relevance `0.0` and an explicit "score not applicable" classification.

The bounded, whitespace-normalized query is exposed as `related_evidence_query`.
Related search evidence is not automatically a requirement. If no mapping
exists, `requirements` is empty. If no OpenAPI operation exists, the tool
returns `not_found`. Multiple matching operations are not silently selected.
An empty conflict list is not proof that the corpus is logically consistent.

### Exact source retrieval

```json
{
  "name": "read_source",
  "arguments": {
    "source_id": "fixture-v2-guidelines",
    "locator": "page:1"
  }
}
```

`content` is an array of exact catalog evidence records, normally one for a
page or semantic locator. For PDFs, exact means the stored extraction text,
not original PDF bytes or reconstructed visual layout. For OpenAPI, exact
means normalized semantic JSON. Whole pages and individual chunk locators
are both retrievable. Source hashes and evidence hashes have distinct meanings.
There is no raw filesystem-read fallback.

### Trace and comparison

```json
{
  "name": "trace_requirement",
  "arguments": {
    "requirement_id": "FIXTURE-OFV2-TRANSACTIONS-001"
  }
}
```

The fixture trace joins its curated mapping to `page:1` of the V2 PDF and the
OpenAPI path, then GET and its `Transaction` -> `Amount` schema graph.
Acceptance criteria come exclusively from the curated mapping.

```json
{
  "name": "compare_v1_v2",
  "arguments": {
    "path": "/accounts/{accountId}/transactions",
    "method": "GET"
  }
}
```

Version defaults come from TOML. Unknown version corpora are errors rather
than fabricated endpoint removals. If the endpoint exists on only one side,
the change is added/removed with document citations for the absent side.
Absent on both sides is `not_found`; duplicate candidates are `ambiguous_source`.

Each change has `kind`, JSON-pointer-like `location`, `fact`, `before`, `after`,
and `sources`. The fixture reports, among other differences:

| Location | Fact |
|---|---|
| `operation/responses/400` | field added |
| `schemas/Transaction/required` | required changed |
| `schemas/Transaction/properties/status/enum` | enum changed |
| `schemas/Transaction/properties/bookingDate/format` | format changed |
| `schemas/Transaction/properties/bookingDate/nullable` | nullable changed |
| `schemas/Transaction/properties/creditorName` | property added |
| `schemas/Amount/properties/value/type` | property type changed |

These are not claims about actual Berlin Group specifications or an overall
compatibility verdict.

## Errors

`invalid_input`, `not_found`, `ambiguous_source`, `extraction_unavailable`,
`index_integrity`, `internal_error`, `busy`, and `response_too_large` are domain
codes. Internal filesystem/database details go to stderr, not caller error
messages. Oversized transport frames close the session before tool dispatch.
No error path runs indexing, modifies source files, or writes application code.
