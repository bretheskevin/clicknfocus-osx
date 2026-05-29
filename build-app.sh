#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Which architecture to build. Defaults to a universal (Intel + Apple Silicon)
# binary for local use; the CI workflow builds each arch separately.
ARCH="${1:-universal}"
case "$ARCH" in
  universal)           TARGETS=(x86_64-apple-darwin aarch64-apple-darwin); ZIP="clicknfocus-macos.zip" ;;
  intel|x86_64)        TARGETS=(x86_64-apple-darwin);  ZIP="clicknfocus-macos-intel.zip" ;;
  apple|arm64|aarch64) TARGETS=(aarch64-apple-darwin); ZIP="clicknfocus-macos-apple-silicon.zip" ;;
  *) echo "Usage: $0 [universal|intel|apple]" >&2; exit 1 ;;
esac

echo "==> Adding Rust target(s): ${TARGETS[*]}"
rustup target add "${TARGETS[@]}"

BINS=()
for t in "${TARGETS[@]}"; do
  echo "==> Building release for $t..."
  cargo build --release --target "$t"
  BINS+=("target/$t/release/clicknfocus-osx")
done

echo "==> Assembling binary with lipo..."
mkdir -p dist
# lipo with a single input simply copies it, so this works for both the
# universal build and the per-arch builds.
lipo -create -output dist/clicknfocus-osx "${BINS[@]}"

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

echo "==> Creating distribution zip: $ZIP"
(cd dist && zip -r -y "$ZIP" ClicknFocus.app install.sh uninstall.sh)

echo ""
echo "Done! Artifacts are in dist/:"
echo "  dist/ClicknFocus.app   — the signed app bundle ($ARCH)"
echo "  dist/$ZIP              — zip to share"
