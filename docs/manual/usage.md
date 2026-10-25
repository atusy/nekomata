# Usage

## The window

The top row reads `≡ | tab1 | tab2 | …`. The `≡` menu creates and switches
workspaces, adds tabs, splits and closes panes and tabs, attaches sessions,
changes the font size and shows help. Most of these are also in the command
palette (Alt+Shift+P).

## Keys

Default keys are below. `→` means "then press".

| Action | Keys |
| --- | --- |
| New tab | Alt+C |
| Next / previous tab | Alt+T / Alt+Shift+T |
| Split down | Alt+W → Alt+S |
| Split right | Alt+W → Alt+V |
| Close pane | Alt+W → C |
| Focus left / down / up / right pane | Alt+W → H / J / K / L |
| Command palette | Alt+Shift+P |
| Search scrollback | Ctrl+Shift+F, or Cmd+F on macOS |
| Copy selection | Cmd+C on macOS, Ctrl+Shift+C on Linux |
| Paste | Ctrl+Shift+V, or Cmd+V on macOS |
| Larger / smaller text | Ctrl+Shift+= / Ctrl+Shift+- (also Cmd+= / Cmd+- on macOS) |
| Reset text size | Ctrl+Shift+0, or Cmd+0 on macOS |
| Scroll half a page up / down | Shift+PageUp / Shift+PageDown |
| Jump to oldest / live output | Ctrl+Shift+Home / Ctrl+Shift+End |
| Previous / next prompt | Ctrl+Shift+Z / Ctrl+Shift+X |

Tab, pane and palette keys are set in nekomata's configuration; copy, paste
and the other terminal keys are set in felis's (see
[Configuration](configuration.md)).

Prompt navigation needs a shell that reports prompt boundaries with OSC 133.

## Tabs and panes

- Click a pane to focus it. Drag a divider to resize.
- A split opens a new shell on the same felis daemon as the pane it splits, in
  the directory the shell last reported with OSC 7.
- Closing a window, tab or pane does not end its shell: the shell and its
  running command stay in the felis daemon until the shell exits or the felis
  daemon stops, and you can [attach to it again](sessions.md). Closing the
  last pane closes the window.

## Mouse

- Drag to select text; Alt+drag selects a rectangle. Inside a TUI that uses
  the mouse, use Shift+drag and Shift+Alt+drag.

  ![Rectangular selection](../images/terminal-tools-rectangle.png)

- The wheel or trackpad scrolls the scrollback. A TUI that requests mouse
  input receives wheel events instead.

- Hovering a link embedded with OSC 8 shows its URL. Cmd+click (macOS) or
  Ctrl+click (Linux) opens it in the default application. Plain URL text is
  not turned into links.

  ![Link URL preview](../images/terminal-tools-link.png)

## Paste confirmation

Pasting text with newlines or other control characters into a terminal that
has not enabled bracketed paste asks for confirmation first.

![Paste confirmation](../images/terminal-tools-paste.png)

## Searching scrollback

Open the search field at the right of the tab bar with the search keys,
`≡` → `Find in scrollback`, or the command palette.

- Type a query and press Enter or `⌄`; press again for the next result.
  `⌃` or Shift+Enter goes back.
- The funnel icon toggles `Ignore case` (on by default) and `Regex` (off by
  default).
- A `+` after the count means the result limit was reached. Hover `!` to see
  an error.
- Escape or `×` closes the field and clears the highlights.

![Search results](../images/terminal-tools-search.png)

![Search options](../images/terminal-tools-search-options.png)

The exact rules are in the [terminal specification](../spec/terminal.md) and
the [keymap specification](../spec/keymap.md).
