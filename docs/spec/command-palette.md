# Command palette

## Opening and closing

The command palette button at the left of the tab bar and the `command_palette`
binding open the palette. Escape or a click outside closes it without running a
command.

## Finding a command

Each word of the query must appear, ignoring case, in the command's label or
search keywords; the words may come in any order. An empty query lists every
command.

## Labels

A command whose label names no target acts on the active one: closing a tab
closes the active tab.

Caveats, such as that closing a pane leaves its shell running, appear on a
dimmer line under the label.

A label ending in "…" opens a choice or an input before the command acts.
Other commands act as soon as they are chosen, though one may still ask for
confirmation.

Keyboard shortcuts are documented in the manual; the palette has no shortcut
help action. A command bound in nekomata's own keymap shows its shortest
binding at the right of its row; commands bound only in felis's keymap, such
as copy, paste and font size, show none.

`Session: Attach…` states that it opens a local daemon session in a new tab;
it does not follow the active pane to a remote daemon.

## Prompt navigation availability

Both prompt-jump commands stay visible but are disabled while the active
session is disconnected or in the alternate screen. Their detail line shows
the reason (disconnection takes precedence). Clicking a disabled command or
pressing Enter on it neither executes it nor closes the palette. Availability
is refreshed each frame.

Otherwise the detail explains the OSC 133 requirement. The client cannot
infer whether the daemon has a prompt in the requested direction from the
current screen; an enabled command may therefore leave the viewport unchanged.
