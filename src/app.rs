use std::{collections::BTreeMap, sync::Arc};

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use felis_client_core::{GridPos, Selection};
#[cfg(test)]
use felis_grid::Grapheme;
use felis_protocol::messages::{InputMsg, RequestedDims};

use crate::{
    control::{self, Reply, Request},
    daemon::{self, Payload},
    layout::{Axis, Layout},
    picker::SessionPicker,
    session::{Session, Target},
    shortcuts::{Action, Shortcuts},
    theme::{ACCENT, BG, FG, TAB_BAR_BG, TAB_HOVER_BG, TAB_INACTIVE_FG},
    workspace::{PaneSpec, Snapshot, WindowLayout},
    workspace_client::{Client as WorkspaceClient, Command as WorkspaceCommand, Operation},
    workspace_switch::{self, PaneStatus, Poll, Step, Switch as WorkspaceSwitch},
};

struct Pane {
    session: Arc<Session>,
    /// The size last sent to felis; `None` until one is, or after reconnecting.
    requested: Option<RequestedDims>,
    selection: Option<Selection>,
    /// The session's `selection_epoch` that `selection` was made against.
    selection_epoch: u64,
    reported_focus: Option<bool>,
    scroll_remainder: f32,
    mouse_capture: Option<felis_protocol::messages::MouseButton>,
    link_capture: crate::links::LinkGesture,
    renderer: Option<crate::render::Renderer>,
    connection_generation: u64,
    links: crate::links::LinkOpener,
    search_open: bool,
    search_focus: bool,
    search_query: String,
    search_regex: bool,
    search_ignore_case: bool,
    search_submitted: Option<(String, bool, bool)>,
    search_paint_key: Option<(
        u64,
        u32,
        Option<felis_protocol::messages::ViewportAnchor>,
        u16,
    )>,
    pending_paste: Option<crate::paste::PendingPaste>,
}

/// Drops a selection made before the screen switched, resized or the view
/// moved. Drawing and `Action::Copy` hold `state` until they have used or
/// replaced the selection, so the session thread cannot advance the epoch in
/// between; `Pane::expire_selection` releases it before `terminal_input`
/// copies, which leaves that path a window of one lock hand-off.
fn expire_selection(
    selection: &mut Option<Selection>,
    seen_epoch: &mut u64,
    state: &crate::session::State,
) {
    if *seen_epoch != state.selection_epoch {
        *seen_epoch = state.selection_epoch;
        *selection = None;
    }
}

impl Pane {
    fn expire_selection(&mut self) {
        let state = self.session.state.lock().unwrap();
        expire_selection(&mut self.selection, &mut self.selection_epoch, &state);
    }

    fn from_session(session: Arc<Session>) -> Self {
        Self {
            session,
            requested: None,
            selection: None,
            selection_epoch: 0,
            reported_focus: None,
            scroll_remainder: 0.0,
            mouse_capture: None,
            link_capture: Default::default(),
            renderer: None,
            connection_generation: 0,
            links: Default::default(),
            search_open: false,
            search_focus: false,
            search_query: String::new(),
            search_regex: false,
            search_ignore_case: true,
            search_submitted: None,
            search_paint_key: None,
            pending_paste: None,
        }
    }

    fn spec(&self) -> PaneSpec {
        let state = self.session.state.lock().unwrap();
        PaneSpec {
            target: self.session.target.clone(),
            // An attach by prefix names no session until felis resolves it;
            // recording the prefix would make the daemon reject every
            // snapshot while it is pending or after it fails.
            session: state.session_id.or_else(|| {
                self.session
                    .attachment
                    .as_deref()
                    .and_then(|id| id.parse().ok())
            }),
        }
    }

    fn close_search(&mut self) {
        self.search_open = false;
        self.session.search.cancel_search();
    }

    fn navigate_search(&mut self, older: bool) {
        if self.search_query.is_empty() {
            self.session.search.cancel_search();
            self.search_submitted = None;
            return;
        }
        let mut search = self.session.search.search.lock().unwrap();
        let unchanged =
            self.search_submitted
                .as_ref()
                .is_some_and(|(query, regex, ignore_case)| {
                    query == &self.search_query
                        && *regex == self.search_regex
                        && *ignore_case == self.search_ignore_case
                });
        if unchanged
            && search.active
            && search.error.is_none()
            && (search.running || !search.hits.is_empty())
        {
            if !search.running {
                search.advance(older);
            }
        } else {
            drop(search);
            self.session.search.search(
                self.search_query.clone(),
                felis_protocol::messages::SearchOptions {
                    regex: self.search_regex,
                    case_insensitive: self.search_ignore_case,
                },
            );
            self.search_submitted = Some((
                self.search_query.clone(),
                self.search_regex,
                self.search_ignore_case,
            ));
        }
        self.search_focus = true;
    }

    #[allow(clippy::too_many_arguments)]
    fn connect(
        runtime: &tokio::runtime::Runtime,
        ctx: &egui::Context,
        window: u32,
        workspace: Option<(u128, Option<&str>)>,
        id: u64,
        target: Target,
        attach: Option<String>,
        cwd: Option<String>,
    ) -> Self {
        let mut env = vec![
            ("NEKOMATA_WINDOW_ID".into(), window.to_string()),
            ("NEKOMATA_PANE_ID".into(), id.to_string()),
        ];
        if let Some((id, instance)) = workspace {
            env.push((control::WORKSPACE_ENV.into(), format!("{id:032x}")));
            if let Some(instance) = instance {
                env.push((control::DAEMON_INSTANCE_ENV.into(), instance.into()));
            }
        }
        Self::from_session(Arc::new(Session::start(
            runtime,
            ctx.clone(),
            target,
            attach,
            cwd,
            env,
        )))
    }
}

struct PendingAttach {
    session: Arc<Session>,
    reply: tokio::sync::oneshot::Sender<Reply>,
}

/// A window the workspace picker made taller, so that it can be restored.
struct GrownWindow {
    original: egui::Vec2,
    requested: egui::Vec2,
}

pub struct App {
    workspace: Option<WorkspaceClient>,
    workspace_id: Option<u128>,
    /// The daemon instance that issued `workspace_id`, when it reports one.
    daemon_instance: Option<String>,
    workspace_error: Option<String>,
    last_snapshot: Option<Snapshot>,
    pending_workspace: Option<WorkspaceSwitch<Pane>>,
    reconnect: crate::workspace_client::Reconnect,
    paste_requested: Option<(u64, u64, Arc<Session>)>,
    workspace_roster: Option<Vec<daemon::Summary>>,
    /// The window size before the workspace picker grew it.
    grown_window: Option<GrownWindow>,
    runtime: tokio::runtime::Runtime,
    control: control::Server,
    pending_attach: BTreeMap<u64, PendingAttach>,
    panes: BTreeMap<u64, Pane>,
    window: WindowLayout,
    picker: SessionPicker,
    shortcuts: Shortcuts,
    overlay: Option<Overlay>,
    geometry: GeometryAutosave,
}

/// A window-level overlay. The window shows at most one, so opening one
/// closes the one already open.
enum Overlay {
    Palette(crate::command_palette::CommandPalette),
    FontSize,
    Workspaces,
    SessionPicker,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        crate::terminal_config::load(&cc.egui_ctx)?;
        let crate::app_config::Settings {
            ui_size,
            ui_face,
            keymap,
        } = crate::app_config::load()?;

