# Name palette commands by category and verb phrase

## Context

Palette labels must stay easy to scan and to search even when the list grows
long.

## Decision

Every palette label is `Category: verb phrase`, such as `Tab: Switch to next`.

- Categories form a closed set owned by the command registry.
- The phrase starts with an imperative verb.
- Alternative words go in search keywords, not in the label.

## Considered Options

1. **`Category: verb phrase`.** Chosen. Typing a category narrows the list to
   its group, and paired commands line up. The fixed word order needs
   order-independent matching and keywords to keep natural phrasings such as
   "new tab" findable.
2. **Verb phrase alone ("Switch to next tab").** Reads naturally, but a long
   list shows no grouping.
3. **Category as a separate dimmed column.** Groups visually, but repeats the
   object ("Tab  Switch to next tab").

## Consequences

- Labels alone no longer match every natural search phrase; matching relies on
  keywords.
- Configuration values use the same words as labels, so the keymap's `split`
  direction is `below` rather than `down`.

## Related Decisions

- [Make the command palette the only command menu](20261026-230754-make-the-command-palette-the-only-command-menu.md)
