#!/usr/bin/env bash
set -euo pipefail

script_directory="$(cd "$(dirname "$0")" && pwd)"
unset SOURCE_REPOSITORY
test_directory="$(mktemp -d)"
trap 'rm -rf "$test_directory"' EXIT
fixture_repository="$test_directory/repository with spaces"
mkdir -p "$fixture_repository/packaging/flatpak" "$fixture_repository/native/gtk/tests/fixtures/demo" "$fixture_repository/native/gtk/src" "$test_directory/bin"
cp "$script_directory/build.sh" "$script_directory/dev.typsmthng.Typsmthng.yml" "$fixture_repository/packaging/flatpak/"
printf '[package]\nname = "typsmthng-gtk"\nversion = "0.1.3"\n' > "$fixture_repository/native/gtk/Cargo.toml"
printf 'committed source\n' > "$fixture_repository/native/gtk/tests/fixtures/demo/main.typ"
printf 'fn main() { print!("{}", include_str!("../tests/fixtures/demo/main.typ")); }\n' > "$fixture_repository/native/gtk/src/main.rs"
export FLATPAK_TEST_REAL_CARGO
FLATPAK_TEST_REAL_CARGO="$(command -v cargo)"
"$FLATPAK_TEST_REAL_CARGO" generate-lockfile --offline --manifest-path "$fixture_repository/native/gtk/Cargo.toml"
git -C "$fixture_repository" init -q
git -C "$fixture_repository" add .
GIT_AUTHOR_DATE='2020-01-02T00:00:00Z' GIT_COMMITTER_DATE='2020-01-02T00:00:00Z' \
  git -C "$fixture_repository" -c user.name='Flatpak test' -c user.email='flatpak-test@example.invalid' commit -qm fixture
printf 'uncommitted source\n' > "$fixture_repository/native/gtk/tests/fixtures/demo/main.typ"
mkdir -p "$fixture_repository/target"
printf 'host build output\n' > "$fixture_repository/target/host-only"

