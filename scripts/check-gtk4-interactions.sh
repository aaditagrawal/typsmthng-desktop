#!/usr/bin/env bash
set -euo pipefail
# Run in an isolated X11 display: dbus-run-session -- xvfb-run -a -s '-screen 0 1600x1000x24' timeout 40s scripts/check-gtk4-interactions.sh
# Requires openbox, xdotool and ImageMagick. Uses a disposable project and settings.
ulimit -c 0
# Xvfb has no desktop portal. Test GTK's local picker backend in this display.
export GDK_DEBUG=no-portals
export GSK_RENDERER="${GSK_RENDERER:-cairo}"
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
output="${1:-$repo_root/build/gtk4-input}"
mkdir -p "$output"
openbox >"$output/window-manager.log" 2>&1 &
wm_pid=$!
fixture=$(mktemp -d /tmp/gtk4-keyboard.XXXXXX)
cp native/gtk/tests/fixtures/demo/* "$fixture/"
env GTK_A11Y=none TYPSMTHNG_SMOKE_THEME=light TYPSMTHNG_SMOKE_HOLD_MS=60000 "${TYPSMTHNG_BINARY:-target/debug/typsmthng}" --smoke-test "$fixture" >"$output/application.log" 2>&1 &
app_pid=$!
trap 'kill "$app_pid" "$wm_pid" 2>/dev/null || true' EXIT
for attempt in $(seq 1 50); do
  main=$(xdotool search --onlyvisible --class 'typsmthng' 2>/dev/null | head -1) || true
  if [[ -n "$main" ]]; then break; fi
  sleep 0.1
done
test -n "$main"
xdotool windowactivate --sync "$main"
sleep 1
xdotool mousemove --window "$main" 400 190 click 1
xdotool key ctrl+End Return
xdotool type --clearmodifiers '// keyboard test '
xdotool key parenleft
xdotool key ctrl+s
sleep 0.5
grep -q '// keyboard test ()' "$fixture/main.typ"
xdotool key ctrl+z ctrl+s
sleep 0.3
if grep -q '// keyboard test ()' "$fixture/main.typ"; then echo 'UNDO FAILED'; exit 1; fi
grep -q 'Presenter tools' "$fixture/main.typ"
xdotool key ctrl+shift+z ctrl+s
sleep 0.3
grep -q '// keyboard test ()' "$fixture/main.typ"
for iteration in 1 2; do
  xdotool key ctrl+comma
  sleep 0.3
  settings=$(xdotool getactivewindow)
  xdotool windowactivate --sync "$settings" key Escape
  sleep 0.2
  test "$(xdotool getactivewindow)" != "$settings"
  xdotool windowactivate --sync "$main"
done
# Exercise the GTK 4 asynchronous folder picker, including cancellation.
xdotool key ctrl+o
sleep 0.5
picker=$(xdotool getactivewindow)
test "$picker" != "$main"
xdotool key Escape
sleep 0.3
xdotool windowactivate --sync "$main"
xdotool key ctrl+j
sleep 0.2
xdotool key ctrl+j
sleep 0.2
xdotool key ctrl+j
xdotool key ctrl+f
sleep 0.3
find=$(xdotool getactivewindow)
xdotool windowactivate --sync "$find" type --clearmodifiers typsmthng
xdotool key Return Escape
sleep 0.2
xdotool windowactivate --sync "$main" key F5
sleep 0.5
presentation=$(xdotool getactivewindow)
xdotool windowactivate --sync "$presentation" key Right b
sleep 0.2
import -window "$presentation" "$output/blackout.png"
test "$(convert "$output/blackout.png" -format '%[fx:p{500,335}.r<0.01]' info:)" = 1
xdotool key b d
xdotool mousemove --window "$presentation" 350 280 mousedown 1
sleep 0.15
xdotool mousemove --window "$presentation" 500 335
sleep 0.15
xdotool mousemove --window "$presentation" 650 390
sleep 0.15
xdotool mouseup 1
sleep 0.2
import -window "$presentation" "$output/annotation.png"
test "$(convert "$output/annotation.png" -format '%[fx:p{500,335}.r>0.9 && p{500,335}.g<0.5 && p{500,335}.b<0.2]' info:)" = 1
xdotool key g
sleep 0.2
grid=$(xdotool getactivewindow)
xdotool windowactivate --sync "$grid" key Escape
sleep 0.2
test "$(xdotool getactivewindow)" != "$grid"
xdotool windowactivate --sync "$presentation" key Escape
sleep 0.2
test "$(xdotool getactivewindow)" != "$presentation"
xdotool windowactivate --sync "$main" key ctrl+q
wait "$app_pid"
env GTK_A11Y=none TYPSMTHNG_SMOKE_VIM=1 TYPSMTHNG_SMOKE_HOLD_MS=60000 "${TYPSMTHNG_BINARY:-target/debug/typsmthng}" --smoke-test "$fixture" >"$output/vim.log" 2>&1 &
app_pid=$!
sleep 1
main=$(xdotool search --onlyvisible --class 'typsmthng' | head -1)
xdotool windowactivate --sync "$main"
xdotool mousemove --window "$main" 400 190 click 1
xdotool key Escape G o
xdotool type --clearmodifiers '// native vim test'
xdotool key Escape
xdotool type --clearmodifiers ':w'
xdotool key Return
sleep 0.3
grep -q '// native vim test' "$fixture/main.typ"
xdotool type --clearmodifiers ':wq'
xdotool key Return
wait "$app_pid"
printf 'GTK4_KEYBOARD_READY pairs undo redo save vim-write-quit file-picker theme-cycle settings-reopen find presentation blackout pen grid escape\n'
