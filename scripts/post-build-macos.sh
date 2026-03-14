#!/usr/bin/env bash
set -euo pipefail

# Post-build script for macOS: adds CFBundleDocumentTypes to Info.plist
# so that .typ files can be associated with typsmthng.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV="${ELECTROBUN_ENV:-stable}"
ARCH="${1:-arm64}"
APP_NAME="typsmthng"
BUILD_DIR="$ROOT_DIR/build"

if [[ "$ENV" == "stable" ]]; then
  APP_BUNDLE_NAME="${APP_NAME}.app"
else
  APP_BUNDLE_NAME="${APP_NAME}-${ENV}.app"
fi

PLATFORM_DIR="$BUILD_DIR/${ENV}-macos-${ARCH}"
PLIST="$PLATFORM_DIR/$APP_BUNDLE_NAME/Contents/Info.plist"

if [[ ! -f "$PLIST" ]]; then
  # Fallback: search for any .app
  FOUND_APP=$(find "$BUILD_DIR" -maxdepth 2 -name "*.app" -type d 2>/dev/null | head -1)
  if [[ -n "$FOUND_APP" ]]; then
    PLIST="$FOUND_APP/Contents/Info.plist"
  else
    echo "Warning: Info.plist not found, skipping file association setup"
    exit 0
  fi
fi

echo "==> Patching Info.plist for .typ file association"

# Add CFBundleDocumentTypes using PlistBuddy
/usr/libexec/PlistBuddy -c "Add :CFBundleDocumentTypes array" "$PLIST" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :CFBundleDocumentTypes:0 dict" "$PLIST" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :CFBundleDocumentTypes:0:CFBundleTypeName string 'Typst Document'" "$PLIST" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :CFBundleDocumentTypes:0:CFBundleTypeRole string Editor" "$PLIST" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :CFBundleDocumentTypes:0:CFBundleTypeExtensions array" "$PLIST" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :CFBundleDocumentTypes:0:CFBundleTypeExtensions:0 string typ" "$PLIST" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :CFBundleDocumentTypes:0:LSHandlerRank string Default" "$PLIST" 2>/dev/null || true

echo "==> Info.plist patched successfully"