        leave_zoom_keys_to_the_terminal(&cc.egui_ctx);
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*cc.egui_ctx.style_of(egui::Theme::Dark)).clone();
        style.visuals.panel_fill = BG;
        style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.3);
        style.spacing.button_padding = Vec2::new(12.0, 7.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(ui_size));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(ui_size));
        style.visuals.window_fill = TAB_BAR_BG;
        style.visuals.window_stroke = Stroke::new(1.0_f32, crate::theme::MENU_BORDER);
        style.visuals.window_corner_radius = egui::CornerRadius::same(8);
        cc.egui_ctx.set_style_of(egui::Theme::Dark, style);
        cc.egui_ctx.set_fonts(crate::theme::chrome_fonts(ui_face));
        crate::render::initialize(cc)?;
        Self::connect(tokio::runtime::Runtime::new()?, &cc.egui_ctx, keymap)
    }

    fn connect(
        runtime: tokio::runtime::Runtime,
        ctx: &egui::Context,
        keymap: crate::shortcuts::AppKeymap,
    ) -> anyhow::Result<Self> {
        let control = control::Server::bind(&runtime, ctx.clone())?;
        let workspace = WorkspaceClient::start(&runtime, ctx.clone(), control.window);
        let app = Self {
            workspace: Some(workspace),
            workspace_id: None,
            daemon_instance: None,
            workspace_error: None,
            last_snapshot: None,
            pending_workspace: None,
            reconnect: Default::default(),
            paste_requested: None,
            workspace_roster: None,
            grown_window: None,
            runtime,
            control,
            pending_attach: BTreeMap::new(),
            panes: BTreeMap::new(),
            window: WindowLayout {
                font_size: crate::terminal_config::font_size(&crate::terminal_config::get(ctx)),
                ..Default::default()
            },
            picker: SessionPicker::default(),
            shortcuts: Shortcuts::from_config(&crate::terminal_config::get(ctx), keymap),
            overlay: None,
            geometry: GeometryAutosave::default(),
        };
        Ok(app)
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            window: self.window.clone(),
            panes: self
                .panes
                .iter()
                .map(|(&id, pane)| (id, pane.spec()))
                .collect(),
        }
    }

    /// The workspace identity new local shells carry.
    fn workspace(&self) -> Option<(u128, Option<&str>)> {
        self.workspace_id
            .map(|id| (id, self.daemon_instance.as_deref()))
    }

    fn restore_workspace(&mut self, ctx: &egui::Context, snapshot: Snapshot) {
        self.panes.clear();
        self.pending_attach.clear();
        // Startup restores layout and shells, but terminal config owns the
        // initial font size. A saved absolute zoom must not hide config edits.
        self.window = WindowLayout {
            font_size: crate::terminal_config::font_size(&crate::terminal_config::get(ctx)),
            ..snapshot.window
        };
        for (id, spec) in snapshot.panes {
            self.panes.insert(
                id,
                Pane::connect(
                    &self.runtime,
                    ctx,
                    self.control.window,
                    self.workspace(),
                    id,
                    spec.target,
                    spec.session.map(|id| id.to_string()),
                    None,
                ),
            );
        }
        if self.window.tabs.is_empty() {
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
            if self.workspace_id.is_some() {
                self.reconnect.observe(
                    event.operation,
                    event.result.is_ok(),
                    std::time::Instant::now(),
                );
            }
            match (event.operation, event.result) {
                (
                    Operation::Open,
                    Ok(Payload::Workspace {
                        id,
                        snapshot,
                        instance,
                    }),
                ) => {
                    if let Err(error) = snapshot.validate() {
                        self.workspace_error = Some(error.to_string());
                        continue;
                    }
                    self.workspace_id = Some(id);
                    self.daemon_instance = Some(instance).filter(|instance| !instance.is_empty());
                    self.last_snapshot = Some(snapshot.clone());
                    self.restore_workspace(ctx, snapshot);
                }
                (Operation::List, Ok(Payload::Workspaces(workspaces))) => {
                    self.workspace_roster = Some(workspaces);
                }
                (
                    Operation::Prepare,
                    Ok(Payload::Workspace {
                        id, mut snapshot, ..
                    }),
                ) => {
                    if let Err(error) = snapshot.validate() {
                        self.cancel_workspace(error.to_string());
                        continue;
                    }
                    if snapshot.window.tabs.is_empty() {
                        match Target::default_local() {
                            Ok(target) => workspace_switch::add_first_tab(
                                &mut snapshot,
                                crate::terminal_config::font_size(&crate::terminal_config::get(
                                    ctx,
                                )),
                                target,
                            ),
                            Err(error) => {
                                self.cancel_workspace(error.to_string());
                                continue;
                            }
                        }
                    }
                    let instance = self.daemon_instance.as_deref();
                    let Some(pending) = &mut self.pending_workspace else {
                        continue;
                    };
                    let current = &self.panes;
                    let (runtime, window) = (&self.runtime, self.control.window);
                    pending.prepared(id, snapshot, |pane_id, spec| {
                        let existing = spec.session.as_ref().and_then(|session| {
                            current.values().find(|pane| {
                                let state = pane.session.state.lock().unwrap();
                                pane.session.target == spec.target
                                    && state.session_id.as_ref() == Some(session)
                            })
                        });
                        if let Some(existing) = existing {
                            Pane::from_session(existing.session.clone())
                        } else {
                            Pane::connect(
                                runtime,
                                ctx,
                                window,
                                Some((id, instance)),
                                pane_id,
                                spec.target.clone(),
                                spec.session.map(|id| id.to_string()),
                                None,
                            )
                        }
                    });
                }
                (Operation::Finish, result) => {
                    let Some(pending) = self.pending_workspace.take() else {
                        if let Err(error) = result {
                            self.workspace_error = Some(error);
                        }
                        continue;
                    };
                    match pending.finished(result.map(|_| ())) {
                        Step::Pending(pending) => {
                            self.workspace_error = pending.error().map(str::to_owned);
                            self.pending_workspace = Some(pending);
                        }
                        Step::Committed {
                            id,
                            snapshot,
                            panes,
                            reply,
                        } => {
                            self.panes = panes;
                            self.workspace_id = Some(id);
                            for (pane_id, spec) in snapshot.panes {
                                if !self.panes.contains_key(&pane_id) {
                                    let pane = Pane::connect(
                                        &self.runtime,
                                        ctx,
                                        self.control.window,
                                        Some((id, self.daemon_instance.as_deref())),
                                        pane_id,
                                        spec.target,
                                        spec.session.map(|id| id.to_string()),
                                        None,
                                    );
                                    self.panes.insert(pane_id, pane);
                                }
                            }
                            self.window = snapshot.window;
                            self.last_snapshot = None;
                            self.workspace_error = None;
                            if let Some(reply) = reply {
                                let _ = reply.send(Ok(format!("Attached workspace {id:032x}")));
                            }
                        }
                        Step::Failed { error, reply } => {
                            if let Some(reply) = reply {
                                let _ = reply.send(Err(error.clone()));
                            }
                            self.workspace_error = Some(error);
                        }
                    }
                }
                (operation @ (Operation::Prepare | Operation::Disconnected), Err(error)) => {
                    if let Some(reply) = self
                        .pending_workspace
                        .take()
                        .and_then(WorkspaceSwitch::abort)
                    {
                        let _ = reply.send(Err(error.clone()));
                    }
                    if operation == Operation::Disconnected && self.workspace_id.is_some() {
                        // No countdown: it would go stale between repaints.
                        self.workspace_error =
                            Some(format!("{error}. Reconnecting automatically…"));
                    } else {
                        self.workspace_error = Some(error);
                    }
                }
                (
                    Operation::Resume,
                    Ok(Payload::Workspace {
                        id,
                        snapshot,
                        instance,
                    }),
                ) => {
                    self.workspace_id = Some(id);
                    self.daemon_instance = Some(instance).filter(|instance| !instance.is_empty());
                    self.last_snapshot = Some(snapshot);
                    self.workspace_error = None;
                }
                (Operation::Resume, Err(error)) => {
                    self.workspace_error = Some(format!("Could not resume the workspace: {error}"));
                }
                (_, Err(error)) => self.workspace_error = Some(error),
                _ => {}
            }
        }
    }

    fn begin_workspace(
        &mut self,
        workspace: Option<u128>,
        reply: Option<tokio::sync::oneshot::Sender<Reply>>,
    ) {
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                self.pending_workspace.is_none() && self.pending_attach.is_empty(),
                "another attach is in progress"
            );
            anyhow::ensure!(self.workspace_id.is_some(), "workspace is not ready");
            // A switch queued behind a resume would never be answered if the
            // daemon refused the resume.
            anyhow::ensure!(
                !self.reconnect.resuming() && !self.daemon_lost(),
                "the nekomata daemon is reconnecting"
            );
            anyhow::ensure!(
                workspace != self.workspace_id,
                "workspace is already attached to this window"
            );
            let client = self
                .workspace
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("nekomata daemon is unavailable"))?;
            client.prepare(
                self.snapshot(),
                workspace
                    .map(daemon::Selection::Id)
                    .unwrap_or(daemon::Selection::New),
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.workspace_error = None;
                self.overlay = None;
                self.pending_workspace = Some(WorkspaceSwitch::new(reply));
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
            pending.cancel(error.clone());
        }
        self.workspace_error = Some(error);
    }

    fn poll_workspace(&mut self) {
        let Some(pending) = &mut self.pending_workspace else {
            return;
        };
        let reply_closed = pending.reply_closed();
        let poll = pending.poll(
            |pane: &Pane| {
                let state = pane.session.state.lock().unwrap();
                if let Some(error) = state.error() {
                    PaneStatus::Failed(error.to_owned())
                } else if state.lifecycle.is_ended() {
                    PaneStatus::Ended
                } else if state.lifecycle.is_live() {
                    PaneStatus::Connected(state.session_id)
                } else {
                    PaneStatus::Connecting
                }
            },
            reply_closed,
        );
        let Poll::Finish(snapshot) = poll else {
            return;
        };
        if snapshot.is_none() {
            self.workspace_error = pending.error().map(str::to_owned);
        }
        if let Some(client) = &self.workspace
            && client.send(WorkspaceCommand::Finish { snapshot }).is_ok()
            && let Some(pending) = &mut self.pending_workspace
        {
            pending.finish_sent();
        }
    }

    /// Makes a short window tall enough to list the workspaces, up to seven.
    /// The window never shrinks for the list, and is not touched when
    /// maximized or fullscreen.
    fn grow_window_for_workspaces(&mut self, ctx: &egui::Context) {
        let Some(count) = self.workspace_roster.as_ref().map(Vec::len) else {
            return;
        };
        let (inner, monitor, fixed) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.inner_rect.map(|rect| rect.size()),
                viewport.monitor_size,
                viewport.maximized == Some(true) || viewport.fullscreen == Some(true),
            )
        });
        let Some(inner) = inner else {
            return;
        };
        let mut wanted = crate::workspace_picker::wanted_height(ctx, count);
        if let Some(monitor) = monitor {
            wanted = wanted.min(monitor.y * 0.9);
        }
        let requested = self.grown_window.as_ref().map(|grown| grown.requested.y);
        if fixed || wanted <= inner.y + 1.0 || requested.is_some_and(|height| height >= wanted) {
            return;
        }
        let size = egui::vec2(inner.x, wanted);
        let original = self
            .grown_window
            .take()
            .map_or(inner, |grown| grown.original);
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        self.grown_window = Some(GrownWindow {
            original,
            requested: size,
        });
    }

    /// Undoes the growth unless the user has resized the window since.
    fn restore_window_size(&mut self, ctx: &egui::Context) {
        let Some(grown) = self.grown_window.take() else {
            return;
        };
        let inner = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size()));
        if inner.is_some_and(|size| (size - grown.requested).abs().max_elem() < 2.0) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(grown.original));
        }
    }

    fn workspace_picker(&mut self, ctx: &egui::Context) {
        if !matches!(self.overlay, Some(Overlay::Workspaces)) {
            self.restore_window_size(ctx);
            return;
        }
        self.grow_window_for_workspaces(ctx);
        let outcome =
            crate::workspace_picker::show(ctx, self.workspace_roster.as_deref(), self.workspace_id);
        if outcome.dismissed {
            self.overlay = None;
        }
        if outcome.refresh
            && let Some(client) = &self.workspace
        {
            let _ = client.send(WorkspaceCommand::List);
        }
        if let Some(workspace) = outcome.selected {
            self.begin_workspace(workspace, None);
        }
    }

    /// Whether the window holds a workspace but no daemon connection.
    fn daemon_lost(&self) -> bool {
        self.workspace_id.is_some()
            && self
                .workspace
                .as_ref()
                .is_none_or(WorkspaceClient::is_closed)
    }

    /// Re-leases a workspace with the current layout once the reconnect
    /// delay after losing the daemon has passed.
    fn resume_workspace(&mut self, ctx: &egui::Context) {
        if !self.daemon_lost()
            || self.pending_workspace.is_some()
            || !self.reconnect.start(std::time::Instant::now())
        {
            return;
        }
        let Some(previous) = self.workspace_id else {
            return;
        };
        self.workspace = Some(WorkspaceClient::resume(
            &self.runtime,
            ctx.clone(),
            self.control.window,
            crate::workspace_client::Resume {
                previous,
                saved: self
                    .workspace
                    .as_ref()
                    .map(WorkspaceClient::saved)
                    .unwrap_or_default(),
                current: self.snapshot(),
            },
        ));
    }

    fn session_picker(&mut self, ctx: &egui::Context) -> Option<String> {
        if !matches!(self.overlay, Some(Overlay::SessionPicker)) {
            return None;
        }
        let mut open = true;
        let selected = self.picker.show(&self.runtime, ctx, &mut open);
        if !open {
            self.overlay = None;
        }
        selected
    }

    fn save_workspace(&mut self, ctx: &egui::Context) {
        if self.workspace_id.is_none() || self.pending_workspace.is_some() {
            return;
        }
        let snapshot = self.snapshot();
        if self.last_snapshot.as_ref() == Some(&snapshot) {
            return;
        }
        let Some(client) = &self.workspace else {
            return;
        };
        if client
            .send(WorkspaceCommand::Save(snapshot.clone()))
            .is_ok()
        {
            self.last_snapshot = Some(snapshot);
        } else if !client.is_closed() {
            // The command queue is full. A queued save completes without
            // waking the GUI, so retry on a timer rather than on the next event.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn add_pane(
        &mut self,
        ctx: &egui::Context,
        attach: Option<String>,
        cwd: Option<String>,
        target: Target,
    ) -> u64 {
        let id = self.window.allocate_pane();
        self.panes.insert(
            id,
            Pane::connect(
                &self.runtime,
                ctx,
                self.control.window,
                self.workspace(),
                id,
                target,
                attach,
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
        self.window.open_tab(id);
    }

    fn close_pane(&mut self, ctx: &egui::Context, id: u64) {
        if !self.window.close_pane(id) {
            return;
        }
        self.panes.remove(&id);
        if self.window.tabs.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn control_requests(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.control.requests.try_recv() {
            match message.request {
                Request::Switch { workspace } => {
                    self.begin_workspace(workspace, Some(message.reply))
                }
                Request::List => {
                    let active = self.window.focused();
                    let mut output = format!(
                        "WINDOW {}\nPANE\tACTIVE\tDAEMON\tSESSION\n",
                        self.control.window
                    );
                    for (id, pane) in &self.panes {
                        let state = pane.session.state.lock().unwrap();
                        let session = state
                            .session_id
                            .map(|id| id.to_string())
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
                    let id = pane.or_else(|| self.window.focused());
                    let Some((id, pane)) =
                        id.and_then(|id| self.panes.get(&id).map(|pane| (id, pane)))
                    else {
                        let _ = message.reply.send(Err("pane not found".into()));
                        continue;
                    };
                    if self.pending_workspace.is_some() {
                        let _ = message
                            .reply
                            .send(Err("workspace switch is in progress".into()));
                        continue;
                    }
                    if self.pending_attach.contains_key(&id) {
                        let _ = message
                            .reply
                            .send(Err("pane already has an attach in progress".into()));
                        continue;
                    }
                    let Ok(ssh) = ssh.map(|host| host.parse()).transpose() else {
                        let _ = message.reply.send(Err("invalid attach target".into()));
                        continue;
                    };
                    if session.is_empty() {
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
        let ids: Vec<_> = self.pending_attach.keys().copied().collect();
        for id in ids {
            let pending = &self.pending_attach[&id];
            let session = pending.session.clone();
            // Hold the session state through the swap so the session task
            // cannot end it between resolving the attach and replacing the pane.
            let state = session.state.lock().unwrap();
            let Some(result) = attach_result(
                id,
                &state,
                self.panes.contains_key(&id),
                pending.reply.is_closed(),
            ) else {
                continue;
            };
            let pending = self.pending_attach.remove(&id).unwrap();
            if result.is_ok() {
                let pane = self.panes.get_mut(&id).unwrap();
                pane.session = pending.session;
                pane.links = Default::default();
                pane.pending_paste = None;
                pane.requested = None;
                pane.selection = None;
                pane.reported_focus = None;
                pane.scroll_remainder = 0.0;
                pane.mouse_capture = None;
                pane.link_capture = Default::default();
            }
            drop(state);
            let _ = pending.reply.send(result);
        }
    }

    fn input_blocked(&self) -> bool {
        InputBlockers {
            overlay: self.overlay.is_some(),
            searching: self.searching(),
            paste_pending: self.panes.values().any(|pane| pane.pending_paste.is_some()),
            switching: self.pending_workspace.is_some(),
        }
        .any()
    }

    fn searching(&self) -> bool {
        self.window
            .focused()
            .and_then(|id| self.panes.get(&id))
            .is_some_and(|pane| pane.search_open)
    }

    fn open_search(&mut self) {
        if let Some(pane) = self.window.focused().and_then(|id| self.panes.get_mut(&id)) {
            pane.search_open = true;
            pane.search_focus = true;
        }
    }

    fn search_bar(&mut self, ui: &mut egui::Ui) {
        let Some(focused) = self.window.focused() else {
            return;
        };
        let pane = self.panes.get_mut(&focused).unwrap();
        if !pane.search_open {
            return;
        }
        let mut navigate = None;
        let mut close = consume_search_escape(ui.ctx());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.spacing_mut().button_padding = Vec2::new(5.0, 4.0);
            // Leave room for the icons and the result count after the entry.
            let entry_width =
                (ui.available_width() - crate::theme::ui_extent(ui.style(), 132.0)).max(48.0);
            let entry = ui.add_sized(
                [entry_width, crate::theme::ui_extent(ui.style(), 30.0)],
                egui::TextEdit::singleline(&mut pane.search_query)
                    .id_salt(("scrollback-query", focused))
                    .hint_text("Search scrollback")
                    .align(egui::Align2::LEFT_CENTER)
                    .desired_width(entry_width)
                    .char_limit(1024),
            );
            if std::mem::take(&mut pane.search_focus) {
                entry.request_focus();
            }
            if entry.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                navigate = Some(!ui.input(|input| input.modifiers.shift));
            }
            let options = search_icon_button(ui, SearchIcon::Options);
            egui::Popup::menu(&options).show(|ui| {
                ui.checkbox(&mut pane.search_ignore_case, "Ignore case");
                ui.checkbox(&mut pane.search_regex, "Regex");
            });
            ui.add_enabled_ui(!pane.search_query.is_empty(), |ui| {
                ui.vertical(|ui| {
                    ui.set_width(24.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if search_icon_button(ui, SearchIcon::Previous).clicked() {
                        navigate = Some(false);
                    }
                    if search_icon_button(ui, SearchIcon::Next).clicked() {
                        navigate = Some(true);
                    }
                });
            });
            {
                let search = pane.session.search.search.lock().unwrap();
                let response = ui.add_sized(
                    [
                        crate::theme::ui_extent(ui.style(), 40.0),
                        crate::theme::ui_extent(ui.style(), 30.0),
                    ],
                    egui::Label::new(if search.error.is_some() {
                        egui::RichText::new("!").color(crate::theme::ERROR_FG)
                    } else if search.running {
                        egui::RichText::new("…")
                    } else {
                        egui::RichText::new(format!(
                            "{}/{}{}",
                            search.current_number(),
                            search.hits.len(),
                            if search.limited { "+" } else { "" }
                        ))
                    })
                    .truncate(),
                );
                if let Some(error) = &search.error {
                    response.on_hover_text(error);
                } else if search.running {
                    response.on_hover_text("Searching…");
                }
            }
            close |= search_icon_button(ui, SearchIcon::Close).clicked();
        });
        if close {
            pane.close_search();
        } else if let Some(older) = navigate {
            pane.navigate_search(older);
        }
    }

    fn terminal_action(&mut self, ctx: &egui::Context, action: felis_client_core::Action) {
        use felis_client_core::{Action as TerminalAction, ClipboardScope, FontSizeStep};
        match action {
            TerminalAction::Copy {
                what: ClipboardScope::System,
            } => self.action(ctx, Action::Copy),
            TerminalAction::Paste {
                from: ClipboardScope::System,
            } => self.action(ctx, Action::Paste),
            TerminalAction::FontSize(step) => self.action(
                ctx,
                match step {
                    FontSizeStep::Increase => Action::FontScale(crate::font_size::STEP),
                    FontSizeStep::Decrease => Action::FontScale(1.0 / crate::font_size::STEP),
                    FontSizeStep::Reset => Action::FontReset,
                },
            ),
            TerminalAction::Scroll(step) => {
                let pane = self
                    .panes
                    .get_mut(&self.window.active_tab().focused)
                    .unwrap();
                let state = pane.session.state.lock().unwrap();
                if state.lifecycle.is_live()
                    && let Some(request) = state.scroll_action(step)
                {
                    pane.selection = None;
                    pane.session.send(request);
                }
            }
            TerminalAction::Ipc(felis_client_core::IpcAction::OpenScrollbackSearch) => {
                self.open_search()
            }
            TerminalAction::ScrollToPrompt(direction) => {
                let pane = self
                    .panes
                    .get_mut(&self.window.active_tab().focused)
                    .unwrap();
                let state = pane.session.state.lock().unwrap();
                if state.lifecycle.is_live() && !state.screen.alt_screen() {
                    pane.selection = None;
                    pane.session.send(InputMsg::JumpPrompt { direction });
                }
            }
            // Only supported terminal actions are admitted by Shortcuts. Keep
            // unsupported shared defaults available to the terminal program.
            _ => {}
        }
    }

    fn paste_confirmation(&mut self, ctx: &egui::Context) {
        let Some((&id, pane)) = self
            .panes
            .iter_mut()
            .find(|(_, pane)| pane.pending_paste.is_some())
        else {
            return;
        };
        use crate::theme;
        let response =
            egui::Modal::new(egui::Id::unique("paste-confirmation").with(id))
                .frame(
                    egui::Frame::new()
                        .fill(theme::TAB_BAR_BG)
                        .stroke(egui::Stroke::new(1.0, theme::MENU_BORDER))
                        .corner_radius(12)
                        .inner_margin(24),
                )
                .show(ctx, |ui| {
                    ui.set_width((ctx.content_rect().width() - 64.0).clamp(180.0, 380.0));
                    ui.spacing_mut().item_spacing.y = 12.0;
                    ui.label(
                        egui::RichText::new("Paste into terminal?")
                            .size(theme::ui_size(ui.style(), 20.0))
                            .strong()
                            .color(theme::FG),
                    );
                    ui.label(
                        egui::RichText::new(
                            "This text contains line breaks or control characters that may run commands immediately.",
                        )
                        .size(theme::ui_size(ui.style(), 14.0))
                        .color(theme::FG),
                    );
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let confirm = ui
                            .add_sized(
                                [88.0, 34.0],
                                egui::Button::new(
                                    egui::RichText::new("Paste").color(theme::BG).strong(),
                                )
                                .fill(theme::ACCENT)
                                .corner_radius(6),
                            )
                            .clicked();
                        let cancel = ui
                            .add_sized(
                                [88.0, 34.0],
                                egui::Button::new(egui::RichText::new("Cancel").color(theme::FG))
                                    .fill(theme::TAB_ACTIVE_BG)
                                    .corner_radius(6),
                            )
                            .clicked();
                        (confirm, cancel)
                    })
                    .inner
                });
        if response.inner.0 {
            let pending = pane.pending_paste.take().unwrap();
            let state = pane.session.state.lock().unwrap();
            // Sent under the lock: a reconnect bumps `reconnects` under it,
            // so the paste cannot reach a connection it was not confirmed for.
            if pending.is_current(state.lifecycle.is_live(), state.reconnects) {
                send_to_program(
                    &mut pane.selection,
                    &pane.session,
                    InputMsg::Paste(pending.into_bytes()),
                );
            }
        } else if response.inner.1 || response.should_close() {
            pane.pending_paste = None;
        }
    }

    fn action(&mut self, ctx: &egui::Context, action: Action) {
        if self.pending_workspace.is_some() {
            return;
        }
        match action {
            Action::Search => self.open_search(),
            Action::Terminal(action) => self.terminal_action(ctx, action),
            Action::NewWorkspace => self.begin_workspace(None, None),
            Action::OpenWorkspaces => {
                self.overlay = Some(Overlay::Workspaces);
                self.workspace_roster = None;
                if let Some(client) = &self.workspace {
                    let _ = client.send(WorkspaceCommand::List);
                }
            }
            Action::Palette => {
                self.overlay = Some(Overlay::Palette(Default::default()));
            }
            Action::AttachSession => {
                self.overlay = Some(Overlay::SessionPicker);
                self.picker.open(&self.runtime, ctx);
            }
            Action::ChangeFontSize => self.overlay = Some(Overlay::FontSize),
            Action::FontScale(scale) => {
                self.window.font_size = felis_client_core::config::FontConfig::clamped_font_size(
                    self.window.font_size * scale,
                    0.0,
                )
            }
            Action::FontReset => {
                self.window.font_size =
                    crate::terminal_config::font_size(&crate::terminal_config::get(ctx))
            }
            Action::Paste => {
                // The palette closes before acting; close search before requesting paste.
                let pane = self
                    .panes
                    .get_mut(&self.window.active_tab().focused)
                    .unwrap();
                if pane.search_open {
                    pane.close_search();
                }
                let state = pane.session.state.lock().unwrap();
                if !state.lifecycle.is_live() {
                    pane.session.input_lost();
                } else if self.paste_requested.is_none() {
                    self.paste_requested = Some((
                        ctx.cumulative_frame_nr(),
                        state.reconnects,
                        pane.session.clone(),
                    ));
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                }
            }
            Action::Copy => {
                let pane = self
                    .panes
                    .get_mut(&self.window.active_tab().focused)
                    .unwrap();
                let state = pane.session.state.lock().unwrap();
                expire_selection(&mut pane.selection, &mut pane.selection_epoch, &state);
                if let Some(selection) = pane.selection {
                    ctx.copy_text(selection.extract_text(state.screen.screen()));
                }
            }
            Action::Focus(direction) => {
                let tab = self.window.active_tab_mut();
                if let Some(id) = tab.layout.neighbor(tab.focused, direction) {
                    tab.focus(id);
                }
            }
            Action::NewTab => self.add_tab(ctx, None),
            Action::Split(axis) => {
                let source = self.window.active_tab().focused;
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
                let tab = self.window.active_tab_mut();
                let split = tab.layout.split(source, id, axis);
                // Validation keeps a tab's focused pane in its layout, so this
                // is a broken invariant. Release builds drop the pane to keep
                // the workspace valid; its shell stays detached and attachable.
                debug_assert!(split, "the focused pane is missing from its layout");
                if split {
                    tab.focus(id);
                } else {
                    self.panes.remove(&id);
                }
            }
            Action::ClosePane => self.close_pane(ctx, self.window.active_tab().focused),
            Action::CloseTab => {
                let tab = self.window.close_active_tab();
                for id in tab.layout.panes() {
                    self.panes.remove(&id);
                }
                if self.window.tabs.is_empty() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Action::NextPane => {
                let tab = self.window.active_tab_mut();
                let ids = tab.layout.panes();
                let pos = ids.iter().position(|id| *id == tab.focused).unwrap_or(0);
                tab.focus(ids[(pos + 1) % ids.len()]);
            }
            Action::NextTab => self.window.cycle_tab(true),
            Action::PreviousTab => self.window.cycle_tab(false),
        }
    }
}

impl App {
    fn show(&mut self, root_ui: &mut egui::Ui) {
        let context = root_ui.ctx().clone();
        let ctx = &context;
        self.workspace_events(ctx);
        self.poll_workspace();
        self.resume_workspace(ctx);
        if self.workspace.is_some() && self.workspace_id.is_none() {
            egui::CentralPanel::default().show(root_ui, |ui| {
                if let Some(error) = &self.workspace_error {
                    ui.colored_label(crate::theme::ERROR_FG, error);
                    if ui.button("Retry connecting to nekomata daemon").clicked() {
                        self.workspace = Some(WorkspaceClient::start(
                            &self.runtime,
                            ctx.clone(),
                            self.control.window,
                        ));
                        self.workspace_error = None;
                    }
                } else {
                    ui.label("Connecting to workspace…");
                }
            });
            control::reject_all(&mut self.control.requests, "workspace is not ready");
            return;
        }
        self.control_requests(ctx);
        if let Some(error) = &self.workspace_error {
            let lost = self.daemon_lost();
            egui::Panel::bottom("workspace-error").show(root_ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(crate::theme::ERROR_FG, error);
                    if lost && ui.button("Reconnect now").clicked() {
                        self.reconnect.retry_now();
                    }
                });
            });
        }
        for pane in self.panes.values() {
            pane.session.search.invalidate_changed_session();
            if let Some(text) = pane.session.state.lock().unwrap().clipboard.take() {
                ctx.copy_text(text);
            }
        }
        let closed: Vec<_> = self
            .panes
            .iter()
            .filter_map(|(&id, pane)| {
                let state = pane.session.state.lock().unwrap();
                (self.pending_workspace.is_none() && state.lifecycle.is_ended()).then_some(id)
            })
            .collect();
        for id in closed {
            self.close_pane(ctx, id);
        }
        if self.window.tabs.is_empty() {
            egui::CentralPanel::default().show(root_ui, |_| {});
            if let Some(id) = self.session_picker(ctx) {
                self.add_tab(ctx, Some(id));
            } else if !matches!(self.overlay, Some(Overlay::SessionPicker)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        for pane in self.panes.values_mut() {
            pane.links.tick(&self.runtime, ctx);
            let state = pane.session.state.lock().unwrap();
            if pane.pending_paste.as_ref().is_some_and(|pending| {
                !pending.is_current(state.lifecycle.is_live(), state.reconnects)
            }) {
                pane.pending_paste = None;
            }
        }
        let paste_pending = self.panes.values().any(|pane| pane.pending_paste.is_some());
        let mut action = None;
        // Consume application shortcuts before forwarding terminal events.
        if !self.input_blocked() {
            let actions = ctx.input_mut(|input| self.shortcuts.consume(input));
            for action in actions {
                self.action(ctx, action);
                if self.window.tabs.is_empty() {
                    return;
                }
            }
        }
        // Requesting terminal focus would take it from an open menu. Only this
        // request waits for menus: one left by a search field that vanished
        // with its pane or tab lingers a frame, and keys then belong to the
        // terminal.
        if !self.input_blocked() && !egui::Popup::is_any_open(ctx) {
            focus_terminal(ctx);
        }
        let tabs = egui::Panel::top("tabs")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(TAB_BAR_BG)
                    .inner_margin(egui::Margin::symmetric(8, 6)),
            )
            .show(root_ui, |ui| {
                if self.pending_workspace.is_some() || paste_pending {
                    ui.disable();
                }
                ui.spacing_mut().button_padding = Vec2::new(15.0, 6.0);
                // Reserve the full button height before either nested horizontal layout.
                ui.spacing_mut().interact_size.y = crate::theme::ui_extent(ui.style(), 30.0).max(
                    ui.text_style_height(&egui::TextStyle::Button)
                        + 2.0 * ui.spacing().button_padding.y,
                );
                ui.horizontal(|ui| {
                    let palette_button = crate::theme::palette_button(
                        ui,
                        matches!(self.overlay, Some(Overlay::Palette(_))),
                    );
                    let search_width = if self.searching() {
                        ui.available_width()
                            .min(crate::theme::ui_extent(ui.style(), 360.0))
                    } else {
                        0.0
                    };
                    egui::ScrollArea::horizontal()
                        .max_width(
                            (ui.available_width() - search_width - ui.spacing().item_spacing.x)
                                .max(0.0),
                        )
                        .min_scrolled_width(0.0)
                        .auto_shrink([false, true])
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
                                let mut rects = Vec::with_capacity(self.window.tabs.len());
                                let mut dragged = None;
                                for (i, tab) in self.window.tabs.iter().enumerate() {
                                    let title = self.panes[&tab.focused]
                                        .session
                                        .state
                                        .lock()
                                        .unwrap()
                                        .screen
                                        .title()
                                        .unwrap_or(&tab.name)
                                        .to_owned();
                                    let selected = self.window.active == i;
                                    let response = crate::theme::tab_button(ui, &title, selected);
                                    if response.clicked() {
                                        self.window.active = i;
                                    }
                                    if response.dragged() || response.drag_stopped() {
                                        dragged = Some((i, response.drag_stopped()));
                                    }
                                    rects.push(response.rect);
                                }
                                if let Some((from, released)) = dragged
                                    && let Some(pointer) = ui.ctx().pointer_interact_pos()
                                {
                                    let to = tab_drop_slot(&rects, pointer.x);
                                    if released {
                                        self.window.move_tab(from, to);
                                    } else {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                        // Dropping beside the dragged tab changes nothing.
                                        if to != from && to != from + 1 {
                                            let x = tab_drop_indicator_x(
                                                &rects,
                                                to,
                                                ui.spacing().item_spacing.x,
                                                ui.clip_rect().x_range(),
                                            );
                                            ui.painter().vline(
                                                x,
                                                rects[from].y_range(),
                                                Stroke::new(2.0, ACCENT),
                                            );
                                        }
                                    }
                                }
                            });
                        });
                    if self.searching() {
                        ui.allocate_ui_with_layout(
                            Vec2::new(search_width, crate::theme::ui_extent(ui.style(), 30.0)),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| self.search_bar(ui),
                        );
                    }
                    palette_button
                })
                .inner
            });
        let palette_key = self.shortcuts.hints().get(&Action::Palette).to_owned();
        let palette_hint = match palette_key.as_str() {
            "" => "Command palette".to_owned(),
            key => format!("Command palette · {key}"),
        };
        if tabs.inner.on_hover_text(palette_hint).clicked() {
            action = Some(Action::Palette);
        }
        if let Some(action) = action {
            self.action(ctx, action);
        }
        if self.window.tabs.is_empty() {
            return;
        }
        let modal = self.input_blocked();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(root_ui, |ui| {
                if self.pending_workspace.is_some() || paste_pending {
                    ui.disable();
                }
                ui.interact(
                    ui.max_rect(),
                    terminal_focus_id(),
                    Sense::focusable_noninteractive(),
                );
                let font_size = self.window.font_size;
                let tab = self.window.active_tab_mut();
                let previous = tab.focused;
                draw_layout(
                    ui,
                    &mut tab.layout,
                    &mut self.panes,
                    &mut tab.focused,
                    font_size,
                    !modal,
                );
                if tab.focused != previous {
                    tab.focus(tab.focused);
                }
            });
        let focused = self.window.active_tab().focused;
        let os_focused = ctx.input(|i| i.focused);
        for (&id, pane) in &mut self.panes {
            let desired = id == focused && os_focused && !modal;
            if pane.reported_focus != Some(desired) {
                pane.session
                    .send(InputMsg::FocusChange { focused: desired });
                pane.reported_focus = Some(desired);
            }
        }
        // eframe delivers RequestPaste after the requesting frame. Consume its
        // intent even for an empty clipboard, so a later native Ctrl-V stays a key.
        let mut explicit_paste = false;
        let mut stale_paste = false;
        if self
            .paste_requested
            .as_ref()
            .is_some_and(|(frame, _, _)| ctx.cumulative_frame_nr() > *frame)
        {
            let (frame, generation, origin) = self.paste_requested.take().unwrap();
            let pane = &self.panes[&focused];
            explicit_paste = os_focused
                && !modal
                && requested_paste_is_current(
                    (frame, generation, &origin),
                    ctx.cumulative_frame_nr(),
                    &pane.session,
                );
            stale_paste = !explicit_paste;
        }
        if stale_paste {
            ctx.input_mut(|input| {
                input.events.retain(|event| {
                    !matches!(event, egui::Event::Paste(_) | egui::Event::PasteImage(_))
                })
            });
        }
        if !modal {
            let pane = self.panes.get_mut(&focused).unwrap();
            pane.expire_selection();
            terminal_input(
                ctx,
                &mut pane.selection,
                &pane.session,
                &mut pane.pending_paste,
                explicit_paste,
            );
        } else {
            ctx.output_mut(|output| {
                if output
                    .ime
                    .as_ref()
                    .is_some_and(|ime| ime.purpose == egui::IMEPurpose::Terminal)
                {
                    output.ime = None;
                }
            });
        }
        self.paste_confirmation(ctx);
        if let Some(id) = self.session_picker(ctx) {
            self.add_tab(ctx, Some(id));
        }
        if let Some(Overlay::Palette(palette)) = &mut self.overlay {
            let prompt_unavailable = {
                let state = self.panes[&self.window.active_tab().focused]
                    .session
                    .state
                    .lock()
                    .unwrap();
                if !state.lifecycle.is_live() {
                    Some("Unavailable while disconnected")
                } else if state.screen.alt_screen() {
                    Some("Unavailable in the alternate screen")
                } else {
                    None
                }
            };
            let (chosen, dismissed) = palette.show(ctx, self.shortcuts.hints(), prompt_unavailable);
            // Close before acting, so a chosen command can open another overlay.
            if dismissed || chosen.is_some() {
                self.overlay = None;
            }
            if let Some(action) = chosen {
                self.action(ctx, action);
            }
        }
        if matches!(self.overlay, Some(Overlay::FontSize)) {
            let (chosen, dismissed) = crate::font_size::show(ctx, self.window.font_size);
            if dismissed {
                self.overlay = None;
            }
            if let Some(action) = chosen {
                self.action(ctx, action);
            }
        }
        self.workspace_picker(ctx);
        if self.pending_workspace.is_some() {
            egui::Window::new("Switching workspace")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.spinner();
                    ui.label("Connecting to destination panes…");
                });
        }
        self.save_workspace(ctx);
        let interval = eframe::App::auto_save_interval(self);
        self.geometry
            .schedule(ctx, std::time::Instant::now(), interval);
    }
}

