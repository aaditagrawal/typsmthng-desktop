#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="typsmthng"
APP_ID="dev.typsmthng.desktop"
ENV="${ELECTROBUN_ENV:-stable}"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT_DIR="$BUILD_DIR/release"
VERSION=$(grep '"version"' "$ROOT_DIR/package.json" | head -1 | sed 's/.*"\([0-9][^"]*\)".*/\1/')

# Electrobun outputs to build/{env}-linux-x64/
PLATFORM_DIR="$BUILD_DIR/${ENV}-linux-x64"
if [[ ! -d "$PLATFORM_DIR" ]]; then
  PLATFORM_DIR=$(find "$BUILD_DIR" -maxdepth 1 -name "*-linux-*" -type d 2>/dev/null | head -1)
  if [[ -z "$PLATFORM_DIR" ]]; then
    echo "Error: Electrobun build output not found in $BUILD_DIR"
    exit 1
  fi
fi

FLATPAK_NAME="${APP_NAME}-${VERSION}-linux-x64.flatpak"
FLATPAK_BUILD_DIR="$BUILD_DIR/flatpak-build"
FLATPAK_REPO="$BUILD_DIR/flatpak-repo"
FLATPAK_STAGING="$BUILD_DIR/flatpak-staging"

if ! command -v flatpak-builder &>/dev/null; then
  echo "Error: flatpak-builder is not installed."
  echo "Install with: sudo apt install flatpak-builder"
  exit 1
fi

# Ensure runtime is available
flatpak install --noninteractive flathub org.freedesktop.Platform//23.08 org.freedesktop.Sdk//23.08 2>/dev/null || true

# Stage files so the manifest can reference a known path
rm -rf "$FLATPAK_STAGING"
mkdir -p "$FLATPAK_STAGING"
cp -R "$PLATFORM_DIR"/* "$FLATPAK_STAGING/"
cp "$ROOT_DIR/flatpak/dev.typsmthng.desktop.desktop" "$FLATPAK_STAGING/"

# Generate manifest pointing to the staging dir
MANIFEST="$BUILD_DIR/flatpak-manifest.yml"
cat > "$MANIFEST" <<MANIFEST_EOF
app-id: $APP_ID
runtime: org.freedesktop.Platform
runtime-version: "23.08"
sdk: org.freedesktop.Sdk
command: $APP_NAME

finish-args:
  - --share=ipc
  - --socket=x11
  - --socket=wayland
  - --socket=pulseaudio
  - --share=network
  - --filesystem=home
  - --device=dri

modules:
  - name: $APP_NAME
    buildsystem: simple
    build-commands:
      - install -Dm755 $APP_NAME -t /app/bin/ || cp -r * /app/bin/
      - cp -r views /app/bin/views 2>/dev/null || true
      - cp -r bun /app/bin/bun 2>/dev/null || true
      - install -Dm644 $APP_ID.desktop -t /app/share/applications/
    sources:
      - type: dir
        path: flatpak-staging
      - type: file
        path: flatpak-staging/$APP_ID.desktop
MANIFEST_EOF

mkdir -p "$OUTPUT_DIR"
rm -rf "$FLATPAK_BUILD_DIR" "$FLATPAK_REPO"

echo "==> Building Flatpak"
flatpak-builder --force-clean "$FLATPAK_BUILD_DIR" "$MANIFEST"

echo "==> Exporting to repo"
flatpak-builder --repo="$FLATPAK_REPO" --force-clean "$FLATPAK_BUILD_DIR" "$MANIFEST"

echo "==> Creating Flatpak bundle"
flatpak build-bundle "$FLATPAK_REPO" "$OUTPUT_DIR/$FLATPAK_NAME" "$APP_ID"

rm -rf "$FLATPAK_BUILD_DIR" "$FLATPAK_REPO" "$FLATPAK_STAGING" "$MANIFEST"

echo "==> Flatpak created: $OUTPUT_DIR/$FLATPAK_NAME"
