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
# GtkSourceView styles/languages and symbolic icons are runtime data and are
# not found by linuxdeploy's ELF dependency traversal.
cp -R "$(pkg-config --variable=prefix gtksourceview-5)/share/gtksourceview-5" "$appdir/usr/share/"
mkdir -p "$appdir/usr/share/icons/hicolor/512x512/apps"
cp -R /usr/share/icons/Adwaita "$appdir/usr/share/icons/"
icon="$appdir/usr/share/icons/hicolor/512x512/apps/dev.typsmthng.Typsmthng.png"
install -m644 "$repo_root/icon.iconset/icon_512x512.png" "$icon"
query_loaders="$(pkg-config --variable=gdk_pixbuf_binarydir gdk-pixbuf-2.0)/../gdk-pixbuf-query-loaders"
install -m755 "$query_loaders" "$appdir/usr/bin/gdk-pixbuf-query-loaders"
output_dir="$repo_root/build/appimage-output"
mkdir -p "$output_dir"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/native/gtk/Cargo.toml" | head -1)"
cd "$output_dir"
output="typsmthng-$version-linux-x64.AppImage"
OUTPUT="$output" DEPLOY_GTK_VERSION=4 linuxdeploy --appdir "$appdir" \
  --desktop-file "$repo_root/packaging/linux/dev.typsmthng.Typsmthng.desktop" \
  --icon-file "$icon" --plugin gtk --output appimage
mv "$output" "$repo_root/build/release/"