impl eframe::App for App {
    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(root_ui);
    }

    // Not egui memory: felis and the workspace daemon own persistent state,
    // every window process shares one eframe store while pane ids repeat
    // across workspaces, and `App::new` sets the style and theme anyway.
    fn persist_egui_memory(&self) -> bool {
        false
    }
    // Blocks the GUI thread: eframe tears the window down as soon as this
    // returns, so the final save must finish here. `close` bounds the wait
    // by `CLOSE_DEADLINE`, after which the lease ends with the connection.
    fn on_exit(&mut self) {
        if let Some(client) = &self.workspace {
            // An unanswered resume would hold the close behind up to four
            // daemon exchanges; the window has no lease to save into yet.
            if client.is_closed() || self.reconnect.resuming() {
                return;
            }
            let snapshot = self
                .workspace_id
                .filter(|_| self.pending_workspace.is_none())
                .map(|_| self.snapshot());
            if let Err(error) = client.close(&self.runtime, snapshot) {
                eprintln!("Could not save workspace: {error:#}");
            }
        }
    }
}

/// The window geometry eframe persists.
type Geometry = (Option<Rect>, Option<Rect>, Option<bool>, Option<bool>);

/// eframe autosaves window geometry only inside a frame, once its interval has
/// passed since its last save. An idle window runs no frames, so after a move
/// or resize keep one scheduled that is sure to find the autosave due.
#[derive(Default)]
struct GeometryAutosave {
    seen: Option<Geometry>,
    /// egui keeps only the current pass's shortest repaint delay, so an
    /// intervening frame would drop a one-shot request; every frame re-arms it.
    due: Option<std::time::Instant>,
}

