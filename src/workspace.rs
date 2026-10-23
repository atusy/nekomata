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
    /// The size this session was zoomed to; `None` follows the configured default.
    pub font_size: Option<f32>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            tabs: vec![],
            panes: Default::default(),
            active: 0,
            next_id: 1,
            font_size: None,
        }
    }
}

const MIN_FONT_SIZE: f32 = 6.0;
const MAX_FONT_SIZE: f32 = 72.0;
const DEFAULT_FONT_SIZE: f32 = 15.0;

/// Terminal text size in logical pixels: a configured default that a
/// session may override by zooming.
///
/// A future per-pane override would sit in front of `zoom`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontSize {
    default: f32,
    pub zoom: Option<f32>,
}

impl Default for FontSize {
    fn default() -> Self {
        Self::new(None)
    }
}

impl FontSize {
    /// Clamps `configured` into the band snapshots accept, or uses nekomata's
    /// own default when unset.
    pub fn new(configured: Option<f32>) -> Self {
        Self {
            default: configured.map_or(DEFAULT_FONT_SIZE, |size| {
                size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
            }),
            zoom: None,
        }
    }

    pub fn get(&self) -> f32 {
        self.zoom.unwrap_or(self.default)
    }

    pub fn scale(&mut self, factor: f32) {
        self.zoom = Some((self.get() * factor).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE));
    }

    /// Returns to the configured default and keeps following it.
    pub fn reset(&mut self) {
        self.zoom = None;
    }
}

impl Snapshot {
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        use std::collections::BTreeSet;
        ensure!(
            self.font_size
                .is_none_or(|size| (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&size)),
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

    #[test]
    fn unzoomed_sessions_follow_the_configured_default() {
        let size = FontSize::new(Some(18.0));
        assert_eq!(size.get(), 18.0);
        assert_eq!(size.zoom, None);
        assert_eq!(FontSize::new(None).get(), 15.0);
    }

    #[test]
    fn configured_defaults_clamp_into_the_snapshot_band() {
        assert_eq!(FontSize::new(Some(4.0)).get(), 6.0);
        assert_eq!(FontSize::new(Some(90.0)).get(), 72.0);
    }

    #[test]
    fn zooming_overrides_and_reset_returns_to_the_default() {
        let mut size = FontSize::new(Some(20.0));
        size.scale(1.5);
        assert_eq!(size.zoom, Some(30.0));
        size.scale(10.0);
        assert_eq!(size.get(), 72.0);
        size.reset();
        assert_eq!(size.zoom, None);
        assert_eq!(size.get(), 20.0);
    }

    #[test]
    fn snapshots_accept_no_override_and_reject_sizes_outside_the_band() {
        let mut snapshot = Snapshot::default();
        assert!(snapshot.validate().is_ok());
        snapshot.font_size = Some(5.0);
        assert!(snapshot.validate().is_err());
        snapshot.font_size = Some(f32::NAN);
        assert!(snapshot.validate().is_err());
        snapshot.font_size = Some(6.0);
        assert!(snapshot.validate().is_ok());
    }
}
