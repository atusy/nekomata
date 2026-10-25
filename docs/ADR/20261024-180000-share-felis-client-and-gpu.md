# Share felis client policies and the GPU renderer

## Context

felis provides more than a daemon: its client libraries implement terminal
client policy (frame scheduling, outgoing input queues, selection, palette
effects, image handling, reconnection) and its renderer draws a terminal on
the GPU. A frontend that reimplements these duplicates felis and drifts from
it with every upstream change. nekomata's distinct value is the application
around the terminal: windows, tabs, split panes and workspaces.

## Decision

nekomata is a felis client that embeds felis's own terminal implementation and
adds only its application layer.

- **felis renders panes.** The felis GPU renderer draws each pane into a
  texture on the same GPU device egui uses, and egui composes those textures
  with the application chrome.
- **felis client libraries own terminal client policy.** nekomata calls them
  instead of carrying its own copies.
- **egui owns application chrome** — tabs, menus, overlays and dialogs.
- **nekomata owns workspaces**, as decided in
  separate-workspaces-from-windows.
- When the upstream renderer lacks an embedding seam, nekomata carries the
  smallest local patch over vendored upstream source, with attribution, until
  upstream exposes that seam.

## Invariants

> The invariants below are normative; the mechanisms that satisfy them are
> deliberately unspecified.

- Interactive rendering never reads frames back to the CPU. Per-frame
  readback would cost more than the reuse saves.
- Reconnecting a pane never spawns a replacement shell and never replays input
  whose delivery is uncertain. Either would run commands the user did not
  issue, or issued once.
- Closing a pane detaches its felis session explicitly; it does not end the
  shell.
- Keystrokes reach felis in the order they were typed.

## Considered Options

1. **Keep a nekomata-owned renderer and port individual policies.**
   Retains control over drawing but keeps two terminal renderers and requires
   porting every upstream fix.
2. **Share the felis GPU renderer and client policies; keep egui for chrome
   and nekomata for workspaces.** Chosen.
3. **Replace the whole UI with the standalone felis client.** Loses tabs,
   splits and workspaces, which are the reason nekomata exists.

## Consequences

- Terminal behavior follows felis, including glyph shaping, images and
  dynamic palettes, without duplicate implementations.
- egui and felis must agree on a graphics stack, so upgrading either requires
  checking the other and re-validating each platform's backend.
- The local embedding patch must be audited on every felis update.
- Shared code reduces duplication but does not guarantee faster rendering on
  every GPU adapter.

## Decision–Implementation Gap

The renderer is vendored with a local embedding patch in
`vendor/felis-render-wgpu`; upstream does not yet expose the seam.

## Related Decisions

- [Keep workspaces in a daemon independent of GUI windows](20261023-120000-separate-workspaces-from-windows.md)
- [Split configuration into a terminal layer and an application layer](20261024-220000-split-configuration-into-two-layers.md)
- [Each input layer owns consumption and forwarding](20261025-004911-own-input-consumption-at-each-layer.md)