impl GeometryAutosave {
    fn schedule(
        &mut self,
        ctx: &egui::Context,
        now: std::time::Instant,
        interval: std::time::Duration,
    ) {
        let geometry = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.outer_rect,
                viewport.inner_rect,
                viewport.maximized,
                viewport.fullscreen,
            )
        });
        // The first geometry is where the window opened, not a move or resize.
        if self
            .seen
            .replace(geometry)
            .is_some_and(|seen| seen != geometry)
        {
            self.due = Some(now + interval + std::time::Duration::from_secs(1));
        }
        match self.due {
            Some(due) if now < due => ctx.request_repaint_after(due - now),
            _ => self.due = None,
        }
    }
}

fn terminal_focus_id() -> egui::Id {
    egui::Id::unique("terminal-keyboard-focus")
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

#[derive(Clone, Copy)]
enum SearchIcon {
    Previous,
    Next,
    Options,
    Close,
}

fn search_icon_button(ui: &mut egui::Ui, icon: SearchIcon) -> egui::Response {
    let (height, label) = match icon {
        SearchIcon::Previous => (15.0, "Previous result · Shift+Enter"),
        SearchIcon::Next => (15.0, "Search / next result · Enter"),
        SearchIcon::Options => (30.0, "Search options"),
        SearchIcon::Close => (30.0, "Close search · Escape"),
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(24.0, height), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, TAB_HOVER_BG);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    let color = if !ui.is_enabled() {
        ui.visuals().weak_text_color()
    } else if response.hovered() {
        FG
    } else {
        TAB_INACTIVE_FG
    };
    let center = rect.center();
    let stroke = Stroke::new(1.4, color);
    match icon {
        SearchIcon::Previous | SearchIcon::Next => {
            let direction = if matches!(icon, SearchIcon::Previous) {
                1.0
            } else {
                -1.0
            };
            ui.painter().add(egui::Shape::line(
                vec![
                    center + Vec2::new(-4.0, 2.0 * direction),
                    center + Vec2::new(0.0, -2.0 * direction),
                    center + Vec2::new(4.0, 2.0 * direction),
                ],
                stroke,
            ));
        }
        SearchIcon::Options => {
            ui.painter().add(egui::Shape::line(
                vec![
                    center + Vec2::new(-6.0, -5.0),
                    center + Vec2::new(6.0, -5.0),
                    center + Vec2::new(2.0, 0.0),
                    center + Vec2::new(2.0, 5.0),
                    center + Vec2::new(-2.0, 3.0),
                    center + Vec2::new(-2.0, 0.0),
                    center + Vec2::new(-6.0, -5.0),
                ],
                stroke,
            ));
        }
        SearchIcon::Close => {
            for direction in [-1.0, 1.0] {
                let offset = Vec2::new(3.0, direction * 3.0);
                ui.painter()
                    .line_segment([center - offset, center + offset], stroke);
            }
        }
    }
    response.on_hover_text(label)
}

/// What keeps keys from both keymaps, and keyboard input and focus reports
/// from the terminal.
#[derive(Clone, Copy, Default)]
struct InputBlockers {
    /// Any open [`Overlay`].
    overlay: bool,
    /// The focused pane's search field.
    searching: bool,
    paste_pending: bool,
    switching: bool,
}

impl InputBlockers {
    fn any(self) -> bool {
        let Self {
            overlay,
            searching,
            paste_pending,
            switching,
        } = self;
        overlay || searching || paste_pending || switching
    }
}

/// The search field draws before the palette and the font size control, so
/// it must leave Escape to whichever popup or modal sits above it.
fn consume_search_escape(ctx: &egui::Context) -> bool {
    !egui::Popup::is_any_open(ctx)
        && ctx.memory(|memory| memory.top_modal_layer().is_none())
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
}

/// Resolves a pending pane attach, or `None` while it is still connecting.
fn attach_result(
    id: u64,
    state: &crate::session::State,
    pane_open: bool,
    cancelled: bool,
) -> Option<Reply> {
    if cancelled {
        Some(Err("request cancelled".into()))
    } else if !pane_open {
        Some(Err("pane closed during attach".into()))
    } else if state.lifecycle.is_terminal() {
        Some(Err(state
            .lifecycle
            .failure()
            .unwrap_or("session ended during attach")
            .into()))
    } else if state.lifecycle.is_live() {
        Some(Ok(format!("Attached pane {id}")))
    } else {
        None
    }
}

/// Returns the insertion gap for a pointer at `x`: the number of tabs whose
/// centers lie left of it.
fn tab_drop_slot(rects: &[egui::Rect], x: f32) -> usize {
    rects.iter().filter(|rect| rect.center().x < x).count()
}

/// Horizontal position of the drop indicator for `slot`, midway between the
/// neighboring tabs or just outside the outermost one, kept within `visible`.
fn tab_drop_indicator_x(
    rects: &[egui::Rect],
    slot: usize,
    spacing: f32,
    visible: egui::Rangef,
) -> f32 {
    // The scroll area clips at the first tab's left edge, so an indicator in
    // the outer gap would be invisible without clamping.
    let x = match (
        slot.checked_sub(1).and_then(|i| rects.get(i)),
        rects.get(slot),
    ) {
        (Some(left), Some(right)) => (left.right() + right.left()) / 2.0,
        (Some(left), None) => left.right() + spacing / 2.0,
        (None, Some(right)) => right.left() - spacing / 2.0,
        (None, None) => visible.min,
    };
    x.clamp(visible.min + 1.0, visible.max - 1.0)
}

fn draw_layout(
    ui: &mut egui::Ui,
    layout: &mut Layout,
    panes: &mut BTreeMap<u64, Pane>,
    focused: &mut u64,
    font_size: f32,
    terminal_input_enabled: bool,
) -> Option<Rect> {
    let rect = ui.max_rect();
    match layout {
        Layout::Pane(id) => {
            draw_pane(
                ui,
                *id,
                panes.get_mut(id).unwrap(),
                focused,
                font_size,
                terminal_input_enabled,
            );
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
            let response = ui.interact(
                divider.expand(2.0),
                ui.scope_id().with("divider"),
                Sense::drag(),
            );
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
                    draw_layout(ui, first, panes, focused, font_size, terminal_input_enabled)
                })
                .inner;
            let second_focus = ui
                .scope_builder(egui::UiBuilder::new().id_salt("second").max_rect(b), |ui| {
                    draw_layout(
                        ui,
                        second,
                        panes,
                        focused,
                        font_size,
                        terminal_input_enabled,
                    )
                })
                .inner;
            let painter = ui.painter();
            painter.rect_filled(divider, 0, crate::theme::PANE_DIVIDER);
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

fn draw_pane(
    ui: &mut egui::Ui,
    id: u64,
    pane: &mut Pane,
    focused: &mut u64,
    font_size: f32,
    terminal_input_enabled: bool,
) {
    let (generation, background) = {
        let state = pane.session.state.lock().unwrap();
        let background =
            crate::render::margin_color(state.colors.theme(), state.screen.reverse_video());
        (state.reconnects, background)
    };
    if generation != pane.connection_generation {
        pane.connection_generation = generation;
        pane.links = Default::default();
        pane.pending_paste = None;
        pane.requested = None;
        pane.reported_focus = None;
        pane.selection = None;
        pane.mouse_capture = None;
        pane.link_capture = Default::default();
    }
    let rect = ui.max_rect().shrink(2.0);
    let response = ui.interact(rect, ui.scope_id().with(id), Sense::click_and_drag());
    if response.clicked() || response.drag_started() {
        *focused = id;
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0, background);
    let ppp = ui.ctx().pixels_per_point();
    if pane.renderer.is_none() {
        match crate::render::Renderer::new(ui.ctx(), font_size, ppp) {
            Ok(renderer) => pane.renderer = Some(renderer),
            Err(error) => {
                painter.text(
                    rect.min,
                    egui::Align2::LEFT_TOP,
                    format!("Terminal font: {error}"),
                    FontId::monospace(font_size),
                    crate::theme::ERROR_FG,
                );
                return;
            }
        }
    }
    let font = FontId::monospace(font_size);
    let renderer = pane.renderer.as_mut().unwrap();
    renderer.set_font(font_size, ppp);
    let Vec2 { x: cw, y: ch } = renderer.cell_size();
    let requested = crate::render::requested_dims(rect, renderer.cell_px(), ppp);
    // Split the unused fractional row between the top and bottom margins.
    let rows = crate::render::layout_rows(rect, renderer.cell_px()[1], ppp);
    let text_rect = crate::render::terminal_rect(rect, rows, ch, ppp);
    if let Some((dims, grid_changed)) = resize_effect(pane.requested, requested) {
        if grid_changed {
            pane.selection = None;
        }
        pane.requested = Some(dims);
        pane.session.send(InputMsg::Resize { dims });
    }
    let mut state = pane.session.state.lock().unwrap();
    // Held from here through painting and selection gestures below.
    expire_selection(&mut pane.selection, &mut pane.selection_epoch, &state);
    let connected = state.lifecycle.is_live();
    let painter = painter.with_clip_rect(text_rect);
    {
        let mut search = pane.session.search.search.lock().unwrap();
        if let Some(jump) = search.take_jump(&state.screen) {
            pane.session.send(jump);
        }
        let key = (
            search.revision,
            state.screen.viewport(),
            state.screen.viewport_anchor(),
            state.screen.screen().rows(),
        );
        if pane.search_paint_key != Some(key) {
            renderer.set_search_overlay(search.overlay(&state.screen));
            pane.search_paint_key = Some(key);
        }
    }
    match renderer.paint(
        &painter,
        &mut state,
        text_rect,
        terminal_input_enabled && *focused == id && connected && ui.input(|input| input.focused),
        pane.selection,
    ) {
        Ok(rendered) => pane.session.frame_submitted(&mut state, rendered),
        Err(error) => state.renderer_error = Some(format!("Terminal renderer: {error:#}")),
    }
    let text_rect = renderer.content_rect(text_rect);
    let shadow = &state.screen;
    let screen = shadow.screen();
    let cursor = screen.cursor();
    let geometry = crate::render::PaneGeometry {
        text_rect,
        cell: Vec2::new(cw, ch),
        cols: screen.cols(),
        rows: screen.rows(),
    };
    // Enabling XIM before the native window receives focus can consume its
    // pending input/focus events. Only the active native window owns the IME.
    if terminal_input_enabled && *focused == id && ui.input(|input| input.focused) {
        ui.output_mut(|output| {
            output.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Terminal,
                should_interrupt_composition: false,
                rect: text_rect,
                cursor_rect: geometry.cell_rect(GridPos {
                    row: cursor.row,
                    col: cursor.col,
                }),
            })
        });
    }
    if let Some(error) = state.error() {
        painter.text(
            text_rect.min,
            egui::Align2::LEFT_TOP,
            error,
            FontId::proportional(crate::theme::ui_size(ui.style(), 14.0)),
            crate::theme::ERROR_FG,
        );
    } else if !state.lifecycle.is_live() {
        painter.text(
            text_rect.min,
            egui::Align2::LEFT_TOP,
            if state.lifecycle == crate::session::Lifecycle::Reconnecting {
                "Reconnecting to felis…"
            } else {
                "Connecting to felis…"
            },
            font,
            FG,
        );
    } else if let Some(notice) = pane.session.input_notice() {
        painter.text(
            text_rect.min,
            egui::Align2::LEFT_TOP,
            notice.message(),
            FontId::proportional(crate::theme::ui_size(ui.style(), 14.0)),
            Color32::LIGHT_RED,
        );
    }
    if let Some(error) = &pane.links.error {
        let height = crate::theme::ui_extent(ui.style(), 32.0);
        let overlay = Rect::from_min_size(
            text_rect.left_bottom() - Vec2::new(0.0, height),
            Vec2::new(text_rect.width(), height),
        );
        painter.rect_filled(overlay, 0, BG);
        painter.text(
            overlay.left_center(),
            egui::Align2::LEFT_CENTER,
            error,
            FontId::proportional(crate::theme::ui_size(ui.style(), 12.0)),
            crate::theme::ERROR_FG,
        );
    }
    let modifiers = ui.input(|input| input.modifiers);
    let target_at = |pos: Pos2| {
        geometry
            .cell_at(pos)
            .and_then(|cell| link_at(shadow, cell))
            .and_then(|uri| felis_client_core::ActivationTarget::parse(uri).ok())
    };
    // A link owns its whole primary-button gesture, even after the modifier
    // changes or the pointer leaves the URL. A TUI press retains its release.
    let mut link_gesture = pane.link_capture.active();
    for event in ui.input(|input| input.events.clone()) {
        if let egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
            ..
        } = event
        {
            if pressed && response.contains_pointer() {
                pane.link_capture.press(
                    target_at(pos),
                    crate::links::modifier(modifiers, cfg!(target_os = "macos")),
                    pane.mouse_capture.is_some(),
                );
                link_gesture |= pane.link_capture.active();
            } else if !pressed {
                link_gesture |= pane.link_capture.active();
                if let Some(target) = pane
                    .link_capture
                    .release(target_at(pos), response.clicked())
                {
                    pane.links.request(target);
                    ui.ctx().request_repaint();
                }
            }
        }
    }
    if response.hovered()
        && let Some(pos) = ui.input(|input| input.pointer.hover_pos())
        && let Some(uri) = geometry.cell_at(pos).and_then(|cell| link_at(shadow, cell))
    {
        {
            match felis_client_core::ActivationTarget::parse(uri) {
                Ok(target) => {
                    let hint = crate::links::hint(&target, cfg!(target_os = "macos"));
                    response.clone().on_hover_ui_at_pointer(|ui| {
                        ui.label(target.preview(240));
                        ui.label(hint);
                    });
                    if crate::links::modifier(modifiers, cfg!(target_os = "macos")) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                }
                Err(reason) => {
                    response
                        .clone()
                        .on_hover_text(format!("This link cannot be opened: {reason:?}"));
                }
            }
        }
    }
    let mouse_mode = shadow.mouse_protocol_active();
    if let Some(pos) = response.interact_pointer_pos()
        && !link_gesture
        && (!mouse_mode || ui.input(|i| i.modifiers.shift))
    {
        let cell_at = |pos: Pos2| geometry.clamped_cell(pos);
        if response.drag_started() {
            let origin = ui
                .input(|input| input.pointer.press_origin())
                .unwrap_or(pos);
            pane.selection = Some(crate::rectangle::start_selection(
                cell_at(origin),
                ui.input(|input| input.modifiers),
            ));
        }
        if response.dragged()
            && let Some(selection) = &mut pane.selection
        {
            selection.extend(cell_at(pos));
        }
        if response.triple_clicked() {
            pane.selection = Selection::line_at(screen, cell_at(pos).row);
        } else if response.double_clicked() {
            pane.selection = Selection::word_at(screen, cell_at(pos));
        } else if response.clicked() {
            pane.selection = None;
        }
    }

    let report_mouse = mouse_mode
        && (pane.mouse_capture.is_some() || (!link_gesture && !ui.input(|i| i.modifiers.shift)));
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
            pane.session
                .send(mouse_input(ui, &geometry, pos, button, action));
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
                        &geometry,
                        pos,
                        Some(button),
                        MouseAction::Press,
                    ));
                }
            } else if let Some(request) = state.scroll_request(lines) {
                pane.session.send(request);
            }
        }
    }
}

