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
