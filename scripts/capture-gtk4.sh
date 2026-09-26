#!/usr/bin/env bash
# Capture actual GTK widgets. No user projects or settings are modified.
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
output="${1:-$repo_root/build/gtk4-screenshots}"
mkdir -p "$output"
output="$(cd "$output" && pwd)"
binary="${TYPSMTHNG_BINARY:-$repo_root/target/debug/typsmthng}"
export GTK_A11Y=none TYPSMTHNG_SMOKE_HOLD_MS=2500
export GSK_RENDERER="${GSK_RENDERER:-cairo}"
capture() {
  local name="$1"; shift
  dbus-run-session -- xvfb-run -a -s '-screen 0 1920x1200x24' \
    env TYPSMTHNG_SNAPSHOT_DIR="$output/$name" "$@"
}
for appearance in light dark; do
  capture "home-$appearance" env TYPSMTHNG_SMOKE_THEME="$appearance" TYPSMTHNG_SMOKE_PROJECTS=4 "$binary" --smoke-test
  capture "editor-$appearance" env TYPSMTHNG_SMOKE_THEME="$appearance" "$binary" --smoke-test native/gtk/tests/fixtures/demo
  capture "settings-$appearance" env TYPSMTHNG_SMOKE_THEME="$appearance" TYPSMTHNG_SMOKE_VIEW=settings "$binary" --smoke-test
 done
for view in templates import name; do
  capture "$view" env TYPSMTHNG_SMOKE_THEME=light TYPSMTHNG_SMOKE_VIEW="$view" "$binary" --smoke-test
done
capture home-empty env TYPSMTHNG_SMOKE_THEME=light "$binary" --smoke-test
capture editor-narrow env TYPSMTHNG_SMOKE_THEME=light TYPSMTHNG_SMOKE_WIDTH=760 "$binary" --smoke-test native/gtk/tests/fixtures/demo
capture home-125-percent env GDK_DPI_SCALE=1.25 TYPSMTHNG_SMOKE_THEME=light TYPSMTHNG_SMOKE_PROJECTS=4 "$binary" --smoke-test
capture presenter env TYPSMTHNG_SMOKE_THEME=dark "$binary" --presentation-smoke-test native/gtk/tests/fixtures/demo
printf 'Screenshots: %s\n' "$output"
