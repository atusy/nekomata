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
  felis shells keep running. It stops a daemon of any protocol version,
  including one that rejects every other request from this client. Once the
  daemon accepts a stop, it refuses windows that try to attach or switch a
  workspace, and it exits even if the stopping CLI is interrupted before the
  reply.
- The daemon removes a stale socket left at its socket path, but refuses to
  start when anything other than a socket is there; the GUI and commands that
  would start the daemon then report that path instead of starting it.
- Running out of file descriptors or memory while accepting a client does not
  stop the daemon; it retries shortly after.
- Workspaces are held in memory only. Restarting the OS loses every layout.
  Stopping or crashing the daemon loses every layout except those of open
  windows, which [reconnect](#losing-the-nekomata-daemon).
- Client and daemon speak a versioned protocol. On an incompatible version the
  client reports an error and does not replace the running daemon. The daemon
  checks the version before decoding the request, so any request other than a
  stop that carries another version, or none, is answered with that error even
  when the daemon does not recognize the request. Daemons from before this
  rule close the connection instead.
- A UI-only update needs no daemon restart: a restarted GUI reuses a
  compatible running daemon.

## Leases

- A workspace is attached to at most one window at a time. The daemon
  arbitrates simultaneous requests, including concurrent GUI startup.
- There is no operation to take a workspace from another window.
- A window that disconnects releases its lease; the workspace remains unless
  it has no tabs.
- A workspace without tabs is discarded once no window holds it, for example
  after its window closes its last pane or cancels a switch to a new
  workspace. Workspace IDs are generated from 128 bits of OS randomness and
  displayed as 32 lowercase hexadecimal digits. Collisions with current
  workspaces are retried.

## Names

- A workspace may have a name. New workspaces have none. The workspace list
  in a window titles each workspace by its name, or by its full ID when it
  has none.
- Each row of the workspace list has a rename button, shown in place of the
  `Current` marker while the pointer or keyboard focus is on the row,
  including rows of workspaces another window has. It turns the row's title
  into a field holding the current name; Enter saves what it holds, while
  Escape or moving the focus away abandons it and keeps the list open.
- A name is trimmed of surrounding whitespace, is at most 64 characters and
  contains no control characters; a name that breaks these rules is refused.
  Setting a blank name removes it.
- Names need not be unique and never stand in for an ID: commands select
  workspaces by ID only.
- Any client may rename any workspace, including one another window has
  attached. Renaming changes neither the layout nor the lease, and saving a
  layout never changes the name.
- Names live in the daemon's memory with the workspaces. A window that
  [reconnects](#losing-the-nekomata-daemon) to a new daemon restores its
  layout without its name.

## Startup

At startup, a window attaches to the most recently used workspace that no
window has attached. When none exists, it creates a workspace with one tab and
one pane. A window closed by closing its last pane leaves no workspace behind,
so the next window attaches the most recent remaining one, if any. The attached workspace's font size is replaced by the configured
size (see [configuration](configuration.md#font-size-precedence)).

## Saving

- The GUI sends layout snapshots to the daemon as the layout changes.
- Closing a window saves its layout and detaches the workspace, waiting up to
  6 seconds in total for the daemon to acknowledge, including layout changes
  still queued ahead of it; the window stops responding meanwhile. If saving fails or times out, the
  GUI reports it on standard error and exits anyway. A window closed during a
  workspace switch detaches without saving; one closed while it has lost the
  daemon, or is still reconnecting, exits without saving.
- After a GUI crash, the workspace restores up to the last snapshot the daemon
  received.
- A pane opened with a partial session ID is saved without a session until
  felis resolves the prefix, so restoring it before then, or after its attach
  failed, starts a new shell in that pane.

## Switching

`Workspace: Create` and `Workspace: Switch…` replace the window's workspace.

1. The daemon must first accept the current workspace's snapshot.
2. The destination is reserved while the source lease is still held.
3. The GUI connects every pane of the destination that refers to a felis
   session and waits until each has connected, failed or ended. A pane
   already open in this window with the same target and session is reused.
   A pane without a session, such as the first pane of a new workspace,
   starts its shell only after the switch commits, so a cancelled switch
   never leaves a shell behind.
4. Only then does the switch commit and the source lease end. A pane whose
   session ended or was detached leaves the destination layout. A pane that
   failed to connect, for example because its felis session no longer exists
   or its felis daemon is unreachable, stays in the layout and shows its
   error, as it would at startup; closing it removes it.

If the daemon refuses a step, or no destination pane connects and none is
waiting to start a shell, the reservation is cancelled and the window keeps its current workspace.
Interrupting a CLI that requested the switch does not cancel it.

## Closing panes and tabs

Closing a pane or tab removes it from the layout and detaches its felis
sessions. Closing the last one closes the window. nekomata never ends a
shell: it keeps running in felis until it exits or its felis daemon stops.

## Reordering tabs

Dragging a tab marks the gap nearest the pointer with an insertion line and
moves the tab there on release. The active tab stays active. The new order is
saved like any other layout change.

## Losing the nekomata daemon

If the window cannot attach a workspace at startup, it shows the error and a
button to retry.

A window that loses its connection to the nekomata daemon after attaching
notices even while idle, keeps its tabs, panes and shells, and shows the
error with a button to reconnect at once. Until it reconnects, layout changes
reach no daemon and the window refuses to switch workspaces. It reconnects on its own after 1 second, doubling the delay
after each failed attempt up to 30 seconds, and starts the daemon if none is
running. It does not reconnect on its own after the daemon refuses it, for
example for an incompatible protocol version.

On reconnecting, the window attaches a workspace and saves its current layout
there, never into another window's retained layout:

1. Its own workspace, if the daemon still holds it under the previous ID with
   exactly a layout this window saved there: the last one the daemon
   acknowledged, or one of the last few sent after it whose answers were
   lost, including by an earlier reconnect attempt.
2. Otherwise the workspace the daemon offers at startup, if it has no tabs.
3. Otherwise a new workspace.

The workspace ID may therefore change. Shells started before the change keep
exporting the old ID in `NEKOMATA_WORKSPACE_ID`, so CLI commands run from
them may fail because the old workspace no longer exists; pass `--window`
there. Shells started afterwards carry the new ID.

## CLI

```sh
nekomata workspaces list [--format human|json]                # workspaces and the window each is attached to
nekomata workspaces new [--window WINDOW_ID]                   # switch a window to a new workspace
nekomata workspaces attach WORKSPACE_ID [--window WINDOW_ID]   # switch a window to this workspace
nekomata workspaces rename WORKSPACE_ID NAME                   # name a workspace, or clear its name with ""
```

`workspaces list` lists the workspaces most recently used first. The human
format, the default, is a table with a header row and the columns
`WORKSPACE`, `WINDOW` (`-` when no window has the workspace), `TABS`,
`PANES` and `NAME` (`-` when unnamed); it is for reading, not parsing. `WORKSPACE` shows every ID cut to
the same length: the fewest leading digits that keep all listed IDs
distinct, but at least 8. On a terminal, unless `NO_COLOR` is set to a
non-empty value, each ID highlights its shortest prefix that no other listed
ID starts with and dims the digits after it. Both depend on which workspaces exist, so store only full
IDs. `--format json` prints one object,
`{"v":1,"workspaces":[…]}`, whose entries carry:

| Key       | Type             | Value                                       |
| --------- | ---------------- | ------------------------------------------- |
| `id`      | string           | the full 32-digit workspace ID              |
| `name`    | string           | the workspace's name, omitted when unnamed  |
| `windows` | array of numbers | the windows showing the workspace, or empty |
| `tabs`    | number           | how many tabs it has                        |
| `panes`   | number           | how many panes it has                       |

`workspaces attach` takes the full ID or any prefix of it, in either case.
A prefix matches against every listed workspace, including those another
window holds; one that matches none or several fails without contacting a
window. A full ID is passed to the window as is.

`workspaces rename` takes an ID or prefix as `attach` does and names that
workspace, whether or not a window has it; it prints nothing on success and
never starts the daemon. See [names](#names) for what a name may be.

The target window is resolved as in [panes](panes.md#window-resolution).
`nekomata sessions …` is a deprecated alias of `nekomata workspaces …` and
prints a deprecation notice on standard error.
