# Terminal interactions

Terminal parsing, PTYs and scrollback belong to felis. This document covers
what nekomata adds when presenting a felis session in a pane.

## Supported terminal features

Text, 256 and RGB colors, CJK text, selection, copy and paste, scrollback,
named keys, mouse input, committed IME text, Kitty images, dynamic palette
changes and OSC 52 clipboard writes are supported. Braille
(U+2800–U+28FF) is drawn directly into cells, independently of the font.
Text is shaped with the OpenType features configured in felis's `font`
settings.

Not supported: key release events and OSC 52 clipboard reads.

## Keyboard input

- Ctrl or Alt with a character key sends the key's unshifted ASCII character
  with those modifiers (Ctrl+- sends `-`, Ctrl+' sends `'`); felis encodes it
  for the program, as a control byte where one exists, or as an escape
  sequence when the program enables modifyOtherKeys or the Kitty keyboard
  protocol. Keys that name no character, such as Copy, are not sent.

## OSC 52 clipboard writes

A program's OSC 52 write reaches the system clipboard only when felis's
`clipboard.osc_52` is `"system"` (see
[configuration](configuration.md#terminal-layer)). Under the default
`"mirror"`, the write stays in the felis daemon's mirror and the system
clipboard is untouched, so a program, including one reached over SSH, cannot
overwrite it. Writes that target only the PRIMARY selection (`p`) never
reach a system clipboard. nekomata does not check window focus; the felis
daemon delivers each write to at most one window attached to the session.

## Selection

- Dragging selects text; Alt+drag selects a rectangle. Double-click selects a
  word and triple-click a line.
- When the terminal program uses the mouse, Shift+drag selects text and
  Shift+Alt+drag selects a rectangle.

## Links

- Hovering a link embedded with OSC 8 shows its URL.
- Cmd+click (macOS) or Ctrl+click (Linux) passes an `http:`, `https:` or
  `mailto:` URL to the default application, through `xdg-open` on Linux
  (bundled with the Nix package).
- A `file:` link is revealed instead of opened, so a link cannot run a script
  or application whatever its label says. On macOS, Finder shows the file
  selected (`open -R`). On Linux, the file manager shows the file through the
  `org.freedesktop.FileManager1` D-Bus interface (`gdbus`, bundled with the
  Nix package); without that service, `xdg-open` opens the folder that
  contains the file. The path is resolved through symlinks and `..` first,
  and the hover hint says the file will be revealed.
- A `file:` link must name this machine: no host, `localhost` or this
  machine's hostname. A link to another host, a relative path or a missing
  file is not revealed; the reason is shown.
- A link that fails felis's validation is not opened; the reason is shown.
- While a pane's previous link is still opening (at most 10 seconds), a new
  click is not deferred; the pane says the previous link is still opening.
- Plain URL text is not turned into links.
- The modifier counts only without Alt, and on macOS only without Ctrl. It is
  checked when the button is pressed; modifiers at release do not matter.

## Paste

- Pasting text that contains a C0 control character other than tab
  (U+0000–U+001F, such as a newline) into a terminal without bracketed paste
  asks for confirmation.
- Any other text, including tabs, DEL and C1 controls, and any paste while
  bracketed paste is enabled, is sent without confirmation.
- If the pane's connection changes while confirmation is pending, the paste is
  discarded.

## Scrolling

- Wheel and trackpad input scrolls the scrollback; when the terminal program
  requests mouse input, it receives wheel events instead.
- With Shift, or Ctrl (Cmd on macOS), held, wheel input neither scrolls nor
  reaches the program.
- The alternate screen has no scrollback: there, wheel input reaches the
  program only when it requests mouse input.
- Consecutive scroll requests are coalesced and sent to supporting daemons as
  relative moves.
- Prompt navigation requires the shell to report OSC 133 prompt boundaries.

## Search

- Search queries the pane's scrollback with `Ignore case` (default on) and
  `Regex` (default off).
- Repeating a search with the same query and options moves to the next
  result; changing either searches again on the next move.
- Enter in the search field moves to the next result and Shift+Enter to the
  previous one, like the field's next and previous buttons.
- A query holds at most 1,024 characters; further input is not accepted.
- A search returns at most 1,000 results and 1 MiB of response data in total.
  A truncated result count is marked with `+`.
- While the search field is open, keyboard input is not sent to the terminal
  (mouse input still is). Escape closes the field and its highlights.

## Font size

- The terminal font size applies to every pane of the window. Each larger or
  smaller step multiplies or divides it by 1.1, whether it comes from felis's
  `font_size` binding or the font size control, and the result is clamped to
  4–72 logical pixels.
- Reset returns to the resolved size described in
  [configuration](configuration.md#font-size-precedence).
- `Font: Change size…` opens the font size control, which shows the current
  size with buttons to step and reset it; Up and Down arrows step it larger
  and smaller.

## Rendering

- Only visible local panes request new frames; hidden panes stop requesting
  updates. SSH panes receive pushed frames.

## Reconnection

- When the transport to felis is lost, nekomata retries reconnecting to the
  same session for a bounded period. When retries run out, or on any other
  connection error, the pane shows the error.
- Input whose delivery is uncertain is not resent, and reconnection never
  spawns a replacement shell.
