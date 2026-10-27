//! Window-independent tab and pane layout.
use crate::layout::Layout;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "StoredTab")]
pub struct Tab {
    pub name: String,
    pub layout: Layout,
    /// The last entry of `focus_history`, kept so by `focus` and on load.
    pub focused: u64,
    pub focus_history: Vec<u64>,
}

// Snapshots come from other nekomata processes, which serialize both fields
// independently. Re-focusing on load restores the invariant they may not
// have kept, without rejecting the workspace.
#[derive(serde::Deserialize)]
struct StoredTab {
    name: String,
    layout: Layout,
    focused: u64,
    focus_history: Vec<u64>,
}

impl From<StoredTab> for Tab {
    fn from(stored: StoredTab) -> Self {
        let mut tab = Self {
            name: stored.name,
            layout: stored.layout,
            focused: stored.focused,
            focus_history: stored.focus_history,
        };
        tab.focus(tab.focused);
        tab
    }
}

impl Tab {
    pub fn focus(&mut self, id: u64) {
        self.focused = id;
        self.focus_history.retain(|previous| *previous != id);
        self.focus_history.push(id);
    }

    pub fn remove_pane(&mut self, id: u64) -> PaneRemoval {
        if !self.layout.panes().contains(&id) {
            return PaneRemoval::Missing;
        }
        self.focus_history.retain(|previous| *previous != id);
        let layout = std::mem::replace(&mut self.layout, Layout::Pane(id));
        let Some(layout) = layout.remove(id) else {
            return PaneRemoval::LastPane;
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
        PaneRemoval::Removed
    }
}

/// Removes a pane and drops a tab left empty.
pub fn remove_pane(tabs: &mut Vec<Tab>, active: &mut usize, id: u64) {
    let Some(index) = tabs.iter().position(|tab| tab.layout.panes().contains(&id)) else {
        return;
    };
    if tabs[index].remove_pane(id) == PaneRemoval::LastPane {
        tabs.remove(index);
        if index < *active {
            *active -= 1;
        }
        *active = (*active).min(tabs.len().saturating_sub(1));
    }
}

#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneRemoval {
    Removed,
    /// The pane was the tab's only one; the caller removes the tab.
    LastPane,
    Missing,
}

/// The tabs one window shows, independent of the sessions behind its panes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindowLayout {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub next_id: u64,
    pub font_size: f32,
}

impl Default for WindowLayout {
    fn default() -> Self {
        Self {
            tabs: vec![],
            active: 0,
            next_id: 1,
            font_size: felis_render_wgpu::DEFAULT_FONT_SIZE_LOGICAL_PX,
        }
    }
}

impl WindowLayout {
    /// Panics without tabs; a window closes with its last tab.
    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    /// Panics without tabs; a window closes with its last tab.
    pub fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn focused(&self) -> Option<u64> {
        self.tabs.get(self.active).map(|tab| tab.focused)
    }

    pub fn allocate_pane(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Appends a tab holding only `pane` and activates it.
    pub fn open_tab(&mut self, pane: u64) {
        self.tabs.push(Tab {
            name: format!("Terminal {pane}"),
            layout: Layout::Pane(pane),
            focused: pane,
            focus_history: vec![pane],
        });
        self.active = self.tabs.len() - 1;
    }

    /// Removes `pane` and, when it was the last one, its tab. Returns false
    /// when no tab holds it.
    pub fn close_pane(&mut self, pane: u64) -> bool {
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.layout.panes().contains(&pane))
        else {
            return false;
        };
        if self.tabs[index].remove_pane(pane) == PaneRemoval::LastPane {
            self.tabs.remove(index);
            if index < self.active {
                self.active -= 1;
            }
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
        }
        true
    }

    /// Panics without tabs; a window closes with its last tab.
    pub fn close_active_tab(&mut self) -> Tab {
        let tab = self.tabs.remove(self.active);
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        tab
    }

