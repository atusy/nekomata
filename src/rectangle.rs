//! Host gestures mapped onto the shared terminal interaction models.
use eframe::egui::Modifiers;
use felis_client_core::{GridPos, Selection};

pub fn start_selection(cell: GridPos, modifiers: Modifiers) -> Selection {
    if modifiers.alt {
        Selection::new_rectangle(cell)
    } else {
        Selection::new(cell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use felis_client_core::SelectionMode;

    #[test]
    fn alt_drag_keeps_a_rectangle_when_extended_backwards() {
        let mut selection = start_selection(GridPos { row: 3, col: 8 }, Modifiers::ALT);
        selection.extend(GridPos { row: 1, col: 2 });
        assert_eq!(selection.mode(), SelectionMode::Rectangle);
        assert_eq!(selection.range(), ((1, 2), (3, 8)));
        let linear = start_selection(GridPos { row: 3, col: 8 }, Modifiers::SHIFT);
        assert_eq!(linear.mode(), SelectionMode::Linear);
    }
}