fn mouse_input(
    ui: &egui::Ui,
    geometry: &crate::render::PaneGeometry,
    pos: Pos2,
    button: Option<felis_protocol::messages::MouseButton>,
    action: felis_protocol::messages::MouseAction,
) -> InputMsg {
    use felis_protocol::messages::{InputMods, MouseEvent};
    let modifiers = ui.input(|i| i.modifiers);
    let mut mods = InputMods::empty();
    mods.set(InputMods::SHIFT, modifiers.shift);
    mods.set(InputMods::ALT, modifiers.alt);
    mods.set(InputMods::CTRL, modifiers.ctrl);
    let ([x, y], [px, py]) = geometry.mouse_position(pos, ui.ctx().pixels_per_point());
    InputMsg::Mouse(MouseEvent {
        button,
        action,
        mods,
        x,
        y,
        px,
        py,
    })
}

/// Text size is the terminal font size, set by felis's font size actions.
/// egui's built-in Cmd/Ctrl + `+` `=` `-` `0` would also rescale the whole UI
/// whenever those keys are not bound to such an action.
fn leave_zoom_keys_to_the_terminal(ctx: &egui::Context) {
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
}

/// The resize that a frame's size calls for after `sent`: the size to send
/// and whether its rows or columns differ, or `None` when nothing changed or
/// the pane requests no size. Pixel sizes change with display scale and font
/// steps even when the grid does not.
fn resize_effect(
    sent: Option<RequestedDims>,
    now: Option<RequestedDims>,
) -> Option<(RequestedDims, bool)> {
    let now = now.filter(|now| sent != Some(*now))?;
    let grid_changed = sent.is_none_or(|old| (old.rows, old.cols) != (now.rows, now.cols));
    Some((now, grid_changed))
}

/// The OSC 8 URI of the link drawn at `cell`.
fn link_at(shadow: &felis_client_core::ShadowScreen, cell: GridPos) -> Option<&str> {
    let id = shadow.screen().cell(cell.row, cell.col)?.link?;
    shadow.hyperlink(id).map(|link| link.uri.as_str())
}

/// Native clipboard replies are valid for one frame and one connection only.
fn requested_paste_is_current(
    (frame, generation, origin): (u64, u64, &Arc<Session>),
    current_frame: u64,
    focused: &Arc<Session>,
) -> bool {
    let state = focused.state.lock().unwrap();
    current_frame == frame + 1
        && Arc::ptr_eq(origin, focused)
        && state.lifecycle.is_live()
        && state.reconnects == generation
}

