# Reuse felis's renderer core through egui meshes

| | |
| --- | --- |
| Status | proposed |
| Date | 2026-10-24 |
| Decision-makers | atusy |

## Context and Problem Statement

nekomata traverses the terminal twice on every repaint, creates a text layout
for each cell, and submits separate egui shapes for cell backgrounds and glyphs.
The latest felis separates its renderer-neutral painter, glyph atlas and shaping
cache into `felis-render-core`. Sharing those components avoids maintaining a
second terminal renderer and removes the per-cell text layout path.

## Decision Drivers

- Improve large-screen, scrolling and split-pane rendering performance.
- Use the current upstream felis libraries and preserve persistent sessions.
- Keep egui menus, tabs and overlays working over terminal content.
- Preserve a single coordinate system for drawing, mouse input and IME.

## Considered Options

1. Optimize the existing per-cell egui text renderer.
2. Translate `felis-render-core` display lists into batched egui meshes.
3. Embed `felis-render-wgpu` with a shared device and render target.

## Decision Outcome

Choose option 2. Felis owns glyph selection, shaping, rasterization, cell
appearance and terminal decorations. nekomata uploads atlas deltas to egui
textures and translates the display list into meshes. Cache unchanged rows;
regenerate affected rows when screen damage or paint-time state changes.

Option 3 currently needs a new upstream API for sharing a device and render
texture, plus reconciliation of egui's and felis's different wgpu versions.
Its existing offscreen API owns its device and targets CPU frame capture;
reading those pixels back every frame would defeat the performance objective.
The renderer-neutral library permits direct GPU texture drawing through egui
without that transfer or a vendored renderer fork.

### Consequences

Positive:

- Shared rendering behavior and glyph caches with upstream felis.
- Batched terminal geometry without one egui text layout per cell.
- Existing egui compositing and platform backends remain usable.

Negative:

- nekomata owns a small display-list adapter and atlas texture uploads.
- Cached rows and glyph atlas recycling require coordinated invalidation.
- Font metrics and glyphs follow felis and can differ from egui's former output.

### Confirmation

Run ordinary Rust tests, isolated daemon integration tests, formatting, clippy,
and Nix package checks. Add rendering regressions for damage invalidation,
selection/cursor changes, glyph atlas uploads, wide text and DPI changes.
Compare the previous renderer and shared renderer on the same terminal screen
in release mode, including steady repaint and changed-screen workloads.
Report native GUI visual verification separately.
