use std::{collections::BTreeMap, sync::Arc, time::Duration};

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use felis_grid::{AttrFlags, Color, CursorStyle, Grapheme};
use felis_protocol::messages::{InputMsg, RequestedDims};

use crate::{
    control::{self, Reply, Request},
    daemon::{self, Payload},
    layout::{Axis, Layout},
    picker::SessionPicker,
    session::{Session, Target},
    shortcuts::{Action, Shortcuts},
    theme::{ACCENT, BG, FG, TAB_ACTIVE_BG, TAB_BAR_BG, TAB_HOVER_BG, TAB_INACTIVE_FG},
    workspace::{PaneSpec, Snapshot, Tab},
    workspace_client::{Client as WorkspaceClient, Operation},
};

struct Pane {
    session: Arc<Session>,
    dims: (u16, u16),
    selection: Option<(usize, usize)>,
    reported_focus: Option<bool>,
    scroll_remainder: f32,
    mouse_capture: Option<felis_protocol::messages::MouseButton>,
}

impl Pane {
    fn from_session(session: Arc<Session>) -> Self {
        Self {
            session,
            dims: (0, 0),
            selection: None,
            reported_focus: None,
            scroll_remainder: 0.0,
            mouse_capture: None,
        }
    }

    fn connect(
        runtime: &tokio::runtime::Runtime,
        ctx: &egui::Context,
        window: u32,
        workspace: Option<u64>,
        id: u64,
        spec: PaneSpec,
        cwd: Option<String>,
    ) -> Self {
        let mut env = vec![
            ("NEKOMATA_WINDOW_ID".into(), window.to_string()),
            ("NEKOMATA_PANE_ID".into(), id.to_string()),
        ];
        if let Some(id) = workspace {
            env.push(("NEKOMATA_SESSION_ID".into(), id.to_string()));
        }
        Self::from_session(Arc::new(Session::start(
            runtime,
            ctx.clone(),
            spec.target,
            spec.session,
            cwd,
            env,
        )))
    }
}

struct PendingAttach {
    session: Arc<Session>,
    reply: tokio::sync::oneshot::Sender<Reply>,
}

struct WorkspaceSwitch {
    id: Option<u64>,
    snapshot: Option<Snapshot>,
    panes: BTreeMap<u64, Pane>,
    committing: bool,
    cancellation: Option<String>,
    reply: Option<tokio::sync::oneshot::Sender<Reply>>,
}

