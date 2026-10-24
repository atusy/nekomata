# Each input layer owns consumption and forwarding

| | |
| --- | --- |
| Status | proposed |
| Date | 2026-10-25 |
| Decision-makers | atusy |

## Context and Problem Statement

Input crosses several responsibility boundaries: nekomata, the felis client
library, the felis daemon and the terminal application (a shell or TUI). The
client library runs within the frontend process, while the daemon owns the PTY
and terminal state. These components need a clear division of authority over
input interpretation and propagation so that frontend policy and terminal
behavior can evolve independently.

## Decision Drivers

- Keep input policy close to the context and state it depends on.
- Share common terminal input policy across frontends.
- Avoid coupling the daemon to frontend-specific interaction state.
- Respect each component's authority over its own input boundary.

## Considered Options

1. Distribute input authority across layers, sharing common client policy in
   the felis client library.
2. Centralize input interpretation in the felis daemon.
3. Centralize input interpretation in nekomata.

## Decision Outcome

Choose option 1. Each layer owns the interpretation of input it receives and
whether that input, or an operation derived from it, is forwarded to the next
layer. Processing input at one layer does not by itself determine its effect
on subsequent layers; propagation is the responsibility of the layer at that
boundary.

| Layer | Responsibility |
| --- | --- |
| nekomata | Frontend-specific interaction policy and integration with shared client policy. |
| felis client library | Common terminal input interpretation reusable across frontends. |
| felis daemon | Operations on daemon-owned terminal state and delivery through the PTY. |
| Terminal application | Interpretation of input delivered to the shell or TUI. |

This separation places common key interpretation in the client library, where
frontend context is available, and terminal-state operations in the daemon.
The interface between them can express operations rather than requiring the
daemon to interpret frontend interactions.

Authority to bypass input interpretation is also local to each boundary.
nekomata can delegate input to felis without applying its own interpretation,
but cannot thereby determine how felis handles that input. Reaching a terminal
application through such delegation depends on the interfaces felis provides.
Bypassing interpretation does not bypass the architectural layers themselves.

Specific bindings, consumption rules for individual interactions, bypass
mechanisms and configuration loading are outside this decision's scope.

### Consequences

Positive:

- Common input policy can evolve independently of nekomata-specific controls.
- The daemon remains independent of frontend interaction state.
- Ownership of propagation is explicit at each component boundary.

Negative:

- Frontends still need integration between local interactions and shared policy.
- End-to-end input behavior requires coordination across component interfaces.
- A frontend cannot independently guarantee behavior beyond its own boundary.

### Confirmation

Architecture reviews should verify that input policy depends on the context
owned by its layer, that common client policy remains reusable, and that
propagation across each boundary is governed by the component responsible for
that boundary. The daemon should not acquire dependencies on frontend-specific
interaction state.

## Pros and Cons of the Options

### Layer-owned interpretation and forwarding

Aligns authority with local context and state ownership while sharing client
policy. Requires explicit interfaces and coordination across layers.

### Daemon-owned interpretation

Centralizes input policy for connected clients, but couples the daemon to
frontend contexts and requires a return path for frontend-owned operations.

### nekomata-owned interpretation

Gives the frontend direct control, but duplicates common terminal policy and
couples its evolution to each frontend implementation.

## More Information

This refines the input boundary of
[sharing felis client policies and the GPU renderer](20261024-180000-share-felis-client-and-gpu.md).
It preserves the direct GUI-to-felis terminal connection described in
[keeping workspaces independent of GUI windows](20261023-120000-separate-workspaces-from-windows.md).
