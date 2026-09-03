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

# shellcheck source=linux-app-dir.sh
. "$ROOT_DIR/scripts/linux-app-dir.sh"
ensure_linux_app_dir

# Ensure appimagetool is available (pin to a stable release, not continuous)
APPIMAGETOOL="${APPIMAGETOOL:-appimagetool}"
if ! command -v "$APPIMAGETOOL" &>/dev/null; then
  echo "==> Downloading appimagetool 1.9.1"
  APPIMAGETOOL="$BUILD_DIR/appimagetool"
  curl -fSL -o "$APPIMAGETOOL" \
    "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage"
  chmod +x "$APPIMAGETOOL"
fi

# Build AppDir structure
# Electrobun Linux layout: {appname}/bin/launcher, {appname}/bin/*.so, {appname}/Resources/
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"

# Copy the entire electrobun app directory preserving structure
cp -R "$APP_DIR" "$APPDIR/usr/$APP_NAME"
if [[ ! -f "$APPDIR/usr/$APP_NAME/Resources/version.json" ]]; then
  echo "Error: AppDir missing usr/${APP_NAME}/Resources/version.json"
  echo "Electrobun 1.15.1 reads ../Resources/version.json from cwd .../usr/${APP_NAME}/bin"
  exit 1
fi

# Create .desktop file
cat > "$APPDIR/$APP_NAME.desktop" <<DESKTOP
[Desktop Entry]
Name=typsmthng
Exec=$APP_NAME %f
Icon=$APP_NAME
Type=Application
Categories=Office;TextEditor;
Comment=Folder-backed Typst editor
MimeType=text/x-typst;inode/directory;
DESKTOP

# Install MIME type definition for .typ files
mkdir -p "$APPDIR/usr/share/mime/packages"
if [[ -f "$ROOT_DIR/assets/typst.xml" ]]; then
  cp "$ROOT_DIR/assets/typst.xml" "$APPDIR/usr/share/mime/packages/typsmthng-typst.xml"
fi

# Copy icon (required)
ICON_SRC="$APP_DIR/Resources/appIcon.png"
if [[ ! -f "$ICON_SRC" ]]; then
  ICON_SRC="$ROOT_DIR/assets/icon.png"
fi
if [[ ! -f "$ICON_SRC" ]]; then
  echo "Error: No icon found for AppImage."
  echo "Looked for: $APP_DIR/Resources/appIcon.png and $ROOT_DIR/assets/icon.png"
  exit 1
fi
cp "$ICON_SRC" "$APPDIR/$APP_NAME.png"
cp "$ICON_SRC" "$APPDIR/usr/share/icons/hicolor/256x256/apps/$APP_NAME.png"

# Electrobun loads libNativeWrapper.so / libasar.so from the launcher directory
# (bin/), and bun:ffi also dlopens libNativeWrapper.so from process.cwd().
cat > "$APPDIR/AppRun" <<APPRUN
#!/bin/bash
SELF=\$(readlink -f "\$0")
HERE=\${SELF%/*}
APP="\${HERE}/usr/${APP_NAME}"
cd "\${APP}/bin" || exit 1
export LD_LIBRARY_PATH="\${APP}/bin\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
exec "\${APP}/bin/launcher" "\$@"
APPRUN
chmod +x "$APPDIR/AppRun"

# Build AppImage (extract-and-run avoids FUSE in CI/containers)
mkdir -p "$OUTPUT_DIR"
export APPIMAGE_EXTRACT_AND_RUN=1
ARCH=x86_64 "$APPIMAGETOOL" "$APPDIR" "$OUTPUT_DIR/$APPIMAGE_NAME"

rm -rf "$APPDIR"

echo "==> AppImage created: $OUTPUT_DIR/$APPIMAGE_NAME"

# The Release workflow only invokes this script for Linux. Build deb/rpm from
# the same Electrobun tree so they land in build/release/ with the AppImage.
if [[ "${SKIP_LINUX_NATIVE_PACKAGES:-}" != "1" ]]; then
  bash "$ROOT_DIR/scripts/package-linux-deb-rpm.sh"
fi
