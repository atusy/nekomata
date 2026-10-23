# Configure nekomata through the shared felis config file

| | |
| --- | --- |
| Status | accepted |
| Date | 2026-10-24 |
| Decision-makers | atusy |

## Context and Problem Statement

nekomata hardcodes its terminal colors and its 15px font size. Users who
already describe their terminal in felis's `config.toml` have to accept
different colors in nekomata, and there is no way to change the default text
size other than zooming every session by hand. Which file should hold
nekomata settings, and how does a configured font size relate to the size
each retained nekomata session already stores?

## Decision Drivers

- One description of colors and fonts for every felis client.
- Reuse felis's parser, overlay merge and diagnostics instead of inventing a
  second dialect.
- A broken or unknown setting must never stop the GUI from starting.
- Apply only settings nekomata can honor; silently misapplying a section is
  worse than ignoring it.
- Leave room for per-pane font sizes later.

## Considered Options

1. A separate `nekomata/config.toml` with its own schema.
2. The shared felis `config.toml`, resolved with the `[client.nekomata]`
   overlay through `felis-client-core::config`.
3. Environment variables and CLI flags only.

## Decision Outcome

Choose option 2. At GUI startup nekomata reads felis's `config.toml`
(`felis_client_core::config::config_path()`) and resolves it with client id
`nekomata`: top-level sections apply to every felis client and
`[client.nekomata]` is deep-merged over them. Diagnostics are printed to
stderr; a file with errors (or one that cannot be read) falls back to
built-in defaults rather than failing startup. CLI and daemon subcommands do
not read the file.

nekomata honors what maps onto the shared felis renderer it embeds:

- `[theme]` `foreground`, `background`, `palette` (named slots and
  `indexed`) and `minimum_contrast`. Unset colors keep nekomata's current
  Nord-like colors, not felis's xterm defaults. The configured colors are
  also announced to felis (`ConfigureTheme`) so `OSC 4/10/11/12` queries
  answer with what nekomata paints, and the background luminance decides
  the dark/light color-scheme report. felis does not validate hex colors at
  load time, so nekomata reports malformed ones itself.
- `[cursor] color`. Unset keeps the accent-colored cursor rather than
  felis's reverse-video cursor.
- `[font]` `family`, `features`, `fallback`, `bold`, `italic` and
  `bold_italic`, passed to font discovery and the renderer as felis does.
  The packaged app sets `NEKOMATA_FONT` to its bundled font, which
  restricts discovery to that file; naming `font.family` discovers among
  the system's fonts instead, so the setting works in the packaged app.
- `[font] size_px`, clamped into nekomata's 6–72 snapshot band.

Other sections and keys (`keymap`, `shader`, `window`, `mouse`, `clipboard`,
`cursor.blink*`) are not applied: nekomata's shortcuts, window and input
handling do not share felis's model, and the embedded renderer does not run
post-process shaders or translucent backgrounds. Tab-bar and menu chrome
keep their fixed colors.

### Font size is a default, not the session's value

The configured size is the *default* text size. A nekomata session stores a
font size only after the user zooms it (`font_zoom: Option<f32>` in the
snapshot); the effective size is `session override.unwrap_or(config
default)`. Resetting the zoom clears the override, so the session follows
the configuration again. A future per-pane override extends the same chain:
`pane override.or(session override).unwrap_or(config default)`.

The override is a new optional snapshot field rather than a new type for
the existing `font_size`, which keeps carrying the effective size. Older
GUIs and the running nekomata daemon parse the same snapshots, so the
protocol version stays unchanged and upgrading needs no daemon restart.
A daemon older than this change drops the unknown field, so a zoom saved
through it is forgotten when it is restored; changing the protocol version
instead would have stranded users behind a daemon that, as decided in
[20261023-120000](20261023-120000-separate-workspaces-from-windows.md),
rejects even the stop request from a newer client.

### Consequences

Positive:

- felis and nekomata share colors and fonts from one file, with per-client
  overlays.
- Typos and unknown keys produce the same diagnostics felis gives.
- Sessions that were never zoomed follow later changes to the configured
  size; zoomed sessions keep their zoom.

Negative:

- Sessions zoomed before upgrading, or saved through an older running
  daemon, return to the configured default once.
- The snapshot keeps a `font_size` field that only older versions read.
- The file is read once at startup; edits apply after restarting the GUI.
  Hot reload (`felis_client_core::config_watcher`) is deferred: live felis
  connections would need to resend `ConfigureTheme`, and each pane's
  renderer would need to rebuild its fonts and theme.
- Settings that felis honors but nekomata ignores may surprise users; the
  README lists what nekomata applies.
- Warnings reach only stderr, which a Dock or Finder launch does not show;
  the window shows only the error that made nekomata ignore the file.

### Confirmation

Unit tests resolve config text to verify overlay precedence, inert foreign
overlays, color and palette defaults and reports, invalid-value fallback,
font face mapping, font size clamping, and that resetting zoom returns to the
configured default. Snapshot validation covers sessions without a font
override. Native GUI validation is
reported separately.

## Pros and Cons of the Options

### Separate nekomata config file

Free to diverge, but duplicates colors users already wrote for felis and
needs its own parser and diagnostics.

### Shared felis config file

Reuses parsing, overlays and diagnostics. nekomata must document which
shared sections it ignores, and follows felis's schema changes when the
pinned revision moves.

### Environment and flags only

No file format to maintain, but unsuitable for a 16-color palette and
invisible to GUI launches from the Dock.
