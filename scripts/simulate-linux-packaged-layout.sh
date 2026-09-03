#!/usr/bin/env bash
# Build a /opt/typsmthng-shaped tree and prove Electrobun's cwd-relative
# version.json read plus our userData fallback. Used for headless VM checks
# when a full Electrobun/GTK launch is not available.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-with-version-json}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/typsmthng-linux-layout.XXXXXX")"
APP="$WORK/opt/typsmthng"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

mkdir -p "$APP/bin" "$APP/Resources/app/bun"
printf '#!/bin/sh\nexit 0\n' > "$APP/bin/bun"
printf '#!/bin/sh\nexit 0\n' > "$APP/bin/launcher"
chmod +x "$APP/bin/bun" "$APP/bin/launcher"
echo "placeholder" > "$APP/Resources/main.js"
echo "placeholder" > "$APP/Resources/app/bun/index.js"

if [[ "$MODE" == "with-version-json" ]]; then
  (cd "$ROOT_DIR" && bun "$ROOT_DIR/scripts/write-version-json.ts" "$APP/Resources/version.json")
elif [[ "$MODE" != "missing-version-json" ]]; then
  echo "Usage: $0 [with-version-json|missing-version-json]"
  exit 1
fi

echo "==> Simulated install root: $APP (mode=$MODE)"
echo "==> Electrobun read from cwd $APP/bin:"
python3 - <<PY
import os, json
cwd = os.path.join("$APP", "bin")
path = os.path.normpath(os.path.join(cwd, "..", "Resources", "version.json"))
print("join('..', 'Resources', 'version.json') =>", path)
print("exists:", os.path.isfile(path))
if os.path.isfile(path):
    print(open(path).read())
PY

(cd "$ROOT_DIR" && bun "$ROOT_DIR/scripts/check-packaged-linux-layout.ts" "$APP")
echo "==> Simulation $MODE passed"