# Stub package/build tools to test source selection, cache lifetime, argument
# escaping, and failure propagation without downloading a full GNOME SDK.
export FLATPAK_TEST_CACHE="$test_directory/cache with spaces"
export FLATPAK_TEST_MANIFEST="$test_directory/base-manifest.json"
export FLATPAK_TEST_SDK_COMMIT
export FLATPAK_TEST_EXPECTED_SOURCE='committed source'
export FLATPAK_USER_DIR="$test_directory/developer installation"
mkdir -p "$FLATPAK_USER_DIR"
printf 'developer app\n' > "$FLATPAK_USER_DIR/existing-app"
FLATPAK_TEST_SDK_COMMIT="$(printf 'a%.0s' {1..64})"
cat > "$FLATPAK_TEST_MANIFEST" <<'JSON'
{
  "id": "dev.typsmthng.Typsmthng",
  "sdk": "org.gnome.Sdk",
  "runtime-version": "50",
  "modules": [
    {
      "name": "rust-toolchain",
      "build-options": {"no-debuginfo": true}
    },
    {
      "name": "typsmthng",
      "build-options": {
        "build-args": ["--share=network"],
        "env": {"CARGO_HOME": "/run/build/typsmthng/cargo", "CARGO_TARGET_DIR": "/run/build/typsmthng/target"}
      },
      "build-commands": ["cargo build --release --locked", "install -Dm755 \"$CARGO_TARGET_DIR/release/typsmthng\" /app/bin/typsmthng"],
      "sources": [{"type": "dir", "path": "../..", "dest": "source"}]
    }
  ]
}
JSON
cat > "$test_directory/bin/stub" <<'BASH'
#!/usr/bin/env bash
set -euo pipefail
case "$(basename "$0")" in
  flatpak-builder)
    if [[ "$1" == --show-manifest ]]; then
      cat "$FLATPAK_TEST_MANIFEST"
      exit 0
    fi
    manifest="${@: -1}"
    jq -e --arg cache "$FLATPAK_TEST_CACHE" '
      (.modules[] | select(.name == "rust-toolchain") | ."build-options" | ."no-debuginfo" == true and (.strip // false) == false)
      and (.modules[] | select(.name == "typsmthng") |
        ."build-options".env.CARGO_HOME == "/run/build-cache/cargo"
        and ."build-options".env.CARGO_TARGET_DIR == "/run/build-cache/target"
        and (."build-options"."build-args" | index("--share=network") != null)
        and (."build-options"."build-args" | index("--bind-mount=/run/build-cache/cargo=" + $cache + "/cargo") != null)
        and (."build-options"."build-args" | index("--bind-mount=/run/build-cache/target=" + $cache + "/target") != null)
        and (."build-commands" | index("cargo build --release --locked") != null)
        and (."build-commands" | index("install -Dm755 \"$CARGO_TARGET_DIR/release/typsmthng\" /app/bin/typsmthng") != null)
        and (.sources[0].type == "archive")
        and (.sources[0].path == $cache + "/source.tar")
        and (.sources[0].dest == "source")
        and (.sources[0]."strip-components" == 1))
    ' "$manifest" >/dev/null
    archive_checksum="$(jq -r '.modules[] | select(.name == "typsmthng") | .sources[0].sha256' "$manifest")"
    printf '%s  %s\n' "$archive_checksum" "$FLATPAK_TEST_CACHE/source.tar" | sha256sum -c >/dev/null
    test "$(tar -xOf "$FLATPAK_TEST_CACHE/source.tar" source/native/gtk/tests/fixtures/demo/main.typ)" = "$FLATPAK_TEST_EXPECTED_SOURCE"
    test "$(cat "$FLATPAK_TEST_CACHE/source/native/gtk/tests/fixtures/demo/main.typ")" = "$FLATPAK_TEST_EXPECTED_SOURCE"
    test ! -e "$FLATPAK_TEST_CACHE/source/target"
    test ! -e "$FLATPAK_TEST_CACHE/source/.git"
    if [[ "${FLATPAK_TEST_RUN_CARGO:-0}" == 1 ]]; then
      module_directory="$FLATPAK_TEST_CACHE/cargo-module"
      rm -rf "$module_directory"
      mkdir -p "$module_directory"
      tar -xf "$FLATPAK_TEST_CACHE/source.tar" -C "$module_directory"
      # Execute the generated preparation and Cargo commands against a real
      # tiny crate, keeping source paths and target objects across commits.
      while IFS= read -r build_command; do
        case "$build_command" in
          'find source -type f -exec touch {} +')
            (cd "$module_directory" && bash -c "$build_command")
            ;;
          'cargo build --release --locked')
            CARGO_TARGET_DIR="$FLATPAK_TEST_CACHE/target" "$FLATPAK_TEST_REAL_CARGO" build --release --locked --offline --quiet \
              --manifest-path "$module_directory/source/native/gtk/Cargo.toml"
            ;;
        esac
      done < <(jq -r '.modules[] | select(.name == "typsmthng") | ."build-commands"[]' "$manifest")
      compiled_source="$("$FLATPAK_TEST_CACHE/target/release/typsmthng-gtk")"
      if [[ "$compiled_source" != "$FLATPAK_TEST_EXPECTED_SOURCE" ]]; then
        echo "Cached Cargo binary contains '$compiled_source', expected '$FLATPAK_TEST_EXPECTED_SOURCE'" >&2
        exit 1
      fi
    fi
    [[ " $* " == *" --rebuild-on-sdk-change "* ]]
    mkdir -p "$FLATPAK_TEST_CACHE/.flatpak-builder/cache" "$FLATPAK_TEST_CACHE/.flatpak-builder/downloads"
    printf 'builder state\n' > "$FLATPAK_TEST_CACHE/.flatpak-builder/cache/sentinel"
    ;;
  flatpak)
    case "$1" in
      info) printf '%s\n' "$FLATPAK_TEST_SDK_COMMIT" ;;
      build-bundle) printf 'flatpak bundle\n' > "$3" ;;
      install)
        [[ "$FLATPAK_USER_DIR" == "$FLATPAK_TEST_CACHE"/smoke-install.* ]]
        test -d "$FLATPAK_USER_DIR"
        test ! -e "$FLATPAK_USER_DIR/installed-app"
        printf '%s\n' "$FLATPAK_USER_DIR" > "$FLATPAK_TEST_CACHE/last-smoke-directory"
        printf 'test app\n' > "$FLATPAK_USER_DIR/installed-app"
        ;;
      run)
        test "$FLATPAK_USER_DIR" = "$(cat "$FLATPAK_TEST_CACHE/last-smoke-directory")"
        test -f "$FLATPAK_USER_DIR/installed-app"
        if [[ " $* " == *" --presentation-smoke-test "* && "${FLATPAK_TEST_SMOKE_FAIL:-0}" == 1 ]]; then
          exit 7
        fi
        ;;
    esac
    ;;
  dbus-run-session)
    test "$1" = --
    shift
    exec "$@"
    ;;
  xvfb-run)
    test "$1" = -a
    test "$2" = -s
    shift 3
    exec "$@"
    ;;
esac
BASH
chmod +x "$test_directory/bin/stub"
for tool in flatpak flatpak-builder dbus-run-session xvfb-run; do
  ln -s stub "$test_directory/bin/$tool"
done
export PATH="$test_directory/bin:$PATH"

