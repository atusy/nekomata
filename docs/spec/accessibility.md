# Native accessibility

The GUI enables eframe's AccessKit integration so platform accessibility clients
can request the accessibility tree provided by egui. On macOS, this uses the
native accessibility bridge.

This does not promise screen-reader access to terminal cell contents: the
terminal renderer does not currently provide an accessibility tree for its grid.
