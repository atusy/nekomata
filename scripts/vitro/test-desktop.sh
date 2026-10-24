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
    mkdir -p "$NEKOMATA_DESKTOP_ARTIFACTS" || true
    cp "$lab/gui.log" "$NEKOMATA_DESKTOP_ARTIFACTS/gui.log" 2>/dev/null || true
    cp "$lab/"*.png "$NEKOMATA_DESKTOP_ARTIFACTS/" 2>/dev/null || true
    if [[ -n "$gui_pid" ]]; then
      scrot -u -o "$NEKOMATA_DESKTOP_ARTIFACTS/desktop.png" || true
    fi
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
# Start the daemon with an older environment, then give the GUI a new TMPDIR.
felis --socket "$FELIS_SOCKET" sessions spawn >/dev/null
mkdir -p "$lab/gui-tmp" "$lab/bin"
export TMPDIR="$lab/gui-tmp"
cat > "$lab/bin/xdg-open" <<EOF_OPEN
#!/bin/sh
printf '%s\n' "\$@" > "$lab/opened-url"
EOF_OPEN
chmod +x "$lab/bin/xdg-open"
export PATH="$lab/bin:$PATH"
cat > "$lab/test-editor" <<EOF_EDITOR
#!/bin/sh
cat "\$1" > "$lab/editor-data"
printf '%s' "\$1" > "$lab/editor-path"
sleep 5
EOF_EDITOR
export VISUAL="sh $lab/test-editor"
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
xdotool windowmove --sync "$window" 0 0
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

# Safe single-line clipboard data reaches the requested pane.
printf "printf SAFE_PASTE_OK > '%s'" "$lab/safe-paste" | xclip -selection clipboard
xdotool key --clearmodifiers ctrl+shift+v
sleep 0.5
xdotool key Return
for _ in $(seq 1 100); do [[ -f "$lab/safe-paste" ]] && break; sleep 0.05; done
[[ $(cat "$lab/safe-paste") == SAFE_PASTE_OK ]]
echo 'Safe native paste reached the shell without confirmation.'
# A newline would execute immediately in this unbracketed /bin/sh shell.
printf "printf UNSAFE_PASTE > '%s'\n" "$lab/unsafe-paste" | xclip -selection clipboard
xdotool key --clearmodifiers ctrl+shift+v
sleep 1
[[ ! -e "$lab/unsafe-paste" ]]
scrot -u -o "$lab/paste-confirmation.png"
xdotool key Escape
sleep 0.2
xdotool key Return
sleep 0.3
[[ ! -e "$lab/unsafe-paste" ]]
echo 'Unsafe native paste waited for confirmation; Escape cancelled it.'
# Search input and its submit key must not be sent to the terminal.
xdotool key --clearmodifiers ctrl+shift+f
sleep 0.3
xdotool type --clearmodifiers --delay 15 "printf SEARCH_LEAK > '$lab/search-leak'"
xdotool key Return
sleep 0.6
scrot -u -o "$lab/search.png"
xdotool key Escape
sleep 0.2
xdotool key Return
sleep 0.3
[[ ! -e "$lab/search-leak" ]]
echo 'Native search captured its text and Enter without executing shell input.'
xdotool key --clearmodifiers ctrl+shift+f
sleep 0.2
xdotool key --clearmodifiers ctrl+a
xdotool type --clearmodifiers CLIPBOARD_OK
xdotool key Return
sleep 0.6
scrot -u -o "$lab/search.png"
xdotool key Escape
sleep 0.2
xdotool key --clearmodifiers ctrl+shift+End
sleep 0.3
xdotool type --clearmodifiers --delay 15 "printf '\\033[2J\\033[HABCD1111WXYZ\\nEFGH2222QRST\\n'"
xdotool key Return
sleep 0.5
xdotool mousemove "$((window_x + 36))" "$((window_y + 64))"
# Openbox reserves Alt+drag for moving windows; Shift avoids that binding
# while exercising the terminal's documented mouse-reporting override.
xdotool keydown alt shift
xdotool mousedown 1
sleep 0.2
xdotool mousemove "$((window_x + 60))" "$((window_y + 81))"
sleep 0.2
xdotool mouseup 1
xdotool keyup alt shift
sleep 0.3
scrot -u -o "$lab/rectangle.png"
xdotool key --clearmodifiers ctrl+shift+c
sleep 0.5
copied=$(xclip -o -selection clipboard)
[[ "$copied" == $'1111\n2222' ]] || { printf 'Rectangle copy mismatch: <%s>\n' "$copied" >&2; exit 1; }
echo 'Native rectangular selection copied only the selected columns.'

# Validate the native gesture without launching an external application.
xdotool type --clearmodifiers --delay 15 "printf '\\033[2J\\033[H\\033]8;;https://example.org/?q=nekomata\\007OPEN_LINK\\033]8;;\\007\\n'"
xdotool key Return
sleep 0.4
xdotool mousemove "$((window_x + 35))" "$((window_y + 64))"
sleep 0.7
scrot -u -o "$lab/link-preview.png"
xdotool keydown ctrl
xdotool click 1
xdotool keyup ctrl
for _ in $(seq 1 100); do [[ -f "$lab/opened-url" ]] && break; sleep 0.05; done
[[ $(cat "$lab/opened-url") == 'https://example.org/?q=nekomata' ]]
echo 'Native link activation passed the previewed URL as a literal argument.'

xdotool type --clearmodifiers --delay 15 "printf '\033]133;A\007prompt> \033]133;B\007fake-command\n\033]133;C\007OUTPUT_ONLY\n\033]133;D;0\007'"
xdotool key Return
sleep 0.5
xdotool key --clearmodifiers alt+shift+p
sleep 0.2
xdotool type --clearmodifiers --delay 20 'Copy last command output'
xdotool key Return
for _ in $(seq 1 100); do
  copied=$(xclip -o -selection clipboard 2>/dev/null) || true
  [[ "$copied" == *OUTPUT_ONLY* ]] && break
  sleep 0.05
done
[[ "$copied" == *OUTPUT_ONLY* && "$copied" != *fake-command* ]]
echo 'Native palette copied the completed command output.'
xdotool key --clearmodifiers alt+shift+p
sleep 0.2
xdotool key --clearmodifiers ctrl+a
xdotool type --clearmodifiers --delay 20 'Edit last command output'
xdotool key Return
for _ in $(seq 1 100); do [[ -f "$lab/editor-path" ]] && break; sleep 0.05; done
[[ $(cat "$lab/editor-data") == *OUTPUT_ONLY* ]]
staged=$(cat "$lab/editor-path")
[[ -f "$staged" ]]
kill "$gui_pid"
wait "$gui_pid" || true
gui_pid=
[[ -f "$staged" ]]
for _ in $(seq 1 150); do [[ ! -f "$staged" ]] && break; sleep 0.05; done
[[ ! -f "$staged" ]]
echo 'Editor staging survived GUI exit and was removed after editor completion.'
