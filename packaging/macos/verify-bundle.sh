#!/usr/bin/env bash
set -euo pipefail
app="${1:?Application bundle path is required}"
while IFS= read -r -d '' link; do
  [[ -e "$link" ]] || { echo "Broken bundle symlink: $link" >&2; exit 1; }
done < <(find "$app" -type l -print0)
while IFS= read -r -d '' binary; do
  identity="$(otool -D "$binary" | sed -n '2p')"
  while IFS= read -r dependency; do
    [[ "$dependency" != "$identity" ]] || continue
    case "$dependency" in
      @*|/usr/lib/*|/System/Library/*) ;;
      *) echo "Unbundled dependency in $binary: $dependency" >&2; exit 1 ;;
    esac
  done < <(otool -L "$binary" | tail -n +2 | awk '{print $1}')
done < <(find "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Resources/lib" -type f \( -name '*.so' -o -name '*.dylib' -o -perm -111 \) -print0)
codesign --verify --deep --strict "$app"
