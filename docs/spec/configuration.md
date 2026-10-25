# Configuration

nekomata reads two TOML documents. The terminal layer comes from felis's
shared document; the application layer comes from nekomata's own document.
Neither layer reads the other's document.

| Layer | macOS | Linux |
| --- | --- | --- |
| Terminal | `~/Library/Application Support/felis/config.toml` | `$XDG_CONFIG_HOME/felis/config.toml`, else `~/.config/felis/config.toml` |
| Application | `~/Library/Application Support/nekomata/config.toml` | `$XDG_CONFIG_HOME/nekomata/config.toml`, else `~/.config/nekomata/config.toml` |

On macOS, `~/.config/felis/config.toml` and `~/.config/nekomata/config.toml`
are not read.

Both documents are read once at GUI startup. Changes take effect when the GUI
restarts; there is no live reload.

## Terminal layer

felis-client-core resolves the shared document for client `nekomata`:
`[client.nekomata]` is overlaid on the common sections. nekomata ships no
terminal defaults or initial file of its own; unset settings use felis's
defaults.

nekomata applies:

- `font`: size, family, style, fallbacks and OpenType features. Families are
  installed OS fonts selected by name.
- Colors, including the extended palette and minimum contrast.
- Cursor color and blinking.
- `keymap` actions for system-clipboard copy and paste, font size, scrolling,
  prompt navigation and search, and `unbind`. Keys bound to other felis
  actions are passed to the terminal program (see [keymap](keymap.md)).

nekomata does not apply `window`, `shader`, `mouse` or `clipboard`. Settings
nekomata does not apply are accepted and have no effect.
`[client.nekomata]` accepts only felis's vocabulary; nekomata adds no keys to
it.

### Font size precedence

The resolved terminal font size is the configured size, or felis's default
when none is configured.

- At GUI startup, the resolved size replaces the restored workspace's saved
  size, so configuration edits are never hidden by a saved zoom.
- Switching to an existing workspace after startup restores that workspace's
  saved size.
- New workspaces and font-size reset use the resolved size.

### Errors

A missing file at the default path is not an error. Syntax errors, type
errors and read failures stop startup with an error.

## Application layer

A missing file means every default applies. The document has two sections.

```toml
[appearance.base.font]
family = "UDEV Gothic NF"
size_px = 15

[keymap]
"ctrl+a s" = { kind = "split", direction = "down" }
```

### `[appearance.base.font]`

Applies to UI text: tabs, menus, the search field, the command palette and
dialogs. It never affects terminal text.

- `family`: matched case-insensitively against installed families through the
  OS font matcher (CoreText on macOS, fontconfig on Linux). When it is empty or
  not found, nekomata warns and uses the default UI font.
- The default UI font is `/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc` on
  macOS and `/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc` (the
  Debian and Ubuntu location) on Linux, when present; otherwise egui's
  built-in font. A configured family leads proportional UI text and the
  default faces remain its fallbacks.
- `size_px`: integer or float. Defaults to felis's built-in default terminal
  font size (14 px), regardless of the configured terminal size.
  Values outside 4–72 are clamped into range; zero, negative and non-finite
  values use the default.

### `[keymap]`

Specified in [keymap](keymap.md).

### Errors

Unknown keys, unusable values and malformed `[keymap]` entries are reported as
warnings on standard error and ignored; the rest of the document applies.
TOML syntax errors and type errors stop startup with an error.
