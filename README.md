# chatgpt-codex-tools-mcp (Rust)

Rust/Axum port of the local workspace-scoped MCP server. The original
TypeScript checkout remains the reference implementation; this directory is
the independent Rust implementation.

## Build and run

```bash
cargo check --locked
cargo test --locked
cargo build --release --locked
CTM_ALLOWED_ROOTS="$PWD" cargo run --release --locked
```

The MCP endpoint is `http://127.0.0.1:3333/mcp`; health is available at
`http://127.0.0.1:3333/healthz`. Keep the server on loopback and expose it only
through a private MCP tunnel.

## Profiles

`CTM_TOOL_PROFILE=legacy` exposes the workspace, Git, preview/confirm edit,
process, and optional web/SQLite tools. `CTM_TOOL_PROFILE=coding` exposes the
composite project tools (`open_project`, `search_code`, `read_files`,
`apply_patch`, checks, commands, and process management). The Codex bridge is
enabled by default in coding profile and can be disabled with
`CTM_CODEX_BRIDGE=false`.

`CTM_ACCESS_MODE=review` is read-only. `coding` enables bounded direct edits
and safe project checks. `full` allows the broader structured process policy;
shell executables and blocked dangerous patterns remain rejected.

Copy `config.example.json` to `config.json` for file-based configuration.
Environment variables override file values.

## Search engine

`search_code` uses an in-process Rust walker and line searcher, honors Git
ignore rules, and keeps the existing output and safety limits. Repeated ASCII
literal searches use a generation-aware trigram candidate index. A filesystem
watcher marks the index dirty when files change; exact matching still runs on
the selected files, so the index is only a candidate accelerator.
