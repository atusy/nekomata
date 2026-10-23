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

nekomata honors:

- `[theme]` `foreground`, `background` and `palette` (named slots and
  `indexed`). Unset values keep nekomata's current Nord-like colors, not
  felis's xterm defaults. The configured foreground, background and cursor
  are also announced to felis (`ConfigureTheme`) so `OSC 10/11/12` queries
  answer with what nekomata draws, and the background luminance decides the
  dark/light color-scheme report.
- `[cursor] color`. Unset keeps the accent-colored cursor rather than
  felis's reverse-video cursor.
- `[font] size_px`, clamped into nekomata's 6–72 band.

Other sections and keys (`font.family` and the other font faces, `keymap`,
`shader`, `window`, `mouse`, `clipboard`, `cursor.blink*`) are not applied.
egui loads font files by path (`NEKOMATA_FONT`) rather than by family name,
and nekomata's keymap and drawing do not share felis's model. Tab-bar chrome
keeps its fixed colors.

### Font size is a default, not the session's value

The configured size is the *default* text size. A nekomata session stores a
font size only after the user zooms it (`font_size: Option<f32>` in the
snapshot); the effective size is `session override.unwrap_or(config
default)`. Resetting the zoom clears the override, so the session follows
the configuration again. A future per-pane override extends the same chain:
`pane override.or(session override).unwrap_or(config default)`.

Changing the snapshot field changes the nekomata daemon protocol, so its
version is bumped. As decided in
[20261023-120000](20261023-120000-separate-workspaces-from-windows.md), a
newer GUI reports the incompatible running daemon instead of replacing it;
users stop it once (`nekomata daemon stop`), which discards in-memory
layouts but not felis shells.

### Consequences

Positive:

- felis and nekomata share colors from one file, with per-client overlays.
- Typos and unknown keys produce the same diagnostics felis gives.
- Sessions that were never zoomed follow later changes to the configured
  size; zoomed sessions keep their zoom.

Negative:

- Upgrading requires one nekomata daemon restart.
- The file is read once at startup; edits apply after restarting the GUI.
  Hot reload (`felis_client_core::config_watcher`) is deferred: live felis
  connections would need a channel for `ConfigureTheme` besides input.
- Settings that felis honors but nekomata ignores may surprise users; the
  README lists what nekomata applies.

### Confirmation

Unit tests resolve config text to verify overlay precedence, inert foreign
overlays, color and palette defaults, invalid-value fallback, font size
clamping, and that resetting zoom returns to the configured default. Daemon
tests cover snapshots without a font override. Native GUI validation is
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