build_script="$fixture_repository/packaging/flatpak/build.sh"
output_directory="$test_directory/output with spaces"
FLATPAK_TEST_RUN_CARGO=1 "$build_script" 0.1.3 "$output_directory" "$FLATPAK_TEST_CACHE"
test -s "$output_directory/typsmthng_0.1.3_linux_x64.flatpak"
test ! -d "$(cat "$FLATPAK_TEST_CACHE/last-smoke-directory")"
test "$(cat "$FLATPAK_USER_DIR/existing-app")" = 'developer app'
first_checksum="$(sha256sum "$FLATPAK_TEST_CACHE/source.tar")"
printf 'compiled object\n' > "$FLATPAK_TEST_CACHE/target/object-sentinel"
printf 'registry download\n' > "$FLATPAK_TEST_CACHE/cargo/registry-sentinel"
"$build_script" 0.1.3 "$output_directory" "$FLATPAK_TEST_CACHE"
test "$(sha256sum "$FLATPAK_TEST_CACHE/source.tar")" = "$first_checksum"
test -f "$FLATPAK_TEST_CACHE/target/object-sentinel"
test -f "$FLATPAK_TEST_CACHE/.flatpak-builder/cache/sentinel"

FLATPAK_TEST_SDK_COMMIT="$(printf 'b%.0s' {1..64})"
FLATPAK_TEST_RUN_CARGO=1 "$build_script" 0.1.3 "$output_directory" "$FLATPAK_TEST_CACHE"
test ! -e "$FLATPAK_TEST_CACHE/target/object-sentinel"
test -f "$FLATPAK_TEST_CACHE/cargo/registry-sentinel"

if "$build_script" 0.1.4 "$output_directory" "$FLATPAK_TEST_CACHE" > "$test_directory/version-error.log" 2>&1; then
  echo 'Expected a source-version mismatch to fail' >&2
  exit 1
fi
export FLATPAK_TEST_SMOKE_FAIL=1
if "$build_script" 0.1.3 "$output_directory" "$FLATPAK_TEST_CACHE"; then
  echo 'Expected a presentation smoke failure to fail the build' >&2
  exit 1
else
  test "$?" -eq 7
fi
test ! -d "$(cat "$FLATPAK_TEST_CACHE/last-smoke-directory")"
test "$(cat "$FLATPAK_USER_DIR/existing-app")" = 'developer app'

unset FLATPAK_TEST_SMOKE_FAIL
printf 'changed committed source\n' > "$fixture_repository/native/gtk/tests/fixtures/demo/main.typ"
git -C "$fixture_repository" add native/gtk/tests/fixtures/demo/main.typ
GIT_AUTHOR_DATE='2019-01-02T00:00:00Z' GIT_COMMITTER_DATE='2019-01-02T00:00:00Z' \
  git -C "$fixture_repository" -c user.name='Flatpak test' -c user.email='flatpak-test@example.invalid' commit -qm changed
FLATPAK_TEST_EXPECTED_SOURCE='changed committed source'
FLATPAK_TEST_RUN_CARGO=1 "$build_script" 0.1.3 "$output_directory" "$FLATPAK_TEST_CACHE"
test "$(sha256sum "$FLATPAK_TEST_CACHE/source.tar")" != "$first_checksum"

# Release tooling may be newer than the selected source. Use the source
# override even when the script is outside a repository and the manifest
# still uses the old install path with no CARGO_TARGET_DIR.
legacy_repository="$test_directory/legacy source with spaces"
external_tools="$test_directory/tools outside repository"
mkdir -p "$legacy_repository/native/gtk/tests/fixtures/demo" "$legacy_repository/packaging/flatpak" "$external_tools"
cp "$script_directory/build.sh" "$external_tools/build.sh"
awk '
  /^        CARGO_TARGET_DIR:/ { next }
  /install -Dm755 "\$CARGO_TARGET_DIR\/release\/typsmthng"/ {
    print "      - install -Dm755 source/target/release/typsmthng /app/bin/typsmthng"; next
  }
  { print }
' "$script_directory/dev.typsmthng.Typsmthng.yml" > "$legacy_repository/packaging/flatpak/dev.typsmthng.Typsmthng.yml"
cp "$fixture_repository/native/gtk/Cargo.toml" "$legacy_repository/native/gtk/Cargo.toml"
printf 'legacy committed source\n' > "$legacy_repository/native/gtk/tests/fixtures/demo/main.typ"
git -C "$legacy_repository" init -q
git -C "$legacy_repository" add .
git -C "$legacy_repository" -c user.name='Flatpak test' -c user.email='flatpak-test@example.invalid' commit -qm legacy
jq '
  (.modules[] | select(.name == "typsmthng")) |= (
    del(."build-options".env.CARGO_TARGET_DIR)
    | ."build-commands"[1] = "install -Dm755 source/target/release/typsmthng /app/bin/typsmthng"
  )
' "$FLATPAK_TEST_MANIFEST" > "$test_directory/legacy-manifest.json"
FLATPAK_TEST_MANIFEST="$test_directory/legacy-manifest.json"
FLATPAK_TEST_EXPECTED_SOURCE='legacy committed source'
SOURCE_REPOSITORY="$legacy_repository" "$external_tools/build.sh" 0.1.3 "$output_directory" "$FLATPAK_TEST_CACHE"
test ! -d "$(cat "$FLATPAK_TEST_CACHE/last-smoke-directory")"
test "$(cat "$FLATPAK_USER_DIR/existing-app")" = 'developer app'
echo 'Flatpak build orchestration checks passed'
