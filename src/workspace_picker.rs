//! The workspace list that `Workspaces…` opens.
use crate::{daemon::Summary, theme};
use eframe::egui;

/// What the picker asks for this frame.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Outcome {
    /// `Some(None)` asks for a new workspace, `Some(Some(id))` for `id`.
    pub(crate) selected: Option<Option<u64>>,
    pub(crate) refresh: bool,
    pub(crate) dismissed: bool,
}

/// Rows the list shows before it scrolls.
const MAX_VISIBLE_ROWS: f32 = 8.0;

fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

/// The detail line under a workspace's name.
fn detail(workspace: &Summary) -> String {
    let contents = format!(
        "{} · {}",
        plural(workspace.tabs, "tab"),
        plural(workspace.panes, "pane")
    );
    match workspace.window {
        Some(window) => format!("{contents} · in window {window}"),
        None => contents,
    }
}

/// The current workspace first, then those this window can switch to, then
/// those another window holds, which it cannot. Each group keeps the
/// daemon's order.
fn ordered(workspaces: &[Summary], current: Option<u64>) -> Vec<&Summary> {
    let mut ordered: Vec<_> = workspaces.iter().collect();
    ordered.sort_by_key(|workspace| {
        if Some(workspace.id) == current {
            0
        } else if workspace.window.is_none() {
            1
        } else {
            2
        }
    });
    ordered
}

#[derive(Clone, Copy)]
enum Icon {
    Plus,
    Refresh,
}

/// A square button with an icon drawn as strokes, so it scales with the
/// button instead of depending on which font covers a glyph.
fn icon_button(ui: &mut egui::Ui, side: f32, icon: Icon, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        theme::TAB_HOVER_BG
    } else if response.hovered() {
        theme::TAB_ACTIVE_BG
    } else {
        theme::TAB_HOVER_BG
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 5, fill);
    let stroke = egui::Stroke::new(1.6, theme::FG);
    let c = rect.center();
    let r = side * 0.24;
    match icon {
        Icon::Plus => {
            painter.line_segment([c - egui::vec2(r, 0.0), c + egui::vec2(r, 0.0)], stroke);
            painter.line_segment([c - egui::vec2(0.0, r), c + egui::vec2(0.0, r)], stroke);
        }
        Icon::Refresh => {
            // An arc that leaves a gap at the top, closed by a solid
            // arrowhead that points clockwise along it.
            let r = r * 1.1;
            let point = |angle: f32| c + r * egui::vec2(angle.cos(), angle.sin());
            let (start, end) = (-0.9_f32, -0.9 + 5.0);
            let arc: Vec<_> = (0..=24)
                .map(|i| point(start + (end - start) * i as f32 / 24.0))
                .collect();
            painter.add(egui::Shape::line(arc, stroke));
            let base = point(end);
            let tangent = egui::vec2(-end.sin(), end.cos());
            let normal = egui::vec2(end.cos(), end.sin());
            let head = r * 0.7;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    base + tangent * head * 1.2,
                    base + normal * head,
                    base - normal * head,
                ],
                theme::FG,
                egui::Stroke::NONE,
            ));
        }
    }
    response.on_hover_text(tooltip)
}

