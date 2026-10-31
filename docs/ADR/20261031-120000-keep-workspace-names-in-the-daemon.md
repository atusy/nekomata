# Keep workspace names in the daemon

## Context

Workspaces are identified by random IDs, which tell people nothing about
what a workspace is for. Users want to name them, and to rename a workspace
from the CLI or from any window, including one another window holds.

The window that holds a workspace publishes its whole layout as one
snapshot, and the daemon replaces what it stored with each snapshot. Only
that window may publish, and only while it holds the lease.

## Decision

A workspace's name is retained by the nekomata daemon beside the workspace,
not inside the layout a window publishes.

- **The daemon owns names.** Any client may name or rename any workspace,
  whether or not a window holds it, and the daemon reports names with the
  workspace roster.
- **Windows do not publish names.** A window's snapshot carries layout only,
  so the holding window cannot overwrite a name it does not know changed.
- **Names are labels, not identities.** Workspaces are still addressed by
  ID. A name need not be unique and is not accepted where an ID is, as felis
  addresses sessions only by ID.

## Invariants

> The invariants below are normative; the mechanisms that satisfy them are
> deliberately unspecified.

- Saving a layout never changes a workspace's name. Otherwise a rename from
  the CLI would be undone by the holding window's next save.
- Renaming a workspace never changes its layout, its lease or its ID.
  Renaming is not a switch, so it must not disturb the window using it.

## Considered Options

1. **Store the name in the window's snapshot.** No new request, but only the
   holding window could rename, and a concurrent rename elsewhere would be
   lost on the next save.
2. **Retain the name in the daemon beside the snapshot.** Chosen. Every
   client can rename through one arbiter, at the cost of a request outside
   the lease.
3. **Derive a label from the workspace's contents, such as its active tab.**
   Needs no state, but tab names are generic and terminal titles change with
   every command, so the label would not identify a workspace.

## Consequences

- The CLI and every window can rename any workspace without holding it.
- Names last only as long as the daemon. A window that reconnects to a new
  daemon restores its layout, which it holds, but not the name, which it
  never had.
- Because names are not unique, a script must select workspaces by ID.

## Related Decisions

- [Keep workspaces in a daemon independent of GUI windows](20261023-120000-separate-workspaces-from-windows.md)
