//! Layout owns pane identities; terminal connections live outside the tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug)]
pub enum Layout {
    Pane(u64),
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

impl Layout {
    pub fn neighbor(&self, target: u64, direction: eframe::egui::Key) -> Option<u64> {
        use eframe::egui::{Key, Rect, pos2};
        fn collect(layout: &Layout, rect: Rect, out: &mut Vec<(u64, Rect)>) {
            match layout {
                Layout::Pane(id) => out.push((*id, rect)),
                Layout::Split {
                    axis,
                    ratio,
                    first,
                    second,
                } => {
                    let mut a = rect;
                    let mut b = rect;
                    match axis {
                        Axis::Horizontal => {
                            let x = rect.left() + rect.width() * ratio;
                            a.max.x = x;
                            b.min.x = x;
                        }
                        Axis::Vertical => {
                            let y = rect.top() + rect.height() * ratio;
                            a.max.y = y;
                            b.min.y = y;
                        }
                    }
                    collect(first, a, out);
                    collect(second, b, out);
                }
            }
        }
        let mut panes = Vec::new();
        collect(
            self,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            &mut panes,
        );
        let source = panes.iter().find(|(id, _)| *id == target)?.1;
        panes
            .into_iter()
            .filter(|(id, rect)| {
                if *id == target {
                    return false;
                }
                let horizontal = matches!(direction, Key::H | Key::L);
                let overlap = if horizontal {
                    rect.top() < source.bottom() && rect.bottom() > source.top()
                } else {
                    rect.left() < source.right() && rect.right() > source.left()
                };
                overlap
                    && match direction {
                        Key::H => rect.right() <= source.left(),
                        Key::L => rect.left() >= source.right(),
                        Key::K => rect.bottom() <= source.top(),
                        Key::J => rect.top() >= source.bottom(),
                        _ => false,
                    }
            })
            .min_by(|(_, a), (_, b)| {
                a.center()
                    .distance_sq(source.center())
                    .total_cmp(&b.center().distance_sq(source.center()))
            })
            .map(|(id, _)| id)
    }

    pub fn panes(&self) -> Vec<u64> {
        match self {
            Self::Pane(id) => vec![*id],
            Self::Split { first, second, .. } => {
                let mut ids = first.panes();
                ids.extend(second.panes());
                ids
            }
        }
    }

    pub fn split(&mut self, target: u64, new: u64, axis: Axis) -> bool {
        match self {
            Self::Pane(id) if *id == target => {
                *self = Self::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Pane(target)),
                    second: Box::new(Self::Pane(new)),
                };
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                first.split(target, new, axis) || second.split(target, new, axis)
            }
        }
    }

    /// Removing the final leaf returns None; removing a sibling collapses its split.
    pub fn remove(self, target: u64) -> Option<Self> {
        match self {
            Self::Pane(id) => (id != target).then_some(Self::Pane(id)),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => match (first.remove(target), second.remove(target)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (remaining, None) | (None, remaining) => remaining,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbors_follow_geometry_in_nested_splits() {
        use eframe::egui::Key;
        let mut layout = Layout::Pane(1);
        layout.split(1, 2, Axis::Horizontal);
        layout.split(2, 3, Axis::Vertical);
        assert_eq!(layout.neighbor(1, Key::L), Some(2));
        assert_eq!(layout.neighbor(2, Key::J), Some(3));
        assert_eq!(layout.neighbor(3, Key::K), Some(2));
        assert_eq!(layout.neighbor(3, Key::H), Some(1));
        assert_eq!(layout.neighbor(1, Key::H), None);
    }

    #[test]
    fn nested_split_close_preserves_other_panes_and_order() {
        let mut layout = Layout::Pane(1);
        assert!(layout.split(1, 2, Axis::Horizontal));
        assert!(layout.split(2, 3, Axis::Vertical));
        assert!(!layout.split(9, 4, Axis::Vertical));
        let layout = layout.remove(2).unwrap();
        assert_eq!(layout.panes(), vec![1, 3]);
        let layout = layout.remove(1).unwrap();
        assert!(matches!(layout, Layout::Pane(3)));
        assert!(layout.remove(3).is_none());
    }
}
