//! Confirmation of pastes that could run commands as soon as they arrive.

/// What to do with pasted text.
pub enum Paste {
    Send(Vec<u8>),
    Confirm(PendingPaste),
}

impl Paste {
    pub fn new(text: String, bracketed: bool, generation: u64) -> Self {
        let bytes = text.into_bytes();
        if felis_client_core::confirm::paste_needs_confirmation(bracketed, &bytes) {
            Self::Confirm(PendingPaste { bytes, generation })
        } else {
            Self::Send(bytes)
        }
    }
}

/// A confirmation is valid only for the connection that received the paste.
pub struct PendingPaste {
    bytes: Vec<u8>,
    generation: u64,
}

impl PendingPaste {
    pub fn is_current(&self, connected: bool, generation: u64) -> bool {
        connected && self.generation == generation
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_confirmation_preserves_bytes_and_expires_on_reconnect() {
        let text = "echo 猫\nexit\n";
        let Paste::Confirm(pending) = Paste::new(text.into(), false, 7) else {
            panic!("an unbracketed multi-line paste must be confirmed");
        };
        assert!(pending.is_current(true, 7));
        assert!(!pending.is_current(false, 7));
        assert!(!pending.is_current(true, 8));
        assert_eq!(pending.into_bytes(), text.as_bytes());
    }

    #[test]
    fn harmless_or_bracketed_pastes_are_sent_unchanged() {
        for (text, bracketed) in [("echo 猫\nexit\n", true), ("column\tvalue", false)] {
            let Paste::Send(bytes) = Paste::new(text.into(), bracketed, 7) else {
                panic!("{text:?} must not need confirmation");
            };
            assert_eq!(bytes, text.as_bytes());
        }
    }
}
