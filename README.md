# MCP Manager

macOS menu-bar app that turns many MCP server configs into **one** localhost Streamable HTTP endpoint.

AI clients (Cursor, Claude, …) talk to:

```text
http://127.0.0.1:8757/mcp
```

The aggregator prefixes tools as `{serverName}__{toolName}`, hides private tools, and routes `tools/call` to the origin server.

v1 is **macOS only**. OS I/O (keychain, paths, browser) lives behind traits in `crates/mcp-platform` so Linux can be added later.

## What it does

- Local stdio MCP servers, remote Streamable HTTP, and legacy HTTP+SSE remotes
- Start / stop / auto-start, with env and bearer values in the macOS keychain (never SQLite)
- Per-client Bearer tokens (`mcpm_…`, SHA-256 in `tokens.db`, shown once)
- Per-tool public/private toggles
- Remote MCP OAuth (PKCE S256, loopback `http://127.0.0.1:<port>/oauth/callback`)
- Hide-on-close tray; quit and copy endpoint from the menu bar

**Not in v1:** projects, writing client `mcp.json`, marketplace, OAuth refresh-on-start, cross-platform `mcp-platform` impl (Linux, Windows), serving SSE as *our* public transport, stdio CLI bridge.

## Layout

```text
crates/mcp-core      OS-free models, aggregator, tokens, SQLite, SSE handshake
crates/mcp-http      inbound Axum /mcp (Bearer + JSON-RPC)
crates/mcp-platform  SecretStore, AppPaths, BrowserOpener (macOS impl)
crates/mcp-runtime   upstream connectors + OAuth flow
apps/desktop         iced + tray-icon UI (bundle id net.p1kachu.mcp-manager)
```

Bundle identifier: `net.p1kachu.mcp-manager`.

On disk (app data dir): `tokens.db` (hashed client tokens), `state.db` (server configs, env **key names** only). Secrets use keychain service `net.p1kachu.mcp-manager`.

## Development

Requires Rust 1.85+ and a macOS toolchain for the desktop app.

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -p mcp-manager
```

Crate-scoped tests:

```bash
cargo test -p mcp-core --lib
cargo test -p mcp-http
cargo test -p mcp-runtime
cargo test -p mcp-platform
```

Point a client at `http://127.0.0.1:8757/mcp` with `Authorization: Bearer <token>` issued in the UI.
Inbound transport is Streamable HTTP only (JSON-RPC `initialize`, `tools/list`, `tools/call`).

## License

MIT (see `Cargo.toml` workspace package).
