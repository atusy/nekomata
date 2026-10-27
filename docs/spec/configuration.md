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
initial file and no terminal defaults of its own, except that it unbinds
unmodified `End` (see [keymap](keymap.md#default-bindings)); unset settings
use felis's defaults.

nekomata applies:

- `font`: size, family, style, fallbacks and OpenType features. Families are
  installed OS fonts selected by name.
- Colors, including the extended palette and minimum contrast.
- Cursor color and blinking.
- `keymap` actions for system-clipboard copy and paste, font size, scrolling,
  prompt navigation and search, and `unbind`. Keys bound to other felis
  actions are passed to the terminal program (see [keymap](keymap.md)).
  Chords the window turns into clipboard events cannot be bound (see
  [keymap](keymap.md#clipboard-events)).
- `clipboard.osc_52`: where a program's OSC 52 write lands (see
  [terminal](terminal.md#osc-52-clipboard-writes)). `"mirror"`, felis's
  default, keeps it in the felis daemon's mirror; `"system"` also writes it to
  the system clipboard. As in felis, `clipboard.use_os_clipboard = false`
  keeps OSC 52 writes off the system clipboard even under `"system"`.

nekomata does not apply `window`, `shader` or `mouse`, and applies
`clipboard.use_os_clipboard` only to OSC 52 writes: nekomata's own copy and
paste always use the system clipboard. Settings nekomata does not apply are
accepted and have no effect.
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

A missing file at the default path is not an error. felis-client-core
classifies every other problem as an error or a warning. Errors — syntax
errors, read failures and most type errors — stop startup. Warnings do not
stop startup and are not shown: the offending item is skipped (unknown keys,
a malformed individual `keymap` or `font.fallback` entry) or adjusted (an
out-of-range font size is clamped). `felis config check --client nekomata`
reports both kinds.

## Application layer

A missing file means every default applies. The document has two sections.

```toml
[appearance.base.font]
family = "UDEV Gothic NF"
size_px = 15

[keymap]
"ctrl+a s" = { kind = "split", direction = "below" }
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
TOML syntax errors, type errors and read failures other than a missing file
stop startup with an error.
