# Configuration

nekomata reads two files:

- **Terminal settings** — fonts, colors, the cursor, and terminal keys such
  as copy and paste — come from **felis's** `config.toml`, shared with other
  felis clients.
- **Application settings** — the UI font and keys for tabs, panes and
  workspaces — come from **nekomata's** own `config.toml`.

As a rule of thumb, a setting that would make sense in a single-pane terminal
belongs to felis's file.

| | macOS | Linux |
| --- | --- | --- |
| felis | `~/Library/Application Support/felis/config.toml` | `~/.config/felis/config.toml` |
| nekomata | `~/Library/Application Support/nekomata/config.toml` | `~/.config/nekomata/config.toml` |

On Linux, `$XDG_CONFIG_HOME` replaces `~/.config` when set. Neither file is
required. Restart the GUI to apply changes.

## Terminal settings

Write felis's settings as you would for any felis client. To change something
for nekomata only, put it under `[client.nekomata]`:

```toml
[client.nekomata.font]
size_px = 16
```

nekomata honors fonts (family, size, style, fallbacks, OpenType features),
colors (including the extended palette and minimum contrast), the cursor, and
the keymap actions for copy, paste, font size, scrolling, prompt navigation
and search. It does not yet honor `window`, `shader`, `mouse` or `clipboard`;
such settings are silently ignored. Fonts are installed OS fonts chosen by
name.

The configured font size applies when the GUI starts and to new workspaces.
Switching to an existing workspace restores the size it was saved with.

## Application settings

```toml
[appearance.base.font]
# Tabs, menus, the search field, the command palette. Terminal text uses felis's [font].
family = "UDEV Gothic NF"   # default: Hiragino (macOS), Noto Sans CJK at its Debian/Ubuntu path (Linux), else egui's font
size_px = 15                # default: 14, felis's default terminal size

[keymap]
"alt+n"    = { kind = "new_tab" }
"alt+c"    = { kind = "unbind" }
"ctrl+a s" = { kind = "split", direction = "down" }
"ctrl+a v" = { kind = "split", direction = "right" }
```

Keys are written the same way as in felis's `[keymap]`. Separate keystrokes
with spaces for a sequence: `"alt+w c"` is Alt+W, then C. Your entries
replace the defaults for the same keys. Escape cancels a sequence in
progress; any other key that does not continue it is sent to the terminal
program.

| `kind` | Arguments | Action |
| --- | --- | --- |
| `new_tab` | — | New tab |
| `close_tab` | — | Close tab |
| `switch_tab` | `to = "next" \| "previous"` | Next / previous tab |
| `split` | `direction = "down" \| "right"` | New pane below / to the right |
| `close_pane` | — | Close pane (its shell keeps running) |
| `focus_pane` | `direction = "left" \| "down" \| "up" \| "right"` or `to = "next"` | Focus a neighboring / the next pane |
| `command_palette` | — | Command palette |
| `new_workspace` | — | New workspace |
| `open_workspaces` | — | Workspace list |
| `unbind` | — | Remove a binding |

Copy, paste, font size, scrolling and search are terminal actions; bind them
in felis's `[keymap]`.

## When something is wrong

- In nekomata's file, a mistyped key or an unusable value prints a warning on
  standard error and is skipped; the rest of the file still applies.
- In felis's file, such mistakes are silently ignored by nekomata; check the
  file with `felis config check --client nekomata` (see
  [Troubleshooting](troubleshooting.md#a-setting-has-no-effect)).
- In either file, a TOML syntax error, or a type error outside `[keymap]`,
  stops nekomata from starting. A malformed keymap entry is only dropped.

Some bindings are rejected with a warning: keys the window never reports as
key presses, such as Cmd+C, and sequences that can never complete because a
shorter prefix is bound on its own. The full rules are in the
[keymap specification](../spec/keymap.md) and the
[configuration specification](../spec/configuration.md).
