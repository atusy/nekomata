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

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PaneSpec {
    pub target: crate::session::Target,
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub tabs: Vec<Tab>,
    pub panes: std::collections::BTreeMap<u64, PaneSpec>,
    pub active: usize,
    pub next_id: u64,
    pub font_size: f32,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            tabs: vec![],
            panes: Default::default(),
            active: 0,
            next_id: 1,
            font_size: 15.0,
        }
    }
}

impl Snapshot {
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        use std::collections::BTreeSet;
        ensure!(
            self.font_size.is_finite() && (6.0..=72.0).contains(&self.font_size),
            "invalid font size"
        );
        ensure!(self.active < self.tabs.len().max(1), "invalid active tab");
        fn valid_layout(layout: &Layout) -> bool {
            match layout {
                Layout::Pane(_) => true,
                Layout::Split {
                    ratio,
                    first,
                    second,
                    ..
                } => {
                    ratio.is_finite()
                        && (0.1..=0.9).contains(ratio)
                        && valid_layout(first)
                        && valid_layout(second)
                }
            }
        }
        let mut ids = BTreeSet::new();
        for tab in &self.tabs {
            ensure!(valid_layout(&tab.layout), "invalid split layout");
            let panes = tab.layout.panes();
            ensure!(panes.contains(&tab.focused), "focused pane is missing");
            let history: BTreeSet<_> = tab.focus_history.iter().copied().collect();
            ensure!(
                history.len() == tab.focus_history.len()
                    && history.iter().all(|id| panes.contains(id)),
                "invalid focus history"
            );
            for id in panes {
                ensure!(ids.insert(id), "duplicate pane ID");
            }
        }
        ensure!(
            ids == self.panes.keys().copied().collect(),
            "layout and panes disagree"
        );
        ensure!(
            self.next_id > ids.last().copied().unwrap_or(0),
            "invalid next pane ID"
        );
        for pane in self.panes.values() {
            if let Some(id) = &pane.session {
                ensure!(
                    id.len() == 32 && u128::from_str_radix(id, 16).is_ok(),
                    "invalid felis session ID"
                );
            }
            match &pane.target {
                crate::session::Target::Local(path) => {
                    ensure!(path.is_absolute(), "daemon socket must be absolute")
                }
                crate::session::Target::Ssh { host, .. } => ensure!(
                    !host.is_empty() && !host.starts_with('-'),
                    "invalid SSH host"
                ),
            }
        }
        Ok(())
    }
}
