use std::{collections::BTreeMap, time::Duration};

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use felis_grid::{AttrFlags, Color, Grapheme};
use felis_protocol::messages::{InputMsg, RequestedDims};

use crate::{
    layout::{Axis, Layout},
    session::Session,
};

const BG: Color32 = Color32::from_rgb(20, 23, 31);
const FG: Color32 = Color32::from_rgb(216, 222, 233);
const ACCENT: Color32 = Color32::from_rgb(163, 190, 140);

struct Pane {
    session: Session,
    dims: (u16, u16),
    selection: Option<(usize, usize)>,
    reported_focus: Option<bool>,
}

struct Tab {
    name: String,
    layout: Layout,
    focused: u64,
}

pub struct App {
    runtime: tokio::runtime::Runtime,
    panes: BTreeMap<u64, Pane>,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    font_size: f32,
    attach_input: String,
    show_attach: bool,
    help: bool,
}

#[derive(Clone, Copy)]
enum Action {
    NewTab,
    Split(Axis),
    ClosePane,
    CloseTab,
    NextPane,
    NextTab,
    PreviousTab,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, attach: Option<String>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals.panel_fill = BG;
        style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.3);
        style.spacing.button_padding = Vec2::new(12.0, 7.0);
        cc.egui_ctx.set_style(style);
        // egui's bundled fonts lack CJK. Load an installed fallback when available.
        let mut fonts = egui::FontDefinitions::default();
        let candidates = [
            std::env::var("NEKOMATA_FONT").unwrap_or_default(),
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc".into(),
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc".into(),
        ];
        for path in candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("cjk".into(), egui::FontData::from_owned(bytes).into());
                for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
                    fonts.families.entry(family).or_default().push("cjk".into());
                }
                break;
            }
        }
        cc.egui_ctx.set_fonts(fonts);
        let mut app = Self {
            runtime: tokio::runtime::Runtime::new().expect("create IPC runtime"),
            panes: BTreeMap::new(),
            tabs: vec![],
            active: 0,
            next_id: 1,
            font_size: 15.0,
            attach_input: String::new(),
            show_attach: false,
            help: false,
        };
        app.add_tab(&cc.egui_ctx, attach);
        app
    }

    fn add_pane(&mut self, ctx: &egui::Context, attach: Option<String>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.panes.insert(
            id,
            Pane {
                session: Session::new(&self.runtime, ctx.clone(), attach),
                dims: (0, 0),
                selection: None,
                reported_focus: None,
            },
        );
        id
    }

    fn add_tab(&mut self, ctx: &egui::Context, attach: Option<String>) {
        let id = self.add_pane(ctx, attach);
        self.tabs.push(Tab {
            name: format!("Terminal {id}"),
            layout: Layout::Pane(id),
            focused: id,
        });
        self.active = self.tabs.len() - 1;
    }

    fn action(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::NewTab => self.add_tab(ctx, None),
            Action::Split(axis) => {
                let id = self.add_pane(ctx, None);
                let tab = &mut self.tabs[self.active];
                tab.layout.split(tab.focused, id, axis);
                tab.focused = id;
            }
            Action::ClosePane => {
                let tab = &mut self.tabs[self.active];
                let id = tab.focused;
                let layout = std::mem::replace(&mut tab.layout, Layout::Pane(id));
                if let Some(layout) = layout.remove(id) {
                    tab.focused = layout.panes()[0];
                    tab.layout = layout;
                    self.panes.remove(&id);
                } else {
                    self.action(ctx, Action::CloseTab);
                }
            }
            Action::CloseTab => {
                let tab = self.tabs.remove(self.active);
                for id in tab.layout.panes() {
                    self.panes.remove(&id);
                }
                if self.tabs.is_empty() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    self.active = self.active.min(self.tabs.len() - 1);
                }
            }
            Action::NextPane => {
                let tab = &mut self.tabs[self.active];
                let ids = tab.layout.panes();
                let pos = ids.iter().position(|id| *id == tab.focused).unwrap_or(0);
                tab.focused = ids[(pos + 1) % ids.len()];
            }
            Action::NextTab => self.active = (self.active + 1) % self.tabs.len(),
            Action::PreviousTab => {
                self.active = (self.active + self.tabs.len() - 1) % self.tabs.len()
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.tabs.is_empty() {
            return;
        }
        let mut action = None;
        // Consume shortcuts before terminal input sees the events.
        if !self.show_attach && !self.help {
            ctx.input_mut(|input| {
                let mods = if cfg!(target_os = "macos") {
                    egui::Modifiers::COMMAND
                } else {
                    egui::Modifiers::CTRL | egui::Modifiers::SHIFT
                };
                for (key, command) in [
                    (egui::Key::T, Action::NewTab),
                    (egui::Key::D, Action::Split(Axis::Horizontal)),
                    (egui::Key::W, Action::ClosePane),
                    (egui::Key::N, Action::NextPane),
                    (egui::Key::ArrowRight, Action::NextTab),
                    (egui::Key::ArrowLeft, Action::PreviousTab),
                ] {
                    if input.consume_key(mods, key) {
                        action = Some(command);
                    }
                }
                if input.consume_key(mods | egui::Modifiers::SHIFT, egui::Key::D)
                    && cfg!(target_os = "macos")
                {
                    action = Some(Action::Split(Axis::Vertical));
                }
                if input.consume_key(mods, egui::Key::E) {
                    action = Some(Action::Split(Axis::Vertical));
                }
            });
        }
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("nekomata").strong().color(ACCENT));
                ui.separator();
                egui::ScrollArea::horizontal()
                    .id_salt("tab-scroll")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (i, tab) in self.tabs.iter().enumerate() {
                                let title = self.panes[&tab.focused]
                                    .session
                                    .state
                                    .lock()
                                    .unwrap()
                                    .screen
                                    .title()
                                    .unwrap_or(&tab.name)
                                    .to_owned();
                                if ui.selectable_label(self.active == i, title).clicked() {
                                    self.active = i;
                                }
                            }
                        });
                    });
            });
            ui.horizontal(|ui| {
                if ui.button("+ Tab").clicked() {
                    action = Some(Action::NewTab);
                }
                if ui.button("Split ↔").clicked() {
                    action = Some(Action::Split(Axis::Horizontal));
                }
                if ui.button("Split ↕").clicked() {
                    action = Some(Action::Split(Axis::Vertical));
                }
                if ui
                    .button("Close pane")
                    .on_hover_text("Detach; keep the shell running")
                    .clicked()
                {
                    action = Some(Action::ClosePane);
                }
                if ui.button("Close tab").clicked() {
                    action = Some(Action::CloseTab);
                }
                ui.separator();
                if ui.button("Attach…").clicked() {
                    self.show_attach = true;
                }
                if ui.button("−").clicked() {
                    self.font_size = (self.font_size - 1.0).max(9.0);
                }
                if ui.button("+").clicked() {
                    self.font_size = (self.font_size + 1.0).min(32.0);
                }
                if ui.button("?").clicked() {
                    self.help = !self.help;
                }
            });
        });
        if let Some(action) = action {
            self.action(ctx, action);
        }
        if self.tabs.is_empty() {
            return;
        }
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let tab = &self.tabs[self.active];
            let state = self.panes[&tab.focused].session.state.lock().unwrap();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(if state.connected { "●" } else { "○" }).color(ACCENT),
                );
                ui.label(format!(
                    "Tab {} / {}   ·   {} panes",
                    self.active + 1,
                    self.tabs.len(),
                    tab.layout.panes().len()
                ));
                if let Some(id) = state.session_id {
                    let label = format!("{id:032x}");
                    if ui
                        .small_button(&label[..8])
                        .on_hover_text("Copy full felis session ID")
                        .clicked()
                    {
                        ctx.copy_text(label);
                    }
                }
                ui.label("Close to detach · sessions stay alive");
            });
        });
        let modal = self.show_attach || self.help;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG).inner_margin(6))
            .show(ctx, |ui| {
                let tab = &mut self.tabs[self.active];
                draw_layout(
                    ui,
                    &mut tab.layout,
                    &mut self.panes,
                    &mut tab.focused,
                    self.font_size,
                );
            });
        let focused = self.tabs[self.active].focused;
        let os_focused = ctx.input(|i| i.focused);
        for (&id, pane) in &mut self.panes {
            let desired = id == focused && os_focused && !modal;
            if pane.reported_focus != Some(desired) {
                pane.session
                    .send(InputMsg::FocusChange { focused: desired });
                pane.reported_focus = Some(desired);
            }
        }
        if !modal {
            let pane = self.panes.get_mut(&focused).unwrap();
            terminal_input(ctx, &mut pane.selection, &pane.session);
        }
        if self.show_attach {
            let mut open = true;
            let mut attach = false;
            egui::Window::new("Attach a felis session")
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label("Paste a session ID from `felis sessions list`.");
                    ui.text_edit_singleline(&mut self.attach_input);
                    attach = ui.button("Attach in new tab").clicked();
                });
            self.show_attach = open;
            if attach && !self.attach_input.trim().is_empty() {
                let id = self.attach_input.trim().to_owned();
                self.add_tab(ctx, Some(id));
                self.attach_input.clear();
                self.show_attach = false;
            }
        }
        if self.help {
            egui::Window::new("About nekomata").open(&mut self.help).show(ctx, |ui| {
                let modifier = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl+Shift" };
                ui.label("A home for your felis sessions.");
                for (key, text) in [("T", "New tab"), ("D", "Split left / right"), ("E", "Split top / bottom"),
                    ("N", "Next pane"), ("W", "Close pane (detach)"), ("← / →", "Previous / next tab")] {
                    ui.label(format!("{modifier}+{key}    {text}"));
                }
                ui.label("Drag a divider to resize. Drag text to select; copy with Cmd+C / Ctrl+Shift+C.");
                ui.label("Scroll to browse history. Shift+drag selects when mouse reporting is active.");
                ui.separator();
                ui.label("Initial text frontend: Kitty images, OSC 66 sizing and ligatures are not rendered.");
            });
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn draw_layout(
    ui: &mut egui::Ui,
    layout: &mut Layout,
    panes: &mut BTreeMap<u64, Pane>,
    focused: &mut u64,
    font_size: f32,
) {
    let rect = ui.max_rect();
    match layout {
        Layout::Pane(id) => draw_pane(ui, *id, panes.get_mut(id).unwrap(), focused, font_size),
        Layout::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let horizontal = *axis == Axis::Horizontal;
            let extent = if horizontal {
                rect.width()
            } else {
                rect.height()
            };
            let split = (extent - 6.0).max(0.0) * *ratio;
            let (a, divider, b) = if horizontal {
                let x = rect.left() + split;
                (
                    Rect::from_min_max(rect.min, Pos2::new(x, rect.bottom())),
                    Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(x + 6.0, rect.bottom())),
                    Rect::from_min_max(Pos2::new(x + 6.0, rect.top()), rect.max),
                )
            } else {
                let y = rect.top() + split;
                (
                    Rect::from_min_max(rect.min, Pos2::new(rect.right(), y)),
                    Rect::from_min_max(Pos2::new(rect.left(), y), Pos2::new(rect.right(), y + 6.0)),
                    Rect::from_min_max(Pos2::new(rect.left(), y + 6.0), rect.max),
                )
            };
            let response = ui.interact(divider, ui.id().with("divider"), Sense::drag());
            let response = response.on_hover_cursor(if horizontal {
                egui::CursorIcon::ResizeHorizontal
            } else {
                egui::CursorIcon::ResizeVertical
            });
            if response.dragged() && extent > 0.0 {
                let delta = ui.input(|i| i.pointer.delta());
                *ratio =
                    (*ratio + if horizontal { delta.x } else { delta.y } / extent).clamp(0.1, 0.9);
            }
            ui.painter().rect_filled(
                divider.shrink(2.0),
                2,
                if response.hovered() {
                    ACCENT
                } else {
                    Color32::from_gray(50)
                },
            );
            ui.scope_builder(egui::UiBuilder::new().id_salt("first").max_rect(a), |ui| {
                draw_layout(ui, first, panes, focused, font_size)
            });
            ui.scope_builder(egui::UiBuilder::new().id_salt("second").max_rect(b), |ui| {
                draw_layout(ui, second, panes, focused, font_size)
            });
        }
    }
}

