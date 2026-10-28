//! One ordered, bounded input channel and IPC connection per pane.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, bail};
use eframe::egui;
use felis_client_core::{
    Carrier, CarrierConnection, Offer, OutgoingFrame, OutgoingQueue, RemoteSpawn, ShadowScreen,
    connect_carrier, connect_or_spawn_daemon,
};
use felis_protocol::{
    MessageKind,
    messages::{GridMsg, InputMsg},
};
use felis_transport::{Delivery, Incoming};
use std::{path::PathBuf, time::Duration};
use tokio::sync::Notify;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "StoredTarget")]
pub enum Target {
    Local(PathBuf),
    Ssh {
        host: crate::ssh::Host,
        socket: Option<String>,
    },
}

// Retained workspaces written before explicit remote sockets stored SSH hosts
// as strings. Accept that representation alongside the new host/socket pair.
#[derive(serde::Deserialize)]
enum StoredTarget {
    Local(PathBuf),
    Ssh(StoredSsh),
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StoredSsh {
    Host(String),
    Socket {
        host: String,
        socket: Option<String>,
    },
}

impl TryFrom<StoredTarget> for Target {
    type Error = anyhow::Error;

    fn try_from(value: StoredTarget) -> Result<Self> {
        let (host, socket) = match value {
            StoredTarget::Local(path) => return Ok(Self::Local(path)),
            StoredTarget::Ssh(StoredSsh::Host(host)) => (host, None),
            StoredTarget::Ssh(StoredSsh::Socket { host, socket }) => (host, socket),
        };
        Ok(Self::Ssh {
            host: host.parse()?,
            socket,
        })
    }
}

impl Target {
    pub fn default_local() -> Result<Self> {
        Ok(Self::Local(
            felis_client_core::local_socket::resolve_local_socket(None)?,
        ))
    }

    pub(crate) fn carrier(&self) -> Carrier {
        match self {
            Self::Local(path) => Carrier::Local(path.clone().into()),
            Self::Ssh { host, .. } => Carrier::Ssh {
                destination: host.to_string(),
                // SSH panes connect through ssh::connect, not this carrier. felis
                // splices these before the destination, so end options there
                // should a caller ever hand it over.
                ssh_args: vec!["-oBatchMode=yes".into(), "--".into()],
            },
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Local(path) => path.display().to_string(),
            Self::Ssh { host, socket } => match socket {
                Some(socket) => format!("ssh:{host}:{socket}"),
                None => format!("ssh:{host}"),
            },
        }
    }
}

pub struct State {
    pub screen: ShadowScreen,
    pub effective_minor: u16,
    pub pull: felis_client_core::pull::PullScheduler,
    pub redraw: felis_client_core::redraw::RedrawScheduler,
    pub colors: crate::colors::Colors,
    pub images: felis_client_core::ImageShadow,
    pub session_id: Option<u128>,
    pub error: Option<String>,
    pub connected: bool,
    pub reconnects: u64,
    pub reconnecting: bool,
    pub detached: bool,
    pub exited: bool,
    pub clipboard: Option<String>,
    /// Advances when the screen switches, resizes or is rehydrated, or the
    /// viewport moves to other lines; a selection made before is stale.
    pub selection_epoch: u64,
}

impl State {
    pub fn scroll_action(&self, step: felis_client_core::ScrollStep) -> Option<InputMsg> {
        use felis_client_core::ScrollStep;
        let half_page = i32::from(self.screen.screen().rows() / 2).max(1);
        match step {
            ScrollStep::End => Some(InputMsg::Viewport {
                lines_from_bottom: 0,
                anchor: None,
            }),
            ScrollStep::Home => self.scroll_request(i32::MAX),
            ScrollStep::LineUp => self.scroll_request(1),
            ScrollStep::LineDown => self.scroll_request(-1),
            ScrollStep::HalfPageUp => self.scroll_request(half_page),
            ScrollStep::HalfPageDown => self.scroll_request(-half_page),
        }
    }

    pub fn scroll_request(&self, rows: i32) -> Option<InputMsg> {
        if !self.connected || self.screen.alt_screen() {
            return None;
        }
        felis_client_core::viewport::step_request(
            rows,
            self.effective_minor,
            self.screen.viewport(),
            self.screen.viewport_anchor(),
            self.screen
                .viewport_max()
                .saturating_sub(u32::from(self.screen.screen().rows())),
        )
    }

    /// `clipboard` decides whether an OSC 52 write reaches the system
    /// clipboard (see [`system_clipboard_text`]).
    fn apply_grid(
        &mut self,
        msg: &GridMsg,
        clipboard: &felis_client_core::config::ClipboardConfig,
    ) -> Result<()> {
        self.pull.on_frame(msg);
        let alt_screen_before = self.screen.alt_screen();
        let viewport_before = (self.screen.viewport(), self.screen.viewport_anchor());
        let size = |screen: &ShadowScreen| (screen.screen().rows(), screen.screen().cols());
        let size_before = size(&self.screen);
        self.screen.apply(msg)?;
        // A rehydrate (after reconnecting) resends every cell, possibly of a
        // screen that changed while this client was away; a resize, also by
        // another client of the session, reflows them.
        if matches!(msg, GridMsg::RehydrateBegin)
            || size(&self.screen) != size_before
            || crate::selection::grid_change_invalidates_selection(
                msg,
                alt_screen_before,
                self.screen.alt_screen(),
                viewport_before,
            )
        {
            self.selection_epoch += 1;
        }
        self.redraw.request();
        self.colors.apply_grid(msg);
        if matches!(msg, GridMsg::RehydrateBegin) {
            self.images = felis_client_core::ImageShadow::new();
        }
        if let GridMsg::ClipboardSet { write } = msg
            && let Some(text) = system_clipboard_text(write, clipboard)
        {
            self.clipboard = Some(text);
        }
        Ok(())
    }

    pub fn render_ready(&self) -> bool {
        self.connected && !self.screen.is_rehydrating() && !self.pull.cycle_open()
    }
}

/// Queued frames, with whether losing each one loses shell input.
#[derive(Default)]
struct Pending {
    frames: OutgoingQueue,
    shell_input: std::collections::VecDeque<bool>,
}

struct InputQueue {
    pending: Mutex<Pending>,
    ready: Notify,
    accepting: AtomicBool,
    notice: Mutex<Option<InputNotice>>,
}

/// Why some input did not reach the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputNotice {
    Lost,
    PasteRefused { bytes: usize },
}

impl InputNotice {
    pub fn message(self) -> String {
        match self {
            Self::Lost => {
                "Some input was not delivered. Check the terminal before continuing.".into()
            }
            Self::PasteRefused { bytes } => format!(
                "Paste refused: {bytes} bytes is over the {}-byte limit.",
                felis_protocol::messages::MAX_PASTE_BYTES
            ),
        }
    }
}

impl Default for InputQueue {
    fn default() -> Self {
        Self {
            pending: Mutex::default(),
            ready: Notify::new(),
            accepting: AtomicBool::new(true),
            notice: Mutex::default(),
        }
    }
}

/// Input the shell or its program receives. Everything else describes
/// window state (size, focus, viewport, frame pacing) that the pane sends
/// again on every new connection, so refusing it loses nothing.
fn is_shell_input(msg: &InputMsg) -> bool {
    match msg {
        InputMsg::Key(_) | InputMsg::KeyBytes(_) | InputMsg::Paste(_) => true,
        InputMsg::Mouse(_) => felis_client_core::coalesce_key(msg).is_none(),
        _ => false,
    }
}

impl InputQueue {
    fn push(&self, msg: &InputMsg) -> Result<()> {
        let shell_input = is_shell_input(msg);
        let frame = OutgoingFrame::input(msg);
        let mut pending = self.pending.lock().unwrap();
        let result = frame
            .map_err(anyhow::Error::from)
            .and_then(|frame| self.enqueue(&mut pending, frame, shell_input));
        // Updated before releasing the queue, so a close or write failure
        // of this frame cannot be overwritten by its own acceptance.
        if shell_input {
            *self.notice.lock().unwrap() = result.is_err().then_some(InputNotice::Lost);
        }
        result
    }

