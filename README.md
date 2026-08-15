# MCP Manager

Desktop app that aggregates multiple MCP servers into a single localhost Streamable HTTP endpoint.

v1 targets macOS. OS-specific I/O lives in `crates/mcp-platform` so Linux can be added later.

## Layout

- `crates/mcp-core` — aggregator, tokens, tool filters (no OS APIs)
- `crates/mcp-http` — inbound Streamable HTTP `/mcp`
- `crates/mcp-platform` — secrets, paths, browser traits
- `apps/desktop` — Tauri 2 + React UI and tray

## Development

```bash
pnpm install
cargo test
pnpm --filter mcp-manager tauri dev
```

Default endpoint: `http://127.0.0.1:8757/mcp`