pub struct App {
    workspace: Option<WorkspaceClient>,
    workspace_id: Option<u64>,
    workspace_error: Option<String>,
    last_snapshot: Option<Snapshot>,
    pending_workspace: Option<WorkspaceSwitch>,
    workspaces_open: bool,
    workspace_roster: Option<Vec<daemon::Summary>>,
    runtime: tokio::runtime::Runtime,
    control: control::Server,
    pending_attach: BTreeMap<u64, PendingAttach>,
    panes: BTreeMap<u64, Pane>,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    font_size: f32,
    picker: SessionPicker,
    help: bool,
    shortcuts: Shortcuts,
    palette: bool,
    palette_query: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals.panel_fill = BG;
        style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.3);
        style.spacing.button_padding = Vec2::new(12.0, 7.0);
        cc.egui_ctx.set_style(style);
        // Nix supplies UDEV Gothic NF; system CJK fonts remain a fallback.
        let mut fonts = egui::FontDefinitions::default();
        let candidates = [
            std::env::var("NEKOMATA_FONT").unwrap_or_default(),
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc".into(),
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc".into(),
        ];
        for (index, path) in candidates.into_iter().enumerate() {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("cjk".into(), egui::FontData::from_owned(bytes).into());
                for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
                    let family = fonts.families.entry(family).or_default();
                    if index == 0 {
                        family.insert(0, "cjk".into());
                    } else {
                        family.push("cjk".into());
                    }
                }
                break;
            }
        }
        cc.egui_ctx.set_fonts(fonts);
        Self::connect(tokio::runtime::Runtime::new()?, &cc.egui_ctx)
    }

    fn connect(runtime: tokio::runtime::Runtime, ctx: &egui::Context) -> anyhow::Result<Self> {
        let control = control::Server::bind(&runtime, ctx.clone())?;
        let workspace = WorkspaceClient::start(&runtime, ctx.clone(), control.window);
        let app = Self {
            workspace: Some(workspace),
            workspace_id: None,
            workspace_error: None,
            last_snapshot: None,
            pending_workspace: None,
            workspaces_open: false,
            workspace_roster: None,
            runtime,
            control,
            pending_attach: BTreeMap::new(),
            panes: BTreeMap::new(),
            tabs: vec![],
            active: 0,
            next_id: 1,
            font_size: 15.0,
            picker: SessionPicker::default(),
            help: false,
            shortcuts: Shortcuts::default(),
            palette: false,
            palette_query: String::new(),
        };
        Ok(app)
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            tabs: self.tabs.clone(),
            panes: self
                .panes
                .iter()
                .map(|(&id, pane)| {
                    let state = pane.session.state.lock().unwrap();
                    (
                        id,
                        PaneSpec {
                            target: pane.session.target.clone(),
                            session: state
                                .session_id
                                .map(|id| format!("{id:032x}"))
                                .or_else(|| pane.session.attachment.clone()),
                        },
                    )
                })
                .collect(),
            active: self.active,
            next_id: self.next_id,
            font_size: self.font_size,
        }
    }

    fn restore_workspace(&mut self, ctx: &egui::Context, snapshot: Snapshot) {
        self.panes.clear();
        self.pending_attach.clear();
        self.tabs = snapshot.tabs;
        self.active = snapshot.active;
        self.next_id = snapshot.next_id;
        self.font_size = snapshot.font_size;
        for (id, spec) in snapshot.panes {
            self.panes.insert(
                id,
                Pane::connect(
                    &self.runtime,
                    ctx,
                    self.control.window,
                    self.workspace_id,
                    id,
                    spec,
                    None,
                ),
            );
        }
        if self.tabs.is_empty() {
            self.add_tab(ctx, None);
        }
    }

    fn workspace_events(&mut self, ctx: &egui::Context) {
        loop {
            let event = self
                .workspace
                .as_mut()
                .and_then(|client| client.events.try_recv().ok());
            let Some(event) = event else { break };
            match (event.operation, event.result) {
                (Operation::Open, Ok(Payload::Session { id, snapshot })) => {
                    if let Err(error) = snapshot.validate() {
                        self.workspace_error = Some(error.to_string());
                        continue;
                    }
                    self.workspace_id = Some(id);
                    self.last_snapshot = Some(snapshot.clone());
                    self.restore_workspace(ctx, snapshot);
                }
                (Operation::List, Ok(Payload::Sessions(sessions))) => {
                    self.workspace_roster = Some(sessions);
                }
                (Operation::Prepare, Ok(Payload::Session { id, mut snapshot })) => {
                    if let Err(error) = snapshot.validate() {
                        self.cancel_workspace(error.to_string());
                        continue;
                    }
                    if snapshot.tabs.is_empty() {
                        let pane = snapshot.next_id;
                        snapshot.next_id += 1;
                        snapshot.tabs.push(Tab {
                            name: format!("Terminal {pane}"),
                            layout: Layout::Pane(pane),
                            focused: pane,
                            focus_history: vec![pane],
                        });
                        match Target::default_local() {
                            Ok(target) => {
                                snapshot.panes.insert(
                                    pane,
                                    PaneSpec {
                                        target,
                                        session: None,
                                    },
                                );
                            }
                            Err(error) => {
                                self.cancel_workspace(error.to_string());
                                continue;
                            }
                        }
                    }
                    let panes = snapshot
                        .panes
                        .iter()
                        .map(|(&pane_id, spec)| {
                            let existing = spec.session.as_ref().and_then(|id| {
                                self.panes.values().find(|pane| {
                                    let state = pane.session.state.lock().unwrap();
                                    pane.session.target == spec.target
                                        && state.session_id.map(|i| format!("{i:032x}")).as_ref()
                                            == Some(id)
                                })
                            });
                            let pane = if let Some(existing) = existing {
                                Pane::from_session(existing.session.clone())
                            } else {
                                Pane::connect(
                                    &self.runtime,
                                    ctx,
                                    self.control.window,
                                    Some(id),
                                    pane_id,
                                    spec.clone(),
                                    None,
                                )
                            };
                            (pane_id, pane)
                        })
                        .collect();
                    if let Some(pending) = &mut self.pending_workspace {
                        pending.id = Some(id);
                        pending.snapshot = Some(snapshot);
                        pending.panes = panes;
                    }
                }
                (Operation::Finish, Ok(Payload::Done)) => {
                    if let Some(pending) = self.pending_workspace.take() {
                        if let Some(error) = pending.cancellation {
                            if let Some(reply) = pending.reply {
                                let _ = reply.send(Err(error.clone()));
                            }
                            self.workspace_error = Some(error);
                        } else if pending.committing {
                            let snapshot = pending.snapshot.unwrap();
                            self.panes = pending.panes;
                            self.tabs = snapshot.tabs;
                            self.active = snapshot.active;
                            self.next_id = snapshot.next_id;
                            self.font_size = snapshot.font_size;
                            self.workspace_id = pending.id;
                            self.last_snapshot = None;
                            self.workspace_error = None;
                            if let Some(reply) = pending.reply {
                                let _ = reply.send(Ok(format!(
                                    "Attached nekomata session {}",
                                    pending.id.unwrap()
                                )));
                            }
                        }
                    }
                }
                (Operation::Prepare | Operation::Disconnected, Err(error)) => {
                    if let Some(pending) = self.pending_workspace.take()
                        && let Some(reply) = pending.reply
                    {
                        let _ = reply.send(Err(error.clone()));
                    }
                    self.workspace_error = Some(error);
                }
                (Operation::Finish, Err(error)) => {
                    if self
                        .pending_workspace
                        .as_ref()
                        .is_some_and(|pending| pending.cancellation.is_some())
                    {
                        if let Some(pending) = self.pending_workspace.take()
                            && let Some(reply) = pending.reply
                        {
                            let _ = reply.send(Err(error.clone()));
                        }
                        self.workspace_error = Some(error);
                    } else {
                        self.cancel_workspace(error);
                    }
                }
                (_, Err(error)) => self.workspace_error = Some(error),
                _ => {}
            }
        }
    }

    fn begin_workspace(
        &mut self,
        session: Option<u64>,
        reply: Option<tokio::sync::oneshot::Sender<Reply>>,
    ) {
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                self.pending_workspace.is_none() && self.pending_attach.is_empty(),
                "another attach is in progress"
            );
            anyhow::ensure!(self.workspace_id.is_some(), "nekomata session is not ready");
            anyhow::ensure!(
                session != self.workspace_id,
                "session is already attached to this window"
            );
            let client = self
                .workspace
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("nekomata daemon is unavailable"))?;
            client.prepare(
                self.snapshot(),
                session
                    .map(daemon::Selection::Id)
                    .unwrap_or(daemon::Selection::New),
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.workspace_error = None;
                self.workspaces_open = false;
                self.picker.visible = false;
                self.palette = false;
                self.pending_workspace = Some(WorkspaceSwitch {
                    id: None,
                    snapshot: None,
                    panes: BTreeMap::new(),
                    committing: false,
                    cancellation: None,
                    reply,
                });
            }
            Err(error) => {
                let error = error.to_string();
                if let Some(reply) = reply {
                    let _ = reply.send(Err(error.clone()));
                }
                self.workspace_error = Some(error);
            }
        }
    }

    fn cancel_workspace(&mut self, error: String) {
        if let Some(pending) = &mut self.pending_workspace {
            pending.panes.clear();
            pending.cancellation = Some(error.clone());
            pending.committing = false;
        }
        self.workspace_error = Some(error);
    }

    fn poll_workspace(&mut self) {
        let Some(pending) = &mut self.pending_workspace else {
            return;
        };
        if pending.committing {
            return;
        }
        if pending.cancellation.is_some() {
            if self
                .workspace
                .as_ref()
                .unwrap()
                .send(daemon::Request::Finish { snapshot: None })
                .is_ok()
            {
                pending.committing = true;
            }
            return;
        }
        if pending.snapshot.is_none() {
            return;
        }
        let mut ready = true;
        let mut error = None;
        for pane in pending.panes.values() {
            let state = pane.session.state.lock().unwrap();
            if state.error.is_some() || state.detached || state.exited {
                error = Some(
                    state
                        .error
                        .clone()
                        .unwrap_or_else(|| "destination session ended".into()),
                );
                break;
            }
            ready &= state.connected;
        }
        if pending
            .reply
            .as_ref()
            .is_some_and(|reply| reply.is_closed())
        {
            error = Some("session switch cancelled".into());
        }
        if let Some(error) = error {
            self.cancel_workspace(error);
            return;
        }
        if ready {
            let mut snapshot = pending.snapshot.clone().unwrap();
            for (&id, pane) in &pending.panes {
                snapshot.panes.get_mut(&id).unwrap().session = pane
                    .session
                    .state
                    .lock()
                    .unwrap()
                    .session_id
                    .map(|id| format!("{id:032x}"));
            }
            if self
                .workspace
                .as_ref()
                .unwrap()
                .send(daemon::Request::Finish {
                    snapshot: Some(snapshot.clone()),
                })
                .is_ok()
            {
                pending.snapshot = Some(snapshot);
                pending.committing = true;
            }
        }
    }

    fn workspace_picker(&mut self, ctx: &egui::Context) {
        if !self.workspaces_open {
            return;
        }
        let mut open = true;
        let mut selected = None;
        let mut refresh = false;
        egui::Window::new("nekomata sessions")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                if ui.button("New session").clicked() {
                    selected = Some(None);
                }
                refresh = ui.button("Refresh").clicked();
                ui.separator();
                if let Some(sessions) = &self.workspace_roster {
                    for session in sessions {
                        let current = Some(session.id) == self.workspace_id;
                        let owner = session
                            .window
                            .map(|window| format!(" · window {window}"))
                            .unwrap_or_default();
                        let label = format!(
                            "Session {} · {} tabs · {} panes{owner}",
                            session.id, session.tabs, session.panes
                        );
                        if ui
                            .add_enabled(
                                session.window.is_none(),
                                egui::Button::new(label).selected(current),
                            )
                            .clicked()
                        {
                            selected = Some(Some(session.id));
                        }
                    }
                } else {
                    ui.label("Loading sessions…");
                }
            });
        self.workspaces_open = open;
        if refresh && let Some(client) = &self.workspace {
            let _ = client.send(daemon::Request::List);
        }
        if let Some(session) = selected {
            self.begin_workspace(session, None);
        }
    }

    fn save_workspace(&mut self) {
        if self.workspace_id.is_none() || self.pending_workspace.is_some() {
            return;
        }
        let snapshot = self.snapshot();
        if self.last_snapshot.as_ref() != Some(&snapshot)
            && let Some(client) = &self.workspace
            && client.send(daemon::Request::Save(snapshot.clone())).is_ok()
        {
            self.last_snapshot = Some(snapshot);
        }
    }

    fn add_pane(
        &mut self,
        ctx: &egui::Context,
        attach: Option<String>,
        cwd: Option<String>,
        target: Target,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.panes.insert(
            id,
            Pane::connect(
                &self.runtime,
                ctx,
                self.control.window,
                self.workspace_id,
                id,
                PaneSpec {
                    target,
                    session: attach,
                },
                cwd,
            ),
        );
        id
    }

    fn add_tab(&mut self, ctx: &egui::Context, attach: Option<String>) {
        let id = self.add_pane(
            ctx,
            attach,
            None,
            Target::default_local().expect("resolve local daemon socket"),
        );
        self.tabs.push(Tab {
            name: format!("Terminal {id}"),
            layout: Layout::Pane(id),
            focused: id,
            focus_history: vec![id],
        });
        self.active = self.tabs.len() - 1;
    }

    fn close_pane(&mut self, ctx: &egui::Context, id: u64) {
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.layout.panes().contains(&id))
        else {
            return;
        };
        if !self.tabs[index].remove_pane(id) {
            self.tabs.remove(index);
            if index < self.active {
                self.active -= 1;
            }
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
        }
        self.panes.remove(&id);
        if self.tabs.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn control_requests(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.control.requests.try_recv() {
            match message.request {
                Request::Switch { session } => self.begin_workspace(session, Some(message.reply)),
                Request::List => {
                    let active = self.tabs.get(self.active).map(|tab| tab.focused);
                    let mut output = format!(
                        "WINDOW {}\nPANE\tACTIVE\tDAEMON\tSESSION\n",
                        self.control.window
                    );
                    for (id, pane) in &self.panes {
                        let state = pane.session.state.lock().unwrap();
                        let session = state
                            .session_id
                            .map(|id| format!("{id:032x}"))
                            .unwrap_or_else(|| "connecting".into());
                        output.push_str(&format!(
                            "{id}\t{}\t{}\t{session}\n",
                            if Some(*id) == active { "*" } else { "" },
                            pane.session.target.label()
                        ));
                    }
                    let _ = message.reply.send(Ok(output.trim_end().into()));
                }
                Request::Attach {
                    pane,
                    daemon,
                    ssh,
                    session,
                } => {
                    let id = pane.or_else(|| self.tabs.get(self.active).map(|tab| tab.focused));
                    let Some((id, pane)) =
                        id.and_then(|id| self.panes.get(&id).map(|pane| (id, pane)))
                    else {
                        let _ = message.reply.send(Err("pane not found".into()));
                        continue;
                    };
                    if self.pending_workspace.is_some() {
                        let _ = message
                            .reply
                            .send(Err("session switch is in progress".into()));
                        continue;
                    }
                    if self.pending_attach.contains_key(&id) {
                        let _ = message
                            .reply
                            .send(Err("pane already has an attach in progress".into()));
                        continue;
                    }
                    if session.is_empty()
                        || ssh
                            .as_ref()
                            .is_some_and(|s| s.is_empty() || s.starts_with('-'))
                    {
                        let _ = message.reply.send(Err("invalid attach target".into()));
                        continue;
                    }
                    let target = match (daemon, ssh) {
                        (Some(path), None) => Target::Local(path.into()),
                        (socket, Some(host)) => Target::Ssh { host, socket },
                        (None, None) => pane.session.target.clone(),
                    };
                    let session = Arc::new(Session::start(
                        &self.runtime,
                        ctx.clone(),
                        target,
                        Some(session),
                        None,
                        vec![],
                    ));
                    self.pending_attach.insert(
                        id,
                        PendingAttach {
                            session,
                            reply: message.reply,
                        },
                    );
                }
            }
        }
        let ready: Vec<_> = self
            .pending_attach
            .iter()
            .filter_map(|(&id, pending)| {
                let state = pending.session.state.lock().unwrap();
                (state.connected
                    || state.error.is_some()
                    || state.exited
                    || state.detached
                    || !self.panes.contains_key(&id)
                    || pending.reply.is_closed())
                .then_some(id)
            })
            .collect();
        for id in ready {
            let pending = self.pending_attach.remove(&id).unwrap();
            let result = {
                let state = pending.session.state.lock().unwrap();
                if pending.reply.is_closed() {
                    Err("request cancelled".into())
                } else if !self.panes.contains_key(&id) {
                    Err("pane closed during attach".into())
                } else if state.connected {
                    Ok(format!("Attached pane {id}"))
                } else {
                    Err(state
                        .error
                        .clone()
                        .unwrap_or_else(|| "session ended during attach".into()))
                }
            };
            if result.is_ok() {
                let pane = self.panes.get_mut(&id).unwrap();
                pane.session = pending.session;
                pane.dims = (0, 0);
                pane.selection = None;
                pane.reported_focus = None;
                pane.scroll_remainder = 0.0;
                pane.mouse_capture = None;
            }
            let _ = pending.reply.send(result);
        }
    }

    fn action(&mut self, ctx: &egui::Context, action: Action) {
        if self.pending_workspace.is_some() {
            return;
        }
        match action {
            Action::NewSession => self.begin_workspace(None, None),
            Action::Sessions => {
                self.workspaces_open = true;
                self.workspace_roster = None;
                if let Some(client) = &self.workspace {
                    let _ = client.send(daemon::Request::List);
                }
            }
            Action::Palette => {
                self.palette = true;
                self.palette_query.clear();
            }
            Action::FontScale(scale) => self.font_size = (self.font_size * scale).clamp(6.0, 72.0),
            Action::FontReset => self.font_size = 15.0,
            Action::Paste => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            Action::Focus(direction) => {
                let tab = &mut self.tabs[self.active];
                if let Some(id) = tab.layout.neighbor(tab.focused, direction) {
                    tab.focus(id);
                }
            }
            Action::NewTab => self.add_tab(ctx, None),
            Action::Split(axis) => {
                let source = self.tabs[self.active].focused;
                let cwd = self.panes[&source]
                    .session
                    .state
                    .lock()
                    .unwrap()
                    .screen
                    .cwd()
                    .map(felis_client_core::pipe::local_cwd_from_osc7)
                    .filter(|path| !path.is_empty());
                let target = self.panes[&source].session.target.clone();
                let id = self.add_pane(ctx, None, cwd, target);
                let tab = &mut self.tabs[self.active];
                tab.layout.split(tab.focused, id, axis);
                tab.focus(id);
            }
            Action::ClosePane => self.close_pane(ctx, self.tabs[self.active].focused),
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
                tab.focus(ids[(pos + 1) % ids.len()]);
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
        self.workspace_events(ctx);
        self.poll_workspace();
        if self.workspace.is_some() && self.workspace_id.is_none() {
            egui::CentralPanel::default().show(ctx, |ui| {
                if let Some(error) = &self.workspace_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                    if ui.button("Retry connecting to nekomata daemon").clicked() {
                        self.workspace = Some(WorkspaceClient::start(
                            &self.runtime,
                            ctx.clone(),
                            self.control.window,
                        ));
                        self.workspace_error = None;
                    }
                } else {
                    ui.label("Connecting to nekomata session…");
                }
            });
            return;
        }
        self.control_requests(ctx);
        if let Some(error) = &self.workspace_error {
            egui::TopBottomPanel::bottom("workspace-error").show(ctx, |ui| {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            });
        }
        for pane in self.panes.values() {
            if let Some(text) = pane.session.state.lock().unwrap().clipboard.take() {
                ctx.copy_text(text);
            }
        }
        let closed: Vec<_> = self
            .panes
            .iter()
            .filter_map(|(&id, pane)| {
                let state = pane.session.state.lock().unwrap();
                (self.pending_workspace.is_none() && (state.detached || state.exited)).then_some(id)
            })
            .collect();
        for id in closed {
            self.close_pane(ctx, id);
        }
        if self.tabs.is_empty() {
            egui::CentralPanel::default().show(ctx, |_| {});
            if let Some(id) = self.picker.show(&self.runtime, ctx) {
                self.add_tab(ctx, Some(id));
            } else if !self.picker.visible {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        let mut action = None;
        // Consume application shortcuts before forwarding terminal events.
        if !self.picker.visible
            && !self.help
            && !self.palette
            && !self.workspaces_open
            && self.pending_workspace.is_none()
        {
            let actions = ctx.input_mut(|input| self.shortcuts.consume(input));
            for action in actions {
                self.action(ctx, action);
                if self.tabs.is_empty() {
                    return;
                }
            }
        }
        if !self.picker.visible
            && !self.help
            && !self.palette
            && !self.workspaces_open
            && self.pending_workspace.is_none()
            && !egui::Popup::is_any_open(ctx)
        {
            focus_terminal(ctx);
        }
        let mut menu_open = false;
        egui::TopBottomPanel::top("tabs")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(TAB_BAR_BG)
                    .inner_margin(egui::Margin::symmetric(8, 6)),
            )
            .show(ctx, |ui| {
                if self.pending_workspace.is_some() {
                    ui.disable();
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().button_padding = Vec2::new(15.0, 6.0);
                    ui.spacing_mut().interact_size.y = 30.0;
                    let (_, menu) = egui::containers::menu::MenuButton::from_button(
                        egui::Button::new(egui::RichText::new("≡").color(TAB_INACTIVE_FG))
                            .frame(false)
                            .min_size(Vec2::splat(30.0)),
                    )
                    .ui(ui, |ui| {
                        for (label, command) in [
                            ("New session", Action::NewSession),
                            ("Sessions…", Action::Sessions),
                            ("+ Tab", Action::NewTab),
                            ("Split ↔", Action::Split(Axis::Horizontal)),
                            ("Split ↕", Action::Split(Axis::Vertical)),
                            ("Next pane", Action::NextPane),
                        ] {
                            if ui.button(label).clicked() {
                                action = Some(command);
                                ui.close();
                            }
                        }
                        ui.separator();
                        for (label, command) in [
                            ("Close pane", Action::ClosePane),
                            ("Close tab", Action::CloseTab),
                        ] {
                            if ui
                                .button(label)
                                .on_hover_text("Detach; keep the shell running")
                                .clicked()
                            {
                                action = Some(command);
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button("Attach…").clicked() {
                            self.picker.open(&self.runtime, ctx);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Smaller text").clicked() {
                            self.font_size = (self.font_size / 1.1).max(6.0);
                            ui.close();
                        }
                        if ui.button("Larger text").clicked() {
                            self.font_size = (self.font_size * 1.1).min(72.0);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Help").clicked() {
                            self.help = true;
                            ui.close();
                        }
                    });
                    menu_open = menu.is_some();
                    egui::ScrollArea::horizontal()
                        .id_salt("tab-scroll")
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 3.0;
                                ui.spacing_mut().button_padding = Vec2::new(15.0, 6.0);
                                let widgets = &mut ui.visuals_mut().widgets;
                                widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                                widgets.inactive.bg_stroke = Stroke::NONE;
                                widgets.hovered.weak_bg_fill = TAB_HOVER_BG;
                                widgets.hovered.bg_stroke = Stroke::NONE;
                                widgets.active.weak_bg_fill = TAB_HOVER_BG;
                                widgets.active.bg_stroke = Stroke::NONE;
                                for state in [
                                    &mut widgets.inactive,
                                    &mut widgets.hovered,
                                    &mut widgets.active,
                                ] {
                                    state.corner_radius = egui::CornerRadius::same(5);
                                }
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
                                    let selected = self.active == i;
                                    let text = egui::RichText::new(title).color(if selected {
                                        FG
                                    } else {
                                        TAB_INACTIVE_FG
                                    });
                                    let mut button = egui::Button::new(text).stroke(Stroke::NONE);
                                    if selected {
                                        button = button.fill(TAB_ACTIVE_BG);
                                    }
                                    if ui.add(button).clicked() {
                                        self.active = i;
                                    }
                                }
                            });
                        });
                });
            });
        if let Some(action) = action {
            self.action(ctx, action);
        }
        if self.tabs.is_empty() {
            return;
        }
        let modal = self.picker.visible
            || self.help
            || self.palette
            || self.workspaces_open
            || self.pending_workspace.is_some()
            || menu_open;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ctx, |ui| {
                if self.pending_workspace.is_some() {
                    ui.disable();
                }
                ui.interact(
                    ui.max_rect(),
                    terminal_focus_id(),
                    Sense::focusable_noninteractive(),
                );
                let tab = &mut self.tabs[self.active];
                let previous = tab.focused;
                draw_layout(
                    ui,
                    &mut tab.layout,
                    &mut self.panes,
                    &mut tab.focused,
                    self.font_size,
                );
                if tab.focused != previous {
                    tab.focus(tab.focused);
                }
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
        } else {
            ctx.output_mut(|output| output.ime = None);
        }
        if let Some(id) = self.picker.show(&self.runtime, ctx) {
            self.add_tab(ctx, Some(id));
        }
        if self.help {
            egui::Window::new("About nekomata").open(&mut self.help).show(ctx, |ui| {
                ui.label("Alt+C: new tab · Alt+T / Alt+Shift+T: next / previous tab");
                ui.label("Alt+W, then Alt+S / Alt+V: split top/bottom / left/right");
                ui.label("Alt+W, then C: close pane · H/J/K/L: focus left/down/up/right");
                ui.label("Cmd+0 / = / -: reset / enlarge / reduce font · Ctrl+Shift+V: paste");
                ui.label("Alt+Shift+P: command palette");
                ui.label("Drag a divider to resize. Shift+drag selects when mouse reporting is active.");
                ui.label("Initial text frontend: Kitty images, OSC 66 sizing and ligatures are not rendered.");
            });
        }
        if self.palette {
            let mut chosen = None;
            let enter =
                ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            egui::Window::new("Command palette")
                .open(&mut self.palette)
                .show(ctx, |ui| {
                    let response = ui.text_edit_singleline(&mut self.palette_query);
                    if !response.has_focus() {
                        response.request_focus();
                    }
                    for (label, command) in [
                        ("New session", Action::NewSession),
                        ("Switch session", Action::Sessions),
                        ("New tab", Action::NewTab),
                        ("Split left / right", Action::Split(Axis::Horizontal)),
                        ("Split top / bottom", Action::Split(Axis::Vertical)),
                        ("Next tab", Action::NextTab),
                        ("Previous tab", Action::PreviousTab),
                        ("Close pane (detach)", Action::ClosePane),
                        ("Close tab (detach)", Action::CloseTab),
                        ("Reset font size", Action::FontReset),
                    ] {
                        if label
                            .to_lowercase()
                            .contains(&self.palette_query.to_lowercase())
                        {
                            let clicked = ui.button(label).clicked();
                            if clicked || (enter && chosen.is_none()) {
                                chosen = Some(command);
                            }
                        }
                    }
                });
            if let Some(action) = chosen {
                self.palette = false;
                self.action(ctx, action);
            }
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.palette = false;
            }
        }
        self.workspace_picker(ctx);
        if self.pending_workspace.is_some() {
            egui::Window::new("Switching session")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.spinner();
                    ui.label("Connecting to destination panes…");
                });
        }
        self.save_workspace();
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(client) = &self.workspace {
            if client.is_closed() {
                return;
            }
            let snapshot = self
                .workspace_id
                .filter(|_| self.pending_workspace.is_none())
                .map(|_| self.snapshot());
            if let Err(error) = client.close(&self.runtime, snapshot) {
                eprintln!("Could not save nekomata session: {error:#}");
            }
        }
    }
}

