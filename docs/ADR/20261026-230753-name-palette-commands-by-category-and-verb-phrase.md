# Name palette commands by category and verb phrase

## Context

The command palette is nekomata's only command menu, and its list is expected
to grow to thirty or forty commands, more than half of them in pairs (next and
previous) or sets of four (directions). The labels had no shared grammar:
"Next tab" started with an adjective, "Close pane (detach)" put a side effect
in parentheses, and "Switch workspace" lacked the ellipsis the menu gave it.
Labels are an implementation detail, not specification, so this record is the
only place their convention is kept.

## Decision

Every palette label is `Category: verb phrase`.

- The category comes from a closed set owned by the command registry:
  Workspace, Tab, Pane, Session, Scrollback, Clipboard, Font and Window.
  Adding a category is a change to that set.
- The verb phrase is in sentence case and starts with an imperative verb.
  Words such as "New", "Next" and "Previous" are not verbs: "Tab: Create",
  "Tab: Switch to next", "Pane: Focus next".
- The phrase omits its object when the object is the category ("Tab: Close")
  and keeps it otherwise ("Scrollback: Jump to next prompt").
- A phrase that names no target acts on the active one: "Tab: Close" closes
  the active tab.
- A trailing "…" marks a command that asks for a choice or input before it
  acts ("Workspace: Switch…", "Session: Attach…", "Scrollback: Find…",
  "Font: Change size…"). A confirmation alone does not count.
- Side effects and caveats go in the command's description, not in the label.
- Alternative words go in the command's search keywords, not in the label.
- Direction words describe the result: a split names where the new pane goes
  (right, below); focus names the direction of travel (left, down, up, right).
  The application keymap uses the same words.
- Nouns follow the manual: a shell is a "session".

## Considered Options

1. **`Category: verb phrase`.** Chosen. Typing a category narrows the list to
   its group, the list sorts into groups, and the form is familiar from other
   command palettes. It fixes the word order, so the filter has to match words
   in any order and keywords have to cover natural phrasings such as "new
   tab".
2. **Verb phrase alone ("Split right", "Switch to next tab").** Natural
   English, but forty entries show no structure and cannot be grouped.
3. **Object first without a separator ("Tab switch to next").** Groups
   alphabetically, but reads unnaturally and hides where the category ends.
4. **Category as a separate, dimmed column beside a full verb phrase.** Reads
   well, but repeats the object ("Tab  Switch to next tab") and needs a custom
   row layout.

## Consequences

- Paired commands line up ("Tab: Switch to next", "Tab: Switch to previous").
- Labels no longer match natural search phrases on their own; the palette
  depends on keywords and order-independent matching to stay findable.
- The application keymap's `split` direction is spelled `below` rather than
  `down`, so configuration and labels use one word.

## Related Decisions

- [Make the command palette the only command menu](20261026-230754-make-the-command-palette-the-only-command-menu.md)
