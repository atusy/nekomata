//! Adapt native input and repaint deadlines to felis presentation state.
use eframe::egui::{Event, ImeEvent};
use felis_client_core::{config::CursorConfig, cursor_blink::BlinkClock};
use felis_render_core::overlay::PreeditOverlay;
use std::time::Instant;

pub struct Presentation {
    blink: BlinkClock,
    cursor: Option<(u16, u16)>,
    focused: bool,
    pub preedit: Option<PreeditOverlay>,
}

impl Presentation {
    pub fn new(now: Instant, config: &CursorConfig) -> Self {
        Self {
            blink: BlinkClock::new(config.blink, config.blink_interval_ms, now),
            cursor: None,
            focused: false,
            preedit: None,
        }
    }

    pub fn update(
        &mut self,
        now: Instant,
        cursor: (u16, u16),
        focused: bool,
        wants_blink: bool,
        events: &[Event],
    ) -> bool {
        let was_visible = self.blink.visible();
        let mut changed = self.cursor != Some(cursor) || self.focused != focused;
        if changed
            || (focused
                && events.iter().any(|event| {
                    matches!(
                        event,
                        Event::Key { pressed: true, .. }
                            | Event::Text(_)
                            | Event::Paste(_)
                            | Event::Ime(_)
                    )
                }))
        {
            self.blink.reset(now);
        }
        self.cursor = Some(cursor);
        self.focused = focused;
        if !focused {
            changed |= self.preedit.take().is_some();
        } else {
            for event in events {
                match event {
                    Event::Ime(ImeEvent::Preedit {
                        text,
                        active_range_chars,
                    }) => {
                        self.preedit = (!text.is_empty()).then(|| {
                            let byte_index = |index| {
                                text.char_indices()
                                    .nth(index)
                                    .map_or(text.len(), |(byte, _)| byte)
                            };
                            PreeditOverlay {
                                anchor: cursor,
                                text: text.clone(),
                                cursor: active_range_chars
                                    .as_ref()
                                    .map(|range| (byte_index(range.start), byte_index(range.end))),
                            }
                        });
                        changed = true;
                    }
                    Event::Ime(ImeEvent::Commit(_)) => changed |= self.preedit.take().is_some(),
                    _ => {}
                }
            }
        }
        if let Some(overlay) = &mut self.preedit {
            overlay.anchor = cursor;
        }
        changed |= focused && self.blink.tick(now, wants_blink);
        changed || was_visible != self.blink.visible()
    }

    pub fn cursor_visible(&self) -> bool {
        self.blink.visible()
    }
    pub fn next_deadline(&self, wants_blink: bool) -> Option<Instant> {
        self.focused
            .then(|| self.blink.next_deadline(wants_blink))
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn configured_never_blink_keeps_the_cursor_visible() {
        let now = Instant::now();
        let config = CursorConfig {
            blink: felis_client_core::config::CursorBlinkMode::Never,
            ..Default::default()
        };
        let mut state = Presentation::new(now, &config);
        state.update(now, (0, 0), true, true, &[]);
        state.update(now + Duration::from_secs(1), (0, 0), true, true, &[]);
        assert!(state.cursor_visible());
        assert!(state.next_deadline(true).is_none());
    }

    #[test]
    fn ime_ranges_are_converted_from_characters_to_utf8_bytes_and_cleared_on_commit() {
        let now = Instant::now();
        let mut state = Presentation::new(now, &CursorConfig::default());
        assert!(state.update(
            now,
            (2, 3),
            true,
            false,
            &[Event::Ime(ImeEvent::Preedit {
                text: "猫語a".into(),
                active_range_chars: Some(1..2)
            })]
        ));
        let overlay = state.preedit.as_ref().expect("shared IME overlay");
        assert_eq!(overlay.anchor, (2, 3));
        assert_eq!(overlay.cursor, Some((3, 6)));
        assert_eq!(overlay.text, "猫語a");
        assert!(state.update(
            now,
            (2, 3),
            true,
            false,
            &[Event::Ime(ImeEvent::Commit("猫語".into()))]
        ));
        assert!(state.preedit.is_none());
    }

    #[test]
    fn blink_restarts_on_input_and_does_not_schedule_an_unfocused_cursor() {
        let now = Instant::now();
        let mut state = Presentation::new(now, &CursorConfig::default());
        state.update(now, (0, 0), true, true, &[]);
        assert!(state.cursor_visible());
        let later = now + Duration::from_millis(531);
        assert!(state.update(later, (0, 0), true, true, &[]));
        assert!(!state.cursor_visible());
        assert!(state.update(later, (0, 0), true, true, &[Event::Text("x".into())]));
        assert!(state.cursor_visible());
        state.update(later, (0, 0), false, false, &[]);
        assert!(state.next_deadline(false).is_none());
    }
}
