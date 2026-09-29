#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "Usage: $0 VERSION OUTPUT_DIR CACHE_DIR" >&2
  exit 2
fi

version="$1"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; then
  echo "Invalid release version: $version" >&2
  exit 2
fi

for command in git tar sha256sum jq flatpak flatpak-builder dbus-run-session xvfb-run timeout flock realpath mktemp; do
  command -v "$command" >/dev/null || { echo "Missing command: $command" >&2; exit 1; }
done

repository="$(git -C "${SOURCE_REPOSITORY:-$(dirname "$0")}" rev-parse --show-toplevel)"
output_directory="$(realpath -m "$2")"
cache_directory="$(realpath -m "$3")"
mkdir -p "$output_directory" "$cache_directory"
exec 9>"$cache_directory/.build.lock"
flock -n 9 || { echo "Another Flatpak build is using $cache_directory" >&2; exit 1; }

source_directory="$cache_directory/source"
source_archive="$cache_directory/source.tar"
rm -rf "$source_directory"
mkdir -p "$source_directory"
# Archive the selected commit so ignored build output, cache files, and .git
# never enter the module's source checksum or the build sandbox.
git -C "$repository" archive --format=tar --prefix=source/ HEAD > "$source_archive"
source_checksum="$(sha256sum "$source_archive" | awk '{ print $1 }')"
tar -xf "$source_archive" --strip-components=1 -C "$source_directory"
source_version="$(awk '$1 == "version" && $2 == "=" { gsub(/"/, "", $3); print $3; exit }' "$source_directory/native/gtk/Cargo.toml")"
if [[ "$source_version" != "$version" ]]; then
  echo "Source version $source_version does not match requested version $version" >&2
  exit 1
fi

base_manifest="$cache_directory/manifest.base.json"
manifest="$cache_directory/manifest.json"
flatpak-builder --show-manifest "$source_directory/packaging/flatpak/dev.typsmthng.Typsmthng.yml" > "$base_manifest"
sdk="$(jq -er '.sdk' "$base_manifest")"
runtime_version="$(jq -er '."runtime-version"' "$base_manifest")"
application_id="$(jq -er '.id // ."app-id"' "$base_manifest")"
sdk_commit="$(flatpak info --show-commit "$sdk//$runtime_version")"
if [[ ! "$sdk_commit" =~ ^[0-9a-f]{64}$ ]]; then
  echo "Could not identify installed SDK commit: $sdk//$runtime_version" >&2
  exit 1
fi

mkdir -p "$cache_directory/cargo"
# An SDK branch can update without changing its name. Cargo's fingerprints
# do not include that commit, so invalidate compiled objects when it changes.
if [[ ! -f "$cache_directory/target/.sdk-commit" ]] || [[ "$(cat "$cache_directory/target/.sdk-commit")" != "$sdk_commit" ]]; then
  rm -rf "$cache_directory/target"
fi
mkdir -p "$cache_directory/target"
printf '%s\n' "$sdk_commit" > "$cache_directory/target/.sdk-commit"

# Keep sandbox paths stable across runners while storing registry downloads
# and compiled objects outside module directories that builder recreates.
jq --arg archive "$source_archive" --arg checksum "$source_checksum" --arg cargo "$cache_directory/cargo" --arg target "$cache_directory/target" '
  (.modules[] | select(.name == "typsmthng")) |= (
    ."build-options".env.CARGO_HOME = "/run/build-cache/cargo"
    | ."build-options".env.CARGO_TARGET_DIR = "/run/build-cache/target"
    | ."build-options"."build-args" += [
        "--bind-mount=/run/build-cache/cargo=" + $cargo,
        "--bind-mount=/run/build-cache/target=" + $target
      ]
    | ."build-commands" |= map(
        if . == "install -Dm755 source/target/release/typsmthng /app/bin/typsmthng"
        then "install -Dm755 \"$CARGO_TARGET_DIR/release/typsmthng\" /app/bin/typsmthng"
        else . end
      )
    | (.sources[] | select(.type == "dir" and .dest == "source")) = {
        type: "archive", path: $archive, sha256: $checksum,
        dest: "source", "strip-components": 1
      }
  )
' "$base_manifest" > "$manifest"

flatpak-builder \
  --force-clean \
  --rebuild-on-sdk-change \
  --state-dir="$cache_directory/.flatpak-builder" \
  --repo="$cache_directory/repo" \
  "$cache_directory/build" "$manifest"

bundle="$output_directory/typsmthng_${version}_linux_x64.flatpak"
flatpak build-bundle "$cache_directory/repo" "$bundle" "$application_id"
test -s "$bundle"
# Test each bundle in a disposable user installation. FLATPAK_USER_DIR leaves
# the user's installed applications intact and still shares system runtimes.
smoke_installation="$(mktemp -d "$cache_directory/smoke-install.XXXXXX")"
trap 'rm -rf "$smoke_installation"' EXIT
env FLATPAK_USER_DIR="$smoke_installation" flatpak install --user --noninteractive --bundle "$bundle"
dbus-run-session -- xvfb-run -a -s '-screen 0 1920x1080x24' \
  timeout 45s env FLATPAK_USER_DIR="$smoke_installation" \
  flatpak run --user --env=GTK_A11Y=none "$application_id" --smoke-test
dbus-run-session -- xvfb-run -a -s '-screen 0 1920x1080x24' \
  timeout 45s env FLATPAK_USER_DIR="$smoke_installation" flatpak run --user --env=GTK_A11Y=none \
  --filesystem="$source_directory/native/gtk/tests/fixtures/demo:ro" "$application_id" \
  --presentation-smoke-test "$source_directory/native/gtk/tests/fixtures/demo"
