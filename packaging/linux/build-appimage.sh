#!/usr/bin/env bash
set -euo pipefail

command -v linuxdeploy >/dev/null 2>&1 || { echo "linuxdeploy is required" >&2; exit 1; }
command -v appimagetool >/dev/null 2>&1 || { echo "appimagetool is required" >&2; exit 1; }
command -v linuxdeploy-plugin-gtk >/dev/null 2>&1 || { echo "linuxdeploy-plugin-gtk is required" >&2; exit 1; }
repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
appdir="$repo_root/build/Typsmthng.AppDir"
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$repo_root/build/release"
"$repo_root/scripts/build-gtk.sh"
install -m755 "$repo_root/target/release/typsmthng" "$appdir/usr/bin/typsmthng"
install -m755 "$repo_root/target/release/typsmthng-updater" "$appdir/usr/bin/typsmthng-updater"
command -v typst >/dev/null 2>&1 || { echo "Typst 0.15.1 is required for packaging" >&2; exit 1; }
install -Dm644 "$repo_root/native/gtk/data/language-specs/typst.lang" "$appdir/usr/share/typsmthng/language-specs/typst.lang"
install -Dm644 -t "$appdir/usr/share/typsmthng/styles" "$repo_root"/native/gtk/data/styles/*.xml
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
# linuxdeploy excludes these common desktop libraries by default. Include them
# explicitly so the GTK runtime also works without a desktop development stack.
text_libraries=()
for soname in libfribidi.so.0 libfontconfig.so.1 libharfbuzz.so.0 libgraphite2.so.3; do
  library="$(ldconfig -p | awk -v name="$soname" '$1 == name && !found {print $NF; found=1}')"
  [[ -f "$library" ]] || { echo "Missing text runtime: $soname" >&2; exit 1; }
  text_libraries+=(--library "$library")
done
DEPLOY_GTK_VERSION=4 linuxdeploy --appdir "$appdir" "${text_libraries[@]}" \
  --desktop-file "$repo_root/packaging/linux/dev.typsmthng.Typsmthng.desktop" \
  --icon-file "$icon" --plugin gtk
# This editor does not use GtkVideo. The generic GTK plugin copies optional
# media modules without their GStreamer runtime, so exclude those modules.
if [[ -d "$appdir/usr/lib/gtk-4.0" ]]; then
  find "$appdir/usr/lib/gtk-4.0" -path '*/media/*' -type f -delete
fi
# Typst is a static PIE. patchelf's RPATH rewrite corrupts it, so add it only
# after linuxdeploy has finished processing the dynamic GTK runtime.
install -m755 "$(command -v typst)" "$appdir/usr/bin/typst"
"$appdir/usr/bin/typst" --version
cmp "$(command -v typst)" "$appdir/usr/bin/typst"
# GTK 4 and libadwaita choose the native display backend and system appearance.
# The generic plugin's GTK 3 compatibility overrides defeat both behaviors.
sed -i '/^export GTK_THEME=/d; /^export GDK_BACKEND=/d' "$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"
ARCH=x86_64 appimagetool "$appdir" "$output"
mv "$output" "$repo_root/build/release/"
