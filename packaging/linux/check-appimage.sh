#!/usr/bin/env bash
# Verify the shipped runtime without GTK development libraries or host Typst.
set -euo pipefail
appimage="$(realpath "${1:?AppImage path is required}")"
repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
docker run --rm \
  -v "$appimage:/tmp/typsmthng.AppImage:ro" \
  -v "$repo_root/native/gtk/tests/fixtures/demo:/tmp/demo:ro" \
  ubuntu:24.04 bash -euc '
    set -o pipefail
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq xvfb dbus-x11 libgl1 libegl1 libxkbcommon0 \
      fonts-dejavu-core fontconfig-config shared-mime-info \
      libxrender1 libxi6 libxrandr2 libxcursor1 libxinerama1
    if dpkg-query -W libgtk-4-1 >/dev/null 2>&1 || command -v typst; then
      echo "The clean runtime check must not have GTK 4 or Typst installed" >&2
      exit 1
    fi
    dbus-run-session -- xvfb-run -a -s "-screen 0 1920x1080x24" \
      env GTK_A11Y=none GSK_RENDERER=cairo G_DEBUG=fatal-warnings \
      timeout 45s /tmp/typsmthng.AppImage --appimage-extract-and-run \
      --presentation-smoke-test /tmp/demo 2>&1 | tee /tmp/presentation.log
    ! grep -E "Failed to load module|error while loading shared libraries" /tmp/presentation.log
  '
