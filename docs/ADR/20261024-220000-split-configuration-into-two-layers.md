# Split configuration into a terminal layer and an application layer

## Context

nekomata needs user configuration, and its settings fall into two domains.
Each pane is a felis client presenting one felis session: fonts, colors,
cursor and in-terminal actions such as copy, paste and zoom. Around those
panes nekomata adds its own application: windows, tabs, split panes,
workspaces and their chrome.

felis-client-core already provides a configuration document shared by felis
clients, with `[client.<id>]` overlays. Its vocabulary describes a terminal
client; it has no concept of tabs or panes, and its binding actions form a
closed set. felis's own design note
(`docs/explanation/architecture/control-surfaces.md`, "One config document, no
per-client file") names per-client documents as the answer once a client needs
sections that other clients must not read.

## Decision

Configuration has two layers, each read from the document its owner defines.

- **Terminal layer.** Settings describing how one felis session is presented
  and operated come from felis's shared document, resolved for nekomata
  through felis-client-core. nekomata adopts felis's vocabulary there and does
  not extend its schema.
- **Application layer.** Settings that exist only because nekomata manages
  windows, tabs, panes and workspaces come from a document nekomata owns. It
  does not redefine terminal-layer settings.
- **Boundary rule.** A setting belongs to the terminal layer if it would still
  make sense in a single-pane felis client; otherwise it belongs to the
  application layer.
- **Dependency direction.** The application layer wraps the terminal layer, as
  a multiplexer wraps a terminal. Neither layer reads the other's document.
- **Per-pane variation.** Values that vary per pane at runtime, such as a
  future per-pane font size, are runtime state over terminal-layer defaults, not a
  third document.

## Considered Options

1. **Put everything in felis's shared document**, extending
   `[client.nekomata]` with application settings. Rejected: it places
   nekomata's vocabulary in a schema nekomata does not own, where felis's
   later additions can collide with it.
2. **Own a single nekomata document for everything**, ignoring felis's.
   Rejected: users of other felis clients would duplicate fonts and colors,
   and nekomata would reimplement configuration felis-client-core already
   provides.
3. **Two layers, each in its owner's document.** Chosen.

## Consequences

- Users who also run other felis clients share terminal settings without
  duplication.
- nekomata owns its application vocabulary, free from name collisions with
  felis.
- Users must learn two documents and which layer a setting belongs to.
- The shared document may contain terminal sections nekomata does not honor,
  so users can write settings that silently have no effect in nekomata.
- nekomata depends on felis-client-core's configuration API, so felis revision
  updates must check it.

## Related Decisions

- [Share felis client policies and the GPU renderer](20261024-180000-share-felis-client-and-gpu.md)
- [Each input layer owns consumption and forwarding](20261025-004911-own-input-consumption-at-each-layer.md)
