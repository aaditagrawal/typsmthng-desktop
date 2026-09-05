#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
version="${1:-$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/native/gtk/Cargo.toml" | head -1)}"
arch="$(dpkg --print-architecture 2>/dev/null || uname -m)"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

"$repo_root/scripts/build-gtk.sh"
install -Dm755 "$repo_root/target/release/typsmthng" "$stage/usr/bin/typsmthng"
command -v typst >/dev/null 2>&1 || { echo "Typst 0.15.1 is required for packaging" >&2; exit 1; }
install -Dm755 "$(command -v typst)" "$stage/usr/lib/typsmthng/typst"
install -Dm644 "$repo_root/native/gtk/data/language-specs/typst.lang" "$stage/usr/share/typsmthng/language-specs/typst.lang"
install -Dm644 "$repo_root/packaging/linux/dev.typsmthng.Typsmthng.desktop" "$stage/usr/share/applications/dev.typsmthng.Typsmthng.desktop"
install -Dm644 "$repo_root/packaging/linux/dev.typsmthng.Typsmthng.metainfo.xml" "$stage/usr/share/metainfo/dev.typsmthng.Typsmthng.metainfo.xml"
install -Dm644 "$repo_root/icon.iconset/icon_512x512.png" "$stage/usr/share/icons/hicolor/512x512/apps/dev.typsmthng.Typsmthng.png"
install -Dm644 "$repo_root/assets/typst.xml" "$stage/usr/share/mime/packages/typsmthng.xml"
mkdir -p "$stage/DEBIAN" "$repo_root/build/release"
cat >"$stage/DEBIAN/postinst" <<'EOF'
#!/bin/sh
update-mime-database /usr/share/mime >/dev/null 2>&1 || true
EOF
cat >"$stage/DEBIAN/postrm" <<'EOF'
#!/bin/sh
update-mime-database /usr/share/mime >/dev/null 2>&1 || true
EOF
chmod 755 "$stage/DEBIAN/postinst" "$stage/DEBIAN/postrm"
cat >"$stage/DEBIAN/control" <<EOF
Package: typsmthng
Version: $version
Section: editors
Priority: optional
Architecture: $arch
Maintainer: typsmthng contributors
Depends: libgtk-4-1 (>= 4.6), libgtksourceview-5-0 (>= 5.4), librsvg2-common, adwaita-icon-theme
Description: Native GTK Typst editor and presentation studio
EOF
dpkg-deb --build --root-owner-group "$stage" "$repo_root/build/release/typsmthng_${version}_${arch}.deb"

if command -v fpm >/dev/null 2>&1; then
  rpm_arch="$arch"
  [[ "$rpm_arch" == "amd64" ]] && rpm_arch="x86_64"
  rpm_output="$repo_root/build/release/typsmthng_${version}_${rpm_arch}.rpm"
  fpm -s dir -t rpm -n typsmthng -v "$version" -a "$rpm_arch" -p "$rpm_output" --license MIT \
    --description "Native GTK Typst editor and presentation studio" \
    --after-install "$stage/DEBIAN/postinst" --after-remove "$stage/DEBIAN/postrm" \
    --depends gtk4 --depends gtksourceview5 --depends librsvg2 --depends adwaita-icon-theme \
    -C "$stage" usr/bin usr/lib usr/share
fi
