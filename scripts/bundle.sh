#!/bin/sh
# Builds Bezel and wraps it in Bezel.app with an ad-hoc signature, for running on this Mac.
set -eu
cd "$(dirname "$0")/.."
cargo build --locked --release -p bezel
app=target/release/Bezel.app
mkdir -p "$app/Contents/MacOS"
# Replace the executable's inode so a running copy keeps the previous build.
cp target/release/bezel "$app/Contents/MacOS/bezel.new"
mv -f "$app/Contents/MacOS/bezel.new" "$app/Contents/MacOS/bezel"
version=$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"name":"bezel","version":"\([^"]*\)".*/\1/p')
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>bezel</string>
  <key>CFBundleIdentifier</key><string>io.github.maxanderson95.bezel</string>
  <key>CFBundleName</key><string>Bezel</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$app"
codesign --verify --strict "$app"
echo "$app"
