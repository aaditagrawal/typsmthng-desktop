#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="typsmthng"
ENV="${ELECTROBUN_ENV:-stable}"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT_DIR="$BUILD_DIR/release"
STAGING_DIR="$BUILD_DIR/linux-native-staging"
NFPM_VERSION="2.47.0"
VERSION=$(grep '"version"' "$ROOT_DIR/package.json" | head -1 | sed 's/.*"\([0-9][^"]*\)".*/\1/')

PKG_VERSION="${VERSION%%-*}"
PRERELEASE=""
if [[ "$VERSION" == *-* ]]; then
  PRERELEASE="${VERSION#*-}"
fi

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

APP_DIR="$PLATFORM_DIR/$APP_NAME"
if [[ ! -d "$APP_DIR" ]]; then
  APP_DIR=$(find "$PLATFORM_DIR" -maxdepth 1 -type d ! -name "$(basename "$PLATFORM_DIR")" 2>/dev/null | head -1)
  if [[ -z "$APP_DIR" ]]; then
    APP_DIR="$PLATFORM_DIR"
  fi
fi

echo "==> Electrobun app directory: $APP_DIR"
echo "==> Contents:"
ls -la "$APP_DIR"

LAUNCHER="$APP_DIR/bin/launcher"
if [[ ! -f "$LAUNCHER" ]]; then
  echo "Error: Electrobun launcher missing at $LAUNCHER"
  find "$APP_DIR" -maxdepth 2 -type f 2>/dev/null | sed 's/^/  /' || true
  exit 1
fi

ensure_nfpm() {
  if command -v nfpm &>/dev/null; then
    NFPM_BIN="$(command -v nfpm)"
    return
  fi

  local os arch asset
  case "$(uname -s)" in
    Darwin) os="Darwin" ;;
    Linux)  os="Linux" ;;
    *)
      echo "Error: nfpm auto-download is only wired for Darwin/Linux. Install nfpm and retry."
      exit 1
      ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="arm64" ;;
    *)
      echo "Error: unsupported architecture $(uname -m) for nfpm download"
      exit 1
      ;;
  esac

  asset="nfpm_${NFPM_VERSION}_${os}_${arch}.tar.gz"
  echo "==> Downloading nfpm ${NFPM_VERSION} ($asset)"
  mkdir -p "$BUILD_DIR"
  curl -fSL -o "$BUILD_DIR/$asset" \
    "https://github.com/goreleaser/nfpm/releases/download/v${NFPM_VERSION}/${asset}"
  tar -xzf "$BUILD_DIR/$asset" -C "$BUILD_DIR" nfpm
  chmod +x "$BUILD_DIR/nfpm"
  NFPM_BIN="$BUILD_DIR/nfpm"
}

ensure_nfpm

echo "==> Staging FHS tree at $STAGING_DIR"
rm -rf "$STAGING_DIR"
mkdir -p \
  "$STAGING_DIR/opt/$APP_NAME" \
  "$STAGING_DIR/usr/bin" \
  "$STAGING_DIR/usr/share/applications" \
  "$STAGING_DIR/usr/share/icons/hicolor/256x256/apps" \
  "$STAGING_DIR/usr/share/mime/packages"

cp -R "$APP_DIR"/. "$STAGING_DIR/opt/$APP_NAME/"
if [[ -d "$STAGING_DIR/opt/$APP_NAME/bin" ]]; then
  find "$STAGING_DIR/opt/$APP_NAME/bin" -type f -exec chmod +x {} \;
fi

cat > "$STAGING_DIR/usr/bin/$APP_NAME" <<WRAPPER
#!/bin/sh
APP_ROOT="/opt/${APP_NAME}"
cd "\$APP_ROOT" || exit 1
export LD_LIBRARY_PATH="\${APP_ROOT}/lib\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
exec "\${APP_ROOT}/bin/launcher" "\$@"
WRAPPER
chmod +x "$STAGING_DIR/usr/bin/$APP_NAME"

