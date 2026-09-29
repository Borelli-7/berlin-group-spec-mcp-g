# Contributing

Use the pinned stable Rust toolchain and Rust 2024. `Cargo.lock` must remain
checked in for this application workspace. Source changes should be small,
typed, source-traceable, and accompanied by relevant deterministic tests.

## Commands

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --release
./target/release/bg-spec index --config config/config.toml
./target/release/bg-spec doctor --config config/config.toml
python3 scripts/mcp_smoke.py
```

Use `--locked` in reproducible release pipelines. For a focused iteration:

```bash
cargo test -p bg-spec-indexer --test acceptance
cargo test -p bg-spec-mcp --test protocol
```

Unit tests cover manifest parsing, SHA-256, schema/operation diff facts,
Unicode-safe chunking, semantic boundaries, and request-frame limits.
Integration tests create temporary corpora and exercise real PDF extraction,
typed OpenAPI normalization, reference graphs, indexing/search, exact source
lookups, curated traces, conflict bundles, incremental updates, deleted
sources, atomic publication, read-only connections, and path confinement.
MCP tests use the official SDK over a duplex stream; the Python smoke test
additionally verifies the release binary's actual stdio and stdout purity.

Fixtures are synthetic, original, and deterministic. Do not commit copyrighted
specification PDFs, confidential bank data, credentials, or user databases.
`scripts/generate_fixture_pdfs.py` regenerates the two example PDFs and the
blank/OCR diagnostic fixture. `scripts/mcp_smoke.py --output
examples/responses.json` regenerates complete tool examples from the running
server. Review example diffs when changing output or normalization.

## Design invariants

Keep domain interpretation in consuming agents, not the server. Never add an
LLM dependency to extract business requirements. Every source-backed result
must retain its provenance. Do not silently infer authority, substitute a
different source/version, collapse conflicts, or label search hits as curated
requirements.

Keep write operations in the offline indexer/CLI. New runtime repositories
must implement read-only traits. Do not add an HTTP deployment mode without
an explicit requirement and threat-model review.

Use `Result` for expected errors, injected interfaces for document extraction,
and `spawn_blocking` for blocking work. Preserve stable ordering and bounded
graph traversal. Avoid panics, unsafe code, broad error swallowing, silent
fallbacks, and host-path disclosure.

## Updating storage and dependencies

SQLite migrations live in `crates/bg-spec-store/migrations/`. Bump catalog
format and pipeline revisions when changing stored semantics; publish a fresh
generation. Never migrate a live immutable generation in place. After updating
tokenizers, parsers, or normalization, invalidate reused document extraction.

The known upstream constraint is `pdf_oxide` 0.3.78 with `office_oxide` 0.1.9.
`office_oxide` 0.1.12 does not compile with this PDF release. Remove the lock
constraint only after confirming a compatible release and rerunning extraction,
acceptance, and release builds. This is an upstream API compatibility pin, not
a replacement of the required PDF implementation.

## Explicit limitations to preserve or improve

OpenAPI 3.0.x only; no remote reference resolution or nested schema-pointer
references. Structural diffs are not semantic equivalence or compatibility
proofs. OCR-required detection is conservative and OCR is not implemented.
Heading/table extraction is best effort. Natural-language conflicts require
curation/agent review.

The indexer currently assembles normalized records in memory and reconstructs
the search index when a generation changes. Benchmark against the actual
official corpus before setting production memory/startup/search SLOs.
Published generations consume disk until the operator safely retires them.
No verified latency or large-corpus scalability claim follows from small
synthetic fixtures.
