# Terminal interactions

Terminal parsing, PTYs and scrollback belong to felis. This document covers
what nekomata adds when presenting a felis session in a pane.

## Supported terminal features

Text, 256 and RGB colors, CJK text, selection, copy and paste, scrollback,
named keys, mouse input, committed IME text, Kitty images, dynamic palette
changes and OSC 52 writes to the system clipboard are supported. Braille
(U+2800–U+28FF) is drawn directly into cells, independently of the font.
Text is shaped with the OpenType features configured in felis's `font`
settings.

Not supported: key release events and OSC 52 clipboard reads.

## Selection

- Dragging selects text; Alt+drag selects a rectangle. Double-click selects a
  word and triple-click a line.
- When the terminal program uses the mouse, Shift+drag selects text and
  Shift+Alt+drag selects a rectangle.

## Links

- Hovering a link embedded with OSC 8 shows its URL.
- Cmd+click (macOS) or Ctrl+click (Linux) passes the URL to the default
  application; Linux requires `xdg-open`.
- A link that fails felis's validation is not opened; the reason is shown.
- Plain URL text is not turned into links.

## Paste

- Pasting text that contains newlines or other control characters into a
  terminal without bracketed paste asks for confirmation.
- Plain text and tabs, and any paste while bracketed paste is enabled, are sent
  without confirmation.
- If the pane's connection changes while confirmation is pending, the paste is
  discarded.

## Scrolling

- Wheel and trackpad input scrolls the scrollback; when the terminal program
  requests mouse input, it receives wheel events instead, unless Shift is held.
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
- A search returns at most 1,000 results and 1 MiB of response data in total.
  A truncated result count is marked with `+`.
- While the search field is open, input is not sent to the terminal. Escape
  closes the field and its highlights.

## Rendering

- Only visible local panes request new frames; hidden panes stop requesting
  updates. SSH panes receive pushed frames.

## Reconnection

- When the transport to felis is lost, nekomata retries reconnecting to the
  same session for a bounded period. When retries run out, or on any other
  connection error, the pane shows the error.
- Input whose delivery is uncertain is not resent, and reconnection never
  spawns a replacement shell.