cat > "$STAGING_DIR/usr/share/applications/$APP_NAME.desktop" <<DESKTOP
[Desktop Entry]
Name=typsmthng
Exec=typsmthng %f
Icon=typsmthng
Type=Application
Terminal=false
Categories=Office;TextEditor;
Comment=Folder-backed Typst editor
MimeType=text/x-typst;inode/directory;
StartupWMClass=typsmthng
DESKTOP

ICON_SRC="$APP_DIR/Resources/appIcon.png"
if [[ ! -f "$ICON_SRC" ]]; then
  ICON_SRC="$ROOT_DIR/assets/icon.png"
fi
if [[ ! -f "$ICON_SRC" ]]; then
  echo "Error: No icon found for Linux packages."
  echo "Looked for: $APP_DIR/Resources/appIcon.png and $ROOT_DIR/assets/icon.png"
  exit 1
fi
cp "$ICON_SRC" "$STAGING_DIR/usr/share/icons/hicolor/256x256/apps/$APP_NAME.png"

if [[ ! -f "$ROOT_DIR/assets/typst.xml" ]]; then
  echo "Error: MIME definition missing at $ROOT_DIR/assets/typst.xml"
  exit 1
fi
cp "$ROOT_DIR/assets/typst.xml" "$STAGING_DIR/usr/share/mime/packages/${APP_NAME}-typst.xml"

NFPM_CONFIG="$BUILD_DIR/nfpm.yaml"
PRERELEASE_LINE=""
if [[ -n "$PRERELEASE" ]]; then
  PRERELEASE_LINE="prerelease: ${PRERELEASE}"
fi

cat > "$NFPM_CONFIG" <<YAML
name: ${APP_NAME}
arch: amd64
platform: linux
version: ${PKG_VERSION}
${PRERELEASE_LINE}
release: "1"
section: editors
priority: optional
maintainer: typsmthng <hello@typsmthng.dev>
description: Folder-backed Typst editor
vendor: typsmthng
homepage: https://github.com/aaditagrawal/typsmthng-desktop
license: UNLICENSED
contents:
  - src: ${STAGING_DIR}/opt/${APP_NAME}
    dst: /opt/${APP_NAME}
    type: tree
  - src: ${STAGING_DIR}/usr/bin/${APP_NAME}
    dst: /usr/bin/${APP_NAME}
    file_info:
      mode: 0755
  - src: ${STAGING_DIR}/usr/share/applications/${APP_NAME}.desktop
    dst: /usr/share/applications/${APP_NAME}.desktop
  - src: ${STAGING_DIR}/usr/share/icons/hicolor/256x256/apps/${APP_NAME}.png
    dst: /usr/share/icons/hicolor/256x256/apps/${APP_NAME}.png
  - src: ${STAGING_DIR}/usr/share/mime/packages/${APP_NAME}-typst.xml
    dst: /usr/share/mime/packages/${APP_NAME}-typst.xml
scripts:
  postinstall: ${ROOT_DIR}/installer/linux/postinstall.sh
  postremove: ${ROOT_DIR}/installer/linux/postremove.sh
overrides:
  deb:
    depends:
      - libgtk-3-0
      - libwebkit2gtk-4.1-0
      - libayatana-appindicator3-1
      - librsvg2-2
  rpm:
    depends:
      - gtk3
      - webkit2gtk4.1
      - libappindicator-gtk3
      - librsvg2
YAML

mkdir -p "$OUTPUT_DIR"
DEB_NAME="${APP_NAME}-${VERSION}-linux-x64.deb"
RPM_NAME="${APP_NAME}-${VERSION}-linux-x64.rpm"

echo "==> Building deb"
"$NFPM_BIN" package --packager deb --config "$NFPM_CONFIG" --target "$OUTPUT_DIR/$DEB_NAME"
echo "==> Building rpm"
"$NFPM_BIN" package --packager rpm --config "$NFPM_CONFIG" --target "$OUTPUT_DIR/$RPM_NAME"

rm -rf "$STAGING_DIR"

echo "==> Linux native packages created:"
echo "    DEB: $OUTPUT_DIR/$DEB_NAME"
echo "    RPM: $OUTPUT_DIR/$RPM_NAME"
