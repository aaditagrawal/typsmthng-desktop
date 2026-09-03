#!/usr/bin/env bash
# Sourced by Linux packaging scripts. Expects ROOT_DIR, APP_NAME, ENV, BUILD_DIR.
# Sets APP_DIR to the unpacked Electrobun linux app (bin/launcher + bun + native libs).
#
# `electrobun build --env=stable|canary` tars the full app, then replaces the
# bundle folder with a self-extracting stub. System packages need the full app.

linux_bundle_name() {
  if [[ "$ENV" == "stable" ]]; then
    printf '%s\n' "$APP_NAME"
  else
    printf '%s\n' "${APP_NAME}-${ENV}"
  fi
}

is_full_linux_app() {
  local dir="$1"
  [[ -f "$dir/bin/launcher" && -f "$dir/bin/bun" && -f "$dir/bin/libNativeWrapper.so" ]]
}

find_zig_zstd() {
  local candidate
  for candidate in \
    "$ROOT_DIR/node_modules/electrobun/dist-linux-x64/zig-zstd" \
    "$ROOT_DIR/node_modules/electrobun/dist-linux-arm64/zig-zstd" \
    "$ROOT_DIR/node_modules/electrobun/dist-macos-arm64/zig-zstd" \
    "$ROOT_DIR/node_modules/electrobun/dist-macos-x64/zig-zstd"
  do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

unpack_zstd_tar() {
  local src="$1" dest="$2"
  mkdir -p "$dest"
  if command -v zstd &>/dev/null; then
    zstd -d -c "$src" | tar -xf - -C "$dest"
    return
  fi
  local zig_zstd
  zig_zstd="$(find_zig_zstd)" || true
  if [[ -n "$zig_zstd" ]]; then
    local tar_path="${src%.zst}"
    "$zig_zstd" decompress -i "$src" -o "$tar_path"
    tar -xf "$tar_path" -C "$dest"
    rm -f "$tar_path"
    return
  fi
  echo "Error: need zstd (or electrobun zig-zstd) to unpack $src"
  exit 1
}

ensure_linux_app_dir() {
  local bundle expected unpacked zst
  bundle="$(linux_bundle_name)"
  expected="$BUILD_DIR/${ENV}-linux-x64/$bundle"
  unpacked="$BUILD_DIR/${ENV}-linux-x64/${bundle}-full"

  if [[ ! -e "$expected" && ! -f "$BUILD_DIR/${ENV}-linux-x64/${bundle}.tar.zst" ]]; then
    echo "==> Missing $expected — building Electrobun linux-x64 --env=$ENV"
    (cd "$ROOT_DIR" && bunx electrobun build --env="$ENV" --targets linux-x64)
  fi

  if is_full_linux_app "$expected"; then
    APP_DIR="$expected"
  elif is_full_linux_app "$unpacked/$bundle"; then
    APP_DIR="$unpacked/$bundle"
  elif is_full_linux_app "$unpacked"; then
    APP_DIR="$unpacked"
  else
    zst="$BUILD_DIR/${ENV}-linux-x64/${bundle}.tar.zst"
    if [[ ! -f "$zst" ]]; then
      zst="$(find "$expected/Resources" -maxdepth 1 -name '*.tar.zst' -type f 2>/dev/null | head -1 || true)"
    fi
    if [[ -z "$zst" || ! -f "$zst" ]]; then
      echo "Error: full Electrobun linux app not found for --env=$ENV"
      echo "Expected either $expected/bin/{launcher,bun,libNativeWrapper.so}"
      echo "or $BUILD_DIR/${ENV}-linux-x64/${bundle}.tar.zst"
      echo "Contents of $BUILD_DIR:"
      ls -la "$BUILD_DIR" 2>/dev/null || echo "  (directory does not exist)"
      ls -la "$BUILD_DIR/${ENV}-linux-x64" 2>/dev/null || true
      exit 1
    fi
    echo "==> Unpacking $zst (stable/canary linux output is a self-extracting stub)"
    rm -rf "$unpacked"
    unpack_zstd_tar "$zst" "$unpacked"
    if is_full_linux_app "$unpacked/$bundle"; then
      APP_DIR="$unpacked/$bundle"
    elif is_full_linux_app "$unpacked"; then
      APP_DIR="$unpacked"
    else
      echo "Error: unpacked $zst but did not find bin/launcher + bun + libNativeWrapper.so"
      find "$unpacked" -maxdepth 3 -type f | head -40
      exit 1
    fi
  fi

  echo "==> Electrobun app directory: $APP_DIR"
  echo "==> Contents:"
  ls -la "$APP_DIR"
  ls -la "$APP_DIR/bin"
}
