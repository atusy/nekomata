//! Host gestures mapped onto the shared terminal interaction models.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