fn terminal_input(
    ctx: &egui::Context,
    selection: &mut Option<Selection>,
    session: &Session,
    pending_paste: &mut Option<crate::paste::PendingPaste>,
    explicit_paste: bool,
) {
    use crate::input::TerminalInput;
    let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
    for input in crate::input::translate(&events, mods, explicit_paste, cfg!(target_os = "macos")) {
        match input {
            TerminalInput::Control(bytes) => {
                send_to_program(selection, session, InputMsg::KeyBytes(bytes));
            }
            TerminalInput::Paste(text) => {
                let state = session.state.lock().unwrap();
                if !state.lifecycle.is_live() {
                    session.input_lost();
                    continue;
                }
                match crate::paste::Paste::new(
                    text,
                    state.screen.bracketed_paste(),
                    state.reconnects,
                ) {
                    crate::paste::Paste::Confirm(pending) => {
                        *pending_paste = Some(pending);
                        break;
                    }
                    crate::paste::Paste::Send(bytes) => {
                        send_to_program(selection, session, InputMsg::Paste(bytes))
                    }
                    crate::paste::Paste::Refused { bytes } => session.refuse_paste(bytes),
                }
            }
            TerminalInput::Copy => {
                if let Some(selection) = *selection {
                    let state = session.state.lock().unwrap();
                    ctx.copy_text(selection.extract_text(state.screen.screen()));
                }
            }
            TerminalInput::Commit(text) => {
                send_to_program(selection, session, InputMsg::KeyBytes(text.into_bytes()));
            }
            TerminalInput::Key(key) => send_to_program(selection, session, InputMsg::Key(key)),
        }
    }
}

