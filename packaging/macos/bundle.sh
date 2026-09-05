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
command -v typst >/dev/null 2>&1 || { echo "Typst 0.15.1 is required for packaging" >&2; exit 1; }
install -m755 "$(command -v typst)" "$contents/MacOS/typst"
minimum_system_version="$(sw_vers -productVersion | cut -d. -f1).0"
sed -e "s/@VERSION@/$version/g" -e "s/@MINIMUM_SYSTEM_VERSION@/$minimum_system_version/g" \
  "$repo_root/packaging/macos/Info.plist.in" > "$contents/Info.plist"
iconutil -c icns "$repo_root/icon.iconset" -o "$contents/Resources/typsmthng.icns"
mkdir -p "$contents/Resources/language-specs"
cp "$repo_root/native/gtk/data/language-specs/typst.lang" "$contents/Resources/language-specs/"

# Copy and rewrite the complete non-system dylib closure. GTK data resources
# are copied separately because they are discovered at runtime, not by dyld.
command -v dylibbundler >/dev/null 2>&1 || { echo "Install dylibbundler with Homebrew" >&2; exit 1; }
prefix="${PREFIX:-$(brew --prefix)}"
mkdir -p "$contents/Resources/share/glib-2.0" "$contents/Resources/share/icons" "$contents/Resources/lib"
cp -RL "$prefix/share/glib-2.0/schemas" "$contents/Resources/share/glib-2.0/"
cp -RL "$prefix/share/icons/Adwaita" "$contents/Resources/share/icons/"
for candidate in "$prefix/lib/gdk-pixbuf-2.0" "$prefix/lib/gtk-4.0"; do
  [[ -d "$candidate" ]] && cp -RL "$candidate" "$contents/Resources/lib/"
done
candidate="$prefix/lib/gio/modules"
if [[ -d "$candidate" ]]; then
  mkdir -p "$contents/Resources/lib/gio"
  cp -RL "$candidate" "$contents/Resources/lib/gio/"
fi
[[ -d "$prefix/share/gtksourceview-5" ]] && cp -RL "$prefix/share/gtksourceview-5" "$contents/Resources/share/"
query_loaders="$prefix/bin/gdk-pixbuf-query-loaders"
[[ -x "$query_loaders" ]] || { echo "gdk-pixbuf-query-loaders is required" >&2; exit 1; }
install -m755 "$query_loaders" "$contents/MacOS/gdk-pixbuf-query-loaders"
bundle_args=(-od -b -x "$contents/MacOS/typsmthng" -x "$contents/MacOS/gdk-pixbuf-query-loaders")
while IFS= read -r -d '' binary; do
  bundle_args+=(-x "$binary")
done < <(find "$contents/Resources/lib" -type f \( -name '*.so' -o -name '*.dylib' \) -print0)
printf 'quit\n' | dylibbundler "${bundle_args[@]}" -s "$prefix/lib" -d "$contents/Frameworks" -p @executable_path/../Frameworks/

# Unsigned distribution is an explicit release mode. Ad-hoc signing gives Apple
# Silicon valid local code signatures; it does not provide Developer ID trust.
signing_mode="${MACOS_SIGNING_MODE:-signed}"
case "$signing_mode" in
  signed)
    [[ -n "${MACOS_CODESIGN_IDENTITY:-}" ]] || { echo "MACOS_CODESIGN_IDENTITY is required for signed releases" >&2; exit 1; }
    codesign --force --deep --options runtime --timestamp --sign "$MACOS_CODESIGN_IDENTITY" "$app"
    ;;
  unsigned)
    codesign --force --deep --sign - "$app"
    ;;
  *) echo "MACOS_SIGNING_MODE must be signed or unsigned" >&2; exit 1 ;;
esac
"$repo_root/packaging/macos/verify-bundle.sh" "$app"
dmg_source="$(mktemp -d)"
trap 'rm -rf "$dmg_source"' EXIT
cp -R "$app" "$dmg_source/"
ln -s /Applications "$dmg_source/Applications"
hdiutil create -ov -volname "typsmthng $version" -srcfolder "$dmg_source" -format UDZO \
  "$repo_root/build/release/typsmthng-$version-macos-$arch.dmg"
