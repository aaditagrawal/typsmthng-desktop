#!/usr/bin/env bash
# Copy Electrobun updater artifacts (update.json, tar.zst, patches) into the
# GitHub Release upload directory. Filenames must stay exactly as Electrobun
# wrote them — the runtime fetches ${channel}-${os}-${arch}-update.json.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHANNEL="${1:?usage: collect-update-artifacts.sh <channel> <os> <arch> [dest]}"
OS="${2:?}"
ARCH="${3:?}"
DEST="${4:-"$ROOT_DIR/build/release"}"
SRC="$ROOT_DIR/artifacts"
PREFIX="${CHANNEL}-${OS}-${ARCH}"
JSON="$SRC/${PREFIX}-update.json"

if [[ ! -d "$SRC" ]]; then
  echo "Error: Electrobun artifacts directory missing: $SRC"
  echo "electrobun build --env=${CHANNEL} should write ${PREFIX}-update.json there."
  exit 1
fi

if [[ ! -f "$JSON" ]]; then
  echo "Error: missing updater manifest $JSON"
  echo "Electrobun fetches ${PREFIX}-update.json from release.baseUrl."
  echo "Contents of $SRC:"
  ls -la "$SRC" || true
  exit 1
fi

if command -v bun >/dev/null 2>&1 && [[ -f "$ROOT_DIR/scripts/validate-update-json.ts" ]]; then
  bun "$ROOT_DIR/scripts/validate-update-json.ts" "$JSON"
else
  python3 - "$JSON" <<'PY'
import json, sys
path = sys.argv[1]
with open(path, encoding="utf-8") as fh:
    parsed = json.load(fh)
if not parsed.get("hash") or not parsed.get("version"):
    raise SystemExit(f"update.json missing hash/version: {path}")
print(f"update.json version={parsed['version']} hash={parsed['hash']}")
PY
fi

mkdir -p "$DEST"
shopt -s nullglob
files=("$SRC/${PREFIX}"-*)
if [[ ${#files[@]} -eq 0 ]]; then
  echo "Error: no artifacts matching $SRC/${PREFIX}-*"
  exit 1
fi

cp -a "${files[@]}" "$DEST/"
echo "==> Copied Electrobun updater artifacts for ${PREFIX}:"
ls -la "$DEST/${PREFIX}"-*
