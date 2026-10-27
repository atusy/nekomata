# Each input layer owns consumption and forwarding

## Context

Input crosses several responsibility boundaries: nekomata, the felis client
library, the felis daemon and the terminal application (a shell or TUI). The
client library runs within the frontend process, while the daemon owns the PTY
and terminal state. These components need a clear division of authority over
input interpretation and propagation, so that frontend policy and terminal
behavior can evolve independently.

## Decision

Each layer owns the interpretation of input it receives and whether that
input, or an operation derived from it, is forwarded to the next layer.
Processing input at one layer does not by itself determine its effect on
subsequent layers; propagation is the responsibility of the layer at that
boundary.

| Layer | Responsibility |
| --- | --- |
| nekomata | Frontend-specific interaction policy and integration with shared client policy. |
| felis client library | Common terminal input interpretation reusable across frontends. |
| felis daemon | Operations on daemon-owned terminal state and delivery through the PTY. |
| Terminal application | Interpretation of input delivered to the shell or TUI. |

Common key interpretation lives in the client library, where frontend context
is available, and terminal-state operations live in the daemon. The interface
between them expresses operations rather than requiring the daemon to
interpret frontend interactions.

Authority to bypass interpretation is also local to each boundary. nekomata
can delegate input to felis without applying its own interpretation, but
cannot thereby determine how felis handles that input. Bypassing
interpretation does not bypass the layers themselves.

Specific bindings, consumption rules for individual interactions and bypass
mechanisms are specification, not part of this decision.

## Considered Options

1. **Distribute authority across layers, sharing common client policy in the
   felis client library.** Chosen. Aligns authority with local context and
   state ownership, at the cost of explicit interfaces between layers.
2. **Centralize interpretation in the felis daemon.** Couples the daemon to
   frontend contexts and requires a return path for frontend-owned
   operations.
3. **Centralize interpretation in nekomata.** Gives the frontend direct
   control, but duplicates common terminal policy and couples its evolution to
   each frontend.

## Consequences

- Common input policy evolves independently of nekomata-specific controls.
- The daemon stays independent of frontend interaction state.
- Ownership of propagation is explicit at each boundary.
- Frontends still need to integrate local interactions with shared policy.
- End-to-end input behavior requires coordination across component
  interfaces, and a frontend cannot guarantee behavior beyond its own
  boundary.

## Related Decisions

- [Keep workspaces in a daemon independent of GUI windows](20261023-120000-separate-workspaces-from-windows.md)
- [Share felis client policies and the GPU renderer](20261024-180000-share-felis-client-and-gpu.md)
- [Publish commands through a shared registry](20261027-120000-publish-commands-through-a-shared-registry.md)
