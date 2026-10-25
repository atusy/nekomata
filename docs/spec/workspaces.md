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
one pane. The attached workspace's font size is replaced by the configured
size (see [configuration](configuration.md#font-size-precedence)).

## Saving

- The GUI sends layout snapshots to the daemon as the layout changes.
- Closing a window saves its layout and detaches the workspace, waiting up to
  6 seconds for the daemon to acknowledge. If saving fails or times out, the
  GUI reports it on standard error and exits anyway. A window closed during a
  workspace switch detaches without saving.
- After a GUI crash, the workspace restores up to the last snapshot the daemon
  received.

## Switching

`New workspace` and `Switch workspace…` replace the window's workspace.

1. The daemon must first accept the current workspace's snapshot.
2. The destination is reserved while the source lease is still held.
3. The GUI connects every pane of the destination. A pane already open in
   this window with the same target and session is reused.
4. Only then does the switch commit and the source lease end.

If any step fails — including a destination session that errors, ends or
detaches, or the requesting CLI disconnecting — the reservation is cancelled
and the window keeps its current workspace.

## Closing panes and tabs

Closing a pane or tab removes it from the layout and detaches its felis
sessions. Closing the last one closes the window. nekomata never ends a
shell: it keeps running in felis until it exits or its felis daemon stops.

## Losing the nekomata daemon

If the window cannot attach a workspace at startup, it shows the error and a
button to retry. Once attached, a window does not reconnect after losing its
connection to the nekomata daemon.

## CLI

```sh
nekomata workspaces list                                       # workspaces and the window each is attached to
nekomata workspaces new [--window WINDOW_ID]                   # switch a window to a new workspace
nekomata workspaces attach WORKSPACE_ID [--window WINDOW_ID]   # switch a window to this workspace
```

The target window is resolved as in [panes](panes.md#window-resolution).
`nekomata sessions …` is a deprecated alias of `nekomata workspaces …` and
prints a deprecation notice on standard error.