    /// Panics without tabs; a window closes with its last tab.
    pub fn cycle_tab(&mut self, forward: bool) {
        let len = self.tabs.len();
        self.active = if forward {
            (self.active + 1) % len
        } else {
            (self.active + len - 1) % len
        };
    }

    /// Moves `tabs[from]` into the gap before `tabs[to]`, keeping the same
    /// tab active.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        self.active = move_tab(&mut self.tabs, self.active, from, to);
    }
}

/// Moves `tabs[from]` into the gap before `tabs[to]` (`to == tabs.len()` means
/// the end) and returns the index of the tab that was at `active`.
fn move_tab(tabs: &mut Vec<Tab>, active: usize, from: usize, to: usize) -> usize {
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
    #[serde(default, deserialize_with = "resolved_session")]
    pub session: Option<crate::session::SessionId>,
}

/// Older windows saved an unresolved attach prefix here. Failing to decode
/// it would end the daemon connection and release the window's workspace,
/// so it loads as no session, as current windows save it.
fn resolved_session<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::session::SessionId>, D::Error> {
    use serde::Deserialize;
    Ok(Option::<String>::deserialize(deserializer)?.and_then(|id| id.parse().ok()))
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    #[serde(flatten)]
    pub window: WindowLayout,
    pub panes: std::collections::BTreeMap<u64, PaneSpec>,
}