fn draw_pane(ui: &mut egui::Ui, id: u64, pane: &mut Pane, focused: &mut u64, font_size: f32) {
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    if response.clicked() || response.drag_started() {
        *focused = id;
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 4, BG);
    painter.rect_stroke(
        rect.shrink(0.5),
        4,
        Stroke::new(
            1.0_f32,
            if *focused == id {
                ACCENT.gamma_multiply(0.7)
            } else {
                Color32::from_gray(45)
            },
        ),
        egui::StrokeKind::Inside,
    );
    let text_rect = rect.shrink(9.0);
    let font = FontId::monospace(font_size);
    let cw = ui.fonts_mut(|f| f.glyph_width(&font, 'M'));
    let ch = (font_size * 1.4).ceil();
    let cols = (text_rect.width() / cw).floor().clamp(1.0, 1000.0) as u16;
    let rows = (text_rect.height() / ch).floor().clamp(1.0, 500.0) as u16;
    if pane.dims != (rows, cols) {
        pane.dims = (rows, cols);
        pane.selection = None;
        pane.session.send(InputMsg::Resize {
            dims: RequestedDims {
                rows: rows.into(),
                cols: cols.into(),
                pixel_w: (text_rect.width().max(0.0) * ui.ctx().pixels_per_point()) as u32,
                pixel_h: (text_rect.height().max(0.0) * ui.ctx().pixels_per_point()) as u32,
            },
        });
    }
    let state = pane.session.state.lock().unwrap();
    let shadow = &state.screen;
    let screen = shadow.screen();
    let painter = painter.with_clip_rect(text_rect);
    for row in 0..screen.rows().min(rows) {
        for col in 0..screen.cols().min(cols) {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            let attr = screen.style(cell.style);
            let mut bg = color(attr.bg, BG);
            let mut fg = color(attr.fg, FG);
            if attr.flags.contains(AttrFlags::REVERSE) ^ shadow.reverse_video() {
                std::mem::swap(&mut bg, &mut fg);
            }
            let index = usize::from(row) * usize::from(screen.cols()) + usize::from(col);
            if pane
                .selection
                .is_some_and(|(a, b)| (a.min(b)..=a.max(b)).contains(&index))
            {
                bg = Color32::from_rgb(64, 78, 94);
            }
            let pos = text_rect.min + Vec2::new(f32::from(col) * cw, f32::from(row) * ch);
            painter.rect_filled(Rect::from_min_size(pos, Vec2::new(cw, ch)), 0, bg);
        }
    }
    for row in 0..screen.rows().min(rows) {
        for col in 0..screen.cols().min(cols) {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            let attr = screen.style(cell.style);
            if attr.flags.contains(AttrFlags::CONCEAL) {
                continue;
            }
            let mut fg = color(attr.fg, FG);
            if attr.flags.contains(AttrFlags::REVERSE) ^ shadow.reverse_video() {
                fg = color(attr.bg, BG);
            }
            if attr.flags.contains(AttrFlags::FAINT) {
                fg = fg.gamma_multiply(0.6);
            }
            let text = match cell.grapheme {
                Grapheme::Ascii(b) => char::from(b).to_string(),
                Grapheme::Char(c) => c.to_string(),
                Grapheme::Cluster(id) => shadow.cluster_str(id).unwrap_or("�").to_owned(),
                _ => continue,
            };
            let pos = text_rect.min + Vec2::new(f32::from(col) * cw, f32::from(row) * ch);
            painter.text(pos, egui::Align2::LEFT_TOP, &text, font.clone(), fg);
            if attr.flags.contains(AttrFlags::BOLD) {
                painter.text(
                    pos + Vec2::new(0.4, 0.0),
                    egui::Align2::LEFT_TOP,
                    text,
                    font.clone(),
                    fg,
                );
            }
            if attr.flags.contains(AttrFlags::UNDERLINE) {
                painter.line_segment(
                    [
                        pos + Vec2::new(0.0, ch - 2.0),
                        pos + Vec2::new(cw, ch - 2.0),
                    ],
                    Stroke::new(1.0_f32, fg),
                );
            }
        }
    }
    let cursor = screen.cursor();
    if *focused == id && cursor.visible && shadow.viewport() == 0 && state.connected {
        let pos = text_rect.min + Vec2::new(f32::from(cursor.col) * cw, f32::from(cursor.row) * ch);
        painter.rect_stroke(
            Rect::from_min_size(pos, Vec2::new(cw, ch)),
            0,
            Stroke::new(1.5_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    if pane
        .session
        .input_failed
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        painter.text(
            text_rect.min,
            egui::Align2::LEFT_TOP,
            "Input could not be delivered. Reattach this session before continuing.",
            FontId::proportional(14.0),
            Color32::LIGHT_RED,
        );
    } else if let Some(error) = &state.error {
        painter.text(
            text_rect.min,
            egui::Align2::LEFT_TOP,
            error,
            FontId::proportional(14.0),
            Color32::LIGHT_RED,
        );
    } else if !state.connected {
        painter.text(
            text_rect.min,
            egui::Align2::LEFT_TOP,
            "Connecting to felis…",
            font,
            FG,
        );
    }
    let mouse_mode = shadow.mouse_protocol_active();
    if let Some(pos) = response.interact_pointer_pos()
        && (!mouse_mode || ui.input(|i| i.modifiers.shift))
    {
        let col = ((pos.x - text_rect.left()) / cw)
            .floor()
            .clamp(0.0, f32::from(screen.cols() - 1)) as usize;
        let row = ((pos.y - text_rect.top()) / ch)
            .floor()
            .clamp(0.0, f32::from(screen.rows() - 1)) as usize;
        let index = row * usize::from(screen.cols()) + col;
        if response.drag_started() {
            pane.selection = Some((index, index));
        }
        if response.dragged()
            && let Some((_, end)) = &mut pane.selection
        {
            *end = index;
        }
        if response.clicked() {
            pane.selection = None;
        }
    }
    if response.hovered() {
        let delta = ui.input(|i| i.smooth_scroll_delta.y);
        if delta.abs() >= 1.0 {
            let offset = (i64::from(shadow.viewport()) + (delta / ch).round() as i64).max(0) as u32;
            pane.session.send(InputMsg::Viewport {
                lines_from_bottom: offset,
            });
        }
    }
}

fn color(color: Color, default: Color32) -> Color32 {
    match color {
        Color::Default => default,
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        Color::Indexed(n) => {
            const BASE: [[u8; 3]; 16] = [
                [46, 52, 64],
                [191, 97, 106],
                [163, 190, 140],
                [235, 203, 139],
                [129, 161, 193],
                [180, 142, 173],
                [136, 192, 208],
                [229, 233, 240],
                [76, 86, 106],
                [191, 97, 106],
                [163, 190, 140],
                [235, 203, 139],
                [129, 161, 193],
                [180, 142, 173],
                [143, 188, 187],
                [236, 239, 244],
            ];
            let [r, g, b] = if n < 16 {
                BASE[n as usize]
            } else if n >= 232 {
                [8 + (n - 232) * 10; 3]
            } else {
                let n = n - 16;
                let ramp = |v| if v == 0 { 0 } else { 55 + 40 * v };
                [ramp(n / 36), ramp(n / 6 % 6), ramp(n % 6)]
            };
            Color32::from_rgb(r, g, b)
        }
    }
}

fn terminal_input(ctx: &egui::Context, selection: &mut Option<(usize, usize)>, session: &Session) {
    use felis_protocol::messages::{Key, KeyEvent, KeyEventKind, KeyLocation, KeyMods, NamedKey};
    let events = ctx.input(|i| i.events.clone());
    let mods = ctx.input(|i| i.modifiers);
    let mut key_mods = KeyMods::empty();
    key_mods.set(KeyMods::CTRL, mods.ctrl);
    key_mods.set(KeyMods::SHIFT, mods.shift);
    key_mods.set(KeyMods::ALT, mods.alt);
    key_mods.set(KeyMods::SUPER, mods.mac_cmd);
    for event in events {
        if let Some(control) = clipboard_control(&event, mods, cfg!(target_os = "macos")) {
            *selection = None;
            session.send(control);
            continue;
        }
        let key = match event {
            egui::Event::Paste(text) => {
                session.send(InputMsg::Paste(text.into_bytes()));
                continue;
            }
            egui::Event::Copy => {
                if let Some((a, b)) = *selection {
                    let state = session.state.lock().unwrap();
                    let screen = state.screen.screen();
                    let cols = usize::from(screen.cols());
                    let mut text = String::new();
                    for index in a.min(b)..=a.max(b) {
                        if index != a.min(b) && index % cols == 0 {
                            text.push('\n');
                        }
                        if let Some(cell) =
                            screen.cell((index / cols) as u16, (index % cols) as u16)
                        {
                            match cell.grapheme {
                                Grapheme::Ascii(c) => text.push(char::from(c)),
                                Grapheme::Char(c) => text.push(c),
                                Grapheme::Cluster(id) => {
                                    text.push_str(state.screen.cluster_str(id).unwrap_or(""))
                                }
                                Grapheme::Empty => text.push(' '),
                                _ => {}
                            }
                        }
                    }
                    ctx.copy_text(text);
                }
                continue;
            }
            egui::Event::Text(text) => Some((Key::Character(text.clone()), Some(text))),
            egui::Event::Key {
                key, pressed: true, ..
            } => {
                let named = match key {
                    egui::Key::Enter => Some(NamedKey::Enter),
                    egui::Key::Tab => Some(NamedKey::Tab),
                    egui::Key::Escape => Some(NamedKey::Escape),
                    egui::Key::Backspace => Some(NamedKey::Backspace),
                    egui::Key::Insert => Some(NamedKey::Insert),
                    egui::Key::Delete => Some(NamedKey::Delete),
                    egui::Key::Home => Some(NamedKey::Home),
                    egui::Key::End => Some(NamedKey::End),
                    egui::Key::PageUp => Some(NamedKey::PageUp),
                    egui::Key::PageDown => Some(NamedKey::PageDown),
                    egui::Key::ArrowUp => Some(NamedKey::ArrowUp),
                    egui::Key::ArrowDown => Some(NamedKey::ArrowDown),
                    egui::Key::ArrowLeft => Some(NamedKey::ArrowLeft),
                    egui::Key::ArrowRight => Some(NamedKey::ArrowRight),
                    _ => None,
                };
                if let Some(named) = named {
                    Some((Key::Named(named), None))
                } else if mods.ctrl || mods.alt {
                    Some((Key::Character(key.name().to_lowercase()), None))
                } else {
                    None
                }
            }
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                session.send(InputMsg::KeyBytes(text.into_bytes()));
                continue;
            }
            _ => None,
        };
        if let Some((key, text)) = key {
            *selection = None;
            session.send(InputMsg::Viewport {
                lines_from_bottom: 0,
            });
            session.send(InputMsg::Key(KeyEvent {
                key,
                text,
                mods: key_mods,
                kind: KeyEventKind::Press,
                location: KeyLocation::Standard,
            }));
        }
    }
}

fn clipboard_control(event: &egui::Event, mods: egui::Modifiers, macos: bool) -> Option<InputMsg> {
    if macos || !mods.ctrl || mods.shift || mods.alt {
        return None;
    }
    let byte = match event {
        egui::Event::Copy => 3,
        egui::Event::Cut => 24,
        egui::Event::Paste(_) => 22,
        _ => return None,
    };
    Some(InputMsg::KeyBytes(vec![byte]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_control_c_interrupts_instead_of_copying() {
        assert_eq!(
            clipboard_control(&egui::Event::Copy, egui::Modifiers::CTRL, false),
            Some(InputMsg::KeyBytes(vec![3]))
        );
    }
}
