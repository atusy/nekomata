# Share felis client policies and the GPU renderer

| | |
| --- | --- |
| Status | accepted |
| Date | 2026-10-24 |
| Decision-makers | atusy |

## Context

The renderer-core integration removes per-cell text layouts, but nekomata still
owns row meshes, atlas texture uploads, decoration tessellation and several
client policies already implemented in felis. The user requested all remaining
reuse candidates, with analysis and runtime verification through Vitro.

## Options

1. Retain the egui mesh adapter and port individual policies.
2. Share the complete felis GPU renderer and client-core policies while keeping
   egui for application chrome and nekomata for workspace ownership.
3. Replace the entire application UI with the standalone felis client.

## Decision

Implement option 2. Match egui and felis on wgpu 30. Render panes into GPU
textures on egui's device and queue, then compose them with egui. No per-frame
CPU pixel readback. The upstream renderer needs a small embedding API; retain
its source and attribution with a documented local patch until upstream exposes
that seam. Keep the shared renderer's own glyph, row, image and GPU caches.

Reuse client-core's PullScheduler, OutgoingQueue, Selection, RendererEffect,
redial_session and ImageShadow. Preserve ordered keystrokes, bounded queues,
explicit detach on pane closure and session identity across reconnection.

## Consequences

The application keeps its tab/workspace UI and gains shared terminal policies.
The embedding patch must be audited on upstream updates, and migrating egui's
version and graphics backend requires platform checks. Reconnection must not
spawn replacement shells or replay input whose delivery is uncertain.

## Validation

Use a disposable Vitro Linux desktop with private daemons for baseline and
final measurements, screenshots, image placements, palette reset, selection,
input ordering, resize, reconnection and sustained output. Record whether its
GPU adapter is software; VM measurements do not prove native hardware speed.
Run Rust tests, clippy, formatting, isolated live suites and Nix checks. Native
macOS validation remains separate from Linux VM results.

## Completion checklist

- [x] Demand-driven grid updates and complete-cycle presentation.
- [x] Shared bounded/coalescing outgoing queue with ordered input.
- [x] Shared selection and text extraction.
- [x] Shared dynamic theme/palette effects and reset semantics.
- [x] Same-session reconnection without replacement spawning or input replay.
- [x] Shared image reception, placement and rendering.
- [x] Direct shared-device GPU renderer, with no frame readback.
- [x] Vitro desktop analysis and final behavior evidence.

Supersedes [the mesh adapter decision](_20261024-120000-reuse-felis-render-core.md).
See the [reuse audit](../felis-rendering-reuse-audit.md) and
[VM results](../vitro-rendering-validation.md). Changing four-pane workloads
regressed on the software Vulkan adapter; native hardware performance remains
unmeasured. Shared implementation reduces duplication but does not guarantee
faster GPU submission on every adapter.
