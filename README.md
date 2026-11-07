# Berlin Group Specification MCP

A local, read-only Rust MCP server for source-traceable Berlin Group Open
Finance / NextGenPSD2 evidence. It retrieves and compares indexed material;
it never invokes an LLM, invents requirements, resolves business conflicts,
or modifies application repositories.

**The included corpus is entirely synthetic and marked `authority: test`.**
It does not represent actual Berlin Group V1/V2 obligations. Obtain official
documents under their applicable terms, replace the fixtures, and curate the
manifest and mappings before using this as an authoritative engineering source.
No official specification text is redistributed here.

## Quick start

Requires Linux, Rust 1.98.1 (Rust 2024), a linker/C toolchain, and enough disk
space for Rust dependencies and immutable index generations. SQLite is bundled
by SQLx; no database service, PDF executable, OCR service, or API key is needed.

```bash
cargo build --release --locked
./target/release/bg-spec index --config config/config.toml
./target/release/bg-spec doctor --config config/config.toml
./target/release/bg-spec stats --config config/config.toml
./target/release/bg-spec sources --config config/config.toml
./target/release/bg-spec-mcp --config config/config.toml
```

The last command serves MCP over stdin/stdout, not a human REPL. Logs use
stderr. Startup opens an existing snapshot; it does **not** parse source
documents or build indexes.

For a real stdio acceptance run:

```bash
python3 scripts/mcp_smoke.py
# Regenerate the complete, source-backed example responses:
python3 scripts/mcp_smoke.py --output examples/responses.json
```

## Copilot CLI and Herdr

`config/copilot-mcp-config.json` contains valid persistent configuration for
this checkout. Merge its `berlin-group-spec` entry into
`~/.copilot/mcp-config.json`; do not overwrite existing servers. Both that file
and the repository `.mcp.json` use **absolute paths**: update them after moving
the checkout. No user-level configuration is changed by building/indexing.

Alternatively register the release executable using Copilot CLI:

```bash
copilot mcp add berlin-group-spec -- \
  /absolute/path/to/target/release/bg-spec-mcp \
  --config /absolute/path/to/config/config.toml
```

Start Copilot inside a trusted Git repository for project `.mcp.json` discovery.
If this directory is not a Git checkout, use persistent user configuration.
Each Copilot process running under Herdr can launch its own read-only server.
Herdr is an orchestration layer, not a server dependency. Restart MCP processes
after publishing a new generation when they need the new corpus.

The intended engineering workflow is Requirements -> Developer ->
Requirements Review -> Tester -> Architect Review across `aspsp-xs2a`,
`ledgers`, `xs2a-connector-examples`, and `psd2-dynamic-sandbox`. The server does
not inspect or write any of those repositories. V1/V2 interpretation and
backward-compatibility decisions remain with the agents and human reviewers.

## Evidence workflow

Use `get_endpoint_requirements` first, then `read_openapi_endpoint`,
`trace_requirement`, `read_source`, and `compare_v1_v2`. Preserve source IDs,
versions, locators, and SHA-256 values in engineering requirements.

```json
{
  "name": "get_endpoint_requirements",
  "arguments": {
    "version": "openfinance-v2",
    "path": "/accounts/{accountId}/transactions",
    "method": "GET"
  }
}
```

The fixture result contains one normalized operation, the curated ID
`FIXTURE-OFV2-TRANSACTIONS-001`, two reachable schemas, PDF evidence, citations,
and no declared/structural conflicts. `compare_v1_v2` reports concrete enum,
required-field, format, nullable, property, and response-code changes without a
compatibility verdict. Complete requests/results are in `examples/`.

## Engineering checks

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check
cargo build --release
```

`Cargo.lock` is part of the deliverable; update it deliberately and re-run the
extraction and acceptance checks (see `CONTRIBUTING.md`). All
requested architectural dependencies remain in use, including current stable
`rmcp`, `pdf_oxide`, `openapiv3`, `serde-saphyr`, Tantivy, SQLx, and SHA-256.

## Documentation

| Document | Topic |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Crates, services, immutable publication, determinism |
| [CONFIGURATION.md](CONFIGURATION.md) | Paths, limits, Copilot setup |
| [INDEXING.md](INDEXING.md) | Manifest, PDF/OpenAPI extraction, incremental indexing |
| [MCP_TOOLS.md](MCP_TOOLS.md) | Exact tool contract and examples |
| [REQUIREMENT_TRACEABILITY.md](REQUIREMENT_TRACEABILITY.md) | Curated mapping and evidence relationships |
| [SECURITY.md](SECURITY.md) | Threat model, read-only boundaries, deployment |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Development, fixtures, testing, limitations |

**Scope:** OpenAPI 3.0.x only; external references and nested component-schema
references are rejected. OCR is diagnosed but not performed. Conflict detection
does not understand arbitrary prose. Official-corpus coverage and production
capacity must be assessed against the operator's actual documents; synthetic
acceptance tests do not establish regulatory conformance.