/// Input for the program shows the live screen it lands on, so a selection
/// made while browsing no longer marks what is displayed.
fn send_to_program(selection: &mut Option<Selection>, session: &Session, msg: InputMsg) {
    *selection = None;
    session.send(InputMsg::Viewport {
        lines_from_bottom: 0,
        anchor: None,
    });
    session.send(msg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{PaneRemoval, Tab};
    use std::time::Duration;

    #[test]
    fn a_geometry_change_wakes_the_window_once_the_autosave_is_due() {
        let interval = Duration::from_secs(30);
        let start = std::time::Instant::now();
        let ctx = egui::Context::default();
        let mut autosave = GeometryAutosave::default();
        let mut frame = |x: f32, elapsed: u64| {
            let mut input = egui::RawInput::default();
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .outer_rect = Some(Rect::from_min_size(Pos2::new(x, 0.0), Vec2::splat(400.0)));
            let output = crate::test_support::run(&ctx, input, |ui| {
                autosave.schedule(ui.ctx(), start + Duration::from_secs(elapsed), interval)
            });
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
        };
        // egui itself repaints during its first passes.
        assert!((0..10).any(|_| frame(0.0, 0) == Duration::MAX));
        let moved = frame(50.0, 100);
        assert!(moved > interval && moved < Duration::MAX, "{moved:?}");
        // An unrelated frame before the save is due must keep the deadline.
        let pending = frame(50.0, 110);
        assert!(pending > interval - Duration::from_secs(10), "{pending:?}");
        assert!(pending < interval, "{pending:?}");
        assert_eq!(frame(50.0, 140), Duration::MAX, "the due frame saves");
        assert_eq!(frame(50.0, 200), Duration::MAX, "a saved window stays idle");
    }

    #[test]
    fn tab_drop_slot_is_the_gap_nearest_the_pointer() {
        let rects = [0.0, 100.0, 200.0]
            .map(|x| egui::Rect::from_min_size(egui::pos2(x, 0.0), Vec2::new(90.0, 30.0)));
        assert_eq!(tab_drop_slot(&rects, -20.0), 0);
        assert_eq!(tab_drop_slot(&rects, 44.0), 0);
        assert_eq!(tab_drop_slot(&rects, 46.0), 1);
        assert_eq!(tab_drop_slot(&rects, 150.0), 2);
        assert_eq!(tab_drop_slot(&rects, 260.0), 3);
        assert_eq!(tab_drop_slot(&rects, 999.0), 3);
    }

    #[test]
    fn tab_drop_indicator_stays_inside_the_visible_tab_bar() {
        let rects = [0.0, 100.0]
            .map(|x| egui::Rect::from_min_size(egui::pos2(x, 0.0), Vec2::new(90.0, 30.0)));
        let visible = egui::Rangef::new(0.0, 190.0);
        assert_eq!(tab_drop_indicator_x(&rects, 1, 10.0, visible), 95.0);
        let first = tab_drop_indicator_x(&rects, 0, 10.0, visible);
        let last = tab_drop_indicator_x(&rects, 2, 10.0, visible);
        // The 2px line must not be clipped by the scroll area.
        assert!(first - 1.0 >= visible.min, "{first}");
        assert!(last + 1.0 <= visible.max, "{last}");
    }

    #[test]
    fn search_navigation_reuses_results_until_query_or_options_change() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // Never drive this runtime: requests stay queued without contacting a daemon.
        let session = Arc::new(Session::start(
            &runtime,
            egui::Context::default(),
            Target::Local("/unused-search-test.sock".into()),
            None,
            None,
            vec![],
        ));
        {
            let mut state = session.state.lock().unwrap();
            state.lifecycle = crate::session::Lifecycle::Connected;
            state.session_id = Some(crate::session::SessionId(1));
        }
        let mut pane = Pane::from_session(session);
        pane.search_query = "needle".into();
        pane.search_submitted = Some(("needle".into(), false, true));
        {
            let mut search = pane.session.search.search.lock().unwrap();
            search.active = true;
            search.current = Some(0);
            search.hits = (0..3)
                .map(|line| crate::terminal_search::Hit {
                    line,
                    spans: vec![],
                    anchor: None,
                })
                .collect();
        }
        pane.navigate_search(true);
        assert_eq!(pane.session.search.search.lock().unwrap().current, Some(1));
        pane.navigate_search(false);
        assert_eq!(pane.session.search.search.lock().unwrap().current, Some(0));
        pane.search_regex = true;
        pane.navigate_search(true);
        let revision = {
            let search = pane.session.search.search.lock().unwrap();
            assert!(search.running);
            assert!(search.hits.is_empty());
            search.revision
        };
        pane.navigate_search(true);
        assert_eq!(
            pane.session.search.search.lock().unwrap().revision,
            revision
        );
        pane.session.search.search.lock().unwrap().running = false;
        pane.navigate_search(true);
        assert!(
            pane.session.search.search.lock().unwrap().revision > revision,
            "an empty completed search can be retried"
        );
        pane.search_query = "changed".into();
        pane.navigate_search(true);
        assert!(pane.session.search.search.lock().unwrap().revision > revision);
        pane.search_query.clear();
        pane.navigate_search(true);
        assert!(!pane.session.search.search.lock().unwrap().active);
    }

    #[test]
    fn a_resize_is_sent_for_pixel_changes_but_clears_only_on_grid_changes() {
        let dims = |rows, cols, pixel_w| RequestedDims {
            rows,
            cols,
            pixel_w,
            pixel_h: rows * 20,
        };
        let sent = Some(dims(24, 80, 800));
        assert_eq!(resize_effect(sent, sent), None);
        assert_eq!(resize_effect(sent, None), None, "too small to request");
        // A display scale change keeps the grid but not its pixels.
        assert_eq!(
            resize_effect(sent, Some(dims(24, 80, 1600))),
            Some((dims(24, 80, 1600), false))
        );
        assert_eq!(
            resize_effect(sent, Some(dims(24, 81, 810))),
            Some((dims(24, 81, 810), true))
        );
        assert_eq!(resize_effect(None, sent), Some((dims(24, 80, 800), true)));
    }

    #[test]
    fn zoom_keys_do_not_zoom_the_window() {
        let ctx = egui::Context::default();
        leave_zoom_keys_to_the_terminal(&ctx);
        // Num0 last would reset an unwanted zoom and hide it.
        for key in [
            egui::Key::Num0,
            egui::Key::Minus,
            egui::Key::Equals,
            egui::Key::Plus,
            egui::Key::Plus,
        ] {
            let _ = crate::test_support::run(
                &ctx,
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    }],
                    ..Default::default()
                },
                |_| {},
            );
        }
        assert_eq!(ctx.zoom_factor(), 1.0);
    }

    #[test]
    fn every_input_for_the_program_returns_to_the_live_view_and_clears_the_selection() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let live = InputMsg::Viewport {
            lines_from_bottom: 0,
            anchor: None,
        };
        let mut cases = vec![
            (
                egui::Event::Text("a".into()),
                egui::Modifiers::NONE,
                InputMsg::Key(felis_protocol::messages::KeyEvent {
                    key: felis_protocol::messages::Key::Character("a".into()),
                    text: Some("a".into()),
                    mods: felis_protocol::messages::KeyMods::empty(),
                    kind: felis_protocol::messages::KeyEventKind::Press,
                    location: felis_protocol::messages::KeyLocation::Standard,
                }),
            ),
            (
                egui::Event::Ime(egui::ImeEvent::Commit("あ".into())),
                egui::Modifiers::NONE,
                InputMsg::KeyBytes("あ".into()),
            ),
            (
                egui::Event::Paste("plain".into()),
                egui::Modifiers::NONE,
                InputMsg::Paste(b"plain".to_vec()),
            ),
        ];
        if !cfg!(target_os = "macos") {
            cases.push((
                egui::Event::Copy,
                egui::Modifiers::CTRL,
                InputMsg::KeyBytes(vec![3]),
            ));
        }
        for (event, modifiers, expected) in cases {
            let ctx = egui::Context::default();
            // This current-thread runtime is never driven: no daemon is contacted.
            let session = Session::start(
                &runtime,
                ctx.clone(),
                Target::Local("/unused-nekomata-test.sock".into()),
                None,
                None,
                vec![],
            );
            session.state.lock().unwrap().lifecycle = crate::session::Lifecycle::Connected;
            let mut selection = Some(Selection::new(GridPos { row: 0, col: 1 }));
            let mut pending = None;
            let _ = crate::test_support::run(
                &ctx,
                egui::RawInput {
                    events: vec![event.clone()],
                    ..Default::default()
                },
                |ui| {
                    ui.ctx().input_mut(|input| input.modifiers = modifiers);
                    terminal_input(ui.ctx(), &mut selection, &session, &mut pending, true)
                },
            );
            assert_eq!(selection, None, "{event:?}");
            assert!(
                session.drain_queued_matches(&[live.clone(), expected]),
                "{event:?}"
            );
        }
    }

    #[test]
    fn an_advanced_selection_epoch_expires_the_pane_selection() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // This current-thread runtime is never driven: no daemon is contacted.
        let session = Arc::new(Session::start(
            &runtime,
            egui::Context::default(),
            Target::Local("/unused-nekomata-test.sock".into()),
            None,
            None,
            vec![],
        ));
        let mut pane = Pane::from_session(session);
        let selection = Some(Selection::new(GridPos { row: 0, col: 1 }));
        pane.selection = selection;
        pane.expire_selection();
        assert_eq!(pane.selection, selection, "nothing moved yet");
        pane.session.state.lock().unwrap().selection_epoch += 1;
        pane.expire_selection();
        assert_eq!(pane.selection, None);
        // A selection made on the new screen stays.
        pane.selection = selection;
        pane.expire_selection();
        assert_eq!(pane.selection, selection);
    }

    #[test]
    fn panes_attaching_by_an_unresolved_prefix_keep_workspace_snapshots_valid() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // Never drive this runtime: the attach stays unresolved.
        let pane = |attach: &str| {
            Pane::from_session(Arc::new(Session::start(
                &runtime,
                egui::Context::default(),
                Target::Local("/unused-attach-test.sock".into()),
                Some(attach.into()),
                None,
                vec![],
            )))
        };
        let snapshot = |pane: &Pane| {
            let mut window = WindowLayout::default();
            let id = window.allocate_pane();
            window.open_tab(id);
            Snapshot {
                window,
                panes: [(1, pane.spec())].into(),
            }
        };
        let prefix = pane("abc");
        assert_eq!(prefix.spec().session, None);
        snapshot(&prefix).validate().unwrap();
        let resolved = crate::session::SessionId(9);
        prefix.session.state.lock().unwrap().session_id = Some(resolved);
        assert_eq!(prefix.spec().session, Some(resolved));
        let full = format!("{:032x}", 7);
        let exact = pane(&full);
        assert_eq!(exact.spec().session, Some(crate::session::SessionId(7)));
        snapshot(&exact).validate().unwrap();
    }

    #[test]
    fn pending_attaches_resolve_once_their_session_settles() {
        use crate::session::Lifecycle;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // Never drive this runtime: the test sets the session state itself.
        let session = Session::start(
            &runtime,
            egui::Context::default(),
            Target::Local("/unused-attach-test.sock".into()),
            Some("abc".into()),
            None,
            vec![],
        );
        let mut state = session.state.lock().unwrap();
        let ok = || Some(Ok("Attached pane 1".to_owned()));
        let err = |message: &str| Some(Err(message.to_owned()));
        for (lifecycle, expected) in [
            (Lifecycle::Connecting, None),
            (Lifecycle::Reconnecting, None),
            (Lifecycle::Connected, ok()),
            (Lifecycle::Detached, err("session ended during attach")),
            (Lifecycle::Exited, err("session ended during attach")),
            (
                Lifecycle::Failed("no such session".into()),
                err("no such session"),
            ),
        ] {
            state.lifecycle = lifecycle;
            assert_eq!(
                attach_result(1, &state, true, false),
                expected,
                "{:?}",
                state.lifecycle
            );
        }
        state.lifecycle = Lifecycle::Connected;
        assert_eq!(
            attach_result(1, &state, false, false),
            err("pane closed during attach")
        );
        assert_eq!(
            attach_result(1, &state, false, true),
            err("request cancelled")
        );
    }

    #[test]
    fn unsafe_paste_stops_forwarding_the_rest_of_the_input_frame() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let ctx = egui::Context::default();
        // This current-thread runtime is never driven: no daemon is contacted.
        let session = Session::start(
            &runtime,
            ctx.clone(),
            Target::Local("/unused-nekomata-test.sock".into()),
            None,
            None,
            vec![],
        );
        let session = Arc::new(session);
        session.state.lock().unwrap().lifecycle = crate::session::Lifecycle::Connected;
        assert!(requested_paste_is_current((4, 0, &session), 5, &session));
        assert!(!requested_paste_is_current((4, 0, &session), 6, &session));
        assert!(!requested_paste_is_current((4, 1, &session), 5, &session));
        let other = Session::start(
            &runtime,
            ctx.clone(),
            Target::Local("/unused-other.sock".into()),
            None,
            None,
            vec![],
        );
        let other = Arc::new(other);
        other.state.lock().unwrap().lifecycle = crate::session::Lifecycle::Connected;
        assert!(!requested_paste_is_current((4, 0, &session), 5, &other));
        session.state.lock().unwrap().lifecycle = crate::session::Lifecycle::Reconnecting;
        assert!(!requested_paste_is_current((4, 0, &session), 5, &session));
        session.state.lock().unwrap().lifecycle = crate::session::Lifecycle::Connected;
        let original = Some(Selection::new(GridPos { row: 0, col: 1 }));
        let mut selection = original;
        let mut pending = None;
        let _ = crate::test_support::run(
            &ctx,
            egui::RawInput {
                events: vec![
                    egui::Event::Paste("echo test\n".into()),
                    egui::Event::Text("must not reach shell".into()),
                ],
                ..Default::default()
            },
            |ui| {
                ui.ctx()
                    .input_mut(|input| input.modifiers = egui::Modifiers::CTRL);
                terminal_input(ui.ctx(), &mut selection, &session, &mut pending, true)
            },
        );
        assert_eq!(pending.unwrap().into_bytes(), b"echo test\n");
        assert_eq!(selection, original, "a later key would clear the selection");
    }

    #[test]
    fn link_lookup_resolves_the_spacer_of_a_wide_linked_character() {
        use felis_grid::{
            Cell, StyleTable,
            wire::{RowEncode, encode_row},
        };
        use felis_protocol::{RowPayload, messages::GridMsg};
        let mut shadow = felis_client_core::ShadowScreen::new(1, 4);
        shadow
            .apply(&GridMsg::Hyperlink {
                id: 1,
                anchor: None,
                uri: "https://example.com/".into(),
            })
            .unwrap();
        // felis-grid gives the spacer of a wide print the pen's link
        // (editing.rs, `width == 2`); this pins that nekomata reads it there.
        let link = std::num::NonZeroU16::new(1);
        let cells = [
            Cell {
                grapheme: Grapheme::Char('字'),
                link,
                ..Cell::default()
            },
            Cell {
                grapheme: Grapheme::Spacer,
                link,
                ..Cell::default()
            },
        ];
        let bytes = encode_row(
            RowEncode {
                cells: &cells,
                pad_to: 4,
                sized_cells: &[],
                soft_wrap_continued: false,
            },
            &StyleTable::new(),
        )
        .unwrap();
        shadow
            .apply(&GridMsg::RowDelta {
                rows: vec![(0, RowPayload(bytes))],
            })
            .unwrap();
        for col in [0, 1] {
            assert_eq!(
                link_at(&shadow, GridPos { row: 0, col }),
                Some("https://example.com/"),
                "column {col}"
            );
        }
        assert_eq!(link_at(&shadow, GridPos { row: 0, col: 2 }), None);
    }

    #[test]
    fn a_paste_while_disconnected_is_reported_as_lost() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let ctx = egui::Context::default();
        // This current-thread runtime is never driven: no daemon is contacted.
        let session = Session::start(
            &runtime,
            ctx.clone(),
            Target::Local("/unused-nekomata-test.sock".into()),
            None,
            None,
            vec![],
        );
        let mut pending = None;
        let _ = crate::test_support::run(
            &ctx,
            egui::RawInput {
                events: vec![egui::Event::Paste("echo test".into())],
                ..Default::default()
            },
            |ui| terminal_input(ui.ctx(), &mut None, &session, &mut pending, true),
        );
        assert!(pending.is_none());
        assert_eq!(
            session.input_notice(),
            Some(crate::session::InputNotice::Lost)
        );
    }

    #[test]
    fn copying_soft_wrapped_text_keeps_one_logical_line() {
        use felis_grid::{
            Cell, StyleTable,
            wire::{RowEncode, encode_row},
        };
        use felis_protocol::{RowPayload, messages::GridMsg};
        let mut shadow = felis_client_core::ShadowScreen::new(2, 4);
        for (row, text) in [(0, "abcd"), (1, "EFGH")] {
            let cells: Vec<Cell> = text
                .bytes()
                .map(|b| Cell {
                    grapheme: Grapheme::Ascii(b),
                    ..Cell::default()
                })
                .collect();
            let bytes = encode_row(
                RowEncode {
                    cells: &cells,
                    pad_to: 4,
                    sized_cells: &[],
                    soft_wrap_continued: row == 1,
                },
                &StyleTable::new(),
            )
            .unwrap();
            shadow
                .apply(&GridMsg::RowDelta {
                    rows: vec![(row, RowPayload(bytes))],
                })
                .unwrap();
        }
        let mut selection = Selection::new(GridPos { row: 0, col: 0 });
        selection.extend(GridPos { row: 1, col: 3 });
        assert_eq!(selection.extract_text(shadow.screen()), "abcdEFGH");
    }

    #[test]
    #[ignore = "requires isolated nekomata and felis daemons; run scripts/test-live.sh via nix develop"]
    fn live_idle_window_schedules_no_repaint() {
        use std::{sync::mpsc, time::Instant};
        crate::test_support::private_runtime_dir();
        let ctx = egui::Context::default();
        let (tx, repaints) = mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let mut app = App::connect(
            tokio::runtime::Runtime::new().unwrap(),
            &ctx,
            Default::default(),
        )
        .unwrap();
        // Without a GPU host the pane renderer cannot be created, so no blink
        // deadline is requested and, once connected, nothing remains to repaint for.
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            let output =
                crate::test_support::run(&ctx, egui::RawInput::default(), |ui| app.show(ui));
            let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            // Earlier live tests may leave this workspace with panes whose
            // sessions are gone; those settle on an error instead.
            let settled = !app.panes.is_empty()
                && app.panes.values().all(|pane| {
                    let state = pane.session.state.lock().unwrap();
                    state.lifecycle.is_live() || state.error().is_some()
                });
            if settled && delay == Duration::MAX {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "an idle window kept scheduling repaints every {delay:?} (panes settled: {settled})"
            );
            let _ = repaints.recv_timeout(delay.min(Duration::from_millis(50)));
        }
        eframe::App::on_exit(&mut app);
    }

    #[test]
    #[ignore = "requires isolated nekomata and felis daemons; run scripts/test-live.sh via nix develop"]
    fn live_gui_restart_and_switch_preserve_workspaces_and_shells() {
        use std::{sync::mpsc, time::Instant};
        crate::test_support::private_runtime_dir();
        crate::test_support::private_felis_socket();
        let executable = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("nekomata");
        // daemon::connect starts the daemon from current_exe(), which is this
        // test harness rather than nekomata, so start it from the real binary.
        let output = std::process::Command::new(&executable)
            .args(["workspaces", "list"])
            .output()
            .expect("build the nekomata binary first: cargo build --bin nekomata");
        assert!(output.status.success(), "start nekomata daemon: {output:?}");
        let ctx = egui::Context::default();
        let (tx, repaints) = mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let pump = |app: &mut App, done: &dyn Fn(&App) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(25);
            loop {
                let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |_| {});
                app.workspace_events(&ctx);
                app.poll_workspace();
                app.control_requests(&ctx);
                app.save_workspace(&ctx);
                if done(app) {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "GUI workspace test timed out: {:?}",
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
                    .all(|p| p.session.state.lock().unwrap().lifecycle.is_live())
        };
        let mut config = felis_client_core::config::EffectiveConfig::default();
        config.font.size_px = Some(21.0);
        crate::terminal_config::install(&ctx, config);
        let mut app = App::connect(
            tokio::runtime::Runtime::new().unwrap(),
            &ctx,
            Default::default(),
        )
        .unwrap();
        pump(&mut app, &connected);
        assert_eq!(app.window.font_size, 21.0);
        app.window.font_size = 30.0;
        app.action(&ctx, Action::FontReset);
        assert_eq!(app.window.font_size, 21.0);
        app.action(&ctx, Action::OpenWorkspaces);
        app.action(&ctx, Action::AttachSession);
        assert!(
            matches!(app.overlay, Some(Overlay::SessionPicker)),
            "opening an overlay replaces the open one"
        );
        app.overlay = None;
        // Pasting from the palette must reach the terminal, whose paste the
        // search field would otherwise discard by blocking terminal input.
        app.open_search();
        app.action(&ctx, Action::Paste);
        assert!(!app.searching());
        assert!(app.paste_requested.take().is_some());
        app.action(&ctx, Action::Split(Axis::Horizontal));
        app.action(&ctx, Action::NewTab);
        pump(&mut app, &connected);
        app.window.active = 0;
        app.window.tabs[0].focus(2);
        if let Layout::Split { ratio, .. } = &mut app.window.tabs[0].layout {
            *ratio = 0.3;
        }
        app.window.font_size = 18.0;
        let mut before = app.snapshot();
        let workspace_id = app.workspace_id;
        app.save_workspace(&ctx);
        eframe::App::on_exit(&mut app);
        drop(app);

        // A restarted GUI must use the newly loaded terminal configuration,
        // even though the retained workspace was saved at another zoom.
        let mut config = felis_client_core::config::EffectiveConfig::default();
        config.font.size_px = Some(30.0);
        crate::terminal_config::install(&ctx, config);
        let mut replacement = App::connect(
            tokio::runtime::Runtime::new().unwrap(),
            &ctx,
            Default::default(),
        )
        .unwrap();
        pump(&mut replacement, &connected);
        assert_eq!(replacement.window.font_size, 30.0);
        before.window.font_size = 30.0;
        assert_eq!(replacement.workspace_id, workspace_id);
        assert_eq!(replacement.snapshot(), before);
        assert_eq!(replacement.window.tabs.len(), 2);
        assert_eq!(replacement.panes.len(), 3);

        let id = workspace_id.unwrap();
        let instance = replacement.daemon_instance.clone().unwrap();
        // A shell carries the IDs it started with; route as one would.
        let route = |app: &mut App, instance: String| {
            let repaint = ctx.clone();
            let executable = executable.clone();
            let command = app.runtime.spawn(async move {
                let result = tokio::process::Command::new(executable)
                    .args(["panes", "list"])
                    .env(control::WORKSPACE_ENV, format!("{id:032x}"))
                    .env(control::DAEMON_INSTANCE_ENV, instance)
                    .env("NEKOMATA_WINDOW_ID", u32::MAX.to_string())
                    .output()
                    .await
                    .unwrap();
                repaint.request_repaint();
                result
            });
            pump(app, &|_| command.is_finished());
            app.runtime.block_on(command).unwrap()
        };
        let output = route(&mut replacement, instance);
        assert!(
            output.status.success(),
            "restored shell CLI routing failed: {:?}",
            output
        );
        // A shell from an earlier daemon must not reach the window that now
        // holds a workspace with the same ID.
        let output = route(&mut replacement, "0123456789abcdef".into());
        assert!(!output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("restarted"),
            "{output:?}"
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
        let output = command(&mut replacement, vec!["workspaces".into(), "new".into()]);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            replacement.window.font_size, 30.0,
            "new workspaces use terminal config"
        );
        let other = replacement.workspace_id.unwrap();
        assert_ne!(Some(other), workspace_id);
        assert_eq!(replacement.panes.len(), 1);
        // A new workspace's shell starts only once the switch has committed.
        pump(&mut replacement, &connected);
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
                "workspaces".into(),
                "attach".into(),
                format!("{:032x}", workspace_id.unwrap()),
            ],
        );
        assert!(output.status.success(), "{output:?}");
        assert_eq!(replacement.snapshot(), before);

        // No pane of the destination can connect, so the switch is cancelled
        // and the window keeps its workspace and its source connections.
        let missing =
            Target::Local(crate::test_support::private_runtime_dir().join("missing-felis.sock"));
        let mut broken = before.clone();
        for pane in broken.panes.values_mut() {
            pane.target = missing.clone();
        }
        replacement.runtime.block_on(async {
            let mut client = daemon::connect().await.unwrap();
            let Payload::Workspace { id, .. } =
                daemon::exchange(&mut client, daemon::Request::Open { window: u32::MAX })
                    .await
                    .unwrap()
            else {
                panic!("expected workspace")
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
            vec![
                "workspaces".into(),
                "attach".into(),
                format!("{other:032x}"),
            ],
        );
        assert!(!output.status.success(), "{output:?}");
        assert_eq!(replacement.workspace_id, workspace_id);
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
            let Payload::Workspaces(workspaces) =
                daemon::exchange(&mut client, daemon::Request::List)
                    .await
                    .unwrap()
            else {
                panic!("expected workspaces")
            };
            assert!(
                workspaces
                    .iter()
                    .find(|workspace| workspace.id == other)
                    .unwrap()
                    .window
                    .is_none()
            );
        });

        // A fresh switch directly after cancellation must not consume the old
        // cancellation acknowledgement as its own commit.
        let output = command(&mut replacement, vec!["workspaces".into(), "new".into()]);
        assert!(output.status.success(), "{output:?}");
        pump(&mut replacement, &connected);
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
                "workspaces".into(),
                "attach".into(),
                format!("{:032x}", workspace_id.unwrap()),
            ],
        );
        assert!(output.status.success(), "{output:?}");
        assert_eq!(replacement.snapshot(), before);

        // The destination shares two live shells with the source and its
        // other pane is unavailable. The switch still commits: the live panes
        // reuse the source connections and the unavailable one shows its error.
        let mut partly = before.clone();
        partly.panes.get_mut(&2).unwrap().target = missing.clone();
        replacement.runtime.block_on(async {
            let mut client = daemon::connect().await.unwrap();
            daemon::exchange(&mut client, daemon::Request::Open { window: u32::MAX })
                .await
                .unwrap();
            daemon::exchange(
                &mut client,
                daemon::Request::Prepare(daemon::Selection::Id(other)),
            )
            .await
            .unwrap();
            daemon::exchange(
                &mut client,
                daemon::Request::Finish {
                    snapshot: Some(partly),
                },
            )
            .await
            .unwrap();
            daemon::exchange(&mut client, daemon::Request::Detach)
                .await
                .unwrap();
        });
        let original_connection = replacement.panes[&1].session.clone();
        let output = command(
            &mut replacement,
            vec![
                "workspaces".into(),
                "attach".into(),
                format!("{other:032x}"),
            ],
        );
        assert!(output.status.success(), "{output:?}");
        assert_eq!(replacement.workspace_id, Some(other));
        assert!(Arc::ptr_eq(
            &replacement.panes[&1].session,
            &original_connection
        ));
        drop(original_connection);
        {
            let state = replacement.panes[&2].session.state.lock().unwrap();
            assert!(state.error().is_some() && !state.lifecycle.is_live());
        }
        assert_eq!(replacement.snapshot().panes[&2].target, missing);
        let output = command(
            &mut replacement,
            vec![
                "workspaces".into(),
                "attach".into(),
                format!("{:032x}", workspace_id.unwrap()),
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
        eframe::App::on_exit(&mut replacement);
        drop(replacement);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut ops = felis_client_core::connect(
                std::path::PathBuf::from(crate::test_support::private_felis_socket()),
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
    #[ignore = "requires isolated nekomata and felis daemons; run scripts/test-live.sh via nix develop"]
    fn live_window_recovers_its_layout_after_the_nekomata_daemon_dies() {
        use std::{sync::mpsc, time::Instant};
        crate::test_support::private_runtime_dir();
        crate::test_support::private_felis_socket();
        let ctx = egui::Context::default();
        let (tx, repaints) = mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let pump = |app: &mut App, done: &dyn Fn(&App) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(25);
            while !done(app) {
                let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |_| {});
                app.workspace_events(&ctx);
                app.poll_workspace();
                app.resume_workspace(&ctx);
                app.control_requests(&ctx);
                app.save_workspace(&ctx);
                assert!(
                    Instant::now() < deadline,
                    "daemon recovery test timed out: {:?}",
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
                    .all(|p| p.session.state.lock().unwrap().lifecycle.is_live())
        };
        let mut app = App::connect(
            tokio::runtime::Runtime::new().unwrap(),
            &ctx,
            Default::default(),
        )
        .unwrap();
        // Start from a workspace of one fresh pane, whatever earlier tests
        // left in the daemon.
        pump(&mut app, &|app| app.workspace_id.is_some());
        app.begin_workspace(None, None);
        pump(&mut app, &|app| {
            app.pending_workspace.is_none() && connected(app)
        });
        assert_eq!(app.window.tabs.len(), 1, "{:?}", app.workspace_error);
        let shell = app.panes.values().next().unwrap().session.clone();
        // Nothing is in flight, so only the idle watch can notice the loss.
        pump(&mut app, &|app| {
            app.workspace.as_ref().map(WorkspaceClient::saved)
                == Some(crate::workspace_client::Saved {
                    acknowledged: Some(app.snapshot()),
                    unconfirmed: vec![],
                })
        });
        // The test binary cannot start a daemon itself; the real one can.
        let executable = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("nekomata");
        // Only the isolated daemon behind NEKOMATA_RUNTIME_DIR is killed.
        let pid = app.runtime.block_on(async {
            let stream = tokio::net::UnixStream::connect(daemon::socket().unwrap())
                .await
                .unwrap();
            stream.peer_cred().unwrap().pid().unwrap()
        });
        let killed = std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap();
        assert!(killed.success());
        // The window notices while idle and reports it.
        pump(&mut app, &|app| {
            app.daemon_lost() && app.workspace_error.is_some()
        });
        app.action(&ctx, Action::NewTab);
        let started = std::process::Command::new(&executable)
            .args(["workspaces", "list"])
            .output()
            .unwrap();
        assert!(started.status.success(), "{started:?}");
        app.reconnect.retry_now();
        pump(&mut app, &|app| {
            !app.daemon_lost() && app.workspace_error.is_none() && connected(app)
        });
        assert!(
            Arc::ptr_eq(&app.panes.values().next().unwrap().session, &shell),
            "recovery keeps the window's shells"
        );
        let id = app.workspace_id.unwrap();
        let summary = |app: &App| {
            app.runtime.block_on(async {
                let mut client = daemon::connect().await.unwrap();
                let Payload::Workspaces(workspaces) =
                    daemon::exchange(&mut client, daemon::Request::List)
                        .await
                        .unwrap()
                else {
                    panic!("expected workspaces")
                };
                workspaces.into_iter().find(|w| w.id == id).unwrap()
            })
        };
        let resumed = summary(&app);
        assert_eq!(resumed.window, Some(app.control.window));
        assert_eq!(resumed.tabs, 2, "the layout made while disconnected");
        // Later layout changes keep reaching the new daemon.
        app.action(&ctx, Action::NewTab);
        pump(&mut app, &connected);
        pump(&mut app, &|app| {
            app.last_snapshot.as_ref() == Some(&app.snapshot())
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while summary(&app).tabs != 3 {
            assert!(Instant::now() < deadline, "later layout was not saved");
            std::thread::sleep(Duration::from_millis(50));
        }
        let ids: Vec<_> = app
            .panes
            .values()
            .map(|p| p.session.state.lock().unwrap().session_id.unwrap())
            .collect();
        drop(shell);
        eframe::App::on_exit(&mut app);
        drop(app);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut ops = felis_client_core::connect(
                std::path::PathBuf::from(crate::test_support::private_felis_socket()),
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

        crate::test_support::private_runtime_dir();
        let socket = crate::test_support::private_felis_socket();
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
            daemon_instance: None,
            workspace_error: None,
            last_snapshot: None,
            pending_workspace: None,
            reconnect: Default::default(),
            paste_requested: None,
            workspace_roster: None,
            grown_window: None,
            runtime,
            control,
            pending_attach: BTreeMap::new(),
            panes: BTreeMap::new(),
            window: WindowLayout {
                font_size: 15.0,
                ..Default::default()
            },
            picker: SessionPicker::default(),
            shortcuts: Shortcuts::default(),
            overlay: None,
            geometry: GeometryAutosave::default(),
        };
        app.add_tab(&ctx, None);
        let pump = |app: &mut App, done: &dyn Fn(&App) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(25);
            loop {
                let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |_| {});
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
            app.panes[&1]
                .session
                .state
                .lock()
                .unwrap()
                .lifecycle
                .is_live()
        });
        app.action(&ctx, Action::Search);
        assert!(app.searching());
        let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| app.search_bar(ui));
        assert!(
            ctx.egui_wants_keyboard_input(),
            "search field must own typing"
        );
        app.open_search();
        let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| app.show(ui));
        let frame = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| app.show(ui));
        assert!(
            frame.platform_output.ime.is_some(),
            "search IME must survive the complete UI frame"
        );
        assert_ne!(
            frame.platform_output.ime.as_ref().unwrap().purpose,
            egui::IMEPurpose::Terminal
        );
        let _ = crate::test_support::run(
            &ctx,
            egui::RawInput {
                events: vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: "にほん".into(),
                    active_range_chars: None,
                })],
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        let frame = crate::test_support::run(
            &ctx,
            egui::RawInput {
                events: vec![egui::Event::Ime(egui::ImeEvent::Commit("日本".into()))],
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        assert_eq!(app.panes[&1].search_query, "日本");
        assert!(frame.platform_output.ime.is_some());
        let _ = crate::test_support::run(
            &ctx,
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| app.search_bar(ui),
        );
        assert!(!app.searching());
        assert!(!app.panes[&1].session.search.search.lock().unwrap().active);

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
            Some(crate::session::SessionId(remote))
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
            Some(crate::session::SessionId(remote))
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
                    host: "nekomata-test-host".parse().unwrap(),
                    socket: Some(remote_socket),
                }
            );
            assert_eq!(
                app.panes[&1].session.state.lock().unwrap().session_id,
                Some(crate::session::SessionId(remote))
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
                Some(crate::session::SessionId(remote))
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
            state.lifecycle.is_live() || state.lifecycle.failure().is_some()
        });
        assert!(
            app.panes[&2]
                .session
                .state
                .lock()
                .unwrap()
                .lifecycle
                .is_live()
        );
        app.runtime.block_on(async {
            let mut ops = felis_client_core::connect(std::path::Path::new(&socket), Offer::ops())
                .await
                .unwrap();
            assert!(
                ops.list_sessions()
                    .await
                    .unwrap()
                    .iter()
                    .any(|session| crate::session::SessionId(session.id) == original),
                "switching panes killed the old shell"
            );
            ops.destroy_session(format!("{original:032x}"))
                .await
                .unwrap();
        });
    }

    #[test]
    fn input_is_blocked_by_any_blocker() {
        assert!(!InputBlockers::default().any());
        for blockers in [
            InputBlockers {
                overlay: true,
                ..Default::default()
            },
            InputBlockers {
                searching: true,
                ..Default::default()
            },
            InputBlockers {
                paste_pending: true,
                ..Default::default()
            },
            InputBlockers {
                switching: true,
                ..Default::default()
            },
        ] {
            assert!(blockers.any());
        }
    }

    #[test]
    fn search_leaves_escape_to_a_modal_above_it() {
        let escape = || egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let _ = crate::test_support::run(&ctx, escape(), |ui| {
            assert!(consume_search_escape(ui.ctx()));
        });
        let modal =
            |ui: &mut egui::Ui| egui::Modal::new(egui::Id::unique("modal")).show(ui.ctx(), |_| {});
        let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
            modal(ui);
        });
        let _ = crate::test_support::run(&ctx, escape(), |ui| {
            assert!(!consume_search_escape(ui.ctx()));
            assert!(modal(ui).should_close());
        });
    }

    #[test]
    fn terminal_space_does_not_activate_a_previously_focused_menu() {
        let ctx = egui::Context::default();
        let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |root_ui| {
            egui::CentralPanel::default().show(root_ui, |ui| {
                ui.menu_button("Options", |_| {}).response.request_focus();
            });
        });
        let _ = crate::test_support::run(
            &ctx,
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
            |root_ui| {
                let context = root_ui.ctx().clone();
                let ctx = &context;
                focus_terminal(ctx);
                egui::CentralPanel::default().show(root_ui, |ui| {
                    assert!(ui.menu_button("Options", |_| {}).inner.is_none());
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
        assert!(layout.split(1, 2, Axis::Horizontal));
        assert!(layout.split(2, 3, Axis::Vertical));
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
        assert_eq!(tab.remove_pane(3), PaneRemoval::Removed);
        assert_eq!(tab.focused, 2);
        assert_eq!(tab.remove_pane(2), PaneRemoval::Removed);
        assert_eq!(tab.focused, 1);
        assert_eq!(tab.remove_pane(1), PaneRemoval::LastPane);
    }

    #[test]
    fn removing_a_pane_outside_the_tab_reports_it_missing() {
        let mut tab = split_tab();
        tab.focus(2);
        let before = tab.clone();
        assert_eq!(tab.remove_pane(9), PaneRemoval::Missing);
        assert_eq!(tab, before);
    }

    #[test]
    fn closing_background_panes_prunes_only_their_tabs_history() {
        let mut tab = split_tab();
        let mut other = split_tab();
        tab.focus(2);
        tab.focus(3);
        other.focus(2);
        assert_eq!(tab.remove_pane(2), PaneRemoval::Removed);
        assert_eq!(tab.focused, 3);
        assert_eq!(tab.remove_pane(3), PaneRemoval::Removed);
        assert_eq!(tab.focused, 1);
        assert_eq!(other.focused, 2);
        assert_eq!(other.focus_history, vec![1, 2]);
    }
}
