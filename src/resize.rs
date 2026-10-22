use felis_protocol::messages::RequestedDims;

#[derive(Default)]
pub struct PaneResize {
    sent: Option<RequestedDims>,
}

impl PaneResize {
    pub fn update(&mut self, dims: RequestedDims, dragging: bool) -> Option<RequestedDims> {
        // Reflow and SIGWINCH belong to the final layout, not every intermediate
        // drag position: shells can otherwise redraw prompts at obsolete widths.
        if dragging || self.sent == Some(dims) {
            return None;
        }
        self.sent = Some(dims);
        Some(dims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims(cols: u32) -> RequestedDims {
        RequestedDims {
            rows: 24,
            cols,
            pixel_w: cols * 10,
            pixel_h: 480,
        }
    }

    #[test]
    fn dragging_width_back_to_its_start_does_not_resize_the_shell() {
        let mut resize = PaneResize::default();
        assert_eq!(resize.update(dims(80), false), Some(dims(80)));
        for cols in [60, 40, 20, 40, 60, 80] {
            assert_eq!(resize.update(dims(cols), true), None);
        }
        assert_eq!(resize.update(dims(80), false), None);
    }
}
