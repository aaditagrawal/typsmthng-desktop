#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
version="${1:-$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/native/gtk/Cargo.toml" | head -1)}"
arch="${2:-$(uname -m)}"
[[ "$arch" == "aarch64" ]] && arch="arm64"
[[ "$arch" == "x86_64" ]] && arch="x64"
app="$repo_root/build/release/typsmthng.app"
contents="$app/Contents"
rm -rf "$app"
mkdir -p "$contents/MacOS" "$contents/Resources" "$contents/Frameworks"
"$repo_root/scripts/build-gtk.sh"
install -m755 "$repo_root/target/release/typsmthng" "$contents/MacOS/typsmthng"
if command -v typst >/dev/null 2>&1; then install -m755 "$(command -v typst)" "$contents/MacOS/typst"; fi
sed "s/@VERSION@/$version/g" "$repo_root/packaging/macos/Info.plist.in" > "$contents/Info.plist"
iconutil -c icns "$repo_root/icon.iconset" -o "$contents/Resources/typsmthng.icns"
mkdir -p "$contents/Resources/language-specs"
cp "$repo_root/native/gtk/data/language-specs/typst.lang" "$contents/Resources/language-specs/"

# Copy and rewrite the complete non-system dylib closure. GTK data resources
# are copied separately because they are discovered at runtime, not by dyld.
command -v dylibbundler >/dev/null 2>&1 || { echo "Install dylibbundler with Homebrew" >&2; exit 1; }
dylibbundler -od -b -x "$contents/MacOS/typsmthng" -d "$contents/Frameworks" -p @executable_path/../Frameworks/
prefix="${PREFIX:-$(brew --prefix)}"
mkdir -p "$contents/Resources/share/glib-2.0" "$contents/Resources/share/icons" "$contents/Resources/lib"
cp -R "$prefix/share/glib-2.0/schemas" "$contents/Resources/share/glib-2.0/"
cp -R "$prefix/share/icons/Adwaita" "$contents/Resources/share/icons/"
for candidate in "$prefix/lib/gdk-pixbuf-2.0" "$prefix/lib/gtk-4.0"; do
  [[ -d "$candidate" ]] && cp -R "$candidate" "$contents/Resources/lib/"
done
for candidate in "$prefix/lib/gio/modules"; do
  [[ -d "$candidate" ]] && mkdir -p "$contents/Resources/lib/gio" && cp -R "$candidate" "$contents/Resources/lib/gio/"
done
[[ -d "$prefix/share/gtksourceview-5" ]] && cp -R "$prefix/share/gtksourceview-5" "$contents/Resources/share/"
query_loaders="$prefix/bin/gdk-pixbuf-query-loaders"
[[ -x "$query_loaders" ]] || { echo "gdk-pixbuf-query-loaders is required" >&2; exit 1; }
install -m755 "$query_loaders" "$contents/MacOS/gdk-pixbuf-query-loaders"
for binary in "$contents/MacOS/gdk-pixbuf-query-loaders" $(find "$contents/Resources/lib" -type f \( -name '*.so' -o -name '*.dylib' \)); do
  dylibbundler -od -b -x "$binary" -d "$contents/Frameworks" -p @executable_path/../Frameworks/
done

[[ -n "${MACOS_CODESIGN_IDENTITY:-}" ]] || { echo "MACOS_CODESIGN_IDENTITY is required" >&2; exit 1; }
codesign --force --deep --options runtime --timestamp --sign "$MACOS_CODESIGN_IDENTITY" "$app"
if command -v create-dmg >/dev/null 2>&1; then
  dmg_source="$(mktemp -d)"
  cp -R "$app" "$dmg_source/"
  create-dmg --volname "typsmthng $version" "$repo_root/build/release/typsmthng-$version-macos-$arch.dmg" "$dmg_source"
fi
