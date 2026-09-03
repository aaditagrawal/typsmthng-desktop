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
  [[ -f "$dir/bin/launcher" && -f "$dir/bin/bun" && -f "$dir/bin/libNativeWrapper.so" ]] || return 1
  # The stable/canary self-extracting stub also has launcher+bun+wrapper but only
  # a hashed tar.zst in Resources — require the unpacked app payload.
  [[ -d "$dir/Resources/app" || -f "$dir/Resources/app.asar" ]]
}

ensure_version_json() {
  local dest="$APP_DIR/Resources/version.json"
  mkdir -p "$APP_DIR/Resources"
  if [[ -f "$dest" ]]; then
    echo "==> Found $dest"
    return
  fi
  echo "==> Writing fallback $dest (Electrobun stub/tarball omitted it)"
  (cd "$ROOT_DIR" && bun "$ROOT_DIR/scripts/write-version-json.ts" "$dest")
}

find_zig_zstd() {
  local candidate
  for candidate in \
    "$ROOT_DIR/node_modules/electrobun/dist-linux-x64/zig-zstd" \
    "$ROOT_DIR/node_modules/electrobun/dist-linux-arm64/zig-zstd" \
    "$ROOT_DIR/node_modules/electrobun/dist-macos-arm64/zig-zstd" \
    "$ROOT_DIR/node_modules/electrobun/dist-macos-x64/zig-zstd" \
    "$ROOT_DIR/node_modules/electrobun/dist-win-x64/zig-zstd.exe"
  do
    if [[ -f "$candidate" && ( -x "$candidate" || "$candidate" == *.exe ) ]]; then
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
  if tar --zstd -xf "$src" -C "$dest" 2>/dev/null; then
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
      echo "Error: unpacked $zst but did not find bin/launcher + bun + libNativeWrapper.so and Resources/app"
      find "$unpacked" -maxdepth 3 -type f | head -40
      exit 1
    fi
  fi

  echo "==> Electrobun app directory: $APP_DIR"
  echo "==> Contents:"
  ls -la "$APP_DIR"
  ls -la "$APP_DIR/bin"
  ls -la "$APP_DIR/Resources" || true
  ensure_version_json
  if [[ ! -f "$APP_DIR/Resources/version.json" ]]; then
    echo "Error: Resources/version.json missing after ensure_version_json"
    exit 1
  fi
  echo "==> Electrobun version.json (from bin cwd): $APP_DIR/Resources/version.json"
  compile_linux_tray_stub
  bundle_linux_tray_libs
  wrap_linux_launcher
}

compile_linux_tray_stub() {
  local dest src
  src="$ROOT_DIR/native/linux/stub-ayatana-appindicator3.c"
  dest="$APP_DIR/lib/tray-stub"
  if [[ ! -f "$src" ]]; then
    echo "Error: tray stub source missing at $src"
    exit 1
  fi
  if ! command -v gcc >/dev/null 2>&1; then
    echo "Error: gcc is required to compile the optional-tray stub"
    exit 1
  fi
  mkdir -p "$dest"
  echo "==> Compiling optional tray stub into $dest"
  gcc -shared -fPIC -Wl,-soname,libayatana-appindicator3.so.1 \
    -o "$dest/libayatana-appindicator3.so.1" "$src"
}

# Copy Ayatana AppIndicator (and its small ayatana/dbusmenu deps) next to the
# launcher so AppImage / extracted trees work without a system tray package.
# GTK/WebKit stay system dependencies.
bundle_linux_tray_libs() {
  local dest="$APP_DIR/bin"
  local lib=""
  local line needed
  mkdir -p "$dest"

  if command -v ldconfig >/dev/null 2>&1; then
    lib="$(ldconfig -p 2>/dev/null | awk '/libayatana-appindicator3\.so\.1($| )/{print $NF; exit}')"
  fi
  if [[ -z "$lib" || ! -f "$lib" ]]; then
    for candidate in \
      /usr/lib/x86_64-linux-gnu/libayatana-appindicator3.so.1 \
      /usr/lib64/libayatana-appindicator3.so.1 \
      /usr/lib/libayatana-appindicator3.so.1
    do
      if [[ -f "$candidate" ]]; then
        lib="$candidate"
        break
      fi
    done
  fi

  if [[ -z "$lib" || ! -f "$lib" ]]; then
    echo "==> System libayatana-appindicator3.so.1 not found; packaged tree will use the tray stub if needed"
    return 0
  fi

  echo "==> Bundling tray library $lib → $dest"
  cp -L "$lib" "$dest/libayatana-appindicator3.so.1"

  if ! command -v ldd >/dev/null 2>&1; then
    return 0
  fi
  while IFS= read -r line; do
    needed="$(awk '{print $1}' <<<"$line")"
    lib="$(awk '{print $3}' <<<"$line")"
    case "$needed" in
      *ayatana*|*dbusmenu*|*ido3*)
        if [[ -n "$lib" && -f "$lib" ]]; then
          echo "    also $needed"
          cp -L "$lib" "$dest/$(basename "$lib")"
        fi
        ;;
    esac
  done < <(ldd "$dest/libayatana-appindicator3.so.1" 2>/dev/null || true)
}

wrap_linux_launcher() {
  local wrapper_src="$ROOT_DIR/scripts/linux-launcher-wrapper.sh"
  local launcher="$APP_DIR/bin/launcher"
  local real="$APP_DIR/bin/launcher.real"

  if [[ ! -f "$wrapper_src" ]]; then
    echo "Error: launcher wrapper missing at $wrapper_src"
    exit 1
  fi
  if [[ ! -e "$launcher" ]]; then
    echo "Error: Electrobun launcher missing at $launcher"
    exit 1
  fi

  if head -n 2 "$launcher" 2>/dev/null | grep -q "typsmthng-linux-launcher-wrapper"; then
    echo "==> Linux launcher already wrapped"
    if [[ ! -x "$real" ]]; then
      echo "Error: wrapped launcher is missing $real"
      exit 1
    fi
    return 0
  fi

  echo "==> Wrapping $launcher (optional tray; leave GDK backend to GTK)"
  mv "$launcher" "$real"
  chmod +x "$real"
  cp "$wrapper_src" "$launcher"
  chmod +x "$launcher"
}
