# Audit of felis rendering reuse

Audited against felis 0.3.1, revision
`2a3030b237947497708b962ba6be8c2adade4951`, on 2026-10-24.

The production terminal pipeline uses the complete felis GPU renderer. There
is no remaining nekomata glyph rasterizer, row mesh builder or decoration
renderer on that path. Historical implementations under `tests/support` are
comparison fixtures only. This does **not** mean every optional feature of the
standalone felis application is enabled in nekomata.

## Source comparison

Every upstream production module and WGSL shader was compared byte-for-byte.
Only `src/lib.rs` differs: it accepts the host device/queue and shared font stack,
exposes a sampleable offscreen view, and preserves the host's device-loss
callback. `Cargo.toml` replaces workspace inheritance with pinned dependencies.
See [the patch provenance](../vendor/felis-render-wgpu/UPSTREAM.md).
Upstream benchmark programs, proptest regression seeds and eight binary golden
fixtures are omitted. Upstream dependency unit tests were not run; the reported
tests are nekomata integration and regression tests.

## Active reuse and integration boundaries

| Responsibility | Shared implementation | nekomata integration |
| --- | --- | --- |
| Font discovery, fallback, shaping and cell metrics | `felis-shaping`, renderer glyph/atlas modules | `render::load_fonts`, `Renderer::cell_size`, `reload_font_size` |
| Row damage, instance buffers, text/background/decorations, glyph eviction | Complete `felis-render-wgpu` pipeline | `Renderer::paint` submits to egui's device; `take_glyph_repaint_request` schedules atlas retries |
| Terminal colors, palette overrides, resets, reverse video | `renderer_effect`, effective theme, GPU theme handling | `colors::Colors`, `set_theme`, `set_reverse_video` |
| Cursor appearance and focus | GPU cursor implementation | `set_window_focused`; native focus and terminal cursor state gate presentation |
| Cursor blinking and repaint deadlines | `cursor_blink::BlinkClock` | `presentation::Presentation`; reset on input/cursor movement and request next deadline |
| IME composition | GPU `PreeditOverlay` | egui character ranges converted to UTF-8 byte ranges; commit/focus loss clears overlay |
| Selection normalization, word/line selection and soft-wrap/wide-cell copy | `Selection`, `GridPos` | Pane retains shared selection; GPU receives its normalized range; clipboard uses `extract_text` |
| Kitty images, decoding, placement, atlas/upload/draw | `ImageShadow`, `ImageLayout`, GPU image modules | Session receives images; renderer passes dirty images and viewport-adjusted layout |
| Frame demand and unchanged-frame reuse | `PullScheduler`, `RedrawScheduler` | Local visible panes request another complete cycle after GPU submission; hidden panes stop pulling; SSH retains eager delivery |
| Ordered and bounded input | `OutgoingQueue` | Async writer; resize coalescing and transport-close discard |
| Session recovery | `redial_session`, shared retry policy | Reattach original identity, reset presentation state; explicit SSH socket uses literal-safe relay |

The application owns egui event translation, pane geometry, workspace/session
lifecycle, texture registration and scheduling callbacks. These are embedding
boundaries, not parallel implementations of terminal rendering. Frame-pull
acknowledgment follows queue submission, **not** confirmed native surface
presentation; eframe owns the surface and vsync.

## Upstream facilities not activated

| Facility | Reason / prerequisite |
| --- | --- |
| Search, confirmation and hyperlink-preview overlays | nekomata does not currently expose the corresponding terminal UI; upstream renderer code remains available |
| Rectangular selection | Shared model and GPU range support it, but no gesture is assigned |
| Custom post shaders, cursor trail and shader clock | No product configuration; upstream offscreen constructor rejects post shaders, so this needs a further embedding API |
| Font-family/features hot reload, opacity and minimum contrast configuration | No such configuration surface; shared defaults and font-size reload are active |
| Standalone surface acquisition, reconfiguration and presentation retry | eframe owns the native surface; allocating another would conflict with embedding |
| Standalone warmup worker | Creates its own GPU device; embedding must use the host device. Font stack is shared across panes instead |
| Optional mapped upload strategy | Host device does not request its optional features; renderer's staged strategy is active |
| Frame capture/readback | Used only by GPU validation; unnecessary CPU transfer for interactive composition |

The audit found and closed missing glyph-repaint, blink, preedit and selection
model connections. No further duplicate rendering implementation was found for
the currently exposed terminal features. New optional UI/configuration features
should use these upstream entry points when introduced.

See [VM evidence and performance limitations](vitro-rendering-validation.md).
