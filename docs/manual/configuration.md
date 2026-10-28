# Configuration

nekomata reads two files:

- **Terminal settings** — fonts, colors, the cursor, and shared terminal key
  bindings — come from **felis's** `config.toml`, shared with other felis clients.
- **Application settings** — the UI font and bindings for all public commands —
  come from **nekomata's** own `config.toml`.

Terminal appearance belongs to felis's file. nekomata's keymap can bind any
public command, including terminal operations; it takes precedence over the
shared terminal keymap.

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
and search. It does not yet honor `window`, `shader` or `mouse`; such settings
are silently ignored. Fonts are installed OS fonts chosen by name.

Programs can copy to the system clipboard with OSC 52 escape sequences only
after you opt in, as in felis:

```toml
[clipboard]
osc_52 = "system"
```

The configured font size applies when the GUI starts and to new workspaces.
Switching to an existing workspace restores the size it was saved with.

## Application settings

```toml
[appearance.base.font]
# Tabs, menus, the search field, the command palette. Terminal text uses felis's [font].
family = "UDEV Gothic NF"   # default: Hiragino (macOS), Noto Sans CJK at its Debian/Ubuntu path (Linux), else egui's font
size_px = 15                # default: 14, felis's default terminal size

[keymap]
"alt+n"    = { kind = "tab:create" }
"ctrl+a s" = { kind = "pane:split", direction = "below" }
"ctrl+a v" = { kind = "pane:split", direction = "right" }
```

Keys are written the same way as in felis's `[keymap]`. Separate keystrokes
with spaces for a sequence: `"alt+w c"` is Alt+W, then C. The application
keymap has no default bindings. Escape cancels a sequence in
progress; any other key that does not continue it is sent to the terminal
program.

Kinds use `category:operation`, matching the palette's categories and verbs.
For example, bind the session picker and font size controls directly:

```toml
[keymap]
"alt+a" = { kind = "session:attach" }
"alt+f" = { kind = "font:change_size" }
"alt+=" = { kind = "font:increase_size" }
"alt+-" = { kind = "font:decrease_size" }
"alt+0" = { kind = "font:reset_size" }
```

See the [complete kind catalog](../spec/keymap.md#application-keymap-entries).
`unbind` removes bindings and is not an executable command. Shared terminal
bindings can still be configured in felis's `[keymap]` using felis's syntax.

## When something is wrong

- In nekomata's file, a mistyped key or an unusable value prints a warning on
  standard error and is skipped, or, for an out-of-range size, clamped; the
  rest of the file still applies.
- In felis's file, such mistakes are silently ignored by nekomata; check the
  file with `felis config check --client nekomata` (see
  [Troubleshooting](troubleshooting.md#a-setting-has-no-effect)).
- A TOML syntax error in either file stops nekomata from starting, and so
  does a wrongly typed setting in nekomata's file and most wrongly typed
  settings in felis's file. A malformed keymap entry is only dropped, and so
  is a malformed `font.fallback` entry in felis's file.

Some bindings are rejected with a warning: keys the window never reports as
key presses, such as Cmd+C, and sequences that can never complete because a
shorter prefix is bound on its own. The full rules are in the
[keymap specification](../spec/keymap.md) and the
[configuration specification](../spec/configuration.md).
