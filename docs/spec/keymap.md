# Keymap

## Dispatch order

A key press is offered, in order, to:

1. nekomata's application shortcuts, which are fixed,
2. the terminal keymap (felis's `[keymap]`, resolved for `nekomata`),
3. the terminal program.

A layer that does not consume the key forwards it to the next. A terminal
keymap entry bound to an action nekomata does not implement is forwarded to
the terminal program.

Neither the application shortcuts nor the terminal keymap is consulted, and
no keyboard input reaches the terminal, while the search field, the command
palette, the workspace list, the session picker, help or a paste confirmation
is open, or while a workspace switch is in progress. A pending sequence survives these conditions; losing window focus
cancels it only while key dispatch is active.

A key that ends a pending application sequence without completing it skips
the terminal keymap: the keys already pressed are discarded, and this key goes
to the terminal program.

## Default bindings

Application shortcuts:

| Keys | Action |
| --- | --- |
| `alt+c` | Open a tab |
| `alt+t` / `alt+shift+t` | Activate the next / previous tab |
| `alt+w alt+s` | Open a pane below the active pane |
| `alt+w alt+v` | Open a pane right of the active pane |
| `alt+w c` | Close the active pane (detaches its session) |
| `alt+w h` / `j` / `k` / `l` | Focus the pane to the left / below / above / right |
| `alt+shift+p` | Open the command palette |

Escape cancels a pending `alt+w` sequence.

Terminal keymap defaults are felis's, except that nekomata unbinds unmodified
`End` so that shells and editors receive it; binding it in felis's keymap
restores a binding. Copy, paste, font size, scrolling and search are terminal
actions, bound in felis's keymap.

## Clipboard events

The window delivers Cmd+X / Cmd+C / Cmd+V on macOS, and Ctrl+X / Ctrl+C /
Ctrl+V elsewhere, without Alt or Shift, as clipboard events rather than key
presses, so neither keymap ever sees them and binding them has no effect.

- On macOS, Cmd+C copies the selection and Cmd+V pastes (with the paste
  confirmation rules of [terminal](terminal.md#paste)); Cmd+X does nothing.
- Elsewhere, Ctrl+C and Ctrl+X are sent to the terminal program as the
  control bytes 0x03 and 0x18. Ctrl+V sends 0x16 only when the clipboard holds
  text; otherwise it does nothing.
