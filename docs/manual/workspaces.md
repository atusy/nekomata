# Workspaces

A workspace is what one window shows: its tabs, how each tab is split, the
felis session behind each pane, which tab and pane have focus, and the font
size.

The nekomata daemon, not the GUI, keeps workspaces. Quit the GUI or install
an update that keeps the daemon protocol, start it again, and your tabs,
splits and running commands come back.

## On startup

nekomata opens the most recently used workspace that no other window has
open. If there is none, it creates a workspace with one tab and one pane.

A workspace is open in at most one window at a time, and you cannot take one
away from another window.

## Switching

In the command palette (the cat button):

- `Workspace: Create` switches to a fresh workspace.
- `Workspace: Manage…` lists the workspaces: pick one to switch to it, or
  [rename](#naming) them.

nekomata switches once the panes of the workspace you switch to have
connected. A pane whose shell has ended is dropped; a pane that cannot
connect, for example because its felis session is gone, shows its error and
you can close it. If no pane connects, you stay in the current workspace; a new workspace,
whose shell starts after the switch, switches regardless.

## Naming

Name a workspace to tell it apart: in the workspace list, point at its row
and click the pencil, type the name and press Enter. An empty name removes
it; Escape leaves the name as it was. You can also rename a workspace another
window has open, or rename from the [command line](#from-the-command-line).

## From the command line

```sh
nekomata workspaces list                  # workspaces and the windows showing them
nekomata workspaces list --format json    # the same, for scripts
nekomata workspaces new                   # switch this window to a new workspace
nekomata workspaces attach WORKSPACE_ID   # switch to an ID from the WORKSPACE column
nekomata workspaces rename WORKSPACE_ID "dotfiles"   # name a workspace
nekomata workspaces rename WORKSPACE_ID ""           # remove its name
```

`list` shortens IDs, and on a terminal highlights the part of each that is
enough for `attach` and dims the rest: it accepts the start of an ID as long as no other
workspace's ID starts the same way. Scripts should read full IDs from
`--format json`. Names are labels for you to read: commands still take IDs,
and two workspaces may share a name.

Run from a nekomata shell, `new` and `attach` act on the window showing that
shell's workspace. Add `--window WINDOW_ID` to them to pick another, or for a
shell you moved into another workspace. After the nekomata daemon restarts,
commands run from a shell started before the restart fail until you add
`--window`; shells started by older nekomata versions may instead reach an
unrelated window.

## What survives what

The nekomata daemon starts on first use and keeps running after the last
window closes. It holds workspaces in memory only.

- GUI restart, UI-only update or crash: workspaces are restored (after a
  crash, up to the last state the daemon received), except the font size,
  which starts at your configured size. An update that changes
  the daemon protocol requires
  [retiring the old daemon](troubleshooting.md#nekomata-reports-an-incompatible-daemon),
  which loses layouts.
- nekomata daemon stop or crash: **layouts of workspaces no window shows
  are lost**. felis shells survive, so you can
  [attach to them again](sessions.md). An open window keeps its tabs and
  panes, reconnects on its own (or with `Reconnect now`) and stores its
  layout in the new daemon, possibly under a new workspace ID.
- OS restart: **layouts and local shells are both lost**; felis keeps
  sessions in memory too. Sessions on an SSH host survive while that host's
  felis daemon keeps running.

After closing every window, you can stop the daemon. Layouts are lost; shells
keep running.

```sh
nekomata daemon stop
```

A UI-only update does not need a daemon restart.

See the [workspace specification](../spec/workspaces.md) for the exact rules.

Workspace IDs are 32-digit hexadecimal strings generated randomly. Use the full ID when attaching.