fn terminal_focus_id() -> egui::Id {
    egui::Id::new("terminal-keyboard-focus")
}

fn focus_terminal(ctx: &egui::Context) {
    ctx.memory_mut(|memory| {
        let id = terminal_focus_id();
        memory.request_focus(id);
        memory.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            },
        );
    });
}

fn draw_layout(
    ui: &mut egui::Ui,
    layout: &mut Layout,
    panes: &mut BTreeMap<u64, Pane>,
    focused: &mut u64,
    font_size: f32,
) -> Option<Rect> {
    let rect = ui.max_rect();
    match layout {
        Layout::Pane(id) => {
            draw_pane(ui, *id, panes.get_mut(id).unwrap(), focused, font_size);
            (*focused == *id).then_some(rect)
        }
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
            let split = (extent - 1.0).max(0.0) * *ratio;
            let (a, divider, b) = if horizontal {
                let x = rect.left() + split;
                (
                    Rect::from_min_max(rect.min, Pos2::new(x, rect.bottom())),
                    Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(x + 1.0, rect.bottom())),
                    Rect::from_min_max(Pos2::new(x + 1.0, rect.top()), rect.max),
                )
            } else {
                let y = rect.top() + split;
                (
                    Rect::from_min_max(rect.min, Pos2::new(rect.right(), y)),
                    Rect::from_min_max(Pos2::new(rect.left(), y), Pos2::new(rect.right(), y + 1.0)),
                    Rect::from_min_max(Pos2::new(rect.left(), y + 1.0), rect.max),
                )
            };
            let response = ui.interact(divider.expand(2.0), ui.id().with("divider"), Sense::drag());
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
            let first_focus = ui
                .scope_builder(egui::UiBuilder::new().id_salt("first").max_rect(a), |ui| {
                    draw_layout(ui, first, panes, focused, font_size)
                })
                .inner;
            let second_focus = ui
                .scope_builder(egui::UiBuilder::new().id_salt("second").max_rect(b), |ui| {
                    draw_layout(ui, second, panes, focused, font_size)
                })
                .inner;
            let painter = ui.painter();
            painter.rect_filled(divider, 0, Color32::from_gray(45));
            let focus_rect = first_focus.or(second_focus);
            if let Some(focus_rect) = focus_rect {
                let touches = if horizontal {
                    focus_rect.right() == a.right() || focus_rect.left() == b.left()
                } else {
                    focus_rect.bottom() == a.bottom() || focus_rect.top() == b.top()
                };
                if touches {
                    let mut highlight = divider;
                    if horizontal {
                        highlight.min.y = focus_rect.top();
                        highlight.max.y = focus_rect.bottom();
                    } else {
                        highlight.min.x = focus_rect.left();
                        highlight.max.x = focus_rect.right();
                    }
                    painter.rect_filled(highlight, 0, ACCENT.gamma_multiply(0.7));
                }
            }
            focus_rect
        }
    }
}

