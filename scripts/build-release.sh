#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PLATFORM="${1:-}"
ARCH="${2:-x64}"
FORMAT="${3:-}"

if [[ -z "$PLATFORM" ]]; then
  case "$(uname -s)" in
    Darwin) PLATFORM="macos" ;;
    Linux)  PLATFORM="linux" ;;
    MINGW*|MSYS*|CYGWIN*) PLATFORM="win" ;;
    *) echo "Could not detect platform. Usage: $0 <macos|linux|win> [arch] [format]"; exit 1 ;;
  esac
fi

if [[ "$PLATFORM" == "macos" && -z "$ARCH" ]]; then
  case "$(uname -m)" in
    arm64) ARCH="arm64" ;;
    *)     ARCH="x64" ;;
  esac
fi

echo "==> Building for $PLATFORM-$ARCH (format: ${FORMAT:-default})"

# Step 1: Build native dylib (macOS only)
if [[ "$PLATFORM" == "macos" && -f "$ROOT_DIR/scripts/build-macos-effects.sh" ]]; then
  echo "==> Building native macOS effects dylib"
  bash "$ROOT_DIR/scripts/build-macos-effects.sh"
fi

# Step 2: Build frontend assets
echo "==> Building frontend (vite)"
bun run --cwd "$ROOT_DIR" vite build

# Step 3: Build Electrobun app
echo "==> Building Electrobun app"
bunx electrobun build --targets "$PLATFORM-$ARCH"

# Step 4: Package for distribution
SCRIPTS_DIR="$ROOT_DIR/scripts"

case "$PLATFORM" in
  macos)
    bash "$SCRIPTS_DIR/package-macos.sh" "$ARCH"
    ;;
  linux)
    bash "$SCRIPTS_DIR/package-linux-appimage.sh"
    ;;
  win)
    bash "$SCRIPTS_DIR/package-windows.sh"
    ;;
  *)
    echo "Unknown platform: $PLATFORM"
    exit 1
    ;;
esac

echo "==> Build complete for $PLATFORM-$ARCH"
