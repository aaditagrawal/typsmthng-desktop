#!/usr/bin/env bash
set -euo pipefail
triple="${1:?Target triple is required}"
destination="${2:?Destination directory is required}"
packaging_root="$(cd "$(dirname "$0")" && pwd)"
case "$triple" in
  x86_64-pc-windows-msvc) extension=zip; binary=typst.exe ;;
  x86_64-unknown-linux-musl|aarch64-apple-darwin|x86_64-apple-darwin) extension=tar.xz; binary=typst ;;
  *) echo "Unsupported Typst target: $triple" >&2; exit 1 ;;
esac
archive="typst-$triple.$extension"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl --retry 3 -fsSL -o "$work/$archive" "https://github.com/typst/typst/releases/download/v0.15.1/$archive"
awk -v name="$archive" '$2 == name { print }' "$packaging_root/typst-checksums.txt" > "$work/SHA256SUMS"
test -s "$work/SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$work" && sha256sum -c SHA256SUMS)
else
  (cd "$work" && shasum -a 256 -c SHA256SUMS)
fi
if [[ "$extension" == zip ]]; then
  unzip -q "$work/$archive" -d "$work"
else
  tar -xJf "$work/$archive" -C "$work"
fi
mkdir -p "$destination"
install -m755 "$work/typst-$triple/$binary" "$destination/$binary"