fn draw_pane(ui: &mut egui::Ui, id: u64, pane: &mut Pane, focused: &mut u64, font_size: f32) {
    let rect = ui.max_rect().shrink(2.0);
    let response = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    if response.clicked() || response.drag_started() {
        *focused = id;
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0, BG);
    let text_rect = rect;
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
    let visible_rows = screen.rows().min(rows);
    let visible_cols = screen.cols().min(cols);
    for row in 0..visible_rows {
        for col in 0..visible_cols {
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
            // A terminal has whole cells, but the pane can have a fractional
            // final row/column. Continue the edge backgrounds into that space.
            let max = Pos2::new(
                if col + 1 == visible_cols {
                    text_rect.right()
                } else {
                    pos.x + cw
                },
                if row + 1 == visible_rows {
                    text_rect.bottom()
                } else {
                    pos.y + ch
                },
            );
            painter.rect_filled(Rect::from_min_max(pos, max), 0, bg);
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
            let pos = text_rect.min + Vec2::new(f32::from(col) * cw, f32::from(row) * ch);
            let painted = matches!(cell.grapheme, Grapheme::Char(c) if crate::glyph::paint_braille(
                &painter, c, Rect::from_min_size(pos, Vec2::new(cw, ch)), fg,
            ));
            if !painted {
                let text = match cell.grapheme {
                    Grapheme::Ascii(b) => char::from(b).to_string(),
                    Grapheme::Char(c) => c.to_string(),
                    Grapheme::Cluster(id) => shadow.cluster_str(id).unwrap_or("�").to_owned(),
                    _ => continue,
                };
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
    if *focused == id {
        let pos = text_rect.min + Vec2::new(f32::from(cursor.col) * cw, f32::from(cursor.row) * ch);
        ui.output_mut(|output| {
            output.ime = Some(egui::output::IMEOutput {
                rect: text_rect,
                cursor_rect: Rect::from_min_size(pos, Vec2::new(cw, ch)),
            })
        });
    }
    if *focused == id && cursor.visible && shadow.viewport() == 0 && state.connected {
        let pos = text_rect.min + Vec2::new(f32::from(cursor.col) * cw, f32::from(cursor.row) * ch);
        match screen.cursor_style() {
            CursorStyle::Block => {
                painter.rect_filled(
                    Rect::from_min_size(pos, Vec2::new(cw, ch)),
                    0,
                    ACCENT.gamma_multiply(0.5),
                );
            }
            CursorStyle::Bar => {
                painter.rect_filled(Rect::from_min_size(pos, Vec2::new(2.0, ch)), 0, ACCENT);
            }
            CursorStyle::Underline => {
                painter.rect_filled(
                    Rect::from_min_size(pos + Vec2::new(0.0, ch - 2.0), Vec2::new(cw, 2.0)),
                    0,
                    ACCENT,
                );
            }
        }
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
    let report_mouse = mouse_mode && !ui.input(|i| i.modifiers.shift);
    if report_mouse && (response.hovered() || response.dragged() || pane.mouse_capture.is_some()) {
        use felis_protocol::messages::{MouseAction, MouseButton};
        for event in ui.input(|i| i.events.clone()) {
            let (pos, button, action) = match event {
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let button = match button {
                        egui::PointerButton::Primary => MouseButton::Left,
                        egui::PointerButton::Secondary => MouseButton::Right,
                        egui::PointerButton::Middle => MouseButton::Middle,
                        _ => continue,
                    };
                    pane.mouse_capture = if pressed { Some(button) } else { None };
                    (
                        pos,
                        Some(button),
                        if pressed {
                            MouseAction::Press
                        } else {
                            MouseAction::Release
                        },
                    )
                }
                egui::Event::PointerMoved(pos) => {
                    let button = ui.input(|i| {
                        if i.pointer.primary_down() {
                            Some(MouseButton::Left)
                        } else if i.pointer.secondary_down() {
                            Some(MouseButton::Right)
                        } else if i.pointer.middle_down() {
                            Some(MouseButton::Middle)
                        } else {
                            None
                        }
                    });
                    (
                        pos,
                        button,
                        if button.is_some() {
                            MouseAction::Drag
                        } else {
                            MouseAction::Motion
                        },
                    )
                }
                _ => continue,
            };
            pane.session.send(mouse_input(
                ui, text_rect, pos, cw, ch, pane.dims, button, action,
            ));
        }
    }
    if response.hovered() {
        let delta = ui.input(|i| i.smooth_scroll_delta.y);
        pane.scroll_remainder += delta / ch;
        let lines = pane.scroll_remainder.trunc() as i32;
        pane.scroll_remainder -= lines as f32;
        if lines != 0 {
            pane.selection = None;
            if report_mouse {
                use felis_protocol::messages::{MouseAction, MouseButton};
                let pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(text_rect.min);
                let button = if lines > 0 {
                    MouseButton::WheelUp
                } else {
                    MouseButton::WheelDown
                };
                for _ in 0..lines.unsigned_abs().min(100) {
                    pane.session.send(mouse_input(
                        ui,
                        text_rect,
                        pos,
                        cw,
                        ch,
                        pane.dims,
                        Some(button),
                        MouseAction::Press,
                    ));
                }
            } else {
                let offset = (i64::from(shadow.viewport()) + i64::from(lines)).max(0) as u32;
                pane.session.send(InputMsg::Viewport {
                    lines_from_bottom: offset,
                });
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn mouse_input(
    ui: &egui::Ui,
    rect: Rect,
    pos: Pos2,
    cw: f32,
    ch: f32,
    dims: (u16, u16),
    button: Option<felis_protocol::messages::MouseButton>,
    action: felis_protocol::messages::MouseAction,
) -> InputMsg {
    use felis_protocol::messages::{InputMods, MouseEvent};
    let modifiers = ui.input(|i| i.modifiers);
    let mut mods = InputMods::empty();
    mods.set(InputMods::SHIFT, modifiers.shift);
    mods.set(InputMods::ALT, modifiers.alt);
    mods.set(InputMods::CTRL, modifiers.ctrl);
    InputMsg::Mouse(MouseEvent {
        button,
        action,
        mods,
        x: ((pos.x - rect.left()) / cw)
            .floor()
            .clamp(0.0, f32::from(dims.1 - 1)) as u16
            + 1,
        y: ((pos.y - rect.top()) / ch)
            .floor()
            .clamp(0.0, f32::from(dims.0 - 1)) as u16
            + 1,
        px: (((pos.x - rect.left()).max(0.0) * ui.ctx().pixels_per_point()) as u16)
            .saturating_add(1),
        py: (((pos.y - rect.top()).max(0.0) * ui.ctx().pixels_per_point()) as u16)
            .saturating_add(1),
    })
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
    use felis_protocol::messages::{Key, KeyEvent, KeyEventKind, KeyLocation, KeyMods};
    let events = ctx.input(|i| i.events.clone());
    let has_text = events
        .iter()
        .any(|event| matches!(event, egui::Event::Text(_)));
    let mods = ctx.input(|i| i.modifiers);
    for event in events {
        let event_mods = match &event {
            egui::Event::Key { modifiers, .. } => *modifiers,
            _ => mods,
        };
        let mut key_mods = KeyMods::empty();
        key_mods.set(KeyMods::CTRL, event_mods.ctrl);
        key_mods.set(KeyMods::SHIFT, event_mods.shift);
        key_mods.set(KeyMods::ALT, event_mods.alt);
        key_mods.set(KeyMods::SUPER, event_mods.mac_cmd);
        let kind = match &event {
            egui::Event::Key { repeat: true, .. } => KeyEventKind::Repeat,
            _ => KeyEventKind::Press,
        };

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
            } => physical_key(key, event_mods, has_text).map(|key| (key, None)),
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
                kind,
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

fn physical_key(
    key: egui::Key,
    mods: egui::Modifiers,
    has_text: bool,
) -> Option<felis_protocol::messages::Key> {
    use felis_protocol::messages::{FKey, Key, NamedKey};
    let named = match key {
        egui::Key::Space if mods.ctrl || (mods.alt && !has_text) => Some(NamedKey::Space),
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
        _ => key
            .name()
            .strip_prefix('F')
            .and_then(|n| n.parse().ok())
            .and_then(FKey::new)
            .map(NamedKey::F),
    };
    if let Some(named) = named {
        Some(Key::Named(named))
    } else if mods.ctrl || (mods.alt && !has_text) {
        Some(Key::Character(key_character(key)))
    } else {
        None
    }
}

fn key_character(key: egui::Key) -> String {
    key.symbol_or_name().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires isolated nekomata and felis daemons; run scripts/test-live.sh via nix develop"]
    fn live_gui_restart_and_switch_preserve_workspaces_and_shells() {
        use std::{sync::mpsc, time::Instant};
        assert!(
            std::env::var_os("NEKOMATA_RUNTIME_DIR").is_some(),
            "use scripts/test-live.sh"
        );
        let ctx = egui::Context::default();
        let (tx, repaints) = mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let pump = |app: &mut App, done: &dyn Fn(&App) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(25);
            loop {
                let _ = ctx.run(egui::RawInput::default(), |_| {});
                app.workspace_events(&ctx);
                app.poll_workspace();
                app.control_requests(&ctx);
                app.save_workspace();
                if done(app) {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "GUI session test timed out: {:?}",
                    app.workspace_error
                );
                let _ = repaints.recv_timeout(Duration::from_millis(50));
            }
        };
        let connected = |app: &App| {
            !app.panes.is_empty()
                && app
                    .panes
                    .values()
                    .all(|p| p.session.state.lock().unwrap().connected)
        };
        let mut app = App::connect(tokio::runtime::Runtime::new().unwrap(), &ctx).unwrap();
        pump(&mut app, &connected);
        app.action(&ctx, Action::Split(Axis::Horizontal));
        app.action(&ctx, Action::NewTab);
        pump(&mut app, &connected);
        app.active = 0;
        app.tabs[0].focus(2);
        if let Layout::Split { ratio, .. } = &mut app.tabs[0].layout {
            *ratio = 0.3;
        }
        app.font_size = 18.0;
        let before = app.snapshot();
        let session_id = app.workspace_id;
        app.save_workspace();
        eframe::App::on_exit(&mut app, None);
        drop(app);

        let mut replacement = App::connect(tokio::runtime::Runtime::new().unwrap(), &ctx).unwrap();
        pump(&mut replacement, &connected);
        assert_eq!(replacement.workspace_id, session_id);
        assert_eq!(replacement.snapshot(), before);
        assert_eq!(replacement.tabs.len(), 2);
        assert_eq!(replacement.panes.len(), 3);

        let executable = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("nekomata");
        let id = session_id.unwrap();
        let repaint = ctx.clone();
        let routing_executable = executable.clone();
        let command = replacement.runtime.spawn(async move {
            let result = tokio::process::Command::new(routing_executable)
                .args(["panes", "list"])
                .env("NEKOMATA_SESSION_ID", id.to_string())
                .env("NEKOMATA_WINDOW_ID", u32::MAX.to_string())
                .output()
                .await
                .unwrap();
            repaint.request_repaint();
            result
        });
        pump(&mut replacement, &|_| command.is_finished());
        let output = replacement.runtime.block_on(command).unwrap();
        assert!(
            output.status.success(),
            "restored shell CLI routing failed: {:?}",
            output
        );
        let command = |app: &mut App, args: Vec<String>| {
            let executable = executable.clone();
            let window = app.control.window;
            let repaint = ctx.clone();
            let task = app.runtime.spawn(async move {
                let output = tokio::process::Command::new(executable)
                    .args(args)
                    .args(["--window", &window.to_string()])
                    .kill_on_drop(true)
                    .output()
                    .await
                    .unwrap();
                repaint.request_repaint();
                output
            });
            pump(app, &|_| task.is_finished());
            app.runtime.block_on(task).unwrap()
        };
        let output = command(&mut replacement, vec!["sessions".into(), "new".into()]);
        assert!(output.status.success(), "{output:?}");
        let other = replacement.workspace_id.unwrap();
        assert_ne!(Some(other), session_id);
        assert_eq!(replacement.panes.len(), 1);
        let other_shell = replacement
            .panes
            .values()
            .next()
            .unwrap()
            .session
            .state
            .lock()
            .unwrap()
            .session_id
            .unwrap();
        let output = command(
            &mut replacement,
            vec![
                "sessions".into(),
                "attach".into(),
                session_id.unwrap().to_string(),
            ],
        );
        assert!(output.status.success(), "{output:?}");
        assert_eq!(replacement.snapshot(), before);

        // The destination shares two live shells with the source, but its other
        // pane is unavailable. Preparing it must not steal those source connections.
        let mut broken = before.clone();
        broken.panes.get_mut(&2).unwrap().target = Target::Local(
            std::path::PathBuf::from(std::env::var_os("NEKOMATA_RUNTIME_DIR").unwrap())
                .join("missing-felis.sock"),
        );
        replacement.runtime.block_on(async {
            let mut client = daemon::connect().await.unwrap();
            let Payload::Session { id, .. } =
                daemon::exchange(&mut client, daemon::Request::Open { window: u32::MAX })
                    .await
                    .unwrap()
            else {
                panic!("expected session")
            };
            assert_eq!(id, other);
            daemon::exchange(&mut client, daemon::Request::Save(broken))
                .await
                .unwrap();
            daemon::exchange(&mut client, daemon::Request::Detach)
                .await
                .unwrap();
        });
        let original_connection = replacement.panes[&1].session.clone();
        let output = command(
            &mut replacement,
            vec!["sessions".into(), "attach".into(), other.to_string()],
        );
        assert!(!output.status.success(), "{output:?}");
        assert_eq!(replacement.workspace_id, session_id);
        assert_eq!(replacement.snapshot(), before);
        assert!(connected(&replacement));
        assert!(Arc::ptr_eq(
            &replacement.panes[&1].session,
            &original_connection
        ));
        drop(original_connection);
        assert!(replacement.pending_workspace.is_none());
        replacement.runtime.block_on(async {
            let mut client = daemon::connect().await.unwrap();
            let Payload::Sessions(sessions) = daemon::exchange(&mut client, daemon::Request::List)
                .await
                .unwrap()
            else {
                panic!("expected sessions")
            };
            assert!(
                sessions
                    .iter()
                    .find(|session| session.id == other)
                    .unwrap()
                    .window
                    .is_none()
            );
        });

        // A fresh switch directly after cancellation must not consume the old
        // cancellation acknowledgement as its own commit.
        let output = command(&mut replacement, vec!["sessions".into(), "new".into()]);
        assert!(output.status.success(), "{output:?}");
        let third_shell = replacement
            .panes
            .values()
            .next()
            .unwrap()
            .session
            .state
            .lock()
            .unwrap()
            .session_id
            .unwrap();
        let output = command(
            &mut replacement,
            vec![
                "sessions".into(),
                "attach".into(),
                session_id.unwrap().to_string(),
            ],
        );
        assert!(output.status.success(), "{output:?}");
        assert_eq!(replacement.snapshot(), before);

        let ids: Vec<_> = replacement
            .panes
            .values()
            .map(|p| p.session.state.lock().unwrap().session_id.unwrap())
            .chain([other_shell, third_shell])
            .collect();
        eframe::App::on_exit(&mut replacement, None);
        drop(replacement);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut ops = felis_client_core::connect(
                std::path::PathBuf::from(std::env::var_os("NEKOMATA_TEST_SOCKET").unwrap()),
                felis_client_core::Offer::ops(),
            )
            .await
            .unwrap();
            for id in ids {
                ops.destroy_session(format!("{id:032x}")).await.unwrap();
            }
        });
    }

    #[test]
    #[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
    fn live_pane_attach_routes_cli_and_preserves_sessions_on_failure() {
        use felis_client_core::Offer;
        use felis_protocol::messages::SpawnArgs;
        use std::{sync::mpsc, time::Instant};

        let socket = std::env::var("NEKOMATA_TEST_SOCKET").expect("use scripts/test-live.sh");
        assert_eq!(std::env::var("FELIS_SOCKET").unwrap(), socket);
        let other_socket = std::path::Path::new(&socket).with_file_name("other.sock");
        struct StopDaemon(std::path::PathBuf);
        impl Drop for StopDaemon {
            fn drop(&mut self) {
                let _ = std::process::Command::new("felis")
                    .args([
                        "--socket",
                        self.0.to_str().unwrap(),
                        "daemon",
                        "stop",
                        "--force",
                    ])
                    .output();
            }
        }
        let _cleanup = StopDaemon(other_socket.clone());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let (tx, repaints) = mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let control = control::Server::bind(&runtime, ctx.clone()).unwrap();
        let window = control.window;
        let mut app = App {
            workspace: None,
            workspace_id: None,
            workspace_error: None,
            last_snapshot: None,
            pending_workspace: None,
            workspaces_open: false,
            workspace_roster: None,
            runtime,
            control,
            pending_attach: BTreeMap::new(),
            panes: BTreeMap::new(),
            tabs: vec![],
            active: 0,
            next_id: 1,
            font_size: 15.0,
            picker: SessionPicker::default(),
            help: false,
            shortcuts: Shortcuts::default(),
            palette: false,
            palette_query: String::new(),
        };
        app.add_tab(&ctx, None);
        let pump = |app: &mut App, done: &dyn Fn(&App) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(25);
            loop {
                let _ = ctx.run(egui::RawInput::default(), |_| {});
                app.control_requests(&ctx);
                if done(app) {
                    break;
                }
                repaints
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .expect("control test timed out");
            }
        };
        pump(&mut app, &|app| {
            app.panes[&1].session.state.lock().unwrap().connected
        });
        let original = app.panes[&1]
            .session
            .state
            .lock()
            .unwrap()
            .session_id
            .unwrap();
        let remote = app.runtime.block_on(async {
            let mut conn =
                felis_client_core::connect_or_spawn_daemon(&other_socket, Offer::window(false))
                    .await
                    .unwrap();
            conn.create_with(SpawnArgs::default()).await.unwrap().id
        });
        let executable = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("nekomata");
        let command = |app: &mut App, args: Vec<String>| {
            let executable = executable.clone();
            let context = ctx.clone();
            let task = app.runtime.spawn(async move {
                let output = tokio::process::Command::new(executable)
                    .args(args)
                    .args(["--window", &window.to_string()])
                    .kill_on_drop(true)
                    .output()
                    .await
                    .unwrap();
                context.request_repaint();
                output
            });
            pump(app, &|_| task.is_finished());
            app.runtime.block_on(task).unwrap()
        };
        let output = command(&mut app, vec!["panes".into(), "list".into()]);
        assert!(output.status.success(), "{:?}", output);
        assert!(String::from_utf8_lossy(&output.stdout).contains(&format!("{original:032x}")));
        let attach = |daemon: Option<&str>, session: String, pane: Option<&str>| {
            let mut args = vec!["panes".into(), "attach".into(), "--session".into(), session];
            if let Some(id) = pane {
                args.push(id.into());
            }
            if let Some(socket) = daemon {
                args.extend(["--daemon".into(), socket.into()]);
            }
            args
        };
        let missing = other_socket.with_file_name("missing.sock");
        let output = command(
            &mut app,
            attach(
                Some(missing.to_str().unwrap()),
                format!("{remote:032x}"),
                None,
            ),
        );
        assert!(!output.status.success());
        assert_eq!(
            app.panes[&1].session.state.lock().unwrap().session_id,
            Some(original)
        );
        let output = command(
            &mut app,
            attach(
                Some(other_socket.to_str().unwrap()),
                format!("{remote:032x}"),
                Some("999"),
            ),
        );
        assert!(!output.status.success());
        let output = command(
            &mut app,
            attach(
                Some(other_socket.to_str().unwrap()),
                format!("{remote:032x}"),
                None,
            ),
        );
        assert!(output.status.success(), "{:?}", output);
        assert_eq!(
            app.panes[&1].session.state.lock().unwrap().session_id,
            Some(remote)
        );
        assert_eq!(
            app.panes[&1].session.target,
            Target::Local(other_socket.clone())
        );
        // Omitted daemon follows the target pane, even after changing daemons.
        let output = command(&mut app, attach(None, format!("{remote:032x}"), Some("1")));
        assert!(output.status.success(), "{:?}", output);
        let output = command(&mut app, attach(None, "not-a-session".into(), None));
        assert!(!output.status.success());
        assert_eq!(
            app.panes[&1].session.state.lock().unwrap().session_id,
            Some(remote)
        );
        if std::env::var("NEKOMATA_TEST_SSH").as_deref() == Ok("1") {
            // A relative path containing spaces, an apostrophe and shell syntax
            // must remain remote-relative throughout CLI, IPC and SSH parsing.
            let directory = other_socket.parent().unwrap();
            let name = "remote 'socket' $(printf ignored).sock";
            let alias = directory.join(name);
            std::os::unix::fs::symlink(&other_socket, &alias).unwrap();
            let remote_socket = name.to_owned();
            let mut args = attach(Some(&remote_socket), format!("{remote:032x}"), Some("1"));
            args.extend(["--ssh".into(), "nekomata-test-host".into()]);
            let output = command(&mut app, args);
            assert!(output.status.success(), "{:?}", output);
            assert_eq!(
                app.panes[&1].session.target,
                Target::Ssh {
                    host: "nekomata-test-host".into(),
                    socket: Some(remote_socket),
                }
            );
            assert_eq!(
                app.panes[&1].session.state.lock().unwrap().session_id,
                Some(remote)
            );
            let mut args = attach(
                Some("missing remote.sock"),
                format!("{remote:032x}"),
                Some("1"),
            );
            args.extend(["--ssh".into(), "nekomata-test-host".into()]);
            let output = command(&mut app, args);
            assert!(!output.status.success());
            assert_eq!(
                app.panes[&1].session.state.lock().unwrap().session_id,
                Some(remote)
            );
            // Omitting both flags must retain the remote socket, too.
            let output = command(&mut app, attach(None, format!("{remote:032x}"), Some("1")));
            assert!(output.status.success(), "{:?}", output);
        }
        let target = app.panes[&1].session.target.clone();
        app.action(&ctx, Action::Split(Axis::Horizontal));
        assert_eq!(app.panes[&2].session.target, target);
        pump(&mut app, &|app| {
            let state = app.panes[&2].session.state.lock().unwrap();
            state.connected || state.error.is_some()
        });
        assert!(app.panes[&2].session.state.lock().unwrap().connected);
        app.runtime.block_on(async {
            let mut ops = felis_client_core::connect(std::path::Path::new(&socket), Offer::ops())
                .await
                .unwrap();
            assert!(
                ops.list_sessions()
                    .await
                    .unwrap()
                    .iter()
                    .any(|session| session.id == original),
                "switching panes killed the old shell"
            );
            ops.destroy_session(format!("{original:032x}"))
                .await
                .unwrap();
        });
    }

    #[test]
    fn terminal_space_does_not_activate_a_previously_focused_menu() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.menu_button("≡", |_| {}).response.request_focus();
            });
        });
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    egui::Event::Key {
                        key: egui::Key::Space,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Text(" ".into()),
                ],
                ..Default::default()
            },
            |ctx| {
                focus_terminal(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(ui.menu_button("≡", |_| {}).inner.is_none());
                    ui.interact(
                        ui.max_rect(),
                        terminal_focus_id(),
                        Sense::focusable_noninteractive(),
                    );
                });
                assert!(ctx.input(|input| {
                    input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Text(text) if text == " "))
                }));
            },
        );
    }

    fn split_tab() -> Tab {
        let mut layout = Layout::Pane(1);
        layout.split(1, 2, Axis::Horizontal);
        layout.split(2, 3, Axis::Vertical);
        Tab {
            name: "test".into(),
            layout,
            focused: 1,
            focus_history: vec![1],
        }
    }

    #[test]
    fn closing_focused_panes_retraces_tab_focus_history() {
        let mut tab = split_tab();
        tab.focus(3);
        tab.focus(2);
        tab.focus(3);
        assert!(tab.remove_pane(3));
        assert_eq!(tab.focused, 2);
        assert!(tab.remove_pane(2));
        assert_eq!(tab.focused, 1);
        assert!(!tab.remove_pane(1));
    }

    #[test]
    fn closing_background_panes_prunes_only_their_tabs_history() {
        let mut tab = split_tab();
        let mut other = split_tab();
        tab.focus(2);
        tab.focus(3);
        other.focus(2);
        assert!(tab.remove_pane(2));
        assert_eq!(tab.focused, 3);
        assert!(tab.remove_pane(3));
        assert_eq!(tab.focused, 1);
        assert_eq!(other.focused, 2);
        assert_eq!(other.focus_history, vec![1, 2]);
    }

    #[test]
    fn linux_control_c_interrupts_instead_of_copying() {
        assert_eq!(
            clipboard_control(&egui::Event::Copy, egui::Modifiers::CTRL, false),
            Some(InputMsg::KeyBytes(vec![3]))
        );
    }

    #[test]
    fn control_bracket_keeps_the_character_needed_for_escape() {
        assert_eq!(key_character(egui::Key::OpenBracket), "[");
    }
    #[test]
    fn composed_alt_text_does_not_emit_a_second_physical_character() {
        assert_eq!(physical_key(egui::Key::E, egui::Modifiers::ALT, true), None);
    }

    #[test]
    fn function_keys_reach_terminal_programs_without_text_events() {
        use felis_protocol::messages::{FKey, Key, NamedKey};
        assert_eq!(
            physical_key(egui::Key::F1, egui::Modifiers::NONE, false),
            Some(Key::Named(NamedKey::F(FKey::lit(1))))
        );
    }
}
