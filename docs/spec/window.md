# Window

## Size and position

- A window opens with the size and position most recently saved by any
  nekomata window; every window process shares one saved geometry. A window
  saves its geometry within about 30 seconds of a move or resize, even while
  idle, and when it closes. Where the
  platform does not let applications place windows, only the size applies.
  Maximized and fullscreen states are restored as well.
- With no saved geometry, a window opens at 1100×740 logical pixels.
- A saved or default size is limited to the widest monitor's width and the
  tallest monitor's height, which may belong to different monitors. On macOS, a saved position that lies on no monitor is discarded and
  the system places the window; on Linux it is used as saved.
- A window cannot be made smaller than 480×320 logical pixels.
- Window geometry is not part of a [workspace](workspaces.md): switching
  workspaces neither moves nor resizes the window.

## Overlays

The command palette, the font size control, the workspace list and the
session picker are overlays: the window shows at most one of them. Opening
one, for example with the command palette button while the workspace list is
open, closes the one already open.
