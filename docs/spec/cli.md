# Command line

`nekomata` with no arguments opens a window. The subcommands are specified
with the behavior they control: workspaces and the nekomata daemon in
[workspaces](workspaces.md), panes and felis sessions in [panes](panes.md).

## Help and errors

- `--help` or `-h`, directly after `nekomata` or a subcommand, prints the
  usage to standard output and exits with status 0.
- An unknown command or invalid options to nekomata's own commands print the
  problem and the usage to standard error and exit with status 2. Options
  after `felis sessions list` are passed to felis unchecked.
- A command that fails, including a window that cannot start, prints the
  error to standard error and exits with status 1.
- `nekomata felis sessions list` exits with felis's status, or 1 when felis
  is ended by a signal, and exits with status 2 when felis cannot be run.

## Internal commands

`nekomata daemon run` runs the nekomata daemon in the foreground. nekomata
starts the daemon this way on demand, as specified in
[workspaces](workspaces.md#the-nekomata-daemon), so the command is not listed
in the usage and is not meant to be run by hand.

## Frontend diagnostics

`nekomata doctor [--json]` probes GPU device creation, clipboard backend
initialization, and terminal font resolution without starting a daemon or
reading or writing clipboard contents. JSON includes the shared felis probe
report, `issues`, and `clipboard_status` (`available`, `unavailable`, or
`unverified`). On Wayland, a failed standalone clipboard probe is unverified
because the native clipboard requires a window; that alone is not a failure.
The command exits 0 when the GPU and fonts are available and the clipboard is
not unavailable, 1 on a failed probe or execution error, and 2 on invalid
arguments. It does not test native surface presentation or clipboard transfer.
