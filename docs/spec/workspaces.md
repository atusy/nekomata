# Workspaces

A workspace is the working environment one window presents: its tabs, the
split layout of each tab, the felis connection target and session of each
pane, the active tab and pane, focus history and font size. A workspace has
its own ID, distinct from felis session IDs; a felis session is one shell in a
felis daemon.

## The nekomata daemon

- The GUI and `nekomata workspaces list` start the nekomata daemon on first
  use. It keeps running after the last window closes.
- `nekomata daemon stop` stops it, and refuses while any window has a
  workspace attached. It never starts a daemon. Retained layouts are lost;
  felis shells keep running.
- Workspaces are held in memory only. Stopping or crashing the daemon, or
  restarting the OS, loses every layout.
- Client and daemon speak a versioned protocol. On an incompatible version the
  client reports an error and does not replace the running daemon.
- A UI-only update needs no daemon restart: a restarted GUI reuses a
  compatible running daemon.

## Leases

- A workspace is attached to at most one window at a time. The daemon
  arbitrates simultaneous requests, including concurrent GUI startup.
- There is no operation to take a workspace from another window.
- A window that disconnects releases its lease; the workspace remains.

## Startup

At startup, a window attaches to the most recently used workspace that no
window has attached. When none exists, it creates a workspace with one tab and
one pane.

## Saving

- The GUI sends layout snapshots to the daemon as the layout changes.
- Closing a window saves its layout and detaches the workspace; the GUI exits
  after the daemon acknowledges the detach.
- After a GUI crash, the workspace restores up to the last snapshot the daemon
  received.

## Switching

`New workspace` and `Switch workspace…` replace the window's workspace.

1. The daemon must first accept the current workspace's snapshot.
2. The destination is reserved while the source lease is still held.
3. The GUI connects every pane of the destination.
4. Only then does the switch commit and the source lease end.

If any step fails, the reservation is cancelled and the window keeps its
current workspace.

## Closing panes and tabs

Closing a pane or tab removes it from the layout and detaches its felis
sessions. The shells keep running in felis. Only the shell's own exit ends it.

## Reconnection

Startup attachment and reconnection after a nekomata daemon communication
failure are different operations; the latter is not supported.

## CLI

```sh
nekomata workspaces list                     # workspaces and the window each is attached to
nekomata workspaces new [--window WINDOW_ID]                   # switch a window to a new workspace
nekomata workspaces attach WORKSPACE_ID [--window WINDOW_ID]   # switch a window to this workspace
```

The target window is resolved as in [panes](panes.md#window-resolution).
`nekomata sessions …` is a deprecated alias of `nekomata workspaces …` and
prints a deprecation notice on standard error.
