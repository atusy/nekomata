//! When a grid change leaves a selection pointing at cells it never covered.
//!
//! Ported from felis-client's `grid_change_invalidates_selection`, which is
//! private to that binary, so both frontends drop a selection on the same
//! frames.
use felis_protocol::messages::{GridMsg, ViewportAnchor};

/// True if scrollback entered under the highlight or the `?1049` alt-screen
/// swapped. `alt_screen_before`/`alt_screen_now` bracket `ShadowScreen::apply`
/// because `ModeFlags` resends unchanged alt-screen bits on unrelated mode
/// updates; only an actual transition indicates a screen swap.
pub fn grid_change_invalidates_selection(
    msg: &GridMsg,
    alt_screen_before: bool,
    alt_screen_now: bool,
    viewport_before: (u32, Option<ViewportAnchor>),
) -> bool {
    alt_screen_before != alt_screen_now || viewport_message_enters_scrollback(msg, viewport_before)
}

/// The selection holds visible-row coordinates, so a view that moves
/// drops it (xterm), into history or back to live. An offset grown by
/// exactly the rows output pushed keeps it: same lines on screen.
fn viewport_message_enters_scrollback(
    msg: &GridMsg,
    (offset_before, anchor_before): (u32, Option<ViewportAnchor>),
) -> bool {
    let GridMsg::ViewportState {
        lines_from_bottom,
        anchor,
        ..
    } = msg
    else {
        return false;
    };
    if *lines_from_bottom == 0 {
        return offset_before > 0;
    }
    let same_lines = anchor_before
        .zip(*anchor)
        .and_then(|(before, now)| before.rows_until(now))
        .is_some_and(|grown| i64::from(*lines_from_bottom) - i64::from(offset_before) == grown);
    !same_lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use felis_protocol::messages::{ModifyOtherKeys, MouseProtocol};

    const LIVE: (u32, Option<ViewportAnchor>) = (0, None);

    const fn browsing(offset: u32, pushed: u64) -> (u32, Option<ViewportAnchor>) {
        (
            offset,
            Some(ViewportAnchor {
                pushed,
                generation: 0,
            }),
        )
    }

    fn state((offset, anchor): (u32, Option<ViewportAnchor>)) -> GridMsg {
        GridMsg::ViewportState {
            lines_from_bottom: offset,
            max: 1_000,
            anchor,
        }
    }

    const fn mode_flags(alt_screen: bool, mouse_protocol: MouseProtocol) -> GridMsg {
        GridMsg::ModeFlags {
            bracketed_paste: false,
            alt_screen,
            mouse_protocol,
            application_cursor: false,
            modify_other_keys: ModifyOtherKeys::Off,
            application_keypad: false,
            win32_input_mode: false,
            reverse_video: false,
        }
    }

    #[test]
    fn output_growing_a_browsed_offset_keeps_the_selection() {
        assert!(!viewport_message_enters_scrollback(
            &state(browsing(7, 13)),
            browsing(5, 11),
        ));
    }

    #[test]
    fn a_browsed_view_that_moves_off_its_lines_clears_the_selection() {
        assert!(viewport_message_enters_scrollback(
            &state(browsing(6, 11)),
            browsing(5, 11),
        ));
        assert!(viewport_message_enters_scrollback(
            &state(browsing(5, 13)),
            browsing(5, 11),
        ));
        let resized = (
            7,
            Some(ViewportAnchor {
                pushed: 13,
                generation: 1,
            }),
        );
        assert!(viewport_message_enters_scrollback(
            &state(resized),
            browsing(5, 11),
        ));
    }

    #[test]
    fn viewport_state_into_scrollback_clears_selection() {
        assert!(viewport_message_enters_scrollback(
            &GridMsg::ViewportState {
                lines_from_bottom: 1,
                max: 100,
                anchor: None,
            },
            LIVE,
        ));
    }

    #[test]
    fn leaving_a_browsed_view_for_live_clears_the_selection() {
        assert!(viewport_message_enters_scrollback(
            &state(LIVE),
            browsing(5, 11)
        ));
    }

    #[test]
    fn viewport_state_at_bottom_keeps_selection() {
        assert!(!viewport_message_enters_scrollback(&state(LIVE), LIVE));
    }

    #[test]
    fn unrelated_grid_messages_never_request_a_selection_clear() {
        assert!(!viewport_message_enters_scrollback(
            &GridMsg::Attention {
                source: felis_protocol::messages::AttentionSource::Bell,
            },
            browsing(5, 11),
        ));
    }

    #[test]
    fn entering_or_leaving_the_alt_screen_clears_the_selection() {
        let msg = mode_flags(true, MouseProtocol::Off);
        assert!(grid_change_invalidates_selection(&msg, false, true, LIVE));
        assert!(grid_change_invalidates_selection(&msg, true, false, LIVE));
    }

    #[test]
    fn a_mode_flip_that_leaves_the_screen_alone_keeps_the_selection() {
        assert!(!grid_change_invalidates_selection(
            &mode_flags(true, MouseProtocol::AnyMotion),
            true,
            true,
            LIVE,
        ));
        assert!(!grid_change_invalidates_selection(
            &mode_flags(false, MouseProtocol::ButtonEvents),
            false,
            false,
            LIVE,
        ));
    }
}
