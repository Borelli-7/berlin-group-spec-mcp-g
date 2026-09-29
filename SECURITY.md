# Security model

## Boundaries

The MCP is a read-only evidence adapter, not an execution agent. Its entire
tool surface consists of the ten tools documented in `MCP_TOOLS.md`. There are
no arbitrary file, shell, network, write, delete, source-modification, indexing,
or application-code tools. MCP startup opens existing indexes only.

The indexer is a **separate privileged offline command**. It reads a curated
corpus and writes only its configured data directory; it does not rewrite
sources. Give MCP processes read-only access to configuration, corpus, and
published data. Give only the indexing identity write access to data. The
runtime can serve evidence from catalog snapshots without opening original
document files; `read_source` is not an arbitrary filesystem read.

The application enforces read-only SQLite connection options and opens no
Tantivy writer. This is **not an OS sandbox**: a process started by Copilot
inherits the user's ambient privileges. For a strong filesystem boundary,
deploy using a dedicated identity or a Linux mount/network sandbox with only
the release binary, required shared libraries, config/corpus/data mounted
read-only. Do not grant it access to home directories, credentials, application
repositories, or network devices. Mount/build data separately for the indexer.
No secrets are needed.

## Input and path confinement

Manifest/configuration files are operator-controlled. Source paths must be
relative, without traversal components; canonicalized targets must remain
inside the corpus. Symlink escapes are rejected. Source ID -> catalog lookup
-> exact stored locator is the only runtime evidence path. User-provided source
IDs and locators are SQL parameters, not SQL syntax or host paths.

Tantivy queries are literal tokens combined with separately constructed exact
filters, preventing caller-supplied query syntax from overriding source/version
constraints. Input schemas reject unknown fields, text lengths are checked,
and search/result limits are enforced. Requests above 64 KiB close the stdio
transport. The service limits concurrent requests and rejects oversized outputs
with explicit errors rather than silently truncating provenance.

The manifest/corpus must not be concurrently mutated by an adversarial local
process during offline indexing: canonical path validation is not a kernel
capability-secure `openat` sandbox and cannot prevent every filesystem race.
Use a read-only corpus mount while indexing if that threat applies.

## Documents are untrusted data

Specification prose can contain prompt injection. The MCP's initialization
instructions tell clients to treat corpus text as data, not instructions.
Retrieved text is never executed by this server, but consuming agents must
also enforce that separation. Source traceability is not proof that a document
is trustworthy or officially normative.

PDF, YAML, and OpenAPI parsing happen only offline. `serde-saphyr` is used
instead of deprecated `serde_yaml`; YAML/JSON parse errors are not swallowed.
PDF page count/extracted text and source-byte limits bound common failures,
but third-party parsers can still consume excessive CPU/memory on pathological
inputs. Run unreviewed-document indexing with OS memory/CPU limits and a
network-disabled identity. No OCR model downloads or cloud extraction occur.
Remote/external OpenAPI `$ref` values are rejected; URLs and examples inside
contracts are never fetched.

Our workspace forbids unsafe Rust. Dependencies may contain unsafe/native code;
that distinction is not a claim that the entire dependency graph is unsafe-free.
Review dependency updates and retain the tested lockfile.

## Integrity and publication

Published generations must never be modified in place: SQLite `immutable`
assumes that promise. One writer builds off to the side, validates, fsyncs, and
atomically changes `CURRENT`. Reader processes remain on their original
generation. Data directories need OS permissions preventing untrusted writes;
SHA-256 detects accidental drift but is not a digital signature or an
authentication mechanism.

Do not delete generations while readers may hold them. Backup/restore the
pointer and referenced catalog/search generation together. Generation IDs,
index schema, counts, SQLite integrity, and evidence hashes support diagnostics.
An attacker able to rewrite the catalog can also rewrite its hashes; use
operator-signed corpus distribution if authenticated provenance is required.

## Logging and failure handling

All server logs go to stderr. Stdout is reserved for MCP protocol messages.
Tool errors are explicit `isError` results or SDK protocol errors. Internal
storage errors are redacted from tool responses and available in stderr.
Do not collect debug logs into a shared place without checking for local paths
and specification-sensitive identifiers.

Index failures do not publish partial results. OCR-required pages are visible
diagnostics and cannot silently become empty authoritative evidence. Ambiguous
contracts are reported, not automatically selected by authority/precedence.

## Reporting

Report suspected issues privately to the deployment/repository maintainer,
including a minimal non-sensitive reproduction and affected versions. Do not
attach licensed official documents, credentials, or production banking data
without authorization. This implementation has regression coverage for its
boundaries; it is not a substitute for an independent deployment security audit.
