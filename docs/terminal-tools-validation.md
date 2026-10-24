# Terminal tools validation

Validated on 2026-10-24 against felis revision `2a3030b`.

The Vitro guest used Debian 13 aarch64, X11/Openbox and Mesa software Vulkan.
`scripts/vitro/test-desktop.sh` starts private nekomata/felis daemons and uses
native X11 input and clipboard operations. It verified:

- Keyboard input, word selection and drag selection reach the private shell/clipboard.
- Single-line paste proceeds; newline paste in an unbracketed shell waits for confirmation and Escape cancels it.
- Search owns text/Enter input; a matching result is highlighted. Ctrl+Shift+End returns to live output.
- Alt+Shift+drag copies `1111\n2222` from a two-line rectangle. Shift also avoids Openbox's Alt+drag window binding.
- OSC 8 hover displays the URL; Ctrl+click sends that exact URI to a test opener as one argument. This checks dispatch without launching a browser.
- The command palette copies only the completed command output and opens it in an editor tab.
- An editor inherits the GUI's temporary directory even when the daemon started with an older environment. Its staged file survives GUI exit and is removed after editor completion.

The isolated daemon suite additionally covers OSC 133 prompt navigation,
hidden-pane attention without grid pulls, no attention replay on reattach,
search/output over a fake SSH relay, and workspace switching while an editor
is pending. These are protocol/UI-state tests, not native visual assertions.

`nekomata doctor --json` resolved the GPU device, clipboard backend and packaged
font on both Apple M4/Metal and the Linux guest. An invalid `NEKOMATA_FONT`
produced a failed font probe and exit status 1. Doctor does not test native
surface presentation or read/write clipboard contents. A standalone Wayland
clipboard probe failure is reported as `clipboard_status: "unverified"`,
without failing the command solely for that result; its GUI backend needs a
window. This classification has a unit regression, not a native Wayland check.
Native macOS keyboard,
mouse, link-launch and editor-tab operations were not exercised in this pass.

## Search

Search is displayed at the right of the tab bar. Enter / ⌄ submits a changed
query or moves to the next result; ⌃ moves back. Ignore case and Regex are in
the ⋯ menu. The chevrons are drawn shapes stacked vertically; search-button
backgrounds appear only on hover. The minimum 480-point window was also inspected
for clipped controls.

![Search result and highlight](images/terminal-tools-search.png)

![Search options at minimum window width](images/terminal-tools-search-options.png)

![Only the hovered chevron has a background](images/terminal-tools-search-hover.png)

## Conditional paste confirmation

![Unbracketed multiline paste confirmation](images/terminal-tools-paste.png)

## Link preview

![URL shown before activation](images/terminal-tools-link.png)

## Rectangular selection

![Selected columns across two rows](images/terminal-tools-rectangle.png)
