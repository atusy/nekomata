use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use felis_client_core::{Offer, connect};
use felis_protocol::messages::SessionInfo;

type Roster = Arc<Mutex<Option<Result<Vec<SessionInfo>, String>>>>;

/// A session's row: the short ID, the title its program set, and its size.
/// The title can hold emoji, so the row is laid out by presentation, in the
/// color the selectable label gives it.
fn row(
    id: &str,
    title: &str,
    cols: impl std::fmt::Display,
    rows: impl std::fmt::Display,
    size: f32,
    line_height: f32,
) -> egui::text::LayoutJob {
    crate::theme::presented(
        &format!("{}  ·  {title}  ·  {cols}×{rows}", &id[..8]),
        size,
        line_height,
        egui::Color32::PLACEHOLDER,
    )
}

#[derive(Default)]
pub struct SessionPicker {
    query: String,
    roster: Roster,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl SessionPicker {
    /// Starts loading the roster the picker shows next.
    pub fn open(&mut self, runtime: &tokio::runtime::Runtime, ctx: &egui::Context) {
        self.refresh(runtime, ctx);
    }

    fn refresh(&mut self, runtime: &tokio::runtime::Runtime, ctx: &egui::Context) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        // Each refresh owns its result slot, so a late old reply cannot replace it.
        self.roster = Arc::default();
        let roster = self.roster.clone();
        let ctx = ctx.clone();
        self.task = Some(runtime.spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(3), async {
                let socket = felis_client_core::local_socket::resolve_local_socket(None)
                    .map_err(|err| err.to_string())?;
                let mut conn = connect(socket, Offer::ops())
                    .await
                    .map_err(|err| err.to_string())?;
                conn.list_sessions().await.map_err(|err| err.to_string())
            })
            .await
            .unwrap_or_else(|_| Err("Timed out loading felis sessions".into()));
            *roster.lock().unwrap() = Some(result);
            ctx.request_repaint();
        }));
    }

    /// Clears `open` when the picker closes, including after a selection.
    pub fn show(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        ctx: &egui::Context,
        open: &mut bool,
    ) -> Option<String> {
        let mut refresh = false;
        let mut selected = None;
        egui::Window::new("Attach a felis session")
            .open(open)
            .collapsible(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Session ID");
                    ui.text_edit_singleline(&mut self.query);
                    if ui.button("Attach").clicked() && !self.query.trim().is_empty() {
                        selected = Some(self.query.trim().to_owned());
                    }
                });
                refresh = ui.button("Refresh").clicked();
                ui.separator();
                let roster = self.roster.lock().unwrap();
                match &*roster {
                    None => {
                        ui.label("Loading sessions…");
                    }
                    Some(Err(err)) => {
                        ui.colored_label(crate::theme::ERROR_FG, err);
                    }
                    Some(Ok(sessions)) if sessions.is_empty() => {
                        ui.label("No felis sessions. Run nekomata to start one.");
                    }
                    Some(Ok(sessions)) => {
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                let size = egui::TextStyle::Button.resolve(ui.style()).size;
                                for session in sessions {
                                    let id = format!("{:032x}", session.id);
                                    let label = row(
                                        &id,
                                        session.title.as_deref().unwrap_or("Terminal"),
                                        session.dims.cols,
                                        session.dims.rows,
                                        size,
                                        // The spacing egui adds to plain
                                        // label text, which rows were before.
                                        crate::theme::title_line_height(ui, size)
                                            + ui.spacing().extra_text_line_spacing,
                                    );
                                    if ui
                                        .selectable_label(false, label)
                                        .on_hover_text(&id)
                                        .clicked()
                                    {
                                        selected = Some(id);
                                    }
                                }
                            });
                    }
                }
            });
        *open &= selected.is_none();
        if refresh {
            self.refresh(runtime, ctx);
        }
        if selected.is_some() {
            self.query.clear();
        }
        selected
    }
}

