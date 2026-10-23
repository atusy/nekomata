#!/usr/bin/env bash
# Run through check-linux.sh inside the disposable X11 guest.
set -euo pipefail
: "${DISPLAY:?run inside the Vitro desktop}"
lab=$(mktemp -d /tmp/nekomata-desktop.XXXXXX)
export FELIS_SOCKET="$lab/felis.sock"
export NEKOMATA_RUNTIME_DIR="$lab/gui"
export SHELL=/bin/sh
app="${NEKOMATA_TEST_BINARY:-target/debug/nekomata}"
gui_pid=
cleanup() {
  if [[ -n ${NEKOMATA_DESKTOP_ARTIFACTS:-} ]]; then
    mkdir -p "$NEKOMATA_DESKTOP_ARTIFACTS"
    cp "$lab/gui.log" "$NEKOMATA_DESKTOP_ARTIFACTS/gui.log"
    scrot -o "$NEKOMATA_DESKTOP_ARTIFACTS/desktop.png" || true
  fi
  if [[ -n "$gui_pid" ]]; then
    kill "$gui_pid" 2>/dev/null || true
    wait "$gui_pid" 2>/dev/null || true
  fi
  "$app" daemon stop >/dev/null 2>&1 || true
  felis --socket "$FELIS_SOCKET" daemon stop --force >/dev/null 2>&1 || true
  rm -rf "$lab"
}
trap cleanup EXIT
"$app" >"$lab/gui.log" 2>&1 &
gui_pid=$!
window=
for _ in $(seq 1 200); do
  kill -0 "$gui_pid" || { cat "$lab/gui.log" >&2; exit 1; }
  window=$(xdotool search --onlyvisible --pid "$gui_pid" 2>/dev/null | head -1) || true
  [[ -n "$window" ]] && break
  sleep 0.05
done
[[ -n "$window" ]]
xdotool windowactivate --sync "$window"
xdotool type --clearmodifiers --delay 25 "printf NATIVE_INPUT_OK > '$lab/input-marker'"
xdotool key Return
for _ in $(seq 1 200); do
  [[ -f "$lab/input-marker" ]] && break
  sleep 0.05
done
if [[ ! -f "$lab/input-marker" ]]; then
  echo 'Native keyboard input did not reach the shell after window activation.' >&2
  cat "$lab/gui.log" >&2
  exit 1
fi
[[ $(cat "$lab/input-marker") == NATIVE_INPUT_OK ]]
echo 'Native keyboard input reached the private shell.'
# Put a known word at the first terminal row, then exercise the native clipboard.
xdotool type --clearmodifiers --delay 25 "printf '\\033[2J\\033[HCLIPBOARD_OK\\n'"
xdotool key Return
sleep 1
window_x=$(xwininfo -id "$window" | awk '/Absolute upper-left X:/ {print $NF}')
window_y=$(xwininfo -id "$window" | awk '/Absolute upper-left Y:/ {print $NF}')
xdotool mousemove "$((window_x + 35))" "$((window_y + 64))"
xdotool click --repeat 2 --delay 150 1
sleep 0.5
xdotool key --clearmodifiers ctrl+shift+c
copied=
for _ in $(seq 1 100); do
  copied=$(xclip -o -selection clipboard 2>/dev/null) || true
  [[ "$copied" == CLIPBOARD_OK ]] && break
  sleep 0.05
done
if [[ "$copied" != CLIPBOARD_OK ]]; then
  printf 'Native selection copy mismatch: <%s>\n' "$copied" >&2
  exit 1
fi
echo 'Native word selection reached the desktop clipboard.'
printf SENTINEL | xclip -selection clipboard
xdotool mousemove "$((window_x + 4))" "$((window_y + 64))"
xdotool mousedown 1
sleep 0.2
xdotool mousemove "$((window_x + 92))" "$((window_y + 64))"
sleep 0.2
xdotool mouseup 1
sleep 0.5
xdotool key --clearmodifiers ctrl+shift+c
for _ in $(seq 1 100); do
  copied=$(xclip -o -selection clipboard 2>/dev/null) || true
  [[ "$copied" == CLIPBOARD_OK ]] && break
  sleep 0.05
done
[[ "$copied" == CLIPBOARD_OK ]] || { printf 'Drag copy mismatch: <%s>\n' "$copied" >&2; exit 1; }
echo 'Native drag selection preserved the initial pressed cell.'
