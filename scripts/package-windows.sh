#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="typsmthng"
ENV="${ELECTROBUN_ENV:-stable}"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT_DIR="$BUILD_DIR/release"
VERSION=$(grep '"version"' "$ROOT_DIR/package.json" | head -1 | sed 's/.*"\([0-9][^"]*\)".*/\1/')

# unpack_zstd_tar / find_zig_zstd — Electrobun stable/canary wraps delete the
# unpacked Windows app after tarring (same as Linux). NSIS needs the full tree.
# shellcheck source=linux-app-dir.sh
. "$ROOT_DIR/scripts/linux-app-dir.sh"

# Electrobun outputs to build/{env}-win-x64/. Never silently package a
# `dev-win-*` tree as a stable/canary release — that happens when
# `--env=$ELECTROBUN_ENV` is expanded by PowerShell (empty) instead of bash.
PLATFORM_DIR="$BUILD_DIR/${ENV}-win-x64"
if [[ ! -d "$PLATFORM_DIR" ]]; then
  PLATFORM_DIR=$(find "$BUILD_DIR" -maxdepth 1 -name "${ENV}-win-*" -type d 2>/dev/null | head -1 || true)
fi
if [[ -z "${PLATFORM_DIR}" ]]; then
  PLATFORM_DIR="$BUILD_DIR/${ENV}-win-x64"
  mkdir -p "$PLATFORM_DIR"
fi

win_bundle_name() {
  if [[ "$ENV" == "stable" ]]; then
    printf '%s\n' "$APP_NAME"
  else
    printf '%s\n' "${APP_NAME}-${ENV}"
  fi
}

is_full_win_app() {
  local dir="$1"
  [[ -f "$dir/bin/launcher.exe" ]]
}

find_win_tarball() {
  shopt -s nullglob
  local candidates=(
    "$ROOT_DIR/artifacts/${ENV}-win-x64-${APP_NAME}.tar.zst"
    "$ROOT_DIR/artifacts/${ENV}-win-x64-"*.tar.zst
    "$PLATFORM_DIR/${APP_NAME}.tar.zst"
    "$PLATFORM_DIR/"*.tar.zst
  )
  shopt -u nullglob
  local f
  for f in "${candidates[@]}"; do
    if [[ -f "$f" ]]; then
      printf '%s\n' "$f"
      return 0
    fi
  done
  return 1
}

pick_win_app_dir() {
  local d bundle
  bundle="$(win_bundle_name)"
  if is_full_win_app "$PLATFORM_DIR/$bundle"; then
    APP_DIR="$PLATFORM_DIR/$bundle"
    return 0
  fi
  if is_full_win_app "$PLATFORM_DIR/$APP_NAME"; then
    APP_DIR="$PLATFORM_DIR/$APP_NAME"
    return 0
  fi
  shopt -s nullglob
  for d in "$PLATFORM_DIR"/*; do
    if is_full_win_app "$d"; then
      APP_DIR="$d"
      shopt -u nullglob
      return 0
    fi
  done
  shopt -u nullglob
  return 1
}

if ! pick_win_app_dir; then
  ZST="$(find_win_tarball || true)"
  if [[ -z "$ZST" ]]; then
    echo "Error: Electrobun ${ENV} Windows app not found in $BUILD_DIR"
    echo "Expected $PLATFORM_DIR/$(win_bundle_name)/bin/launcher.exe"
    echo "or artifacts/${ENV}-win-x64-*.tar.zst (stable/canary wraps delete the unpacked app)."
    echo "Contents of $BUILD_DIR:"
    ls -la "$BUILD_DIR" 2>/dev/null || echo "  (directory does not exist)"
    ls -la "$PLATFORM_DIR" 2>/dev/null || true
    ls -la "$ROOT_DIR/artifacts" 2>/dev/null || echo "  artifacts/ missing"
    exit 1
  fi
  UNPACKED="$PLATFORM_DIR/${APP_NAME}-full"
  echo "==> Unpacking $ZST (Electrobun ${ENV} Windows wrap deletes the unpacked app)"
  rm -rf "$UNPACKED"
  unpack_zstd_tar "$ZST" "$UNPACKED"
  if is_full_win_app "$UNPACKED/$(win_bundle_name)"; then
    APP_DIR="$UNPACKED/$(win_bundle_name)"
  elif is_full_win_app "$UNPACKED/$APP_NAME"; then
    APP_DIR="$UNPACKED/$APP_NAME"
  elif is_full_win_app "$UNPACKED"; then
    APP_DIR="$UNPACKED"
  else
    pick_win_app_dir || true
    if [[ -z "${APP_DIR:-}" ]] || ! is_full_win_app "$APP_DIR"; then
      echo "Error: unpacked $ZST but did not find bin/launcher.exe"
      find "$UNPACKED" -maxdepth 3 -type f | head -40
      exit 1
    fi
  fi
fi

echo "==> Electrobun app directory: $APP_DIR"

# Electrobun ships bin/launcher.exe; NSIS shortcuts target that path.
LAUNCHER_EXE="$APP_DIR/bin/launcher.exe"
if [[ ! -f "$LAUNCHER_EXE" ]]; then
  echo "Error: Electrobun launcher missing at $LAUNCHER_EXE"
  echo "installer/typsmthng.nsi shortcuts target bin\\launcher.exe"
  echo "Contents of $APP_DIR:"
  ls -la "$APP_DIR" 2>/dev/null || echo "  (directory does not exist)"
  if [[ -d "$APP_DIR/bin" ]]; then
    echo "Contents of $APP_DIR/bin:"
    ls -la "$APP_DIR/bin"
  fi
  find "$APP_DIR" -maxdepth 2 -type f -name '*.exe' 2>/dev/null | sed 's/^/  /' || true
  exit 1
fi
echo "==> Using Electrobun launcher: $LAUNCHER_EXE"

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

bash "$ROOT_DIR/scripts/collect-update-artifacts.sh" "$ENV" "win" "x64" "$OUTPUT_DIR"
