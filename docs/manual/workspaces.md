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

In the command palette (the cat button or Alt+Shift+P):

- `Workspace: Create` switches to a fresh workspace.
- `Workspace: Switch…` picks an existing one.

If nekomata cannot connect the panes of
the workspace you switch to, you stay in the current one.

## From the command line

```sh
nekomata workspaces list                  # workspaces and the windows showing them
nekomata workspaces new                   # switch this window to a new workspace
nekomata workspaces attach WORKSPACE_ID   # switch to an ID from the WORKSPACE column
```

Run from a nekomata shell, `new` and `attach` act on the window showing that
shell's workspace. Add `--window WINDOW_ID` to them to pick another, or for a
shell you moved into another workspace. After the nekomata daemon restarts, a
shell's old workspace is gone, so these commands fail there until you add
`--window`.

## What survives what

The nekomata daemon starts on first use and keeps running after the last
window closes. It holds workspaces in memory only.

- GUI restart, UI-only update or crash: workspaces are restored (after a
  crash, up to the last state the daemon received), except the font size,
  which starts at your configured size. An update that changes
  the daemon protocol requires
  [retiring the old daemon](troubleshooting.md#nekomata-reports-an-incompatible-daemon),
  which loses layouts.
- nekomata daemon stop or crash: **layouts are lost**. felis shells survive,
  so you can [attach to them again](sessions.md).
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