impl Drop for SessionPicker {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_keep_emoji_in_titles_colored_and_the_rest_in_the_label_color() {
        let job = row("0123456789abcdef", "✳ 😀", 80, 24, 14.0, 21.0);
        assert_eq!(job.text, "01234567  ·  ✳ 😀  ·  80×24");
        assert!(
            job.break_on_newline,
            "rows break where a title has a newline, as plain label rows did"
        );
        let emoji: Vec<_> = job
            .sections
            .iter()
            .filter(|section| {
                section.format.font_id.family == egui::FontFamily::Name("emoji".into())
            })
            .map(|section| &job.text[section.byte_range.start.0..section.byte_range.end.0])
            .collect();
        assert_eq!(emoji, ["😀"]);
        assert!(
            job.sections
                .iter()
                .all(|section| section.format.color == egui::Color32::PLACEHOLDER),
            "rows take the color the selectable label gives them"
        );
    }

    #[test]
    fn the_picker_lays_session_rows_out_by_presentation_at_the_button_size() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::chrome_fonts(None));
        let mut picker = SessionPicker::default();
        *picker.roster.lock().unwrap() = Some(Ok(vec![SessionInfo {
            id: 1,
            dims: felis_protocol::messages::GridDims {
                rows: 24,
                cols: 80,
                pixel_w: 0,
                pixel_h: 0,
            },
            title: Some("✳ 😀".into()),
            cwd: None,
            idle_seconds: None,
            tags: vec![],
            last_notification: None,
            foreground: None,
            exited: false,
            last_exit_code: None,
            attachments: vec![],
            sequence: std::num::NonZeroU64::MIN,
            program_status: vec![],
            spawn_cwd: None,
            terminal_progress: None,
        }]));
        let mut open = true;
        let mut sections = Vec::new();
        for _ in 0..2 {
            let output = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
                picker.show(&runtime, ui.ctx(), &mut open);
            });
            sections = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().contains("😀") => {
                        let job = &text.galley.job;
                        Some(
                            job.sections
                                .iter()
                                .map(|section| {
                                    (
                                        job.text
                                            [section.byte_range.start.0..section.byte_range.end.0]
                                            .to_owned(),
                                        section.format.font_id.clone(),
                                        section.format.line_height,
                                    )
                                })
                                .collect::<Vec<_>>(),
                        )
                    }
                    _ => None,
                })
                .flatten()
                .collect();
        }
        let style = ctx.global_style();
        let size = egui::TextStyle::Button.resolve(&style).size;
        let font = |family: &str| egui::FontId::new(size, egui::FontFamily::Name(family.into()));
        // As tall as the plain label text rows were, whatever the emoji.
        let line = ctx.fonts_mut(|fonts| fonts.row_height(&font("text")))
            + style.spacing.extra_text_line_spacing;
        assert_eq!(
            sections,
            [
                ("00000000  ·  ✳ ".to_owned(), font("text"), Some(line)),
                ("😀".to_owned(), font("emoji"), Some(line)),
                ("  ·  80×24".to_owned(), font("text"), Some(line)),
            ]
        );
    }

    #[test]
    fn attaching_closes_the_picker() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let mut picker = SessionPicker::default();
        picker.query = "0123".into();
        *picker.roster.lock().unwrap() = Some(Ok(vec![]));
        let mut open = true;
        let mut frame = || {
            crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
                assert_eq!(picker.show(&runtime, ui.ctx(), &mut open), None);
            })
        };
        // A window lays itself out invisibly in its first frame.
        drop(frame());
        let output = frame();
        assert!(open, "showing alone keeps the picker open");
        let button = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Attach" => {
                    Some(text.pos + egui::vec2(5.0, 5.0))
                }
                _ => None,
            })
            .expect("the picker must show its Attach button");
        let mut selected = None;
        for pressed in [true, false] {
            let _ = crate::test_support::run(
                &ctx,
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(button),
                        egui::Event::PointerButton {
                            pos: button,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    selected = selected
                        .take()
                        .or(picker.show(&runtime, ui.ctx(), &mut open))
                },
            );
        }
        assert_eq!(selected.as_deref(), Some("0123"));
        assert!(!open, "a selection must close the picker");
    }
}
