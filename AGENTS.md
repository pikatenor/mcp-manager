# Agent notes

This file is the working contract for agents in this repo. Product overview and human setup live in `README.md`.

## Product constraints

- **One inbound MCP endpoint:** Streamable HTTP at `http://127.0.0.1:8757/mcp`. Do not add public SSE or a stdio CLI unless explicitly asked.
- No `#[cfg(target_os = ...)]` in `mcp-core`. Platform I/O belongs in `mcp-platform`.

## Crate boundaries


| Crate          | May contain                                                                          | Must not contain                        |
| -------------- | ------------------------------------------------------------------------------------ | --------------------------------------- |
| `mcp-core`     | Models, aggregator, SQLite (no secret *values*), URL SSRF checks, SSE endpoint parse | `target_os`, keychain, browser, reqwest |
| `mcp-http`     | Axum router, Bearer auth, JSON-RPC `initialize` / `tools/list` / `tools/call`        | Spawning upstream servers               |
| `mcp-platform` | `SecretStore`, `AppPaths`, `BrowserOpener`                                           | Aggregator logic                        |
| `mcp-runtime`  | `McpConnector` (stdio / Streamable HTTP / SSE), `OAuthFlow`                          | Desktop UI                              |
| `apps/desktop` | iced daemon, tray-icon, window                                                       | Business rules that belong in crates    |


Public tool names are `{serverName}__{toolName}` (`TOOL_DELIMITER`). Missing/`true` in `tool_permissions` is public; `false` hides and blocks.

Secrets: env values, bearer, OAuth tokens → keychain via `server_env_key` / `server_bearer_key` / `server_oauth_key`. SQLite stores key *names* only. Remote URLs: https anywhere, http only to localhost; refuse link-local and `metadata.google.internal` (`validate_remote_url`).

OAuth loopback must stay on `http://127.0.0.1:<ephemeral>/oauth/callback`. After a successful flow, persist access token as the server bearer so `McpConnector` can use it.

## TDD

Default workflow for crates:

1. Write tests for the expected I/O. No production implementation yet (`unimplemented!()` is fine).
2. Run tests; confirm they fail for the right reason.
3. Implement until tests pass. **Do not change those tests** while implementing.
4. Commit the implementation and tests (short why-focused message).
5. `cargo clippy --all-targets -- -D warnings` on touched crates.

UI wiring is a thin shell over crate APIs; still keep crate logic test-first.

## Git

- Commits are the smallest revertible deploy unit. Message: 1–2 sentences, **why** not a file list.
- Background belongs in the PR/issue or `tmp-docs/`, not in the commit body.



## Commands

```bash
cargo test
cargo test -p mcp-core --lib
cargo test -p mcp-http
cargo test -p mcp-runtime
cargo clippy --all-targets -- -D warnings
cargo run -p mcp-manager
```

Lint for this repo is **clippy**, not golangci-lint.

## Do not

- Store secret values in SQLite, logs, or commit messages.
- Run GraphQL mutations or gRPC write RPCs (`Create*`, `Delete*`, `Update*`, `Set*`, `Put*`, `Add*`, `Remove*`, `Modify*`, `Insert*`, `Upsert*`, `Save*`, `Submit*`, `Generate*`, `Apply*`, `Replace*`, `Patch*`, `Register*`, `Upload*`, `Archive*`, `Assign*`, `Execute*`, `Run*`).
- Broaden inbound transports or add a second MCP SDK “just in case”. Upstream stdio/Streamable-HTTP connections ride on `rmcp` in `mcp-runtime`; the hand-rolled JSON-RPC client stays only for the deprecated legacy SSE transport.



## Follow-ups (only if asked)

JSON config import, writing Cursor/Claude `mcp.json`, request logs, OAuth refresh-on-start, Linux `mcp-platform` impl, public SSE endpoint, stdio CLI bridge.
