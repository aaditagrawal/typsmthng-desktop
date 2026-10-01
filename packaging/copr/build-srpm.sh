#!/usr/bin/env bash
# Network access is used only to prepare the SRPM, never during the RPM build.
set -euo pipefail
for tool in git cargo uv curl tar gzip sha256sum rpmbuild; do
  command -v "$tool" >/dev/null || { echo "Missing source RPM tool: $tool" >&2; exit 1; }
done
repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
output="${1:-$repo_root/build/copr}"
mkdir -p "$output"
output="$(cd "$output" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Always archive the committed source, not untracked files or local edits.
git -C "$repo_root" archive HEAD | tar -xf - -C "$work"
mkdir -p "$work/.cargo"
(cd "$work" && cargo vendor --locked --versioned-dirs vendor > .cargo/config.toml)
version="$(uv run --no-project "$repo_root/packaging/copr/prepare.py" "$work" "$work/vendor" "$work")"
archive="typst-x86_64-unknown-linux-musl.tar.xz"
curl --retry 3 -fsSL -o "$output/$archive" "https://github.com/typst/typst/releases/download/v0.15.1/$archive"
awk -v name="$archive" '$2 == name { print }' "$repo_root/packaging/typst-checksums.txt" > "$output/typst-SHA256SUMS"
(cd "$output" && sha256sum -c typst-SHA256SUMS)

git -C "$repo_root" archive --prefix="typsmthng-$version/" HEAD | gzip -n > "$output/typsmthng-$version.tar.gz"
# cargo vendor emits a relative directory because it runs inside the archive.
tar -czf "$output/typsmthng-$version-vendor.tar.gz" -C "$work" vendor .cargo bundled-licenses
cp "$work/typsmthng.spec" "$output/typsmthng.spec"
rpmbuild -bs "$output/typsmthng.spec" \
  --define "_sourcedir $output" --define "_srcrpmdir $output" \
  --define "_builddir $work/rpmbuild"
