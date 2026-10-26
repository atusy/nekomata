# Make the command palette the only command menu

## Context

Commands should be listed and named in one place, so that surfaces do not
drift apart as the command list grows.

## Decision

A single command registry owns every window command and its label. The
command palette is the only surface that lists them, and the button at the
left of the tab bar opens it. A command that needs further input, such as
changing the font size, opens its own UI from the palette.

## Invariants

> The invariants below are normative; the mechanisms that satisfy them are
> deliberately unspecified.

- Removing the menu removes no capability: every former menu command is in
  the palette.
- The palette stays operable with the pointer alone.
- A command's label is written once, in the registry.

## Considered Options

1. **Palette only.** Chosen. One list, one vocabulary, and the button keeps a
   pointer entry point.
2. **Menu and palette both reading the registry.** Removes the duplication, but
   the menu needs its own layout, which tends to bring back its own wording.
3. **Separate hand-written lists.** No migration, but they had already drifted.

## Consequences

- Adding a command means adding one registry entry.
- Font size moves from the menu's inline row into a control one click further
  away, and the menu's visual grouping is replaced by label categories.

## Related Decisions

- [Name palette commands by category and verb phrase](20261026-230753-name-palette-commands-by-category-and-verb-phrase.md)
- [Split configuration into a terminal layer and an application layer](20261024-220000-split-configuration-into-two-layers.md)
