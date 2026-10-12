#!/bin/sh
# Builds humon.app so humon shows up in the Dock with its icon.
#
#   scripts/bundle-macos.sh                  build, then target/release/humon.app
#   scripts/bundle-macos.sh BINARY APP_PATH  bundle an existing binary (CI uses a universal one)
set -eu
cd "$(dirname "$0")/.."

if [ $# -ge 2 ]; then
  binary=$1
  app=$2
else
  cargo build --release
  binary=target/release/humon
  app=target/release/humon.app
fi

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/humon"
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
# Ad-hoc signature so macOS runs it; not notarized (that needs an Apple developer account).
codesign --force --sign - "$app" >/dev/null 2>&1 || true
echo "Built $app ($version)"
