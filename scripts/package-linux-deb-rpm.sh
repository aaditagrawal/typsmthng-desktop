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

# shellcheck source=linux-app-dir.sh
. "$ROOT_DIR/scripts/linux-app-dir.sh"
ensure_linux_app_dir

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
  expected_sha=""
  case "${os}_${arch}" in
    Darwin_arm64) expected_sha="e8c9d1d9ac218eeed479375143dc46b8d51a2b8dbba8e2f9f15ecc8faa2e404b" ;;
    Darwin_x86_64) expected_sha="2b04108f8757313dde92ed729560845aadfb7782887eb6988a5dd96f9c146861" ;;
    Linux_arm64) expected_sha="1c0f5f2999b9a974bfb04fdb0cc3306096de530ac5dbb25d739cc5f5219c919c" ;;
    Linux_x86_64) expected_sha="0660ca602b2d2d2ae4781a06c692b3eeb9d437ffea05b831d76e41f4a3188783" ;;
  esac
  if [[ -z "$expected_sha" ]]; then
    echo "Error: no pinned SHA-256 for nfpm ${os}_${arch}"
    exit 1
  fi
  echo "==> Downloading nfpm ${NFPM_VERSION} ($asset)"
  mkdir -p "$BUILD_DIR"
  curl -fSL -o "$BUILD_DIR/$asset" \
    "https://github.com/goreleaser/nfpm/releases/download/v${NFPM_VERSION}/${asset}"
  actual_sha="$(
    if command -v sha256sum &>/dev/null; then
      sha256sum "$BUILD_DIR/$asset"
    else
      shasum -a 256 "$BUILD_DIR/$asset"
    fi | awk '{print $1}'
  )"
  if [[ "$actual_sha" != "$expected_sha" ]]; then
    echo "Error: nfpm checksum mismatch for $asset"
    echo "  expected $expected_sha"
    echo "  actual   $actual_sha"
    exit 1
  fi
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
BIN_DIR="\${APP_ROOT}/bin"
cd "\$BIN_DIR" || exit 1
export LD_LIBRARY_PATH="\${BIN_DIR}\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
exec "\${BIN_DIR}/launcher" "\$@"
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
license: MIT
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
      - libjavascriptcoregtk-4.1-0
      - libayatana-appindicator3-1
      - librsvg2-2
      - libsoup-3.0-0
  rpm:
    depends:
      - gtk3
      - webkit2gtk4.1
      - libayatana-appindicator-gtk3
      - librsvg2
      - libsoup3
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
