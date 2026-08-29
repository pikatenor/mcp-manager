#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

version="$(awk '/^\[workspace\.package\]/{in_tbl=1; next} /^\[/{in_tbl=0} in_tbl && $1=="version"{gsub(/"/, "", $3); print $3; exit}' "$root/Cargo.toml")"
if [[ -z "$version" ]]; then
  echo "could not read workspace version from Cargo.toml" >&2
  exit 1
fi

cargo build --release -p mcp-manager

app="$root/target/release/bundle/macos/MCP Manager.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/mcp-manager" "$app/Contents/MacOS/mcp-manager"
cp "$root/apps/desktop/macos/Info.plist" "$app/Contents/Info.plist"
cp "$root/apps/desktop/icons/icon.icns" "$app/Contents/Resources/icon.icns"

plist="$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$plist" ||
  /usr/libexec/PlistBuddy -c "Add :CFBundleShortVersionString string $version" "$plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$plist" ||
  /usr/libexec/PlistBuddy -c "Add :CFBundleVersion string $version" "$plist"

dmg_dir="$root/target/release/bundle/dmg"
mkdir -p "$dmg_dir"
dmg="$dmg_dir/MCP Manager_${version}_aarch64.dmg"
rm -f "$dmg"
hdiutil create -volname "MCP Manager" -srcfolder "$app" -ov -format UDZO "$dmg"
echo "wrote $app"
echo "wrote $dmg"