/// `roster` is `None` until the daemon has answered; `current` is the
/// workspace this window holds.
pub(crate) fn show(
    ctx: &egui::Context,
    roster: Option<&[Summary]>,
    current: Option<u64>,
) -> Outcome {
    let mut outcome = Outcome::default();
    let style = ctx.global_style();
    let viewport = ctx.content_rect();
    let top = theme::overlay_top(viewport.height());
    let width =
        (viewport.width() - 48.0).clamp(100.0, theme::ui_extent(&style, theme::PALETTE_WIDTH));
    let detail_row_height = theme::ui_extent(&style, theme::PALETTE_DETAIL_ROW_HEIGHT);
    // The list grows with the workspaces, up to eight rows or the room the
    // window leaves below the header. It holds whole rows, so a row is
    // never cut off mid-text; the rest scrolls.
    let mut inner = (*style).clone();
    theme::menu_style(&mut inner);
    let gap = inner.spacing.item_spacing.y;
    let pitch = detail_row_height + gap;
    let room = viewport.height() - top - theme::ui_extent(&style, 64.0);
    let rows = ((room + gap) / pitch).floor().clamp(1.0, MAX_VISIBLE_ROWS);
    let list_height = rows * pitch - gap;
    let id = egui::Id::unique("workspace_picker");
    let response = egui::Modal::new(id)
        .area(egui::Modal::default_area(id).anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, top)))
        .frame(theme::overlay_frame())
        .show(ctx, |ui| {
            theme::menu_style(ui.style_mut());
            ui.visuals_mut().weak_text_color = Some(theme::TAB_INACTIVE_FG);
            ui.style_mut().visuals.selection.bg_fill = theme::TAB_ACTIVE_BG;
            ui.style_mut().visuals.selection.stroke = egui::Stroke::new(1.0, theme::FG);
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Workspaces").color(theme::TAB_INACTIVE_FG));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let side = theme::ui_extent(ui.style(), 30.0);
                    if icon_button(ui, side, Icon::Plus, "New workspace").clicked() {
                        outcome.selected = Some(None);
                    }
                    if icon_button(ui, side, Icon::Refresh, "Refresh").clicked() {
                        outcome.refresh = true;
                    }
                });
            });
            ui.add_space(6.0);
            let Some(workspaces) = roster else {
                ui.label(egui::RichText::new("Loading workspaces…").color(theme::TAB_INACTIVE_FG));
                return;
            };
            egui::ScrollArea::vertical()
                .id_salt("workspace_results")
                .max_height(list_height)
                // egui keeps a scrolled area at least 64 px tall, which in a
                // short window shows part of the next row.
                .min_scrolled_height(list_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for workspace in ordered(workspaces, current) {
                        let held = workspace.window.is_some();
                        let unavailable = held && Some(workspace.id) != current;
                        let text = egui::RichText::new(format!("Workspace {}", workspace.id));
                        let detail = egui::RichText::new(detail(workspace))
                            .size(theme::ui_size(ui.style(), 12.0))
                            .color(theme::TAB_INACTIVE_FG);
                        let row = ui.add_enabled_ui(!held, |ui| {
                            ui.add_sized(
                                [ui.available_width(), detail_row_height],
                                egui::Button::new(egui::text::LayoutJob::default())
                                    .selected(Some(workspace.id) == current),
                            )
                        });
                        let rect = row.inner.rect;
                        let painter = ui.painter();
                        let (ink, detail_ink) = if unavailable {
                            (
                                theme::TAB_INACTIVE_FG.gamma_multiply(0.7),
                                theme::TAB_INACTIVE_FG.gamma_multiply(0.5),
                            )
                        } else {
                            (theme::FG, theme::TAB_INACTIVE_FG)
                        };
                        let x = rect.left() + 10.0;
                        painter.text(
                            egui::pos2(x, rect.center().y - 8.0),
                            egui::Align2::LEFT_CENTER,
                            text.text(),
                            egui::FontId::proportional(theme::ui_size(ui.style(), 14.0)),
                            ink,
                        );
                        painter.text(
                            egui::pos2(x, rect.center().y + 9.0),
                            egui::Align2::LEFT_CENTER,
                            detail.text(),
                            egui::FontId::proportional(theme::ui_size(ui.style(), 12.0)),
                            detail_ink,
                        );
                        if Some(workspace.id) == current {
                            painter.text(
                                egui::pos2(rect.right() - 10.0, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                "Current",
                                egui::FontId::proportional(theme::ui_size(ui.style(), 12.0)),
                                theme::ACCENT,
                            );
                        }
                        if row.inner.clicked() {
                            outcome.selected = Some(Some(workspace.id));
                        }
                    }
                });
        });
    outcome.dismissed = response.should_close();
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(id: u64, window: Option<u32>, tabs: usize, panes: usize) -> Summary {
        Summary {
            id,
            window,
            tabs,
            panes,
            last_used: 0,
            instance: String::new(),
        }
    }

    #[test]
    fn detail_counts_contents_and_names_the_holding_window() {
        assert_eq!(detail(&summary(1, None, 1, 1)), "1 tab · 1 pane");
        assert_eq!(
            detail(&summary(2, Some(7), 3, 5)),
            "3 tabs · 5 panes · in window 7"
        );
    }

    #[test]
    fn workspaces_held_elsewhere_sink_below_those_that_can_be_chosen() {
        let roster = [
            summary(1, Some(9), 1, 1),
            summary(2, None, 1, 1),
            summary(3, Some(4), 1, 1),
            summary(4, None, 1, 1),
        ];
        let ids =
            |current| -> Vec<u64> { ordered(&roster, current).iter().map(|w| w.id).collect() };
        assert_eq!(ids(Some(3)), [3, 2, 4, 1]);
        assert_eq!(ids(None), [2, 4, 1, 3]);
    }

    #[test]
    fn a_short_window_clips_the_list_at_a_whole_row() {
        let ctx = egui::Context::default();
        let roster: Vec<_> = (1..=3).map(|id| summary(id, None, 1, 1)).collect();
        let mut clips = Vec::new();
        for _ in 0..2 {
            let output = crate::test_support::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(588.0, 161.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    show(ui.ctx(), Some(&roster), Some(1));
                },
            );
            clips = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Workspace 1" => {
                        Some(shape.clip_rect.height())
                    }
                    _ => None,
                })
                .collect();
        }
        assert_eq!(clips, [theme::PALETTE_DETAIL_ROW_HEIGHT]);
    }

    fn texts(ctx: &egui::Context, roster: Option<&[Summary]>) -> Vec<String> {
        let output = crate::test_support::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                show(ui.ctx(), roster, Some(1));
            },
        );
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn rows_show_name_detail_and_the_current_marker() {
        let ctx = egui::Context::default();
        let roster = [summary(1, None, 2, 3), summary(2, Some(4), 1, 1)];
        texts(&ctx, Some(&roster));
        let shown = texts(&ctx, Some(&roster));
        for expected in [
            "Workspace 1",
            "2 tabs · 3 panes",
            "Current",
            "Workspace 2",
            "1 tab · 1 pane · in window 4",
        ] {
            assert!(shown.iter().any(|t| t == expected), "{expected}: {shown:?}");
        }
    }

    #[test]
    fn waits_for_the_roster() {
        let ctx = egui::Context::default();
        texts(&ctx, None);
        assert!(texts(&ctx, None).iter().any(|t| t == "Loading workspaces…"));
    }
}
