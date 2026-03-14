#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARCH="${1:-arm64}"
ENV="${ELECTROBUN_ENV:-stable}"
APP_NAME="typsmthng"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT_DIR="$BUILD_DIR/release"
VERSION=$(grep '"version"' "$ROOT_DIR/package.json" | head -1 | sed 's/.*"\([0-9][^"]*\)".*/\1/')

# Electrobun outputs to build/{env}-macos-{arch}/ with app name suffixed by env
PLATFORM_DIR="$BUILD_DIR/${ENV}-macos-${ARCH}"
if [[ "$ENV" == "stable" ]]; then
  APP_BUNDLE_NAME="${APP_NAME}.app"
else
  APP_BUNDLE_NAME="${APP_NAME}-${ENV}.app"
fi
APP_BUNDLE="$PLATFORM_DIR/$APP_BUNDLE_NAME"

# Fallback: search for any .app in the build directory
if [[ ! -d "$APP_BUNDLE" ]]; then
  FOUND_APP=$(find "$BUILD_DIR" -maxdepth 2 -name "*.app" -type d 2>/dev/null | head -1)
  if [[ -n "$FOUND_APP" ]]; then
    APP_BUNDLE="$FOUND_APP"
    APP_BUNDLE_NAME=$(basename "$APP_BUNDLE")
    echo "==> Found app bundle at $APP_BUNDLE"
  else
    echo "Error: App bundle not found at $APP_BUNDLE"
    echo "Searched in $BUILD_DIR for .app bundles."
    echo "Contents of $BUILD_DIR:"
    ls -la "$BUILD_DIR" 2>/dev/null || echo "  (directory does not exist)"
    exit 1
  fi
fi

# Patch Info.plist for .typ file association
if [[ -f "$ROOT_DIR/scripts/post-build-macos.sh" ]]; then
  bash "$ROOT_DIR/scripts/post-build-macos.sh" "$ARCH"
fi

DMG_NAME="${APP_NAME}-${VERSION}-macos-${ARCH}.dmg"
ZIP_NAME="${APP_NAME}-${VERSION}-macos-${ARCH}.zip"

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
BUNDLE_PARENT=$(dirname "$APP_BUNDLE")
cd "$BUNDLE_PARENT"
ditto -c -k --keepParent "$APP_BUNDLE_NAME" "$OUTPUT_DIR/$ZIP_NAME"

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