impl Snapshot {
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        use std::collections::BTreeSet;
        let window = &self.window;
        ensure!(
            window.font_size.is_finite()
                && (felis_client_core::config::MIN_FONT_SIZE_LOGICAL_PX
                    ..=felis_client_core::config::MAX_FONT_SIZE_LOGICAL_PX)
                    .contains(&window.font_size),
            "invalid font size"
        );
        ensure!(
            window.active < window.tabs.len().max(1),
            "invalid active tab"
        );
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
        for tab in &window.tabs {
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
            window.next_id > ids.last().copied().unwrap_or(0),
            "invalid next pane ID"
        );
        for pane in self.panes.values() {
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
    fn removing_a_tabs_last_pane_drops_the_tab_and_keeps_the_active_tab() {
        let mut list = tabs(&["a", "b", "c"]);
        let mut active = 2;
        remove_pane(&mut list, &mut active, 0);
        assert_eq!(names(&list), ["b", "c"]);
        assert_eq!(active, 1);
        remove_pane(&mut list, &mut active, 2);
        assert_eq!(names(&list), ["b"]);
        assert_eq!(active, 0);
    }

    #[test]
    fn persisted_session_ids_keep_their_hex_wire_format() {
        let json = serde_json::json!({
            "tabs": [{
                "name": "Terminal 1",
                "layout": {"Pane": 1},
                "focused": 1,
                "focus_history": [1],
            }],
            "panes": {
                "1": {
                    "target": {"Local": "/tmp/felis.sock"},
                    "session": "0123456789abcdef0123456789abcdef",
                },
            },
            "active": 0,
            "next_id": 2,
            "font_size": 14.0,
        });
        let snapshot: Snapshot = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(
            snapshot.panes[&1].session,
            Some(crate::session::SessionId(
                0x0123456789abcdef0123456789abcdef
            ))
        );
        snapshot.validate().unwrap();
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), json);
    }

    #[test]
    fn loaded_tabs_end_their_focus_history_on_the_focused_pane() {
        let tab: Tab = serde_json::from_value(serde_json::json!({
            "name": "test",
            "layout": split_tab().layout,
            "focused": 2,
            "focus_history": [2, 3],
        }))
        .unwrap();
        assert_eq!(tab.focus_history, [3, 2]);
        let tab: Tab = serde_json::from_value(serde_json::json!({
            "name": "test",
            "layout": {"Pane": 1},
            "focused": 1,
            "focus_history": [],
        }))
        .unwrap();
        assert_eq!(tab.focus_history, [1]);
    }

    #[test]
    fn panes_saved_with_a_session_prefix_load_without_a_session() {
        // Windows built before session IDs were typed saved the raw prefix
        // of an attach felis had not resolved.
        let pane: PaneSpec = serde_json::from_value(serde_json::json!({
            "target": {"Local": "/tmp/felis.sock"},
            "session": "abc",
        }))
        .unwrap();
        assert_eq!(pane.session, None);
    }

    #[test]
    fn persisted_session_ids_must_be_complete() {
        use crate::session::SessionId;
        let parse = |id: &str| serde_json::from_value::<SessionId>(serde_json::json!(id));
        assert!(parse("0123456789abcdef").is_err(), "prefix");
        assert!(parse("0123456789abcdef0123456789abcdeg").is_err());
        assert_eq!(
            parse("0000000000000000000000000000000A")
                .unwrap()
                .to_string(),
            "0000000000000000000000000000000a"
        );
    }

    fn split_tab() -> Tab {
        let mut layout = Layout::Pane(1);
        layout.split(1, 2, crate::layout::Axis::Horizontal);
        layout.split(2, 3, crate::layout::Axis::Vertical);
        Tab {
            name: "test".into(),
            layout,
            focused: 1,
            focus_history: vec![1],
        }
    }

    #[test]
    fn closing_focused_panes_retraces_tab_focus_history() {
        let mut tab = split_tab();
        tab.focus(3);
        tab.focus(2);
        tab.focus(3);
        assert!(tab.remove_pane(3));
        assert_eq!(tab.focused, 2);
        assert!(tab.remove_pane(2));
        assert_eq!(tab.focused, 1);
        assert!(!tab.remove_pane(1));
    }

    #[test]
    fn closing_background_panes_prunes_only_their_tabs_history() {
        let mut tab = split_tab();
        let mut other = split_tab();
        tab.focus(2);
        tab.focus(3);
        other.focus(2);
        assert!(tab.remove_pane(2));
        assert_eq!(tab.focused, 3);
        assert!(tab.remove_pane(3));
        assert_eq!(tab.focused, 1);
        assert_eq!(other.focused, 2);
        assert_eq!(other.focus_history, vec![1, 2]);
    }

    fn window(panes: u64) -> WindowLayout {
        let mut window = WindowLayout::default();
        for _ in 0..panes {
            let id = window.allocate_pane();
            window.open_tab(id);
        }
        window
    }

    #[test]
    fn opened_tabs_take_fresh_pane_ids_and_become_active() {
        let window = window(3);
        assert_eq!(
            names(&window.tabs),
            ["Terminal 1", "Terminal 2", "Terminal 3"]
        );
        assert_eq!(window.active, 2);
        assert_eq!(window.focused(), Some(3));
        assert_eq!(window.next_id, 4);
    }

    #[test]
    fn closing_a_tabs_last_pane_keeps_the_same_tab_active() {
        let mut window = window(3);
        window.active = 2;
        assert!(window.close_pane(1));
        assert_eq!(window.focused(), Some(3));
        assert!(window.close_pane(3));
        assert_eq!(window.focused(), Some(2));
        assert!(!window.close_pane(3), "pane is already gone");
        assert!(window.close_pane(2));
        assert!(window.tabs.is_empty());
        assert_eq!(window.focused(), None);
    }

    #[test]
    fn closing_the_active_tab_activates_its_right_neighbor_or_the_last_tab() {
        let mut window = window(3);
        window.active = 1;
        assert_eq!(window.close_active_tab().name, "Terminal 2");
        assert_eq!(window.focused(), Some(3));
        assert_eq!(window.close_active_tab().name, "Terminal 3");
        assert_eq!(window.focused(), Some(1));
    }

    #[test]
    fn cycling_tabs_wraps_around() {
        let mut window = window(3);
        window.cycle_tab(true);
        assert_eq!(window.active, 0);
        window.cycle_tab(false);
        assert_eq!(window.active, 2);
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
