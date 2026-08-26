#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

cargo build --release -p mcp-manager

app="$root/target/release/bundle/macos/MCP Manager.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/mcp-manager" "$app/Contents/MacOS/mcp-manager"
cp "$root/apps/desktop/macos/Info.plist" "$app/Contents/Info.plist"
cp "$root/apps/desktop/icons/icon.icns" "$app/Contents/Resources/icon.icns"

dmg_dir="$root/target/release/bundle/dmg"
mkdir -p "$dmg_dir"
dmg="$dmg_dir/MCP Manager_0.1.0_aarch64.dmg"
rm -f "$dmg"
hdiutil create -volname "MCP Manager" -srcfolder "$app" -ov -format UDZO "$dmg"
echo "wrote $app"
echo "wrote $dmg"
