# Usage

## The window

The top row holds the command palette button (the cat icon) followed by the
tabs. The button opens the command palette, as does Alt+Shift+P. The palette lists every window command: creating and
switching workspaces, adding tabs, splitting, focusing and closing panes and
tabs, attaching sessions, searching and scrolling, the clipboard, the font
size and the keyboard shortcuts. Type any words of a command, in any order, to
narrow the list; click a command or select it with the arrow keys and Enter.

## Keys

Default keys are below. `→` means "then press".

| Action | Keys |
| --- | --- |
| New tab | Alt+C |
| Next / previous tab | Alt+T / Alt+Shift+T |
| Split below | Alt+W → Alt+S |
| Split right | Alt+W → Alt+V |
| Close pane | Alt+W → C |
| Focus left / down / up / right pane | Alt+W → H / J / K / L |
| Command palette | Alt+Shift+P |
| Search scrollback | Ctrl+Shift+F, or Cmd+F (with or without Shift) on macOS |
| Copy selection | Ctrl+Shift+C, or Cmd+C on macOS |
| Paste | Ctrl+Shift+V, or Cmd+V on macOS |
| Larger / smaller font | Ctrl+Shift+= / Ctrl+Shift+- (also Cmd+= / Cmd+- on macOS) |
| Reset font size | Ctrl+Shift+0, or Cmd+0 on macOS |
| Scroll half a page up / down | Shift+PageUp / Shift+PageDown |
| Jump to oldest / live output | Ctrl+Shift+Home / Ctrl+Shift+End |
| Previous / next prompt | Ctrl+Shift+Z / Ctrl+Shift+X |

Tab, pane and palette keys are set in nekomata's configuration; copy, paste
and the other terminal keys are set in felis's (see
[Configuration](configuration.md)). Cmd+C and Cmd+V on macOS are fixed and
cannot be rebound; on Linux, plain Ctrl+C always goes to the program, and so
does Ctrl+V while the clipboard holds text.

Each font size step scales by 1.1; `Font: Change size…` in the command
palette shows the current size with buttons to step or reset it. Prompt
navigation needs a shell that reports prompt boundaries with OSC 133.
`Scrollback: Scroll to top`, `Scrollback: Scroll to bottom`,
`Scrollback: Jump to previous prompt` and `Scrollback: Jump to next prompt`
are also in the command palette.

## Tabs and panes

- Drag a tab to reorder it; the tab order is kept with the
  [workspace](workspaces.md).
- Click a pane to focus it. Drag a divider to resize.
- A split opens a new shell on the same felis daemon as the pane it splits.
  For a local pane, it starts in the directory the shell last reported with
  OSC 7.
- Closing a window, tab or pane does not end its shell: the shell and its
  running command stay in the felis daemon until the shell exits or the felis
  daemon stops, and you can [attach to it again](sessions.md). Closing the
  last pane closes the window.

## Mouse

- Drag to select text; Alt+drag selects a rectangle. Double-click selects a
  word and triple-click a line. Inside a TUI that uses the mouse, use
  Shift+drag and Shift+Alt+drag.

  ![Rectangular selection](../images/terminal-tools-rectangle.png)

- The wheel or trackpad scrolls the scrollback. A TUI that requests mouse
  input receives wheel events instead. Holding Shift, or Ctrl (Cmd on macOS),
  disables the wheel.

- Hovering a link embedded with OSC 8 shows its URL. Cmd+click (macOS) or
  Ctrl+click (Linux) opens it in the default application. Plain URL text is
  not turned into links.

  ![Link URL preview](../images/terminal-tools-link.png)

## Paste confirmation

Pasting text that contains a line break or another C0 control character
(other than tab), such as Escape, into a terminal that has not enabled
bracketed paste asks for confirmation first. See the
[terminal specification](../spec/terminal.md#paste) for the exact rule.

![Paste confirmation](../images/terminal-tools-paste.png)

## Searching scrollback

Open the search field at the right of the tab bar with the search keys,
or `Scrollback: Find…` in the command palette.

- Type a query and press Enter or the down-arrow button (`⌄`); press again for
  the next result. The up-arrow button (`⌃`) or Shift+Enter goes back.
- The funnel icon toggles `Ignore case` (on by default) and `Regex` (off by
  default).
- A `+` after the count means the result limit was reached. Hover `!` to see
  an error.
- Escape or `×` closes the field and clears the highlights.

![Search results](../images/terminal-tools-search.png)

![Search options](../images/terminal-tools-search-options.png)

The exact rules are in the [terminal specification](../spec/terminal.md) and
the [keymap specification](../spec/keymap.md).
