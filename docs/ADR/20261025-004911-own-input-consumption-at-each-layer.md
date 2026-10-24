# Each input layer owns consumption and forwarding

| | |
| --- | --- |
| Status | proposed |
| Date | 2026-10-25 |
| Decision-makers | atusy |

## Context and Problem Statement

Input passes through nekomata, the felis client library, the felis daemon and
the terminal application (a shell or TUI). These are responsibility boundaries:
the client library runs inside nekomata, while the daemon owns the PTY and
terminal state. Sharing terminal keybindings in [PR #9](https://github.com/atusy/nekomata/pull/9)
raises the question of which layer interprets a key and whether handling it
also forwards input to the next layer.

Users may need to bypass a frontend shortcut to reach felis or a terminal
application. Without an explicit boundary, "direct input" could incorrectly
promise that bypassing nekomata also bypasses felis's own input handling.

## Decision Drivers

- Avoid duplicate actions and unintended input reaching a shell or TUI.
- Reuse felis's common keybindings without duplicating them in nekomata.
- Keep platform modifiers, IME and GUI focus decisions with the local client.
- Keep terminal history and PTY operations with the daemon that owns them.
- Make bypass behavior explicit without imposing an upstream felis feature.

## Considered Options

1. Let each layer own consumption and forwarding, with common key interpretation
   in the felis client library.
2. Forward keys to the felis daemon and centralize shortcut interpretation there.
3. Implement all shortcut interpretation in nekomata.

## Decision Outcome

Choose option 1. Each layer decides whether input it receives is consumed or
forwarded to its immediate next layer. Handling an input consumes it by default;
handling and forwarding the same input requires an explicit behavior contract.
An unhandled input remains available to the next layer, subject to the input
context: text entered in a GUI search field, for example, belongs to that field
rather than to the terminal.

Forwarding does not require preserving a raw key event. A layer may translate
input into a semantic action, a daemon request or terminal input bytes. A
consumed shortcut can cause a downstream operation without forwarding its
original keystroke to the shell.

| Layer | Responsibility |
| --- | --- |
| nekomata | Route GUI input and handle workspace/UI-specific shortcuts, including tab and pane prefixes. Adapt remaining terminal input to the felis client library. |
| felis client library | Interpret common terminal keybindings and expose their actions. Keep this policy reusable across frontends. |
| nekomata and the client library adapter | Execute local actions such as font or clipboard changes, or translate terminal actions into daemon requests. |
| felis daemon | Execute terminal/history requests and deliver application input through the PTY according to felis's protocol. |
| Terminal application | Interpret the input that reaches its shell or TUI. |

For example, the client library maps `Ctrl+Shift+End` to a scroll-to-live action.
The adapter sends the corresponding viewport request, and the daemon updates
the view. The shell does not also receive `Ctrl+Shift+End`. The exact keybinding
belongs to felis's keymap; this ADR does not freeze that binding or require a
particular adapter API.

Sharing keymap ownership does not establish user-configuration support. The
PR #9 implementation examined for this decision constructs
`Keymap::default_for_platform()`; that alone does not load user overrides or
propagate daemon configuration changes to nekomata. Configuration sources,
loading and reload behavior need a separate contract. A future shared client
configuration loader could avoid a duplicate nekomata keybinding configuration.

### Bypassing shortcut interpretation

nekomata may provide an explicit operation, such as a prefix followed by a key,
that bypasses its own shortcut interpretation and passes input to felis. Here,
"direct" means bypassing a selected layer's shortcut interpretation, not
skipping libraries, transport or the PTY.

Bypassing nekomata does not bypass felis. Delivering a key directly to the
terminal application requires felis to support the corresponding passthrough
contract, including bypassing any applicable felis shortcut interpretation.
Whether and how felis provides that mechanism is an upstream decision. nekomata
must not promise application passthrough merely because it forwards a key to
felis. This ADR permits a bypass feature but does not require or implement one.

### Consequences

Positive:

- Common keybindings have one reusable interpretation policy, while nekomata
  retains its workspace-specific controls.
- The daemon receives terminal operations without needing GUI focus, platform
  modifier policy or a return path for locally executed shortcuts.
- Consumption and bypass contracts make duplicate input and shortcut conflicts
  testable at each boundary.

Negative:

- nekomata still needs an adapter for supported actions and GUI input events.
- Reaching a terminal application through conflicting shortcuts can require
  support at multiple layers; a nekomata-only bypass cannot guarantee it.
- Upstream keymap changes require compatibility checks for supported actions
  and for keys that must continue to reach terminal applications.

### Confirmation

When implementing or changing input routing, verify that:

- A nekomata shortcut executes once and does not leak its key or accompanying
  text event into the terminal input path.
- A supported common shortcut resolves through the shared keymap and executes
  once, with the expected local action or daemon request.
- Unhandled terminal input, including bare `End`, Option composition and IME
  commits, remains available to the terminal input path. Unsupported common
  actions do not silently swallow their keys.
- GUI text fields retain their input, and workspace prefixes retain their
  documented scope.
- If a bypass feature is added, tests demonstrate exactly which interpretation
  layer it skips. Application passthrough is verified against felis's contract.
- Isolated daemon tests confirm history/navigation requests without delivering
  the original shortcut to the shell. Native GUI input checks are reported
  separately from unit and daemon tests.

This record is proposed pending implementation and confirmation of the shared
keymap routing. It does not claim that optional bypass support exists today.

## Pros and Cons of the Options

### Layer-owned routing with a shared client keymap

Keeps common policy reusable and interprets local input where GUI context is
available. Requires explicit adapters and consumption contracts between layers.

### Daemon-owned shortcut interpretation

Could centralize bindings across connected clients, but would couple daemon
input policy to client capabilities and contexts. Local actions such as font
changes would need to return to the originating client, and remote connections
would need a policy for local versus remote shortcut configuration.

### nekomata-owned shortcut interpretation

Gives the frontend complete control but duplicates common felis bindings and
their platform behavior. Each frontend would need to maintain that policy.

## More Information

This refines the input boundary of
[sharing felis client policies and the GPU renderer](20261024-180000-share-felis-client-and-gpu.md).
It preserves the direct GUI-to-felis terminal connection described in
[keeping workspaces independent of GUI windows](20261023-120000-separate-workspaces-from-windows.md).