    fn enqueue(
        &self,
        pending: &mut Pending,
        frame: OutgoingFrame,
        shell_input: bool,
    ) -> Result<()> {
        if !self.accepting.load(Ordering::Acquire) {
            bail!("connection is not accepting input");
        }
        let queued = pending.frames.len();
        pending
            .frames
            .push(frame)
            .map_err(|full| anyhow::anyhow!("input queue exceeded {} bytes", full.cap))?;
        // Coalescing replaces a frame in place; only an appended frame
        // gets its own entry.
        if pending.frames.len() > queued {
            pending.shell_input.push_back(shell_input);
        }
        self.ready.notify_one();
        Ok(())
    }

    /// Stops accepting input and discards the backlog, reporting lost
    /// input when the backlog held any.
    fn close(&self) {
        let mut pending = self.pending.lock().unwrap();
        self.accepting.store(false, Ordering::Release);
        if pending.shell_input.contains(&true) {
            self.mark_lost();
        }
        *pending = Pending::default();
    }

    fn reopen(&self) {
        self.accepting.store(true, Ordering::Release);
    }

    fn mark_lost(&self) {
        self.set_notice(InputNotice::Lost);
    }

    fn set_notice(&self, notice: InputNotice) {
        *self.notice.lock().unwrap() = Some(notice);
    }

    fn notice(&self) -> Option<InputNotice> {
        *self.notice.lock().unwrap()
    }

    #[cfg(test)]
    fn input_lost(&self) -> bool {
        self.notice() == Some(InputNotice::Lost)
    }

    /// The oldest frame, and whether it carries shell input.
    async fn next(&self) -> (OutgoingFrame, bool) {
        loop {
            let notified = self.ready.notified();
            {
                let mut pending = self.pending.lock().unwrap();
                if let Some(frame) = pending.frames.pop() {
                    let shell_input = pending.shell_input.pop_front().unwrap_or(false);
                    return (frame, shell_input);
                }
            }
            notified.await;
        }
    }
}

pub struct Session {
    pub state: Arc<Mutex<State>>,
    pub target: Target,
    pub attachment: Option<String>,
    pub search: crate::terminal_search::Tools,
    tx: Arc<InputQueue>,
    task: tokio::task::JoinHandle<()>,
}

impl Session {
    #[cfg(test)]
    pub fn new(
        runtime: &tokio::runtime::Runtime,
        ctx: egui::Context,
        attach: Option<String>,
        cwd: Option<String>,
    ) -> Self {
        Self::start(
            runtime,
            ctx,
            Target::default_local().expect("resolve local daemon socket"),
            attach,
            cwd,
            vec![],
        )
    }

    pub fn start(
        runtime: &tokio::runtime::Runtime,
        ctx: egui::Context,
        target: Target,
        attach: Option<String>,
        cwd: Option<String>,
        env: Vec<(String, String)>,
    ) -> Self {
        let state = Arc::new(Mutex::new(State {
            screen: ShadowScreen::new(24, 80),
            effective_minor: 0,
            pull: felis_client_core::pull::PullScheduler::new(matches!(target, Target::Local(_))),
            redraw: felis_client_core::redraw::RedrawScheduler::new(),
            colors: crate::colors::Colors::new(felis_render_wgpu::Renderer::theme_from_config(
                &crate::terminal_config::renderer(&crate::terminal_config::get(&ctx)),
            )),
            images: felis_client_core::ImageShadow::new(),
            session_id: None,
            error: None,
            connected: false,
            reconnects: 0,
            reconnecting: false,
            detached: false,
            exited: false,
            clipboard: None,
            selection_epoch: 0,
        }));
        let search =
            crate::terminal_search::Tools::new(runtime, ctx.clone(), target.clone(), state.clone());
        let tx = Arc::new(InputQueue::default());
        let rx = tx.clone();
        let attachment = attach.clone();
        let shared = state.clone();
        let destination = target.clone();
        let task = runtime.spawn(async move {
            let result = run(
                shared.clone(),
                ctx.clone(),
                rx.clone(),
                destination,
                attach,
                cwd,
                env,
            )
            .await;
            rx.close();
            let mut state = shared.lock().unwrap();
            state.connected = false;
            state.reconnecting = false;
            state.error = if state.detached || state.exited {
                None
            } else {
                Some(match result {
                    Ok(()) => {
                        "Session disconnected. The shell may still be running in felis.".into()
                    }
                    Err(err) => format!("{err:#}"),
                })
            };
            ctx.request_repaint();
        });
        Self {
            state,
            tx,
            task,
            target,
            attachment,
            search,
        }
    }

    /// Called once per visible pane frame that painted without error; `rendered` is whether this frame
    /// submitted new terminal content to the GPU rather than reusing the last
    /// texture. Only a rendered frame may pull (felis `PullScheduler::after_frame`).
    pub fn frame_submitted(&self, state: &mut State, rendered: bool) {
        if state.render_ready() && state.pull.after_frame(rendered) {
            self.send(InputMsg::NextGridFrame);
        }
    }

    /// Drains the input queue and reports whether it held exactly
    /// `expected`, in order. A test drives no runtime, so nothing else
    /// drains it.
    #[cfg(test)]
    pub fn drain_queued_matches(&self, expected: &[InputMsg]) -> bool {
        let mut pending = self.tx.pending.lock().unwrap();
        // Compare wire frames: queueing later frames rewrites coalescing state.
        let queued: Vec<_> = std::iter::from_fn(|| pending.frames.pop())
            .map(|frame| frame.frame().clone())
            .collect();
        let expected: Vec<_> = expected
            .iter()
            .map(|msg| OutgoingFrame::input(msg).unwrap().frame().clone())
            .collect();
        queued == expected
    }

    pub fn send(&self, msg: InputMsg) {
        let _ = self.tx.push(&msg);
    }

    /// Why recent input may not have reached the shell, until shell
    /// input is accepted again.
    pub fn input_notice(&self) -> Option<InputNotice> {
        self.tx.notice()
    }

    /// Reports shell input dropped before it reached the queue.
    pub fn input_lost(&self) {
        self.tx.mark_lost();
    }

    /// Reports a paste the pane would not send.
    pub fn refuse_paste(&self, bytes: usize) {
        self.tx.set_notice(InputNotice::PasteRefused { bytes });
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Closing a pane detaches; it must never kill its shell.
        self.task.abort();
    }
}

/// The text an OSC 52 write puts on the system clipboard, if any.
///
/// Follows felis-client: program writes reach the system clipboard only with
/// `clipboard.osc_52 = "system"`, and never when `use_os_clipboard = false`
/// keeps felis off the system clipboard. Under the default `mirror` policy
/// the daemon's mirror still answers the program's own OSC 52 queries.
/// Unlike felis-client, PRIMARY (`p`) writes are never applied.
fn system_clipboard_text(
    write: &felis_protocol::messages::ClipboardWrite,
    config: &felis_client_core::config::ClipboardConfig,
) -> Option<String> {
    if !(config.use_os_clipboard && config.osc_52.writes_to_system())
        || !write
            .selection
            .contains(felis_protocol::messages::ClipboardSelection::CLIPBOARD)
    {
        return None;
    }
    std::str::from_utf8(&write.data).ok().map(str::to_owned)
}

