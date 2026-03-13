#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="typsmthng"
ENV="${ELECTROBUN_ENV:-stable}"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT_DIR="$BUILD_DIR/release"
VERSION=$(grep '"version"' "$ROOT_DIR/package.json" | head -1 | sed 's/.*"\([0-9][^"]*\)".*/\1/')

# Electrobun outputs to build/{env}-win-x64/
PLATFORM_DIR="$BUILD_DIR/${ENV}-win-x64"
if [[ ! -d "$PLATFORM_DIR" ]]; then
  PLATFORM_DIR=$(find "$BUILD_DIR" -maxdepth 1 -name "*-win-*" -type d 2>/dev/null | head -1)
  if [[ -z "$PLATFORM_DIR" ]]; then
    echo "Error: Electrobun build output not found in $BUILD_DIR"
    ls -la "$BUILD_DIR" 2>/dev/null || echo "  (directory does not exist)"
    exit 1
  fi
  echo "==> Found build output at $PLATFORM_DIR"
fi

# The app lives inside a subdirectory named after the app
APP_DIR="$PLATFORM_DIR/$APP_NAME"
if [[ ! -d "$APP_DIR" ]]; then
  APP_DIR=$(find "$PLATFORM_DIR" -maxdepth 1 -type d ! -name "$(basename "$PLATFORM_DIR")" 2>/dev/null | head -1)
  if [[ -z "$APP_DIR" ]]; then
    APP_DIR="$PLATFORM_DIR"
  fi
fi

echo "==> Electrobun app directory: $APP_DIR"

INSTALLER_NAME="${APP_NAME}-${VERSION}-win-x64-setup.exe"
NSI_SCRIPT="$ROOT_DIR/installer/typsmthng.nsi"

if ! command -v makensis &>/dev/null; then
  echo "Error: makensis (NSIS) is not installed."
  echo "Install with: choco install nsis  (or apt install nsis on Linux cross-compile)"
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

echo "==> Building Windows installer"
makensis \
  -DVERSION="$VERSION" \
  -DBUILD_DIR="$APP_DIR" \
  -DOUTPUT_DIR="$OUTPUT_DIR" \
  -DOUTPUT_NAME="$INSTALLER_NAME" \
  "$NSI_SCRIPT"

echo "==> Windows installer created: $OUTPUT_DIR/$INSTALLER_NAME"
