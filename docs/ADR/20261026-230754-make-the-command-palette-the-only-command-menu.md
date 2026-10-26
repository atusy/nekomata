# Make the command palette the only command menu

## Context

nekomata listed its window commands in three hand-written places: the command
palette, the tab bar's menu and the help window. Each place named the same
action differently (the palette's "Split top / bottom" was the menu's "Split below"),
and each new command had to be added to every place by hand. Some commands
existed in only one of them: attaching a session, showing the keyboard
shortcuts and focusing the next pane were menu-only, and moving focus in a
direction was in neither.

The command list is expected to grow from about fifteen entries to thirty or
forty as felis actions such as session switching, fullscreen and piping become
reachable from the window. A second menu would grow with it.

## Decision

A single command registry in nekomata owns the list of window commands: for
each command, its action, its category, its label and any search keywords and
detail line. The command palette is the only surface that lists these
commands. The button at the left of the tab bar, which opened the menu, opens
the command palette instead and is named the command palette button.

A command that needs further input opens its own UI from the palette rather
than being split into several palette entries. Font size is one such command:
the palette entry opens a size control that holds the decrease, increase and
reset buttons.

Other surfaces that mention commands, such as the help window, take labels
from the registry rather than spelling their own.

## Invariants

> The invariants below are normative; the mechanisms that satisfy them are
> deliberately unspecified.

- Every command reachable from a window menu is reachable from the command
  palette, so removing the menu removes no capability.
- The palette stays operable with the pointer alone, because the command
  palette button is the mouse user's entry to every command.
- A command's label is written once, in the registry.

## Considered Options

1. **Palette only; the tab bar button opens it.** Chosen. One list and one vocabulary, and
   the menu's mouse entry point is kept.
2. **Keep the menu and the palette, both reading the registry.** Removes the
   duplicated lists, but the menu needs its own layout decisions (grouping,
   headings or menu-only labels), and a menu-only label would bring back a
   second vocabulary.
3. **Keep both hand-written lists.** No migration cost, but they had already
   drifted at fifteen entries.

## Consequences

- Adding a command means adding one registry entry.
- The palette's filter must find commands by any of their words, because it is
  now the only way to reach them.
- The menu's inline font size row (−, current size, +) moves into a separate
  size control, which is one more click away.
- The menu's visual grouping by separators is lost; categories in the label
  take its place.
- The manual can no longer describe commands as menu paths and refers to the
  palette instead.

## Decision–Implementation Gap

The help window takes the labels of commands bound in nekomata's own keymap
from the registry, but still spells its lines for commands bound in felis's
keymap (paste, scrolling, prompt jumps, font size). felis's keymap offers no
way to list its bindings, so help cannot find their keys.

## Related Decisions

- [Name palette commands by category and verb phrase](20261026-230753-name-palette-commands-by-category-and-verb-phrase.md)
- [Split configuration into a terminal layer and an application layer](20261024-220000-split-configuration-into-two-layers.md)
