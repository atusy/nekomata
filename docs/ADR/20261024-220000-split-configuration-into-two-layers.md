# Split configuration into a terminal layer and an application layer

| | |
| --- | --- |
| Status | proposed |
| Date | 2026-10-24 |
| Decision-makers | atusy |

## Context

nekomata needs user configuration, and its settings fall into two domains.
Each pane is a felis client presenting one felis session: fonts, colors, cursor
and in-terminal actions such as copy, paste and zoom. Around those panes
nekomata adds its own application: windows, tabs, split panes, nekomata
sessions and their chrome.

felis-client-core already provides a configuration document shared by felis
clients, with `[client.<id>]` overlays. Its vocabulary describes a terminal
client; it has no concept of tabs or panes, and its binding actions form a
closed set. felis's own design note
(`docs/explanation/architecture/control-surfaces.md`, "One config document, no
per-client file") names per-client documents as the answer once a client needs
sections that other clients must not read.

## Options

1. Put all configuration in felis's shared document, extending
   `[client.nekomata]` with application settings.
2. Own a single nekomata document for everything, ignoring felis's.
3. Two layers: felis's shared document configures the terminal layer, and a
   separate nekomata-owned document configures the application layer.

## Decision

Implement option 3.

- **Terminal layer.** Settings describing how one felis session is presented
  and operated come from felis's shared document, resolved for nekomata through
  felis-client-core. nekomata adopts felis's vocabulary there and does not
  extend its schema.
- **Application layer.** Settings that exist only because nekomata manages
  windows, tabs, panes and nekomata sessions come from a document nekomata owns.
  It does not redefine terminal-layer settings.
- **Boundary rule.** A setting belongs to the terminal layer if it would still
  make sense in a single-pane felis client; otherwise it belongs to the
  application layer.
- **Dependency direction.** The application layer wraps the terminal layer, as
  a multiplexer wraps a terminal. Neither layer reads the other's document.
- **Per-pane variation.** Values that vary per pane at runtime, such as a
  future per-pane font size, are runtime state over terminal-layer defaults,
  not a third document.

Option 1 is rejected because it places nekomata's vocabulary in a schema
nekomata does not own, where felis's later additions can collide with it.
Option 2 is rejected because users of the felis GUI would duplicate fonts and
colors, and nekomata would reimplement configuration felis-client-core already
provides.

## Consequences

Users who also run the felis GUI share terminal settings without duplication.
nekomata owns its application vocabulary, free from name collisions with felis.

Users must learn two documents and which layer a setting belongs to. The
shared document may contain terminal sections nekomata does not honor, so users
can write settings that silently have no effect there. nekomata depends on
felis-client-core's configuration API, so felis revision updates must check it.

Continues [sharing felis client policies](20261024-180000-share-felis-client-and-gpu.md)
by reusing felis-client-core for the terminal layer.
