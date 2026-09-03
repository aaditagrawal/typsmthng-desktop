#!/bin/sh
# typsmthng-linux-launcher-wrapper
# Installed as bin/launcher next to Electrobun's launcher.real.
#
# Do not set GDK_BACKEND. Electrobun 1.15.1 forces the X11 GDK backend
# inside libNativeWrapper.so (blackboardsh/electrobun#281); no local knob.

BIN_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd) || exit 1
APP_ROOT=$(CDPATH= cd -- "$BIN_DIR/.." && pwd) || exit 1
REAL_LAUNCHER="$BIN_DIR/launcher.real"
STUB_DIR="$APP_ROOT/lib/tray-stub"

# Electrobun's zig launcher also prepends this directory to LD_LIBRARY_PATH.
LD_LIBRARY_PATH="${BIN_DIR}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export LD_LIBRARY_PATH

have_shared_lib() {
  name=$1
  if [ -e "$BIN_DIR/$name" ]; then
    return 0
  fi
  if [ -n "${LD_LIBRARY_PATH:-}" ]; then
    old_ifs=$IFS
    IFS=:
    for dir in $LD_LIBRARY_PATH; do
      IFS=$old_ifs
      if [ -n "$dir" ] && [ -e "$dir/$name" ]; then
        return 0
      fi
    done
    IFS=$old_ifs
  fi
  if command -v ldconfig >/dev/null 2>&1; then
    if ldconfig -p 2>/dev/null | grep -F -q "$name"; then
      return 0
    fi
  fi
  for dir in /usr/lib/x86_64-linux-gnu /usr/lib64 /lib/x86_64-linux-gnu /usr/lib /lib; do
    if [ -e "$dir/$name" ]; then
      return 0
    fi
  done
  return 1
}

if ! have_shared_lib libwebkit2gtk-4.1.so.0; then
  echo "typsmthng: missing runtime library libwebkit2gtk-4.1.so.0" >&2
  echo "Install it with:" >&2
  echo "  Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-0 libjavascriptcoregtk-4.1-0" >&2
  echo "  Fedora:        sudo dnf install webkit2gtk4.1" >&2
  exit 1
fi

if ! have_shared_lib libayatana-appindicator3.so.1; then
  if [ -e "$STUB_DIR/libayatana-appindicator3.so.1" ]; then
    echo "typsmthng: libayatana-appindicator3.so.1 not found; continuing without tray" >&2
    LD_LIBRARY_PATH="${STUB_DIR}:${LD_LIBRARY_PATH}"
    export LD_LIBRARY_PATH
  fi
fi

if [ ! -x "$REAL_LAUNCHER" ]; then
  echo "typsmthng: missing $REAL_LAUNCHER" >&2
  exit 1
fi

cd "$BIN_DIR" || exit 1
exec "$REAL_LAUNCHER" "$@"
