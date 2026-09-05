#!/usr/bin/env bash
set -euo pipefail

missing=()
for command_name in cargo rustc pkg-config; do
  command -v "$command_name" >/dev/null 2>&1 || missing+=("$command_name")
done

for module_name in gtk4 gtksourceview-5; do
  pkg-config --exists "$module_name" 2>/dev/null || missing+=("pkg-config:$module_name")
done

if ! command -v typst >/dev/null 2>&1 && [[ ! -x "native/gtk/vendor/typst/typst" ]] && [[ ! -x "native/gtk/vendor/typst/typst.exe" ]]; then
  missing+=("typst")
fi

if [[ "$(uname -s)" == "Darwin" ]] && command -v brew >/dev/null 2>&1; then
  [[ -d "$(brew --prefix)/share/icons/Adwaita" ]] || missing+=("adwaita-icon-theme")
fi

if ((${#missing[@]})); then
  echo "Missing GTK port prerequisites: ${missing[*]}" >&2
  case "$(uname -s)" in
    Darwin) echo "Install with: brew install rust gtk4 gtksourceview5 adwaita-icon-theme typst" >&2 ;;
    Linux) echo "Ubuntu/Debian: sudo apt install build-essential libgtk-4-dev libgtksourceview-5-dev pkg-config" >&2 ;;
  esac
  exit 1
fi

echo "rustc=$(rustc --version)"
echo "gtk=$(pkg-config --modversion gtk4)"
echo "gtksourceview=$(pkg-config --modversion gtksourceview-5)"
if command -v typst >/dev/null 2>&1; then
  echo "typst=$(typst --version)"
fi
