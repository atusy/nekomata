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
