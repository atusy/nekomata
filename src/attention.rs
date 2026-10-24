//! Host attention mapped onto the shared terminal state.
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
}