#[allow(clippy::too_many_arguments)]
async fn run(
    shared: Arc<Mutex<State>>,
    ctx: egui::Context,
    rx: Arc<InputQueue>,
    target: Target,
    attach: Option<String>,
    cwd: Option<String>,
    env: Vec<(String, String)>,
) -> Result<()> {
    let (mut conn, info, mut relay) = tokio::time::timeout(Duration::from_secs(20), async {
        let carrier = target.carrier();
        if attach.is_none()
            && let Target::Local(socket) = &target
        {
            // Preserve local daemon auto-start for new shells only.
            drop(connect_or_spawn_daemon(socket, Offer::window(false)).await?);
        }
        let (mut conn, relay): (CarrierConnection, _) =
            if let Target::Ssh { host, socket } = &target {
                let (conn, child) =
                    crate::ssh::connect(host, socket.as_deref(), Offer::window(false)).await?;
                (conn, Some(child))
            } else {
                (
                    connect_carrier(
                        carrier.clone(),
                        Offer::window(matches!(target, Target::Local(_))),
                        RemoteSpawn::Refuse,
                    )
                    .await?,
                    None,
                )
            };
        let info = if let Some(id) = attach {
            conn.attach_by_prefix(id, felis_client_core::AttachIntent::Deliberate)
                .await?
        } else {
            let mut args = felis_client_core::launch_args(vec![], &carrier)?;
            if matches!(target, Target::Local(_)) {
                args.command = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
                if cfg!(target_os = "macos") {
                    args.args = vec!["-l".into()];
                }
                if let Some(cwd) = cwd {
                    args.cwd = cwd;
                }
                args.env = env;
                args.env
                    .push(("NEKOMATA_SHELL_INTEGRATION".into(), "1".into()));
            }
            args.tags = vec!["nekomata".into()];
            conn.create_with(args)
                .await
                .context("create felis session")?
        };
        conn.writer
            .send(&crate::terminal_config::configure(
                &crate::terminal_config::get(&ctx),
            ))
            .await?;
        conn.writer
            .send(&InputMsg::ColorScheme { dark: true })
            .await?;
        Ok::<_, anyhow::Error>((conn, info, relay))
    })
    .await
    .context("daemon attach timed out")??;
    {
        let mut state = shared.lock().unwrap();
        state.screen = ShadowScreen::new(info.dims.rows, info.dims.cols);
        state.effective_minor = conn.effective_minor;
        state.session_id = Some(info.id);
        state.connected = true;
    }
    ctx.request_repaint();
    loop {
        let result = pump(&mut conn, &shared, &ctx, &rx).await;
        let Err(error) = result else {
            return Ok(());
        };
        if !transport_loss(&error) {
            return Err(error);
        }
        rx.close();
        {
            let mut state = shared.lock().unwrap();
            state.connected = false;
            state.reconnecting = true;
            let gate = state.pull.cycle_open();
            state.pull =
                felis_client_core::pull::PullScheduler::new(matches!(target, Target::Local(_)))
                    .holding_gate(gate);
        }
        ctx.request_repaint();
        drop(conn);
        drop(relay.take());
        let (mut recovered, child) = redial_target(&target, info.id).await?;
        recovered
            .writer
            .send(&crate::terminal_config::configure(
                &crate::terminal_config::get(&ctx),
            ))
            .await?;
        recovered
            .writer
            .send(&InputMsg::ColorScheme { dark: true })
            .await?;
        conn = recovered;
        relay = child;
        {
            let mut state = shared.lock().unwrap();
            // Accept input before publishing the new generation: the GUI
            // re-sends size and focus as soon as it sees it.
            rx.reopen();
            state.connected = true;
            state.reconnecting = false;
            state.reconnects = state.reconnects.wrapping_add(1);
            state.effective_minor = conn.effective_minor;
        }
        ctx.request_repaint();
    }
}

fn transport_loss(error: &anyhow::Error) -> bool {
    use felis_client_core::ConnectError;
    use felis_transport::TransportError;
    let io_loss = |error: &std::io::Error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::NotConnected
                | std::io::ErrorKind::TimedOut
        )
    };
    let framing_loss = |error: &TransportError| match error {
        TransportError::Io(error) => io_loss(error),
        TransportError::UnexpectedEof { .. } => true,
        _ => false,
    };
    if let Some(error) = error.downcast_ref::<ConnectError>() {
        return match error {
            ConnectError::Io(error) => io_loss(error),
            ConnectError::Transport(error) => framing_loss(error),
            _ => false,
        };
    }
    error
        .downcast_ref::<TransportError>()
        .is_some_and(framing_loss)
        || error.downcast_ref::<std::io::Error>().is_some_and(io_loss)
}

async fn redial_target(
    target: &Target,
    id: u128,
) -> Result<(CarrierConnection, Option<tokio::process::Child>)> {
    if let Target::Ssh { host, socket } = target {
        // Retain every SSH child through retry so timed-out handshakes and
        // replaced connections cannot leave a relay running.
        return felis_transport::retry::retry_while(
            || async {
                let attempt = async {
                    let (mut conn, child) =
                        crate::ssh::connect(host, socket.as_deref(), Offer::window(false)).await?;
                    conn.attach_by_prefix(
                        format!("{id:032x}"),
                        felis_client_core::AttachIntent::Automatic,
                    )
                    .await?;
                    Ok::<_, anyhow::Error>((conn, Some(child)))
                };
                tokio::time::timeout(felis_client_core::RECONNECT_ATTEMPT_TIMEOUT, attempt)
                    .await
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::TimedOut, error))?
            },
            felis_transport::retry::RetryPolicy::WINDOW_RECONNECT,
            transport_loss,
        )
        .await
        .map_err(|error| error.source);
    }
    let recovered = felis_client_core::redial_session(
        target.carrier(),
        Offer::window(matches!(target, Target::Local(_))),
        id,
    )
    .await?;
    Ok((recovered.conn, None))
}

/// Applies one daemon frame to the pane's state.
///
/// The GUI thread locks this state for every paint, so a panic in the
/// frame code must not unwind through the guard: that would poison the
/// mutex and take down every pane of the window. It fails this pane
/// instead, resetting what the frame may have left half-applied: the
/// GUI still reads the screen for selection, links and search.
fn apply_frame(shared: &Mutex<State>, apply: impl FnOnce(&mut State) -> Result<()>) -> Result<()> {
    let mut state = shared.lock().unwrap();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| apply(&mut state))) {
        Ok(result) => result,
        Err(panic) => {
            let (rows, cols) = (state.screen.screen().rows(), state.screen.screen().cols());
            state.screen = ShadowScreen::new(rows, cols);
            state.images = felis_client_core::ImageShadow::new();
            let reason = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            bail!("felis frame could not be applied: {reason}")
        }
    }
}

