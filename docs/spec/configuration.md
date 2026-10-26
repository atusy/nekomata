# Configuration

nekomata reads one TOML document: felis's shared document, which provides the
terminal layer of the
[two-layer configuration](../ADR/20261024-220000-split-configuration-into-two-layers.md).
nekomata has no application-layer document yet; its tab, pane and workspace
keys and its UI font are fixed.

| macOS | Linux |
| --- | --- |
| `~/Library/Application Support/felis/config.toml` | `$XDG_CONFIG_HOME/felis/config.toml`, else `~/.config/felis/config.toml` |

On macOS, `~/.config/felis/config.toml` is not read.

The document is read once at GUI startup. Changes take effect when the GUI
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

A missing file at the default path is not an error. felis-client-core
classifies every other problem as an error or a warning. Errors — syntax
errors, read failures and most type errors — stop startup. Warnings do not
stop startup and are not shown: the offending item is skipped (unknown keys,
a malformed individual `keymap` or `font.fallback` entry) or adjusted (an
out-of-range font size is clamped). `felis config check --client nekomata`
reports both kinds.
