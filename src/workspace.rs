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

/// Moves `tabs[from]` into the gap before `tabs[to]` (`to == tabs.len()` means
/// the end) and returns the index of the tab that was at `active`.
pub fn move_tab(tabs: &mut Vec<Tab>, active: usize, from: usize, to: usize) -> usize {
    let dest = if to > from { to - 1 } else { to };
    let tab = tabs.remove(from);
    tabs.insert(dest, tab);
    if active == from {
        dest
    } else if from < active && active <= dest {
        active - 1
    } else if dest <= active && active < from {
        active + 1
    } else {
        active
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
            font_size: felis_render_wgpu::DEFAULT_FONT_SIZE_LOGICAL_PX,
        }
    }
}

impl Snapshot {
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        use std::collections::BTreeSet;
        ensure!(
            self.font_size.is_finite()
                && (felis_client_core::config::MIN_FONT_SIZE_LOGICAL_PX
                    ..=felis_client_core::config::MAX_FONT_SIZE_LOGICAL_PX)
                    .contains(&self.font_size),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(names: &[&str]) -> Vec<Tab> {
        names
            .iter()
            .enumerate()
            .map(|(id, name)| Tab {
                name: (*name).into(),
                layout: Layout::Pane(id as u64),
                focused: id as u64,
                focus_history: vec![id as u64],
            })
            .collect()
    }

    fn names(tabs: &[Tab]) -> Vec<&str> {
        tabs.iter().map(|tab| tab.name.as_str()).collect()
    }

    #[test]
    fn moving_a_tab_inserts_it_before_the_target_slot() {
        let mut list = tabs(&["a", "b", "c", "d"]);
        move_tab(&mut list, 0, 0, 3);
        assert_eq!(names(&list), ["b", "c", "a", "d"]);
        move_tab(&mut list, 0, 3, 0);
        assert_eq!(names(&list), ["d", "b", "c", "a"]);
        move_tab(&mut list, 0, 1, 4);
        assert_eq!(names(&list), ["d", "c", "a", "b"]);
    }

    #[test]
    fn dropping_a_tab_beside_itself_keeps_the_order() {
        let mut list = tabs(&["a", "b", "c"]);
        assert_eq!(move_tab(&mut list, 2, 1, 1), 2);
        assert_eq!(move_tab(&mut list, 2, 1, 2), 2);
        assert_eq!(names(&list), ["a", "b", "c"]);
    }

    #[test]
    fn moving_tabs_keeps_the_same_tab_active() {
        // Active tab is the one moved.
        let mut list = tabs(&["a", "b", "c"]);
        assert_eq!(move_tab(&mut list, 0, 0, 3), 2);
        // Active tab shifts left when a tab before it moves past it.
        let mut list = tabs(&["a", "b", "c"]);
        assert_eq!(move_tab(&mut list, 1, 0, 3), 0);
        // Active tab shifts right when a tab after it moves before it.
        let mut list = tabs(&["a", "b", "c"]);
        assert_eq!(move_tab(&mut list, 1, 2, 0), 2);
        // Active tab outside the moved range stays put.
        let mut list = tabs(&["a", "b", "c", "d"]);
        assert_eq!(move_tab(&mut list, 3, 0, 2), 3);
    }
}
