# Keymap

## Dispatch order

A key press is offered, in order, to:

1. the application keymap (nekomata's `[keymap]`),
2. the terminal keymap (felis's `[keymap]`, resolved for `nekomata`),
3. the terminal program.

A layer that does not consume the key forwards it to the next. A terminal
keymap entry bound to an action nekomata does not implement is forwarded to
the terminal program.

Neither keymap is consulted, and no keyboard input reaches the terminal,
while the search field, the command palette, the font size control, the
workspace list, the session picker, help or a paste confirmation is open, or while a workspace switch is in
progress. A pending sequence survives these conditions; losing window focus
cancels it only while key dispatch is active.

A key that ends a pending application sequence without completing it skips
the terminal keymap: the keys already pressed are discarded, and this key goes
to the terminal program.

## Default bindings

Application keymap:

| Sequence | Action |
| --- | --- |
| `alt+c` | `new_tab` |
| `alt+t` / `alt+shift+t` | `switch_tab` next / previous |
| `alt+w alt+s` | `split` below |
| `alt+w alt+v` | `split` right |
| `alt+w c` | `close_pane` |
| `alt+w h` / `j` / `k` / `l` | `focus_pane` left / down / up / right |
| `alt+shift+p` | `command_palette` |

Terminal keymap defaults are felis's, except that nekomata unbinds unmodified
`End` so that shells and editors receive it; binding it in felis's keymap
restores a binding.

## Application keymap entries

An entry has the same form as felis's keymap: `"keys" = { kind = "…", … }`.
A single keystroke is written as in felis (`alt+shift+t`, `f5`,
`ctrl+shift++`). Keystrokes separated by spaces form a sequence: `"alt+w c"`
means Alt+W followed by C.

| `kind` | Arguments | Action |
| --- | --- | --- |
| `new_tab` | — | Open a tab |
| `close_tab` | — | Close the active tab |
| `switch_tab` | `to = "next" \| "previous"` | Activate the next / previous tab |
| `split` | `direction = "below" \| "right"` | Open a pane below / right of the active pane |
| `close_pane` | — | Close the active pane (detaches its session) |
| `focus_pane` | `direction = "left" \| "down" \| "up" \| "right"` or `to = "next"` | Focus a neighboring / the next pane |
| `command_palette` | — | Open the command palette |
| `new_workspace` | — | Switch to a new workspace |
| `open_workspaces` | — | Open the workspace list |
| `unbind` | — | Remove bindings (see below) |

## Rules

- A user entry replaces the default binding for the same sequence.
- A sequence that starts with a shorter bound sequence can never complete and
  is removed: silently for a default, with a warning for a user entry. For
  example, binding `alt+w` removes every default `alt+w …` sequence, and a
  user entry `alt+t x` is dropped unless `alt+t` is unbound.
- `unbind` removes the sequence and every sequence that starts with it.
- One sequence can be spelled several ways: modifier names ignore case and
  order, and an uppercase letter implies Shift (`alt+T` is `alt+shift+t`).
  Entries are applied in byte order of their spelling, so of two spellings of
  one sequence the later-sorting one wins, and an `unbind` can remove a user
  entry spelled so that it sorts earlier (for example
  `"alt+w" = { kind = "unbind" }` removes `"Alt+w v"`); this is reported as a
  warning.
- Escape cancels a pending sequence, so it may appear only as the first
  keystroke. Entries such as `"ctrl+a escape"` are ignored with a warning.
- Entries containing keystrokes the window can never report are ignored with
  a warning:
  - Cmd+X / Cmd+C / Cmd+V on macOS, and Ctrl+X / Ctrl+C / Ctrl+V elsewhere,
    without Alt or Shift, which the window turns into clipboard events;
  - `super` outside macOS;
  - characters egui does not treat as keys, such as `é`.
- Copy, paste, font size, scrolling and search are terminal actions and are
  bound in felis's keymap, not here.

## Clipboard events

The window delivers Cmd+X / Cmd+C / Cmd+V on macOS, and Ctrl+X / Ctrl+C /
Ctrl+V elsewhere, without Alt or Shift, as clipboard events rather than key
presses, so neither keymap ever sees them and binding them has no effect.

- On macOS, Cmd+C copies the selection and Cmd+V pastes (with the paste
  confirmation rules of [terminal](terminal.md#paste)); Cmd+X does nothing.
- Elsewhere, Ctrl+C and Ctrl+X are sent to the terminal program as the
  control bytes 0x03 and 0x18. Ctrl+V sends 0x16 only when the clipboard holds
  text; otherwise it does nothing.
