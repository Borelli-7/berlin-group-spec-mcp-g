# Configuration

The binaries accept `--config PATH`; default: `config/config.toml`.
Paths in TOML are relative to the configuration file, not the caller's cwd.
Unknown TOML fields are rejected.

```toml
corpus_dir = "../corpus"
data_dir = "../data"
manifest = "manifest.yaml"          # relative to corpus_dir
chunk_chars = 6000
chunk_overlap = 500
max_document_bytes = 134217728
v1 = "nextgenpsd2-v1"
v2 = "openfinance-v2"
```

| Setting | Constraint |
|---|---|
| `corpus_dir` | Existing directory containing all source files |
| `data_dir` | Writable by indexer; existing generations readable by MCP |
| `manifest` | Nonempty relative path without `.` / `..` components |
| `chunk_chars` | 256..100000 Unicode scalar values |
| `chunk_overlap` | Nonnegative, strictly less than `chunk_chars` |
| `max_document_bytes` | Positive; default 128 MiB per file |
| `v1`, `v2` | Distinct nonempty version labels; comparison defaults only |

Source paths are relative to `corpus_dir`, never to the process cwd. Absolute
paths and traversal are rejected. Canonicalized paths, including symlink
targets, must remain within the corpus. Keep `data_dir` outside `corpus_dir`.

## Runtime limits

Search limit: 1..100 (default 10); input query: 2048 bytes and at most 128
distinct tokens. Version strings: 128 bytes; source/requirement IDs: 256 bytes;
paths: 2048 bytes; locators: 4096 bytes. Strings must not contain control
characters. HTTP methods are case-insensitive and normalized to uppercase.
Tool input structs reject unknown fields.

The stdio binary rejects incoming JSON-RPC lines above 64 KiB and closes that
transport, logging the reason. Up to eight tool requests execute concurrently
per process; excess requests return `busy` for the caller to retry. Structured
tool output is limited to 8 MiB; oversized bundles return `response_too_large`
instead of truncating evidence silently. Use individual source/schema lookups
or narrower searches in that case.

PDFs are limited to 10000 pages, 2 MB extracted text per page, and 64 MB
extracted text per document. These limits detect excessive output, not every
possible parser CPU/memory attack. Apply OS resource limits to the offline
indexer for documents that have not yet been trusted.

## Copilot CLI

The persistent configuration file is `~/.copilot/mcp-config.json`.
Merge `config/copilot-mcp-config.json` into its `mcpServers` object. The example
is valid for this checkout and intentionally does not overwrite user settings.
Update both binary/config absolute paths if you relocate it.

Repository configuration is `.mcp.json` (included); `.github/mcp.json` is also
recognized by Copilot CLI. Do not use VS Code's `servers` wrapper: Copilot
configuration uses `mcpServers`. Project discovery requires a trusted Git
repository; persistent configuration works independently of this checkout's
Git status.

All nine domain tools plus operational `health` are explicitly allowlisted.
No secrets or environment variables are required. For debug logs, add
`"RUST_LOG": "bg_spec_mcp=debug,bg_spec_store=debug"` to the server's `env`;
logs still use stderr. Avoid logging corpus contents in shared terminals.

Official setup reference:
<https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers>

## Troubleshooting

| Symptom | Action |
|---|---|
| Missing `CURRENT` / catalog | Run `bg-spec index` before starting MCP |
| Source or locator not found | Check `list_sources`, indexed version, exact one-based page/locator |
| `ambiguous_source` | Use `get_endpoint_requirements` to inspect all contracts; curate the manifest rather than selecting by precedence |
| `ocr_required` | Obtain a text-bearing document or add a reviewed OCR extractor; no OCR is automatically attempted |
| `index_integrity` | Run `bg-spec doctor`; do not edit published SQLite/Tantivy files |
| Source hash drift | Re-index, then restart MCP processes that need new evidence |
| Writer lock busy | Wait for the current indexer; OS releases locks on process exit |
| Copilot does not discover project config | Confirm trusted Git repository or merge persistent user config |
| External `$ref` rejected | Bundle references offline into named local components, retaining the original and conversion provenance |
| Unsupported OpenAPI version | Supply an explicitly reviewed OpenAPI 3.0.x representation; never relabel a 3.1 file as 3.0 |
