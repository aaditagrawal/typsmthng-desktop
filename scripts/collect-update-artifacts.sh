#!/usr/bin/env bash
# Copy Electrobun updater artifacts (update.json, tar.zst, patches) into the
# GitHub Release upload directory. Filenames must stay exactly as Electrobun
# wrote them — the runtime fetches ${channel}-${os}-${arch}-update.json.
#
# Electrobun 1.15.1 wraps the *host* arch, even when `--targets` asks for
# another (macos-15 is arm64). Do not fail the job for that: pick the
# update.json Electrobun actually produced for this OS. If artifacts/ is
# missing entirely (Windows --env=dev wrap), generate the manifest from
# the packaged version.json.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHANNEL="${1:?usage: collect-update-artifacts.sh <channel> <os> <arch> [dest]}"
OS="${2:?}"
ARCH="${3:?}"
DEST="${4:-"$ROOT_DIR/build/release"}"
BUILD_DIR="$ROOT_DIR/build"
SRC="$ROOT_DIR/artifacts"
REQUESTED_PREFIX="${CHANNEL}-${OS}-${ARCH}"

discover_artifacts_dir() {
  local candidate
  for candidate in \
    "$ROOT_DIR/artifacts" \
    "$ROOT_DIR/Artifacts" \
    "$BUILD_DIR/artifacts"
  do
    if [[ -d "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

# Prefer the requested prefix; otherwise the host-arch file Electrobun wrote.
discover_update_json() {
  local dir="$1"
  local requested="$dir/${REQUESTED_PREFIX}-update.json"
  if [[ -f "$requested" ]]; then
    printf '%s\n' "$requested"
    return 0
  fi

  shopt -s nullglob
  local matches=("$dir/${CHANNEL}-${OS}-"*-update.json)
  shopt -u nullglob
  if [[ ${#matches[@]} -gt 0 ]]; then
    printf '%s\n' "${matches[0]}"
    return 0
  fi
  return 1
}

prefix_from_update_json() {
  local base
  base="$(basename "$1")"
  printf '%s\n' "${base%-update.json}"
}

find_version_json() {
  local d found
  shopt -s nullglob
  local dirs=("$BUILD_DIR/${CHANNEL}-${OS}-${ARCH}" "$BUILD_DIR/${CHANNEL}-${OS}-"*)
  shopt -u nullglob
  for d in "${dirs[@]}"; do
    [[ -d "$d" ]] || continue
    found="$(find "$d" -name version.json -path '*/Resources/version.json' 2>/dev/null | head -1 || true)"
    if [[ -n "$found" ]]; then
      printf '%s\n' "$found"
      return 0
    fi
  done
  return 1
}

copy_prefix_files() {
  local dir="$1" prefix="$2"
  shopt -s nullglob
  local files=("$dir/${prefix}"-*)
  shopt -u nullglob
  if [[ ${#files[@]} -eq 0 ]]; then
    echo "Error: no artifacts matching $dir/${prefix}-*"
    return 1
  fi
  mkdir -p "$DEST"
  cp -a "${files[@]}" "$DEST/"
  echo "==> Copied Electrobun updater artifacts for ${prefix}:"
  ls -la "$DEST/${prefix}"-*
}

generate_manifest() {
  local dest_json="$1" version_json="$2" platform="$3" arch="$4"
  if command -v bun >/dev/null 2>&1 && [[ -f "$ROOT_DIR/scripts/generate-update-json.ts" ]]; then
    bun "$ROOT_DIR/scripts/generate-update-json.ts" "$dest_json" "$version_json" "$platform" "$arch"
    return
  fi
  python3 - "$dest_json" "$version_json" "$platform" "$arch" <<'PY'
import json, sys
from pathlib import Path
dest, source, platform, arch = sys.argv[1:5]
parsed = json.loads(Path(source).read_text(encoding="utf-8"))
version = parsed.get("version")
hash_ = parsed.get("hash")
if not version or not hash_ or hash_ == "unknown":
    raise SystemExit(f"version.json missing usable hash/version: {source}")
out = {"version": version, "hash": hash_}
if platform:
    out["platform"] = platform
if arch:
    out["arch"] = arch
Path(dest).parent.mkdir(parents=True, exist_ok=True)
Path(dest).write_text(json.dumps(out) + "\n", encoding="utf-8")
print(f"generated update.json version={version} hash={hash_} -> {dest}")
PY
}

validate_manifest() {
  local json="$1"
  if command -v bun >/dev/null 2>&1 && [[ -f "$ROOT_DIR/scripts/validate-update-json.ts" ]]; then
    bun "$ROOT_DIR/scripts/validate-update-json.ts" "$json"
    return
  fi
  python3 - "$json" <<'PY'
import json, sys
path = sys.argv[1]
with open(path, encoding="utf-8") as fh:
    parsed = json.load(fh)
if not parsed.get("hash") or not parsed.get("version"):
    raise SystemExit(f"update.json missing hash/version: {path}")
print(f"update.json version={parsed['version']} hash={parsed['hash']}")
PY
}

maybe_copy_tarball() {
  local prefix="$1"
  shopt -s nullglob
  local candidates=(
    "$SRC/${prefix}"-*.tar.zst
    "$BUILD_DIR/${CHANNEL}-${OS}-${ARCH}"/*.tar.zst
    "$BUILD_DIR/${CHANNEL}-${OS}-"*/*.tar.zst
  )
  shopt -u nullglob
  local src
  for src in "${candidates[@]}"; do
    [[ -f "$src" ]] || continue
    local dest_name
    dest_name="$(basename "$src")"
    if [[ "$dest_name" != "${prefix}-"* ]]; then
      dest_name="${prefix}-typsmthng.tar.zst"
    fi
    if [[ ! -f "$DEST/$dest_name" ]]; then
      cp -a "$src" "$DEST/$dest_name"
      echo "==> Copied update archive $src -> $DEST/$dest_name"
    fi
    return 0
  done
  return 0
}

SRC="$(discover_artifacts_dir || true)"
JSON=""
if [[ -n "$SRC" ]]; then
  JSON="$(discover_update_json "$SRC" || true)"
fi

if [[ -z "$JSON" ]]; then
  echo "==> Electrobun artifacts/ update.json not found for ${REQUESTED_PREFIX}."
  if [[ -n "$SRC" ]]; then
    echo "Contents of $SRC:"
    ls -la "$SRC" || true
  else
    echo "Electrobun artifacts directory missing under $ROOT_DIR (dev wrap skips it)."
  fi

  VERSION_JSON="$(find_version_json || true)"
  if [[ -z "$VERSION_JSON" ]]; then
    echo "Error: cannot generate ${REQUESTED_PREFIX}-update.json (no version.json under build/${CHANNEL}-${OS}-*)."
    echo "Contents of $BUILD_DIR:"
    ls -la "$BUILD_DIR" 2>/dev/null || echo "  (directory does not exist)"
    exit 1
  fi

  echo "==> Generating ${REQUESTED_PREFIX}-update.json from $VERSION_JSON"
  mkdir -p "$ROOT_DIR/artifacts"
  SRC="$ROOT_DIR/artifacts"
  JSON="$SRC/${REQUESTED_PREFIX}-update.json"
  generate_manifest "$JSON" "$VERSION_JSON" "$OS" "$ARCH"
fi

PREFIX="$(prefix_from_update_json "$JSON")"
if [[ "$PREFIX" != "$REQUESTED_PREFIX" ]]; then
  echo "==> Requested ${REQUESTED_PREFIX}-update.json missing; using Electrobun host output ${PREFIX}-update.json"
fi

validate_manifest "$JSON"
copy_prefix_files "$SRC" "$PREFIX"
maybe_copy_tarball "$PREFIX"
