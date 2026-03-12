#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARCH="${1:-arm64}"
APP_NAME="typsmthng"
BUILD_DIR="$ROOT_DIR/build"
APP_BUNDLE="$BUILD_DIR/$APP_NAME.app"
OUTPUT_DIR="$BUILD_DIR/release"
VERSION=$(grep '"version"' "$ROOT_DIR/package.json" | head -1 | sed 's/.*"\([0-9][^"]*\)".*/\1/')

DMG_NAME="${APP_NAME}-${VERSION}-macos-${ARCH}.dmg"
ZIP_NAME="${APP_NAME}-${VERSION}-macos-${ARCH}.zip"

if [[ ! -d "$APP_BUNDLE" ]]; then
  echo "Error: App bundle not found at $APP_BUNDLE"
  echo "Run 'electrobun build' first."
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

# Optional codesigning
if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  echo "==> Codesigning $APP_BUNDLE"
  codesign --force --deep --sign "$APPLE_SIGNING_IDENTITY" \
    --options runtime \
    --entitlements "${ENTITLEMENTS_PATH:-}" \
    "$APP_BUNDLE"
fi

# Create zip (used by auto-updater)
echo "==> Creating zip archive"
cd "$BUILD_DIR"
ditto -c -k --keepParent "$APP_NAME.app" "$OUTPUT_DIR/$ZIP_NAME"

# Create DMG
echo "==> Creating DMG"
DMG_TEMP="$BUILD_DIR/dmg-staging"
rm -rf "$DMG_TEMP"
mkdir -p "$DMG_TEMP"
cp -R "$APP_BUNDLE" "$DMG_TEMP/"
ln -s /Applications "$DMG_TEMP/Applications"

hdiutil create -volname "$APP_NAME" \
  -srcfolder "$DMG_TEMP" \
  -ov -format UDZO \
  "$OUTPUT_DIR/$DMG_NAME"

rm -rf "$DMG_TEMP"

# Optional notarization
if [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
  echo "==> Submitting for notarization"
  xcrun notarytool submit "$OUTPUT_DIR/$DMG_NAME" \
    --apple-id "$APPLE_ID" \
    --team-id "$APPLE_TEAM_ID" \
    --password "$APPLE_APP_PASSWORD" \
    --wait

  echo "==> Stapling notarization ticket"
  xcrun stapler staple "$OUTPUT_DIR/$DMG_NAME"
fi

echo "==> macOS packaging complete:"
echo "    DMG: $OUTPUT_DIR/$DMG_NAME"
echo "    ZIP: $OUTPUT_DIR/$ZIP_NAME"
