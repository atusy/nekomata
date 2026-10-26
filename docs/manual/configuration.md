# Configuration

nekomata reads terminal settings — fonts, colors, the cursor, and terminal
keys such as copy and paste — from **felis's** `config.toml`, shared with
other felis clients.

| macOS | Linux |
| --- | --- |
| `~/Library/Application Support/felis/config.toml` | `~/.config/felis/config.toml` |

On Linux, `$XDG_CONFIG_HOME` replaces `~/.config` when set. The file is not
required. Restart the GUI to apply changes.

Keys for tabs, panes and workspaces, and the UI font, are fixed for now.

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

## When something is wrong

- Mistyped keys and unusable values are silently ignored by nekomata; check
  the file with `felis config check --client nekomata` (see
  [Troubleshooting](troubleshooting.md#a-setting-has-no-effect)).
- A TOML syntax error or a wrongly typed setting stops nekomata from
  starting. A malformed `keymap` or `font.fallback` entry is only dropped.

Keys the window never reports as key presses, such as Cmd+C on macOS, cannot
be bound. The full rules are in the [keymap specification](../spec/keymap.md)
and the [configuration specification](../spec/configuration.md).
