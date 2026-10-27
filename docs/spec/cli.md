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
