# Keep workspaces in a daemon independent of GUI windows

| | |
| --- | --- |
| Status | accepted |
| Date | 2026-10-23 |
| Decision-makers | atusy |

## Context and Problem Statement

> Terminology: this record originally called the retained layout a
> "nekomata session". It was renamed to "workspace" on 2026-10-25 so that
> "session" only means a felis session (one shell in the felis daemon).

A UI release currently requires restarting a GUI that owns its tab and pane
layout. Felis keeps shells alive, but the working environment is lost. Users
also need to switch between multiple groups of tabs and panes.

## Decision Drivers

- Restart an updated GUI and resume the latest working environment.
- Preserve felis ownership of shells, PTYs, terminal state and scrollback.
- Avoid assigning one workspace to multiple windows accidentally.
- Keep the current environment usable if a switch cannot connect.

## Considered Options

1. Save window layouts to disk and restore them in each GUI.
2. Retain workspaces in a dedicated daemon; connect GUI panes directly to felis.
3. Proxy all felis terminal traffic through a nekomata daemon.

## Decision Outcome

Choose option 2. A workspace contains tabs, split geometry, focused panes,
font size and felis session references, including local or SSH targets. The
GUI publishes layout snapshots to the daemon over a versioned Unix socket.
Each window holds a connection lease. The daemon atomically claims the most
recently used unclaimed workspace at startup, creating one when none is available.
A dropped connection releases the lease without deleting the workspace. Normal
shutdown saves and waits for an explicit detach acknowledgement before exiting.

Switching first waits for the source snapshot to be accepted, then reserves the
destination while retaining the source lease. The GUI
connects destination panes before committing the switch; failure cancels the
reservation. Explicit pane/tab closure still detaches shells in felis.

### Consequences

Positive:

- GUI replacement retains tab layouts and running commands.
- One daemon arbitrates simultaneous GUI startup and workspace switches.
- Existing direct felis transport and rendering stay reusable.

Negative:

- Clients and daemon need a compatible protocol. Incompatible versions report
  an error without replacing a running daemon.
- Layout changes require asynchronous snapshots; a hard client crash may lose
  the last changes that have not reached the daemon.
- This first implementation retains workspaces in daemon memory. Daemon restart
  does not restore layouts; disk persistence is deferred.

### Confirmation

Use unit/socket tests for latest-unclaimed selection, exclusive leases,
disconnect cleanup, snapshot restoration, version rejection and switch rollback.
Use isolated felis integration tests to verify that a replacement client restores
multiple tabs and split panes with the same live shells. Run Rust formatting,
tests, Clippy and Nix checks. Report native GUI validation separately.

## Pros and Cons of the Options

### Disk-only layouts

Simple process model, but independent clients still need shared locking and
ownership arbitration. It does not provide a central live workspace roster.

### Workspace daemon with direct felis connections

Centralizes workspace ownership while keeping the existing terminal data path.
Requires a second lifecycle and a small control protocol.

### Full terminal proxy

Keeps every terminal connection in the nekomata daemon, but adds another hop and
couples UI upgrades to a larger terminal-facing protocol. It is unnecessary for
retaining layouts because felis already owns terminal state.

## Validation

Implemented on 2026-10-23. Socket tests cover lease arbitration, EOF cleanup,
acknowledged detach, snapshot validation, source-save rejection and protocol
version rejection. The isolated integration runner verifies GUI replacement,
CLI routing from a restored shell, workspace switches and rollback while retaining
shared source connections. A macOS native GUI check also verified workspace
creation/switching and a full process restart with the same two felis shell IDs
and split layout. Linux native GUI validation remains outstanding.
