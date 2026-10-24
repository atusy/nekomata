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

/// A confirmation is valid only for the connection that received the paste.
pub struct PendingPaste {
    question: felis_client_core::confirm::PendingConfirm,
    generation: u64,
}

impl PendingPaste {
    pub fn new(text: String, bracketed: bool, generation: u64) -> Result<Self, Vec<u8>> {
        let bytes = text.into_bytes();
        if felis_client_core::confirm::paste_needs_confirmation(bracketed, &bytes) {
            Ok(Self {
                question: felis_client_core::confirm::PendingConfirm::Paste(bytes),
                generation,
            })
        } else {
            Err(bytes)
        }
    }

    pub fn is_current(&self, connected: bool, generation: u64) -> bool {
        connected && self.generation == generation
    }

    pub fn label(&self) -> String {
        self.question.label().trim_end_matches(" [y/N]").to_owned()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        match self.question {
            felis_client_core::confirm::PendingConfirm::Paste(bytes) => bytes,
            felis_client_core::confirm::PendingConfirm::KillSession => unreachable!(),
        }
    }
}

/// Rehydration is historical state, not a new notification for this window.
pub fn attention_after(
    current: bool,
    message: &felis_protocol::messages::GridMsg,
    rehydrating: bool,
) -> bool {
    use felis_protocol::messages::GridMsg;
    match message {
        GridMsg::RehydrateBegin => false,
        GridMsg::Attention { .. } if !rehydrating => true,
        _ => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use felis_client_core::SelectionMode;

    #[test]
    fn attention_tracks_live_events_without_replaying_rehydration() {
        use felis_protocol::messages::{AttentionSource, GridMsg};
        let bell = GridMsg::Attention {
            source: AttentionSource::Bell,
        };
        assert!(attention_after(false, &bell, false));
        assert!(!attention_after(false, &bell, true));
        assert!(!attention_after(true, &GridMsg::RehydrateBegin, true));
        assert!(!attention_after(false, &GridMsg::RehydrateEnd, false));
    }

    #[test]
    fn paste_confirmation_preserves_bytes_and_expires_on_reconnect() {
        let text = "echo 猫\nexit\n";
        let pending = PendingPaste::new(text.into(), false, 7).ok().unwrap();
        assert!(pending.is_current(true, 7));
        assert!(!pending.is_current(false, 7));
        assert!(!pending.is_current(true, 8));
        assert_eq!(pending.into_bytes(), text.as_bytes());
        assert!(PendingPaste::new(text.into(), true, 7).is_err());
        assert!(PendingPaste::new("column\tvalue".into(), false, 7).is_err());
    }

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
