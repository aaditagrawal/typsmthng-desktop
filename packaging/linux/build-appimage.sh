#!/usr/bin/env bash
set -euo pipefail

command -v linuxdeploy >/dev/null 2>&1 || { echo "linuxdeploy is required" >&2; exit 1; }
command -v linuxdeploy-plugin-gtk >/dev/null 2>&1 || { echo "linuxdeploy-plugin-gtk is required" >&2; exit 1; }
repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
appdir="$repo_root/build/Typsmthng.AppDir"
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$repo_root/build/release"
"$repo_root/scripts/build-gtk.sh"
install -m755 "$repo_root/target/release/typsmthng" "$appdir/usr/bin/typsmthng"
command -v typst >/dev/null 2>&1 || { echo "Typst 0.15.1 is required for packaging" >&2; exit 1; }
install -m755 "$(command -v typst)" "$appdir/usr/bin/typst"
install -Dm644 "$repo_root/native/gtk/data/language-specs/typst.lang" "$appdir/usr/share/typsmthng/language-specs/typst.lang"
install -Dm644 "$repo_root/assets/typst.xml" "$appdir/usr/share/mime/packages/typsmthng.xml"
DEPLOY_GTK_VERSION=4 linuxdeploy --appdir "$appdir" \
  --desktop-file "$repo_root/packaging/linux/dev.typsmthng.Typsmthng.desktop" \
  --icon-file "$repo_root/assets/icon.png" --plugin gtk --output appimage
mv ./*.AppImage "$repo_root/build/release/"
