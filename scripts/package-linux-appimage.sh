#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="typsmthng"
ENV="${ELECTROBUN_ENV:-stable}"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT_DIR="$BUILD_DIR/release"
VERSION=$(grep '"version"' "$ROOT_DIR/package.json" | head -1 | sed 's/.*"\([0-9][^"]*\)".*/\1/')

APPIMAGE_NAME="${APP_NAME}-${VERSION}-linux-x64.AppImage"
APPDIR="$BUILD_DIR/AppDir"

# Electrobun outputs to build/{env}-linux-x64/{appname}/
PLATFORM_DIR="$BUILD_DIR/${ENV}-linux-x64"
if [[ ! -d "$PLATFORM_DIR" ]]; then
  PLATFORM_DIR=$(find "$BUILD_DIR" -maxdepth 1 -name "*-linux-*" -type d 2>/dev/null | head -1)
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
  # Try finding it
  APP_DIR=$(find "$PLATFORM_DIR" -maxdepth 1 -type d ! -name "$(basename "$PLATFORM_DIR")" 2>/dev/null | head -1)
  if [[ -z "$APP_DIR" ]]; then
    APP_DIR="$PLATFORM_DIR"
  fi
fi

echo "==> Electrobun app directory: $APP_DIR"
echo "==> Contents:"
ls -la "$APP_DIR"

# Ensure appimagetool is available
APPIMAGETOOL="${APPIMAGETOOL:-appimagetool}"
if ! command -v "$APPIMAGETOOL" &>/dev/null; then
  echo "==> Downloading appimagetool"
  APPIMAGETOOL="$BUILD_DIR/appimagetool"
  curl -fSL -o "$APPIMAGETOOL" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
  chmod +x "$APPIMAGETOOL"
fi

# Build AppDir structure
# Electrobun Linux layout: {appname}/bin/launcher, {appname}/Resources/, {appname}/lib/
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"

# Copy the entire electrobun app directory preserving structure
cp -R "$APP_DIR" "$APPDIR/usr/$APP_NAME"

# Create .desktop file
cat > "$APPDIR/$APP_NAME.desktop" <<DESKTOP
[Desktop Entry]
Name=typsmthng
Exec=$APP_NAME %f
Icon=$APP_NAME
Type=Application
Categories=Office;TextEditor;
Comment=Folder-backed Typst editor
MimeType=text/x-typst;
DESKTOP

# Install MIME type definition for .typ files
mkdir -p "$APPDIR/usr/share/mime/packages"
if [[ -f "$ROOT_DIR/assets/typst.xml" ]]; then
  cp "$ROOT_DIR/assets/typst.xml" "$APPDIR/usr/share/mime/packages/typsmthng-typst.xml"
fi

# Copy icon or create placeholder
ICON_SRC="$APP_DIR/Resources/appIcon.png"
if [[ ! -f "$ICON_SRC" ]]; then
  ICON_SRC="$ROOT_DIR/assets/icon.png"
fi
if [[ -f "$ICON_SRC" ]]; then
  cp "$ICON_SRC" "$APPDIR/$APP_NAME.png"
  cp "$ICON_SRC" "$APPDIR/usr/share/icons/hicolor/256x256/apps/$APP_NAME.png"
else
  echo "Warning: No icon found, AppImage will have no icon"
  printf '\x89PNG\r\n\x1a\n' > "$APPDIR/$APP_NAME.png"
fi

# Create AppRun that launches the electrobun launcher binary
cat > "$APPDIR/AppRun" <<APPRUN
#!/bin/bash
SELF=\$(readlink -f "\$0")
HERE=\${SELF%/*}
export LD_LIBRARY_PATH="\${HERE}/usr/${APP_NAME}/lib:\${LD_LIBRARY_PATH:-}"
exec "\${HERE}/usr/${APP_NAME}/bin/launcher" "\$@"
APPRUN
chmod +x "$APPDIR/AppRun"

# Build AppImage
mkdir -p "$OUTPUT_DIR"
ARCH=x86_64 "$APPIMAGETOOL" "$APPDIR" "$OUTPUT_DIR/$APPIMAGE_NAME"

rm -rf "$APPDIR"

echo "==> AppImage created: $OUTPUT_DIR/$APPIMAGE_NAME"
