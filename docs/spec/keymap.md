# Keymap

## Dispatch order

A key press is offered, in order, to:

1. the application keymap (nekomata's `[keymap]`),
2. the terminal keymap (felis's `[keymap]`, resolved for `nekomata`),
3. the terminal program.

A layer that does not consume the key forwards it to the next. A terminal
keymap entry bound to an action nekomata does not implement is forwarded to
the terminal program.

Neither keymap is consulted, and no keyboard input reaches the terminal, while
the search field, the command palette, the font size control, the workspace
list, the session picker or a paste confirmation is open, or while a
workspace switch is in progress. A pending sequence survives these conditions;
losing window focus cancels it only while key dispatch is active.

Pressing a modifier key on its own neither continues nor ends a pending
sequence, so `alt+w` may be released and Alt pressed again for `alt+s`.

Key repeat from a held key that a sequence consumed (one that started,
continued or completed it, or the Escape that cancelled it, such as a held
`alt+w` or the `c` of `alt+w c`) is ignored. Holding a key bound to an action on its own repeats
the action.

A key that ends a pending application sequence without completing it skips
the terminal keymap: the keys already pressed are discarded, and this key goes
to the terminal program.

## Default bindings

The application keymap has no default bindings.

Terminal keymap defaults are felis's, except that nekomata unbinds unmodified
`End` so that shells and editors receive it; binding it in felis's keymap
restores a binding.

nekomata keeps felis's Ctrl+Shift+- font-size binding on every platform,
although that chord is Ctrl+_ on US layouts. The window reports it as `-` with
Shift, never as `_`, so with the binding removed it reaches the program as
Ctrl+Shift+-, which has no control byte. Ctrl+/ sends the Ctrl+_ byte (0x1F)
to programs that read control bytes, that is, unless they enable
modifyOtherKeys or the Kitty keyboard protocol.

## Application keymap entries

An entry has the same form as felis's keymap: `"keys" = { kind = "…", … }`.
A single keystroke is written as in felis (`alt+shift+t`, `f5`,
`ctrl+shift++`). Keystrokes separated by spaces form a sequence: `"alt+w c"`
means Alt+W followed by C.

Every public command has a stable, case-sensitive `category:operation` kind.
The palette and application bindings use the same command definitions. All
canonical kinds below take no arguments; extra fields cause that entry to be
dropped with a warning. Labels may change without changing the kind.

| `kind` | Command |
| --- | --- |
| `workspace:create` | Workspace: Create |
| `workspace:switch` | Workspace: Switch… |
| `tab:create` | Tab: Create |
| `tab:switch_next` | Tab: Switch to next |
| `tab:switch_previous` | Tab: Switch to previous |
| `tab:close` | Tab: Close |
| `pane:split_right` | Pane: Split right |
| `pane:split_below` | Pane: Split below |
| `pane:focus_next` | Pane: Focus next |
| `pane:focus_left` | Pane: Focus left |
| `pane:focus_down` | Pane: Focus down |
| `pane:focus_up` | Pane: Focus up |
| `pane:focus_right` | Pane: Focus right |
| `pane:close` | Pane: Close |
| `session:attach` | Session: Attach… |
| `scrollback:find` | Scrollback: Find… |
| `scrollback:jump_next_prompt` | Scrollback: Jump to next prompt |
| `scrollback:jump_previous_prompt` | Scrollback: Jump to previous prompt |
| `scrollback:scroll_line_up` | Scrollback: Scroll one line up |
| `scrollback:scroll_line_down` | Scrollback: Scroll one line down |
| `scrollback:scroll_half_page_up` | Scrollback: Scroll half a page up |
| `scrollback:scroll_half_page_down` | Scrollback: Scroll half a page down |
| `scrollback:scroll_to_top` | Scrollback: Scroll to top |
| `scrollback:scroll_to_bottom` | Scrollback: Scroll to bottom |
| `clipboard:copy_selection` | Clipboard: Copy selection |
| `clipboard:paste` | Clipboard: Paste |
| `font:change_size` | Font: Change size… |
| `font:increase_size` | Font: Increase size |
| `font:decrease_size` | Font: Decrease size |
| `font:reset_size` | Font: Reset size |
| `command_palette:open` | Command palette: Open |

Labels ending in `…` open a selection or control. Binding them opens that same
UI; it does not choose a session or workspace. `command_palette:open` is
bindable but is not listed inside the palette itself. Terminal operations use
the same execution and confirmation rules as their felis bindings.

`unbind` is a configuration directive, not a command. It takes no arguments
and removes bindings as described below.

## Rules

- A sequence that starts with a shorter bound sequence can never complete and
  is removed with a warning. For example, if `alt+t` is bound, a user entry
  `alt+t x` is dropped.
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
- nekomata bindings can invoke public terminal commands as well as workspace,
  tab and pane commands. felis's shared keymap remains the fallback and uses
  its own unchanged vocabulary.
- The window has no zoom of its own: Cmd/Ctrl with `+`, `=`, `-` or `0`, when
  not bound to a font size action, reaches the terminal program like any other key
  and never rescales the UI.

Unknown application `kind` values are dropped with a warning suggesting that
terminal bindings may belong in felis's `config.toml`. This is a general hint,
not a claim that felis recognizes the kind or that nekomata implements it.

## Clipboard events

The window delivers Cmd+X / Cmd+C / Cmd+V on macOS, and Ctrl+X / Ctrl+C /
Ctrl+V elsewhere, without Alt or Shift, as clipboard events rather than key
presses, so neither keymap ever sees them and binding them has no effect.

- On macOS, Cmd+C copies the selection and Cmd+V pastes (with the paste
  confirmation rules of [terminal](terminal.md#paste)); Cmd+X does nothing.
- Elsewhere, Ctrl+C and Ctrl+X are sent to the terminal program as the
  control bytes 0x03 and 0x18. Ctrl+V sends 0x16 only when the clipboard holds
  text; otherwise it does nothing.
