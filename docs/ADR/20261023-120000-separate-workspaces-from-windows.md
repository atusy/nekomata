# Keep workspaces in a daemon independent of GUI windows

## Context

A UI release requires restarting the GUI. felis keeps shells alive across that
restart, but a GUI that owns its tab and pane layout loses the working
environment around them. Users also need to switch between several groups of
tabs and panes, and must not end up with two windows driving the same group.

felis already owns shells, PTYs, terminal state and scrollback. Whatever keeps
the layout must not take that ownership over.

## Decision

A workspace — tabs, split geometry, focus and the felis session each pane
presents — is retained by a dedicated nekomata daemon, not by a GUI window.

- **The daemon owns workspaces.** It is the single arbiter of which window a
  workspace belongs to. A window holds a workspace through a lease; losing
  the window releases the lease, not the workspace.
- **GUI windows own presentation.** A window publishes its layout to the
  daemon and restores layouts it receives from it.
- **Terminal traffic bypasses the nekomata daemon.** Each pane connects to its
  felis daemon directly, local or over SSH. The nekomata daemon stores only
  references to felis sessions.
- **felis owns shells.** Closing a window, tab or pane detaches from a shell
  but never ends it; nekomata never ends shells.

## Invariants

> The invariants below are normative; the mechanisms that satisfy them are
> deliberately unspecified.

- A workspace is leased by at most one window at a time, including while
  several GUIs start simultaneously. Two windows driving one layout would
  publish conflicting snapshots.
- A window that fails to reach a destination workspace keeps its current one.
  A switch that has released the source before the destination is usable
  leaves the user with no working environment.
- An incompatible client and daemon refuse to talk rather than replace each
  other. Replacing a running daemon discards every retained workspace.

## Considered Options

1. **Save layouts to disk and restore them in each GUI.** A simple process
   model, but independent GUIs still need shared locking and ownership
   arbitration, and nothing holds a live roster of workspaces.
2. **Retain workspaces in a dedicated daemon; connect panes directly to
   felis.** Chosen. Centralizes ownership while keeping the existing terminal
   data path, at the cost of a second daemon lifecycle and a small control
   protocol.
3. **Proxy all terminal traffic through the nekomata daemon.** Adds a hop to
   every keystroke and frame and couples UI upgrades to a terminal-facing
   protocol. Unnecessary, because felis already owns terminal state.

## Consequences

- Replacing the GUI keeps tab layouts and running commands.
- One process arbitrates simultaneous startup and workspace switches.
- The direct felis transport and rendering stay unchanged.
- Clients and the daemon need a versioned protocol, and a protocol change
  requires users to retire the running daemon.
- Layout changes travel to the daemon asynchronously, so a GUI crash can lose
  the latest changes that had not reached it.
- Retained layouts last only as long as the daemon unless it also persists
  them, which this decision leaves open.

## Related Decisions

- [Each input layer owns consumption and forwarding](20261025-004911-own-input-consumption-at-each-layer.md)
