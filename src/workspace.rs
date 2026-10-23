//! Window-independent tab and pane layout.
use crate::layout::Layout;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Tab {
    pub name: String,
    pub layout: Layout,
    pub focused: u64,
    pub focus_history: Vec<u64>,
}

impl Tab {
    pub fn focus(&mut self, id: u64) {
        self.focused = id;
        self.focus_history.retain(|previous| *previous != id);
        self.focus_history.push(id);
    }

    pub fn remove_pane(&mut self, id: u64) -> bool {
        self.focus_history.retain(|previous| *previous != id);
        let layout = std::mem::replace(&mut self.layout, Layout::Pane(id));
        let Some(layout) = layout.remove(id) else {
            return false;
        };
        if self.focused == id {
            self.focus(
                self.focus_history
                    .last()
                    .copied()
                    .unwrap_or_else(|| layout.panes()[0]),
            );
        }
        self.layout = layout;
        true
    }
}
