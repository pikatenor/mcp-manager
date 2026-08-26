# Desktop app

iced + tray-icon shell for MCP Manager. Closing the window hides to the menu bar; Quit from the tray exits.

From the **repository root**:

```bash
cargo run -p mcp-manager
```

macOS `.app` / DMG:

```bash
apps/desktop/bundle-macos.sh
```

See the root [README.md](../../README.md) for architecture, endpoint, and tests, and [AGENTS.md](../../AGENTS.md) for how to change this tree.
