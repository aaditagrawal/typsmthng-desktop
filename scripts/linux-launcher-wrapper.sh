#!/bin/sh
# typsmthng-linux-launcher-wrapper
# Installed as bin/launcher next to Electrobun's launcher.real.
#
# Do not set GDK_BACKEND. Electrobun 1.15.1 forces the X11 GDK backend
# inside libNativeWrapper.so (blackboardsh/electrobun#281); no local knob.
#
# WebKitGTK DMA-BUF renderer workaround
# ──────────────────────────────────────
# On Linux with the NVIDIA proprietary driver, WebKitGTK's DMA-BUF renderer
# (default since 2.44) fails to allocate GBM buffers and produces a blank or
# broken window:
#
#   Failed to create GBM buffer of size WxH: Invalid argument
#   KMS: DRM_IOCTL_MODE_CREATE_DUMB failed: Permission denied
#
# The standard workaround is WEBKIT_DISABLE_DMABUF_RENDERER=1, which forces
# the older shared-memory rendering path.  NOTE: on recent WebKitGTK (≥2.50)
# this env var also disables accelerated compositing and threaded scrolling, so
# setting it unconditionally would regress Intel/AMD/Mesa users.  We therefore
# apply it only when:
#   (a) the NVIDIA proprietary kernel module is loaded (/sys/module/nvidia), or
#   (b) a crash-marker file written by a previous failed launch is present.
#
# On Wayland + NVIDIA __NV_DISABLE_EXPLICIT_SYNC=1 is also needed (it has no
# compositing cost and can be applied unconditionally when NVIDIA is detected).
#
# A user who has already set WEBKIT_DISABLE_DMABUF_RENDERER in their
# environment retains their own value unchanged (the guard below skips sets
# when the variable is already exported).

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

# ── WebKitGTK DMA-BUF renderer guard ─────────────────────────────────────────
# Crash-marker location.  If a previous launch exited in <5 s (indicating a
# WebKitGTK initialisation crash rather than a normal quit), the marker file
# is left on disk and subsequent launches apply WEBKIT_DISABLE_DMABUF_RENDERER=1
# automatically.  A clean launch (exit after ≥5 s) removes the marker so it
# only stays active while the crash is reproducible.
_state_dir="${XDG_STATE_HOME:-${HOME:-/tmp}/.local/state}/typsmthng"
_dmabuf_crash_marker="$_state_dir/webkit-dmabuf-crash"

_has_nvidia=0
if [ -d /sys/module/nvidia ]; then
  _has_nvidia=1
fi

_need_dmabuf_disable=0
if [ "$_has_nvidia" -eq 1 ]; then
  _need_dmabuf_disable=1
elif [ -f "$_dmabuf_crash_marker" ]; then
  echo "typsmthng: DMA-BUF crash marker present; applying WEBKIT_DISABLE_DMABUF_RENDERER=1" >&2
  _need_dmabuf_disable=1
fi

if [ "$_need_dmabuf_disable" -eq 1 ]; then
  if [ -z "${WEBKIT_DISABLE_DMABUF_RENDERER+x}" ]; then
    WEBKIT_DISABLE_DMABUF_RENDERER=1
    export WEBKIT_DISABLE_DMABUF_RENDERER
  fi
  if [ "$_has_nvidia" -eq 1 ] && [ -z "${__NV_DISABLE_EXPLICIT_SYNC+x}" ]; then
    __NV_DISABLE_EXPLICIT_SYNC=1
    export __NV_DISABLE_EXPLICIT_SYNC
  fi
fi
# ─────────────────────────────────────────────────────────────────────────────

if [ ! -x "$REAL_LAUNCHER" ]; then
  echo "typsmthng: missing $REAL_LAUNCHER" >&2
  exit 1
fi

cd "$BIN_DIR" || exit 1

# When the DMA-BUF workaround has NOT already been applied, run the launcher
# as a subprocess so we can observe its exit.  If it crashes quickly (< 5 s)
# we write the crash marker and re-exec with WEBKIT_DISABLE_DMABUF_RENDERER=1.
# A clean long-running exit clears the marker so it does not persist forever.
# When the workaround is already active we exec directly (no extra fork).
if [ "$_need_dmabuf_disable" -eq 0 ] && [ -z "${WEBKIT_DISABLE_DMABUF_RENDERER+x}" ]; then
  mkdir -p "$_state_dir" 2>/dev/null || true
  _t0=$(date +%s 2>/dev/null) || _t0=0
  "$REAL_LAUNCHER" "$@"
  _rc=$?
  _t1=$(date +%s 2>/dev/null) || _t1=$((_t0 + 99))
  _elapsed=$((_t1 - _t0))
  if [ "$_rc" -ne 0 ] && [ "$_elapsed" -lt 5 ]; then
    echo "typsmthng: launcher exited with code $_rc after ${_elapsed}s — likely a WebKitGTK DMA-BUF crash; enabling workaround for next launch" >&2
    : > "$_dmabuf_crash_marker" 2>/dev/null || true
  else
    rm -f "$_dmabuf_crash_marker" 2>/dev/null || true
  fi
  exit "$_rc"
fi

exec "$REAL_LAUNCHER" "$@"
