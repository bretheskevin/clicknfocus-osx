#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "==> Adding Rust targets for universal binary..."
rustup target add x86_64-apple-darwin aarch64-apple-darwin

echo "==> Building release for x86_64-apple-darwin..."
cargo build --release --target x86_64-apple-darwin

echo "==> Building release for aarch64-apple-darwin..."
cargo build --release --target aarch64-apple-darwin

echo "==> Creating universal binary with lipo..."
mkdir -p dist
lipo -create \
  -output dist/clicknfocus-osx \
  target/x86_64-apple-darwin/release/clicknfocus-osx \
  target/aarch64-apple-darwin/release/clicknfocus-osx

echo "==> Assembling ClicknFocus.app bundle..."
mkdir -p dist/ClicknFocus.app/Contents/MacOS
mkdir -p dist/ClicknFocus.app/Contents/Resources
cp packaging/Info.plist dist/ClicknFocus.app/Contents/Info.plist
cp dist/clicknfocus-osx dist/ClicknFocus.app/Contents/MacOS/clicknfocus-osx
chmod +x dist/ClicknFocus.app/Contents/MacOS/clicknfocus-osx

echo "==> Ad-hoc signing the app bundle..."
codesign --force --options runtime --sign - dist/ClicknFocus.app

echo "==> Verifying signature..."
codesign --verify --verbose dist/ClicknFocus.app

echo "==> Copying install/uninstall scripts into dist..."
cp install.sh dist/install.sh
cp uninstall.sh dist/uninstall.sh

echo "==> Creating distribution zip..."
(cd dist && zip -r -y clicknfocus-macos.zip ClicknFocus.app install.sh uninstall.sh)

echo ""
echo "Done! Artifacts are in dist/:"
echo "  dist/ClicknFocus.app          — the signed app bundle"
echo "  dist/clicknfocus-macos.zip    — zip to share with friends"
echo ""
echo "Share dist/clicknfocus-macos.zip — it contains the app, install.sh, and uninstall.sh."