async fn pump(
    conn: &mut CarrierConnection,
    shared: &Arc<Mutex<State>>,
    ctx: &egui::Context,
    rx: &InputQueue,
) -> Result<()> {
    let clipboard = crate::terminal_config::get(ctx).clipboard.clone();
    loop {
        tokio::select! {
            (command, shell_input) = rx.next() => {
                let sent = tokio::time::timeout(Duration::from_secs(10), conn.writer.send_checked(command.frame())).await;
                if shell_input && !matches!(sent, Ok(Ok(()))) {
                    rx.mark_lost();
                }
                match sent {
                    Ok(Ok(())) => {},
                    Ok(Err(error)) => return Err(error.into()),
                    Err(error) => return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, error).into()),
                }
            }
            frame = conn.next_frame() => {
                let frame = frame?.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "felis daemon closed the connection"))?;
                match conn.driver.classify(&frame)? {
                    Incoming::Payload(payload) => match payload.kind {
                        MessageKind::Grid => {
                            if let Delivery::Deliver(delivery) = conn.driver.decode::<GridMsg>(&payload)? {
                                apply_frame(shared, |state| state.apply_grid(&delivery.msg, &clipboard))?;
                                ctx.request_repaint();
                            }
                        }
                        MessageKind::Image => {
                            if let Delivery::Deliver(delivery) = conn.driver.decode::<felis_protocol::messages::ImageMsg>(&payload)? {
                                apply_frame(shared, |state| {
                                    state.images.apply(&delivery.msg)?;
                                    state.redraw.request();
                                    Ok(())
                                })?;
                                ctx.request_repaint();
                            }
                        }
                        MessageKind::Push => {
                            if let Delivery::Deliver(delivery) = conn.driver.decode::<felis_protocol::messages::PushMsg>(&payload)?
                            {
                                match delivery.msg {
                                    felis_protocol::messages::PushMsg::Evicted { .. } => {
                                        shared.lock().unwrap().detached = true;
                                        return Ok(());
                                    }
                                    felis_protocol::messages::PushMsg::SessionExited { id }
                                        if shared.lock().unwrap().session_id == Some(id) => {
                                        shared.lock().unwrap().exited = true;
                                        return Ok(());
                                    }
                                    _ => {}
                                }
                            }
                        }
                        other => bail!("Unexpected felis message: {other:?}"),
                    },
                    Incoming::Control(message) => bail!("felis connection ended: {message:?}"),
                    Incoming::CancelIgnored { .. } => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use felis_grid::{Color, Grapheme};
    use felis_protocol::messages::{RequestedDims, SpawnArgs};
    use std::{
        sync::mpsc as sync_mpsc,
        time::{Duration, Instant},
    };

    fn viewport_state(effective_minor: u16) -> State {
        let mut screen = ShadowScreen::new(24, 80);
        screen
            .apply(&GridMsg::ViewportState {
                lines_from_bottom: 10,
                max: 124,
                anchor: Some(felis_protocol::messages::ViewportAnchor {
                    pushed: 42,
                    generation: 1,
                }),
            })
            .unwrap();
        State {
            screen,
            effective_minor,
            pull: felis_client_core::pull::PullScheduler::new(false),
            redraw: felis_client_core::redraw::RedrawScheduler::new(),
            colors: Default::default(),
            images: felis_client_core::ImageShadow::new(),
            session_id: None,
            error: None,
            connected: true,
            reconnects: 0,
            reconnecting: false,
            detached: false,
            exited: false,
            clipboard: None,
            selection_epoch: 0,
        }
    }

    fn mode_flags(alt_screen: bool) -> GridMsg {
        GridMsg::ModeFlags {
            bracketed_paste: false,
            alt_screen,
            mouse_protocol: Default::default(),
            application_cursor: false,
            modify_other_keys: Default::default(),
            application_keypad: false,
            win32_input_mode: false,
            reverse_video: false,
        }
    }

    #[test]
    fn switching_screens_invalidates_the_selection() {
        let mut state = viewport_state(felis_protocol::preface::PROTOCOL_MINOR);
        state
            .apply_grid(&mode_flags(true), &Default::default())
            .unwrap();
        assert_eq!(state.selection_epoch, 1);
        // `ModeFlags` resends an unchanged alt-screen bit with other modes.
        state
            .apply_grid(&mode_flags(true), &Default::default())
            .unwrap();
        assert_eq!(state.selection_epoch, 1);
        state
            .apply_grid(&mode_flags(false), &Default::default())
            .unwrap();
        assert_eq!(state.selection_epoch, 2);
    }

    #[test]
    fn resizing_the_screen_invalidates_the_selection() {
        let mut state = viewport_state(felis_protocol::preface::PROTOCOL_MINOR);
        let size = |rows, cols| GridMsg::Size {
            dims: felis_protocol::messages::GridDims {
                rows,
                cols,
                pixel_w: 0,
                pixel_h: 0,
            },
        };
        // Another client resized the shared session.
        state
            .apply_grid(&size(30, 100), &Default::default())
            .unwrap();
        assert_eq!(state.selection_epoch, 1);
        // A size felis only confirms moves no cell.
        state
            .apply_grid(&size(30, 100), &Default::default())
            .unwrap();
        assert_eq!(state.selection_epoch, 1);
    }

    #[test]
    fn rehydrating_the_screen_invalidates_the_selection() {
        let mut state = viewport_state(felis_protocol::preface::PROTOCOL_MINOR);
        state
            .apply_grid(&GridMsg::RehydrateBegin, &Default::default())
            .unwrap();
        assert_eq!(state.selection_epoch, 1);
    }

    #[test]
    fn moving_the_viewport_invalidates_the_selection() {
        let mut state = viewport_state(felis_protocol::preface::PROTOCOL_MINOR);
        // Output pushed two rows and the view followed them: same lines.
        state
            .apply_grid(
                &GridMsg::ViewportState {
                    lines_from_bottom: 12,
                    max: 126,
                    anchor: Some(felis_protocol::messages::ViewportAnchor {
                        pushed: 44,
                        generation: 1,
                    }),
                },
                &Default::default(),
            )
            .unwrap();
        assert_eq!(state.selection_epoch, 0);
        // A scroll or search jump back to live shows other lines.
        state
            .apply_grid(
                &GridMsg::ViewportState {
                    lines_from_bottom: 0,
                    max: 126,
                    anchor: None,
                },
                &Default::default(),
            )
            .unwrap();
        assert_eq!(state.selection_epoch, 1);
    }

    #[test]
    fn only_a_rendered_frame_pulls_the_next_grid_frame() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // Never drive this runtime: requests stay queued without contacting a daemon.
        let session = Session::start(
            &runtime,
            egui::Context::default(),
            Target::Local("/unused-pull-test.sock".into()),
            None,
            None,
            vec![],
        );
        let mut state = session.state.lock().unwrap();
        state.connected = true;
        state.pull = felis_client_core::pull::PullScheduler::new(true);
        // A frame that reused the previous texture presented nothing new.
        session.frame_submitted(&mut state, false);
        assert_eq!(state.pull.sent(), 0);
        session.frame_submitted(&mut state, true);
        assert_eq!(state.pull.sent(), 1);
    }

    #[test]
    fn rapid_scroll_steps_remain_ordered_before_the_screen_reply() {
        let state = viewport_state(felis_protocol::preface::PROTOCOL_MINOR);
        let queue = InputQueue::default();
        for _ in 0..2 {
            let request = state.scroll_request(1).unwrap();
            assert_eq!(request, InputMsg::ViewportStep { rows: 1 });
            queue.push(&request).unwrap();
        }
        let expected = OutgoingFrame::input(&InputMsg::ViewportStep { rows: 1 }).unwrap();
        let mut pending = queue.pending.lock().unwrap();
        for _ in 0..2 {
            assert_eq!(
                pending.frames.pop().unwrap().frame().body(),
                expected.frame().body()
            );
        }
        assert!(pending.frames.is_empty());
    }

    #[test]
    fn keyboard_scrolls_use_shared_steps_and_live_reset_has_no_history_anchor() {
        use felis_client_core::ScrollStep;
        let state = viewport_state(felis_protocol::preface::PROTOCOL_MINOR);
        assert_eq!(
            state.scroll_action(ScrollStep::HalfPageUp),
            Some(InputMsg::ViewportStep { rows: 12 })
        );
        assert_eq!(
            state.scroll_action(ScrollStep::HalfPageDown),
            Some(InputMsg::ViewportStep { rows: -12 })
        );
        assert_eq!(
            state.scroll_action(ScrollStep::Home),
            Some(InputMsg::ViewportStep { rows: i32::MAX })
        );
        assert_eq!(
            state.scroll_action(ScrollStep::End),
            Some(InputMsg::Viewport {
                lines_from_bottom: 0,
                anchor: None
            })
        );
    }

    #[test]
    fn disconnected_panes_do_not_enqueue_scroll_steps() {
        let mut state = viewport_state(felis_protocol::preface::PROTOCOL_MINOR);
        state.connected = false;
        assert!(state.scroll_request(1).is_none());
    }

    #[test]
    fn old_daemon_scroll_requests_clamp_and_preserve_history_anchors() {
        let state = viewport_state(1);
        assert_eq!(
            state.scroll_request(i32::MAX),
            Some(InputMsg::Viewport {
                lines_from_bottom: 100,
                anchor: state.screen.viewport_anchor(),
            })
        );
        assert_eq!(
            state.scroll_request(-100),
            Some(InputMsg::Viewport {
                lines_from_bottom: 0,
                anchor: None,
            })
        );
        assert!(state.scroll_request(0).is_none());
    }

    #[test]
    fn resize_storms_keep_only_the_latest_unsent_size() {
        use felis_protocol::messages::RequestedDims;
        let queue = InputQueue::default();
        for cols in 1..=1000 {
            queue
                .push(&InputMsg::Resize {
                    dims: RequestedDims {
                        rows: 24,
                        cols,
                        pixel_w: 0,
                        pixel_h: 0,
                    },
                })
                .unwrap();
        }
        let expected = OutgoingFrame::input(&InputMsg::Resize {
            dims: RequestedDims {
                rows: 24,
                cols: 1000,
                pixel_w: 0,
                pixel_h: 0,
            },
        })
        .unwrap();
        let latest = queue.pending.lock().unwrap().frames.pop().unwrap();
        assert_eq!(latest.frame().body(), expected.frame().body());
        assert!(queue.pending.lock().unwrap().frames.is_empty());
    }

    #[test]
    fn lost_connection_discards_input_before_accepting_new_keys() {
        let queue = InputQueue::default();
        queue.push(&InputMsg::KeyBytes(b"old".to_vec())).unwrap();
        queue.close();
        assert!(queue.input_lost());
        assert!(
            queue
                .push(&InputMsg::KeyBytes(b"uncertain".to_vec()))
                .is_err()
        );
        queue.reopen();
        queue.push(&InputMsg::KeyBytes(b"new".to_vec())).unwrap();
        let expected = OutgoingFrame::input(&InputMsg::KeyBytes(b"new".to_vec())).unwrap();
        assert_eq!(
            queue
                .pending
                .lock()
                .unwrap()
                .frames
                .pop()
                .unwrap()
                .frame()
                .body(),
            expected.frame().body()
        );
        assert!(queue.pending.lock().unwrap().frames.is_empty());
        assert!(!transport_loss(&anyhow::anyhow!("malformed protocol")));
        assert!(transport_loss(
            &std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()
        ));
    }

    #[test]
    fn ssh_carrier_ends_options_before_the_host() {
        let target = Target::Ssh {
            host: "dev".parse().unwrap(),
            socket: None,
        };
        let Carrier::Ssh { ssh_args, .. } = target.carrier() else {
            panic!("SSH target must use the SSH carrier");
        };
        assert_eq!(ssh_args.last().map(String::as_str), Some("--"));
    }

    #[test]
    fn retained_ssh_targets_reject_hosts_ssh_would_read_as_options() {
        for saved in [
            r#"{"Ssh":"-oProxyCommand=false"}"#,
            r#"{"Ssh":{"host":"-oProxyCommand=false","socket":null}}"#,
            r#"{"Ssh":""}"#,
        ] {
            let error = serde_json::from_str::<Target>(saved).unwrap_err();
            assert!(error.to_string().contains("invalid SSH host"), "{error}");
        }
    }

    #[test]
    fn retained_ssh_targets_accept_old_hosts_and_round_trip_remote_sockets() {
        let legacy: Target = serde_json::from_str(r#"{"Ssh":"dev"}"#).unwrap();
        assert_eq!(
            legacy,
            Target::Ssh {
                host: "dev".parse().unwrap(),
                socket: None
            }
        );
        let target = Target::Ssh {
            host: "dev".parse().unwrap(),
            socket: Some("relative.sock".into()),
        };
        let saved = serde_json::to_string(&target).unwrap();
        assert_eq!(serde_json::from_str::<Target>(&saved).unwrap(), target);
    }

    fn clipboard_write(
        selection: felis_protocol::messages::ClipboardSelection,
        data: &[u8],
    ) -> felis_protocol::messages::ClipboardWrite {
        felis_protocol::messages::ClipboardWrite {
            selection,
            data: data.to_vec(),
        }
    }

    fn clipboard_config(
        osc_52: felis_client_core::config::Osc52Policy,
        use_os_clipboard: bool,
    ) -> felis_client_core::config::ClipboardConfig {
        felis_client_core::config::ClipboardConfig {
            use_os_clipboard,
            osc_52,
        }
    }

    #[test]
    fn osc52_writes_stay_off_the_system_clipboard_by_default() {
        use felis_protocol::messages::ClipboardSelection;
        let write = clipboard_write(ClipboardSelection::CLIPBOARD, b"hi");
        assert_eq!(system_clipboard_text(&write, &Default::default()), None);
    }

    #[test]
    fn osc52_system_policy_writes_clipboard_targets_to_the_system_clipboard() {
        use felis_client_core::config::Osc52Policy;
        use felis_protocol::messages::ClipboardSelection;
        let config = clipboard_config(Osc52Policy::System, true);
        let both = clipboard_write(
            ClipboardSelection::CLIPBOARD | ClipboardSelection::PRIMARY,
            b"hi",
        );
        assert_eq!(system_clipboard_text(&both, &config).as_deref(), Some("hi"));
        let primary = clipboard_write(ClipboardSelection::PRIMARY, b"hi");
        assert_eq!(system_clipboard_text(&primary, &config), None);
        let binary = clipboard_write(ClipboardSelection::CLIPBOARD, &[0xff]);
        assert_eq!(system_clipboard_text(&binary, &config), None);
    }

    #[test]
    fn osc52_system_policy_is_inert_without_the_os_clipboard() {
        use felis_client_core::config::Osc52Policy;
        use felis_protocol::messages::ClipboardSelection;
        let write = clipboard_write(ClipboardSelection::CLIPBOARD, b"hi");
        assert_eq!(
            system_clipboard_text(&write, &clipboard_config(Osc52Policy::System, false)),
            None
        );
    }

    fn wait_for(
        session: &Session,
        ctx: &egui::Context,
        repaints: &sync_mpsc::Receiver<()>,
        predicate: impl Fn(&State) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            // Acknowledge the repaint before examining state so a racing update
            // either satisfies the predicate or leaves a wakeup for the next pass.
            let _ = crate::test_support::run(ctx, egui::RawInput::default(), |_| {});
            {
                let mut state = session.state.lock().unwrap();
                // Headless tests stand in for a completed visible GPU frame.
                session.frame_submitted(&mut state, true);
                assert!(state.error.is_none(), "{:?}", state.error);
                if predicate(&state) {
                    return;
                }
            }
            repaints
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("timed out waiting for a felis screen update");
        }
    }

    fn has_red_marker(state: &State) -> bool {
        let screen = state.screen.screen();
        (0..screen.rows()).any(|row| {
            let text: String = (0..screen.cols())
                .filter_map(|col| {
                    let cell = screen.cell(row, col)?;
                    if screen.style(cell.style).fg != Color::Indexed(1) {
                        return None;
                    }
                    match cell.grapheme {
                        Grapheme::Ascii(b) => Some(char::from(b)),
                        _ => None,
                    }
                })
                .collect();
            text.contains("NEKOMATA_READY")
        })
    }

    #[test]
    #[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
    fn live_hidden_panes_pause_pulls_and_resume_with_the_latest_screen() {
        let socket = PathBuf::from(crate::test_support::private_felis_socket());
        let marker = socket.with_file_name(format!("hidden-{}.done", std::process::id()));
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let (tx, repaints) = sync_mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let session = Session::start(
            &runtime,
            ctx.clone(),
            Target::Local(socket),
            None,
            None,
            vec![],
        );
        wait_for(&session, &ctx, &repaints, |state| state.render_ready());
        session.send(InputMsg::KeyBytes(b"printf '\nPULL_FIRST\n'\n".to_vec()));
        wait_for(&session, &ctx, &repaints, |state| {
            contains_text(state, "PULL_FIRST")
        });
        let sent = session.state.lock().unwrap().pull.sent();
        // The shell creates the marker after printing, so once it exists the
        // new output has been produced while this pane stayed hidden.
        session.send(InputMsg::KeyBytes(
            format!("printf '\\nPULL_LATEST\\n'; : > '{}'\n", marker.display()).into_bytes(),
        ));
        let deadline = Instant::now() + Duration::from_secs(15);
        while !marker.exists() {
            assert!(Instant::now() < deadline, "the shell never ran the command");
            // Repaints keep arriving, but no visible frame is submitted.
            let _ = repaints.recv_timeout(Duration::from_millis(10));
        }
        std::fs::remove_file(&marker).unwrap();
        // At most the already outstanding pull is answered while hidden.
        assert_eq!(session.state.lock().unwrap().pull.sent(), sent);
        wait_for(&session, &ctx, &repaints, |state| {
            contains_text(state, "PULL_LATEST") && state.render_ready()
        });
        let mut state = session.state.lock().unwrap();
        let sent = state.pull.sent();
        session.frame_submitted(&mut state, true);
        session.frame_submitted(&mut state, true);
        assert!(state.pull.sent() <= sent + 1, "only one outstanding pull");
    }

    fn mouse(action: felis_protocol::messages::MouseAction, button: bool) -> InputMsg {
        use felis_protocol::messages::{InputMods, MouseButton, MouseEvent};
        InputMsg::Mouse(MouseEvent {
            x: 1,
            y: 1,
            px: 0,
            py: 0,
            action,
            button: button.then_some(MouseButton::Left),
            mods: InputMods::empty(),
        })
    }

    fn state_messages() -> Vec<InputMsg> {
        use felis_protocol::messages::{MouseAction, PromptJump, RequestedDims};
        vec![
            InputMsg::Resize {
                dims: RequestedDims {
                    rows: 24,
                    cols: 80,
                    pixel_w: 0,
                    pixel_h: 0,
                },
            },
            InputMsg::FocusChange { focused: true },
            InputMsg::NextGridFrame,
            InputMsg::Viewport {
                lines_from_bottom: 0,
                anchor: None,
            },
            InputMsg::ViewportStep { rows: 1 },
            InputMsg::JumpPrompt {
                direction: PromptJump::Previous,
            },
            mouse(MouseAction::Motion, false),
        ]
    }

    #[test]
    fn refused_view_and_focus_updates_are_not_lost_input() {
        let queue = InputQueue::default();
        queue.close();
        for msg in state_messages() {
            assert!(queue.push(&msg).is_err());
            assert!(!queue.input_lost(), "{msg:?} is not shell input");
        }
        queue.reopen();
        for msg in state_messages() {
            queue.push(&msg).unwrap();
        }
        queue.close();
        assert!(!queue.input_lost());
    }

    #[test]
    fn refused_or_discarded_shell_input_is_reported_until_the_next_delivery() {
        use felis_protocol::messages::MouseAction;
        for msg in [
            InputMsg::KeyBytes(b"x".to_vec()),
            InputMsg::Paste(b"x".to_vec()),
            mouse(MouseAction::Press, true),
        ] {
            let queue = InputQueue::default();
            queue.close();
            assert!(queue.push(&msg).is_err());
            assert!(queue.input_lost(), "{msg:?} is shell input");
            queue.reopen();
            queue
                .push(&InputMsg::FocusChange { focused: true })
                .unwrap();
            assert!(queue.input_lost(), "a focus report is not a delivery");
            queue.push(&msg).unwrap();
            assert!(!queue.input_lost());
            queue.close();
            assert!(queue.input_lost(), "the queued {msg:?} was discarded");
        }
    }

    #[test]
    fn a_paste_refusal_stays_until_shell_input_is_accepted() {
        let queue = InputQueue::default();
        queue.set_notice(InputNotice::PasteRefused { bytes: 9 });
        queue
            .push(&InputMsg::FocusChange { focused: true })
            .unwrap();
        assert_eq!(queue.notice(), Some(InputNotice::PasteRefused { bytes: 9 }));
        assert!(
            InputNotice::PasteRefused { bytes: 9 }
                .message()
                .contains(&felis_protocol::messages::MAX_PASTE_BYTES.to_string())
        );
        queue.push(&InputMsg::KeyBytes(b"x".to_vec())).unwrap();
        assert_eq!(queue.notice(), None);
        let oversized = vec![b'x'; felis_protocol::messages::MAX_PASTE_BYTES + 1];
        assert!(
            queue.push(&InputMsg::Paste(oversized)).is_err(),
            "the encoder drops an oversized paste, so the pane must refuse it first"
        );
    }

    #[test]
    fn a_panic_while_applying_a_frame_fails_only_that_pane() {
        let shared = Arc::new(Mutex::new(viewport_state(
            felis_protocol::preface::PROTOCOL_MINOR,
        )));
        let error = apply_frame(&shared, |state| {
            state.screen.apply(&GridMsg::RehydrateBegin)?;
            panic!("felis bug");
        })
        .unwrap_err();
        assert!(!transport_loss(&error), "a broken frame is not retried");
        assert!(format!("{error:#}").contains("felis bug"));
        assert!(!shared.is_poisoned(), "the GUI keeps locking this pane");
        let state = shared.lock().unwrap();
        assert!(
            !state.screen.is_rehydrating(),
            "half-applied state is reset"
        );
        assert_eq!(
            (state.screen.screen().rows(), state.screen.screen().cols()),
            (24, 80)
        );
    }

    #[test]
    fn dequeued_frames_say_whether_their_loss_matters() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let queue = InputQueue::default();
        // The second focus report replaces the first in place: an entry
        // recorded for it would pair the key with the wrong flag.
        for focused in [true, false] {
            queue.push(&InputMsg::FocusChange { focused }).unwrap();
        }
        queue.push(&InputMsg::KeyBytes(b"x".to_vec())).unwrap();
        queue.push(&InputMsg::NextGridFrame).unwrap();
        let (_, focus) = runtime.block_on(queue.next());
        let (_, key) = runtime.block_on(queue.next());
        let (_, frame) = runtime.block_on(queue.next());
        assert_eq!((focus, key, frame), (false, true, false));
        assert!(queue.pending.lock().unwrap().shell_input.is_empty());
        assert!(queue.pending.lock().unwrap().frames.is_empty());
    }

    #[test]
    #[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
    fn live_reconnect_preserves_the_existing_shell_after_transport_loss() {
        let socket = PathBuf::from(crate::test_support::private_felis_socket());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let original = runtime.block_on(async {
            let mut conn = connect_or_spawn_daemon(&socket, Offer::window(false))
                .await
                .unwrap();
            conn.create_with(SpawnArgs {
                command: "/bin/sh".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .id
        });
        let proxy = socket.with_file_name(format!("reconnect-{}.sock", std::process::id()));
        let listener = {
            let _guard = runtime.enter();
            tokio::net::UnixListener::bind(&proxy).unwrap()
        };
        let (disconnect, _) = tokio::sync::broadcast::channel::<()>(4);
        let signal = disconnect.clone();
        let relay = runtime.spawn(async move {
            loop {
                let (mut client, _) = listener.accept().await.unwrap();
                let mut upstream = tokio::net::UnixStream::connect(&socket).await.unwrap();
                let mut lost = signal.subscribe();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut client, &mut upstream) => {},
                        _ = lost.recv() => {},
                    }
                });
            }
        });
        let ctx = egui::Context::default();
        let (repaint_tx, repaints) = sync_mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = repaint_tx.send(());
        });
        let session = Session::start(
            &runtime,
            ctx.clone(),
            Target::Local(proxy),
            Some(format!("{original:032x}")),
            None,
            vec![],
        );
        wait_for(&session, &ctx, &repaints, |state| state.connected);
        session.send(InputMsg::KeyBytes(
            b"NEKO_RECONNECT_MARKER=kept; printf '\\nBEFORE_%s\\n' \"$NEKO_RECONNECT_MARKER\"\n"
                .to_vec(),
        ));
        wait_for(&session, &ctx, &repaints, |state| {
            contains_text(state, "BEFORE_kept")
        });
        let id = session.state.lock().unwrap().session_id;
        disconnect.send(()).unwrap();
        wait_for(&session, &ctx, &repaints, |state| {
            state.connected && state.reconnects > 0
        });
        assert_eq!(session.state.lock().unwrap().session_id, id);
        session.send(InputMsg::KeyBytes(
            b"printf '\\nAFTER_%s\\n' \"$NEKO_RECONNECT_MARKER\"\n".to_vec(),
        ));
        wait_for(&session, &ctx, &repaints, |state| {
            contains_text(state, "AFTER_kept")
        });
        relay.abort();
    }

    #[test]
    #[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
    fn live_kitty_pixels_reach_the_frontend_image_store() {
        crate::test_support::private_felis_socket();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let (repaint_tx, repaints) = sync_mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = repaint_tx.send(());
        });
        let session = Session::new(&runtime, ctx.clone(), None, None);
        wait_for(&session, &ctx, &repaints, |state| state.connected);
        session.send(InputMsg::KeyBytes(
            b"printf '\\033_Ga=T,f=24,s=1,v=1,i=77,q=2;AP8A\\033\\\\'\n".to_vec(),
        ));
        wait_for(&session, &ctx, &repaints, |state| {
            state
                .images
                .images()
                .values()
                .any(|image| image.is_complete() && image.pixels() == [0, 255, 0])
        });
        assert!(!session.state.lock().unwrap().images.placements().is_empty());
    }

    #[test]
    #[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
    fn live_shell_exit_closes_only_its_pane() {
        crate::test_support::private_felis_socket();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let (tx, repaints) = sync_mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let cwd = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let session = Session::new(&runtime, ctx.clone(), None, Some(cwd.clone()));
        let sibling = Session::new(&runtime, ctx.clone(), None, None);
        wait_for(&session, &ctx, &repaints, |state| state.connected);
        wait_for(&sibling, &ctx, &repaints, |state| state.connected);
        let command = format!(
            "test \"$PWD\" = '{}' && printf '\\033[31mNEKOMATA_READY\\033[0m\\n'\r",
            cwd.replace('\'', "'\\''")
        );
        session.send(InputMsg::KeyBytes(command.into_bytes()));
        wait_for(&session, &ctx, &repaints, has_red_marker);
        session.send(InputMsg::KeyBytes(b"exit\r".to_vec()));
        wait_for(&session, &ctx, &repaints, |state| {
            state.exited && !state.connected
        });
        let state = sibling.state.lock().unwrap();
        assert!(state.connected);
        assert!(!state.exited);
        assert!(state.error.is_none());
    }

    #[test]
    #[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
    fn live_session_input_resize_and_reattach() {
        let socket = crate::test_support::private_felis_socket();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let (tx, repaints) = sync_mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let session = Session::new(&runtime, ctx.clone(), None, None);
        wait_for(&session, &ctx, &repaints, |state| state.connected);
        let id = session.state.lock().unwrap().session_id.unwrap();
        session.send(InputMsg::Resize {
            dims: RequestedDims {
                rows: 12,
                cols: 60,
                pixel_w: 600,
                pixel_h: 240,
            },
        });
        wait_for(&session, &ctx, &repaints, |state| {
            state.screen.screen().rows() == 12 && state.screen.screen().cols() == 60
        });
        session.send(InputMsg::KeyBytes(
            b"printf '\\033[31mNEKOMATA_READY\\033[0m\\n'\r".to_vec(),
        ));
        wait_for(&session, &ctx, &repaints, has_red_marker);
        drop(session);
        let session = Session::new(&runtime, ctx.clone(), Some(format!("{id:032x}")), None);
        wait_for(&session, &ctx, &repaints, has_red_marker);
        assert_eq!(session.state.lock().unwrap().session_id, Some(id));
        let executable = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("nekomata");
        let listed = std::process::Command::new(&executable)
            .args(["felis", "sessions", "list", "--format", "json"])
            .output()
            .unwrap();
        assert!(
            listed.status.success(),
            "{}",
            String::from_utf8_lossy(&listed.stderr)
        );
        assert!(String::from_utf8_lossy(&listed.stdout).contains(&format!("{id:032x}")));
        let detached = std::process::Command::new("felis")
            .args(["sessions", "evict", &format!("{id:032x}")])
            .output()
            .unwrap();
        assert!(
            detached.status.success(),
            "{}",
            String::from_utf8_lossy(&detached.stderr)
        );
        wait_for(&session, &ctx, &repaints, |state| state.detached);
        drop(session);
        runtime.block_on(async {
            let mut ops = felis_client_core::connect(std::path::Path::new(&socket), Offer::ops())
                .await
                .unwrap();
            assert!(
                ops.list_sessions()
                    .await
                    .unwrap()
                    .iter()
                    .any(|session| session.id == id),
                "detach killed the session"
            );
            ops.destroy_session(format!("{id:032x}")).await.unwrap();
        });
    }
    #[test]
    #[ignore = "requires the isolated SSH relay fixture; run scripts/test-live.sh via nix develop"]
    fn live_ssh_carrier_attaches_and_keeps_remote_spawn_environment_remote() {
        assert_eq!(std::env::var("NEKOMATA_TEST_SSH").as_deref(), Ok("1"));
        let socket = crate::test_support::private_felis_socket();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let (tx, repaints) = sync_mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let original = Session::new(&runtime, ctx.clone(), None, None);
        wait_for(&original, &ctx, &repaints, |state| state.connected);
        let id = original.state.lock().unwrap().session_id.unwrap();
        drop(original);
        let remote = Session::start(
            &runtime,
            ctx.clone(),
            Target::Ssh {
                host: "nekomata-test-host".parse().unwrap(),
                socket: None,
            },
            Some(format!("{id:032x}")),
            None,
            vec![],
        );
        wait_for(&remote, &ctx, &repaints, |state| state.connected);
        assert_eq!(remote.state.lock().unwrap().session_id, Some(id));
        remote.send(InputMsg::KeyBytes(
            b"printf 'SSH_%s_OK\\n' CARRIER\r".to_vec(),
        ));
        wait_for(&remote, &ctx, &repaints, |state| {
            contains_text(state, "SSH_CARRIER_OK")
        });
        drop(remote);
        let spawned = Session::start(
            &runtime,
            ctx.clone(),
            Target::Ssh {
                host: "nekomata-test-host".parse().unwrap(),
                socket: None,
            },
            None,
            Some("/nekomata-local-path-must-not-be-forwarded".into()),
            vec![("NEKOMATA_TEST_LOCAL_ENV".into(), "leaked".into())],
        );
        wait_for(&spawned, &ctx, &repaints, |state| state.connected);
        let spawned_id = spawned.state.lock().unwrap().session_id.unwrap();
        spawned.send(InputMsg::KeyBytes(
            b"/bin/sh -c 'printf \"REMOTE_ENV_%s_DONE\\n\" \"${NEKOMATA_TEST_LOCAL_ENV-unset}\"'\r"
                .to_vec(),
        ));
        wait_for(&spawned, &ctx, &repaints, |state| {
            contains_text(state, "REMOTE_ENV_unset_DONE")
        });
        drop(spawned);
        runtime.block_on(async {
            let mut ops = felis_client_core::connect(std::path::Path::new(&socket), Offer::ops())
                .await
                .unwrap();
            assert!(
                ops.list_sessions()
                    .await
                    .unwrap()
                    .iter()
                    .any(|session| session.id == id)
            );
            ops.destroy_session(format!("{id:032x}")).await.unwrap();
            ops.destroy_session(format!("{spawned_id:032x}"))
                .await
                .unwrap();
        });
    }

    fn contains_text(state: &State, needle: &str) -> bool {
        let screen = state.screen.screen();
        (0..screen.rows()).any(|row| {
            let text: String = (0..screen.cols())
                .filter_map(|col| match screen.cell(row, col)?.grapheme {
                    Grapheme::Ascii(b) => Some(char::from(b)),
                    _ => None,
                })
                .collect();
            text.contains(needle)
        })
    }

    #[test]
    #[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
    fn live_prompt_navigation_and_keyboard_scroll_use_daemon_history() {
        let socket = crate::test_support::private_felis_socket();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let id = runtime.block_on(async {
            let mut ops = connect_or_spawn_daemon(std::path::Path::new(&socket), Offer::ops()).await.unwrap();
            ops.spawn_session(SpawnArgs {
                command: "bash".into(),
                args: vec!["--noprofile".into(), "--norc".into(), "-c".into(),
                    r#"read -r line; for prompt in FIRST SECOND THIRD; do printf '\033]133;A\007%s_PROMPT\033]133;B\007\033]133;C\007\n' "$prompt"; for row in {1..35}; do printf 'OUTPUT_%s_%s\n' "$prompt" "$row"; done; printf '\033]133;D;0\007'; done; printf '\033]133;A\007NAVIGATION_READY'; read -r line"#.into()],
                dims: Some(RequestedDims { rows: 24, cols: 80, pixel_w: 800, pixel_h: 480 }),
                env_base: felis_client_core::env_base::capture(), ..Default::default()
            }).await.unwrap().id
        });
        let ctx = egui::Context::default();
        let (tx, repaints) = sync_mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let session = Session::new(&runtime, ctx.clone(), Some(format!("{id:032x}")), None);
        wait_for(&session, &ctx, &repaints, |state| state.connected);
        session.send(InputMsg::KeyBytes(b"\r".to_vec()));
        wait_for(&session, &ctx, &repaints, |state| {
            contains_text(state, "NAVIGATION_READY")
        });
        session.send(InputMsg::JumpPrompt {
            direction: felis_protocol::messages::PromptJump::Previous,
        });
        wait_for(&session, &ctx, &repaints, |state| {
            state.screen.viewport() > 0
        });
        let previous = session.state.lock().unwrap().screen.viewport();
        session.send(InputMsg::JumpPrompt {
            direction: felis_protocol::messages::PromptJump::Next,
        });
        wait_for(&session, &ctx, &repaints, |state| {
            state.screen.viewport() < previous
        });
        let request = session
            .state
            .lock()
            .unwrap()
            .scroll_action(felis_client_core::ScrollStep::End)
            .unwrap();
        session.send(request);
        wait_for(&session, &ctx, &repaints, |state| {
            state.screen.viewport() == 0
        });
        let request = session
            .state
            .lock()
            .unwrap()
            .scroll_action(felis_client_core::ScrollStep::HalfPageUp)
            .unwrap();
        session.send(request);
        wait_for(&session, &ctx, &repaints, |state| {
            state.screen.viewport() == 12
        });
        drop(session);
        runtime.block_on(async {
            let mut ops = felis_client_core::connect(std::path::Path::new(&socket), Offer::ops())
                .await
                .unwrap();
            ops.destroy_session(format!("{id:032x}")).await.unwrap();
        });
    }

    fn live_osc52_clipboard(osc_52: felis_client_core::config::Osc52Policy) -> Option<String> {
        let socket = crate::test_support::private_felis_socket();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let id = runtime.block_on(async {
            let mut ops = connect_or_spawn_daemon(std::path::Path::new(&socket), Offer::ops()).await.unwrap();
            ops.spawn_session(SpawnArgs {
                command: "bash".into(),
                args: vec!["--noprofile".into(), "--norc".into(), "-c".into(),
                    r#"read -r line; printf '\033]52;c;TkVLT01BVEFfQ0xJUEJPQVJECg==\007'; printf 'OSC52_%s\n' WRITTEN; read -r line; printf 'OSC52_%s\n' DONE; read -r line"#.into()],
                env_base: felis_client_core::env_base::capture(), ..Default::default()
            }).await.unwrap().id
        });
        let ctx = egui::Context::default();
        let mut config = felis_client_core::config::EffectiveConfig::default();
        config.clipboard.osc_52 = osc_52;
        crate::terminal_config::install(&ctx, config);
        let (tx, repaints) = sync_mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let session = Session::new(&runtime, ctx.clone(), Some(format!("{id:032x}")), None);
        wait_for(&session, &ctx, &repaints, |state| state.connected);
        session.send(InputMsg::KeyBytes(b"\r".to_vec()));
        wait_for(&session, &ctx, &repaints, |state| {
            contains_text(state, "OSC52_WRITTEN")
        });
        // The daemon may compose the first marker's row ahead of the write; a
        // further round trip guarantees a drive cycle has delivered it.
        session.send(InputMsg::KeyBytes(b"\r".to_vec()));
        wait_for(&session, &ctx, &repaints, |state| {
            contains_text(state, "OSC52_DONE")
        });
        let clipboard = session.state.lock().unwrap().clipboard.take();
        drop(session);
        runtime.block_on(async {
            let mut ops = felis_client_core::connect(std::path::Path::new(&socket), Offer::ops())
                .await
                .unwrap();
            ops.destroy_session(format!("{id:032x}")).await.unwrap();
        });
        clipboard
    }

    #[test]
    #[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
    fn live_osc52_reaches_the_frontend_clipboard_under_the_system_policy() {
        assert_eq!(
            live_osc52_clipboard(felis_client_core::config::Osc52Policy::System).as_deref(),
            Some("NEKOMATA_CLIPBOARD\n")
        );
    }

    #[test]
    #[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
    fn live_osc52_stays_off_the_frontend_clipboard_by_default() {
        assert_eq!(
            live_osc52_clipboard(felis_client_core::config::Osc52Policy::default()),
            None
        );
    }

    #[test]
    #[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
    fn live_background_query_matches_the_rendered_theme() {
        let socket = crate::test_support::private_felis_socket();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let id = runtime.block_on(async {
            let mut ops = connect_or_spawn_daemon(std::path::Path::new(&socket), Offer::ops()).await.unwrap();
            ops.spawn_session(SpawnArgs {
                command: "bash".into(),
                args: vec!["--noprofile".into(), "--norc".into(), "-c".into(),
                    r#"read -r line; stty raw -echo; printf '\033]11;?\007'; IFS= read -r -t 3 -d '\' reply; stty sane; reply=${reply%$'\033'}; printf 'COLOR_REPLY=%s\n' "${reply#*rgb:}"; read -r line"#.into()],
                env_base: felis_client_core::env_base::capture(), ..Default::default()
            }).await.unwrap().id
        });
        let ctx = egui::Context::default();
        let mut config = felis_client_core::config::EffectiveConfig::default();
        config.theme.background = Some("#123456".into());
        crate::terminal_config::install(&ctx, config);
        let (tx, repaints) = sync_mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let session = Session::new(&runtime, ctx.clone(), Some(format!("{id:032x}")), None);
        wait_for(&session, &ctx, &repaints, |state| state.connected);
        session.send(InputMsg::KeyBytes(b"\r".to_vec()));
        wait_for(&session, &ctx, &repaints, |state| {
            contains_text(state, "COLOR_REPLY=")
        });
        assert!(contains_text(
            &session.state.lock().unwrap(),
            "COLOR_REPLY=1212/3434/5656"
        ));
        drop(session);
        runtime.block_on(async {
            let mut ops = felis_client_core::connect(std::path::Path::new(&socket), Offer::ops())
                .await
                .unwrap();
            ops.destroy_session(format!("{id:032x}")).await.unwrap();
        });
    }

    #[test]
    #[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
    fn live_resize_blanks_the_active_prompt_without_erasing_output() {
        let socket = crate::test_support::private_felis_socket();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let id = runtime.block_on(async {
            let mut ops = connect_or_spawn_daemon(std::path::Path::new(&socket), Offer::ops()).await.unwrap();
            ops.spawn_session(SpawnArgs {
                command: "bash".into(), args: vec!["--noprofile".into(), "--norc".into(), "-c".into(),
                    r#"read -r line; printf '\033]133;C\007KEEP_OUTPUT\n\033]133;D;0\007\033]133;A\007ACTIVE_PROMPT /long/path/to/a/project/with/a/branch\n$ \033]133;B\007\033]2;fixture-ready\007'; read -r line"#.into()],
                dims: Some(RequestedDims { rows: 24, cols: 80, pixel_w: 800, pixel_h: 480 }),
                env_base: felis_client_core::env_base::capture(), ..Default::default()
            }).await.unwrap().id
        });
        let ctx = egui::Context::default();
        let (tx, repaints) = sync_mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let session = Session::new(&runtime, ctx.clone(), Some(format!("{id:032x}")), None);
        wait_for(&session, &ctx, &repaints, |state| state.connected);
        session.send(InputMsg::KeyBytes(b"\r".to_vec()));
        wait_for(&session, &ctx, &repaints, |state| {
            state.screen.title() == Some("fixture-ready") && contains_text(state, "ACTIVE_PROMPT")
        });
        session.send(InputMsg::Resize {
            dims: RequestedDims {
                rows: 24,
                cols: 30,
                pixel_w: 300,
                pixel_h: 480,
            },
        });
        wait_for(&session, &ctx, &repaints, |state| {
            state.screen.screen().cols() == 30 && !contains_text(state, "ACTIVE_PROMPT")
        });
        assert!(contains_text(&session.state.lock().unwrap(), "KEEP_OUTPUT"));
        drop(session);
        runtime.block_on(async {
            let mut ops = felis_client_core::connect(std::path::Path::new(&socket), Offer::ops())
                .await
                .unwrap();
            ops.destroy_session(format!("{id:032x}")).await.unwrap();
        });
    }
}
