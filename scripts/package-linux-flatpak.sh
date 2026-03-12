#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="typsmthng"
APP_ID="dev.typsmthng.desktop"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT_DIR="$BUILD_DIR/release"
VERSION=$(grep '"version"' "$ROOT_DIR/package.json" | head -1 | sed 's/.*"\([0-9][^"]*\)".*/\1/')

FLATPAK_NAME="${APP_NAME}-${VERSION}-linux-x64.flatpak"
MANIFEST="$ROOT_DIR/flatpak/$APP_ID.yml"
FLATPAK_BUILD_DIR="$BUILD_DIR/flatpak-build"
FLATPAK_REPO="$BUILD_DIR/flatpak-repo"

if ! command -v flatpak-builder &>/dev/null; then
  echo "Error: flatpak-builder is not installed."
  echo "Install with: sudo apt install flatpak-builder"
  exit 1
fi

# Ensure runtime is available
flatpak install --noninteractive flathub org.freedesktop.Platform//23.08 org.freedesktop.Sdk//23.08 2>/dev/null || true

mkdir -p "$OUTPUT_DIR"
rm -rf "$FLATPAK_BUILD_DIR" "$FLATPAK_REPO"

echo "==> Building Flatpak"
flatpak-builder --force-clean "$FLATPAK_BUILD_DIR" "$MANIFEST"

echo "==> Exporting to repo"
flatpak-builder --repo="$FLATPAK_REPO" --force-clean "$FLATPAK_BUILD_DIR" "$MANIFEST"

echo "==> Creating Flatpak bundle"
flatpak build-bundle "$FLATPAK_REPO" "$OUTPUT_DIR/$FLATPAK_NAME" "$APP_ID"

rm -rf "$FLATPAK_BUILD_DIR" "$FLATPAK_REPO"

echo "==> Flatpak created: $OUTPUT_DIR/$FLATPAK_NAME"
