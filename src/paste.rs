//! Confirmation of pastes that could run commands as soon as they arrive.

/// What to do with pasted text.
pub(crate) enum Paste {
    Send(Vec<u8>),
    Confirm(PendingPaste),
    Refused { bytes: usize },
}

impl Paste {
    pub(crate) fn new(text: String, bracketed: bool, generation: u64) -> Self {
        let bytes = text.into_bytes();
        if bytes.len() > felis_protocol::messages::MAX_PASTE_BYTES {
            Self::Refused { bytes: bytes.len() }
        } else if felis_client_core::confirm::paste_needs_confirmation(bracketed, &bytes) {
            Self::Confirm(PendingPaste { bytes, generation })
        } else {
            Self::Send(bytes)
        }
    }
}

/// A confirmation is valid only for the connection that received the paste.
pub(crate) struct PendingPaste {
    bytes: Vec<u8>,
    generation: u64,
}

impl PendingPaste {
    pub(crate) fn is_current(&self, connected: bool, generation: u64) -> bool {
        connected && self.generation == generation
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
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
    #[test]
    fn pastes_over_the_limit_are_refused_before_confirmation() {
        use felis_protocol::messages::MAX_PASTE_BYTES;
        assert!(matches!(
            Paste::new("\n".repeat(MAX_PASTE_BYTES), false, 7),
            Paste::Confirm(_)
        ));
        assert!(
            matches!(Paste::new("x".repeat(MAX_PASTE_BYTES), true, 7), Paste::Send(bytes) if bytes.len() == MAX_PASTE_BYTES)
        );
        for bracketed in [false, true] {
            assert!(
                matches!(Paste::new("\n".repeat(MAX_PASTE_BYTES + 1), bracketed, 7), Paste::Refused { bytes } if bytes == MAX_PASTE_BYTES + 1)
            );
        }
    }
}
