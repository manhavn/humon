#!/bin/sh
# Builds target/release/humon.app so humon shows up in the Dock with its icon.
set -eu
cd "$(dirname "$0")/.."

cargo build --release

app=target/release/humon.app
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/humon "$app/Contents/MacOS/humon"
cp assets/humon.icns "$app/Contents/Resources/humon.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>humon</string>
  <key>CFBundleDisplayName</key><string>humon</string>
  <key>CFBundleIdentifier</key><string>dev.muzaffer.humon</string>
  <key>CFBundleExecutable</key><string>humon</string>
  <key>CFBundleIconFile</key><string>humon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Ad-hoc signature so macOS runs it locally; not notarized.
codesign --force --sign - "$app" >/dev/null 2>&1 || true
echo "Built $app"
