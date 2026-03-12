#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="typsmthng"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT_DIR="$BUILD_DIR/release"
VERSION=$(grep '"version"' "$ROOT_DIR/package.json" | head -1 | sed 's/.*"\([0-9][^"]*\)".*/\1/')

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
  -DBUILD_DIR="$BUILD_DIR" \
  -DOUTPUT_DIR="$OUTPUT_DIR" \
  -DOUTPUT_NAME="$INSTALLER_NAME" \
  "$NSI_SCRIPT"

echo "==> Windows installer created: $OUTPUT_DIR/$INSTALLER_NAME"
