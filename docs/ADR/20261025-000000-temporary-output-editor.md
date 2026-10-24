# Show output editors temporarily in the originating pane

Status: accepted
Date: 2026-10-25

## Context

Editing completed command output is a temporary operation within an existing terminal workflow. A new tab or split changes the workspace layout and leaves the user to navigate back. The requested interaction switches the displayed session in the same pane and returns when the editor exits.

## Decision

Keep the original pane state and attached shell behind a temporary editor session. Retain the pane ID, tab layout and focus. Restore the original state on editor exit or a terminal attachment failure, while allowing transient reconnects to recover. Workspace snapshots refer to the original shell, including while the editor is displayed.

Associate asynchronous editor creation with the source pane, connection identity and generation. Reject nested editors and dispose of newly-created editors whose source changed before presentation. Closing a pane still detaches sessions; it does not kill a displayed editor or the original shell.

## Alternatives

- A new tab keeps both sessions directly accessible, but requires navigation and changes the workspace.
- A split permits side-by-side comparison, but changes the layout and reduces available space.
- Temporary session replacement preserves the user's place and layout, but needs explicit ownership and restoration of the hidden session.

## Consequences and verification

The original connection and pane state remain in memory until restoration or detachment. Persisted workspaces return to the original shell, rather than reopening the temporary editor view. Editors still run locally for SSH output, and their staged files remain owned by the editor helper until completion.

Regression tests verify preserved pane identity, selection, layout and snapshot; continued original-shell input while hidden; restoration without focus stealing; reconnect/failure behavior; and cancellation of stale creation results. Native verification exercises the menu/palette, editor display and return in the same pane.
