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
#[serde(from = "StoredTarget")]
pub enum Target {
    Local(PathBuf),
    Ssh {
        host: String,
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

impl From<StoredTarget> for Target {
    fn from(value: StoredTarget) -> Self {
        match value {
            StoredTarget::Local(path) => Self::Local(path),
            StoredTarget::Ssh(StoredSsh::Host(host)) => Self::Ssh { host, socket: None },
            StoredTarget::Ssh(StoredSsh::Socket { host, socket }) => Self::Ssh { host, socket },
        }
    }
}

impl Target {
    pub fn default_local() -> Result<Self> {
        Ok(Self::Local(
            felis_client_core::local_socket::resolve_local_socket(None)?,
        ))
    }

    fn carrier(&self) -> Carrier {
        match self {
            Self::Local(path) => Carrier::Local(path.clone().into()),
            Self::Ssh { host, .. } => Carrier::Ssh {
                destination: host.clone(),
                ssh_args: vec!["-oBatchMode=yes".into()],
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
    pub colors: crate::colors::Colors,
    pub images: felis_client_core::ImageShadow,
    pub session_id: Option<u128>,
    pub error: Option<String>,
    pub connected: bool,
    pub detached: bool,
    pub exited: bool,
    pub clipboard: Option<String>,
}

#[derive(Default)]
struct InputQueue {
    frames: Mutex<OutgoingQueue>,
    ready: Notify,
}

impl InputQueue {
    fn push(&self, msg: &InputMsg) -> Result<()> {
        let frame = OutgoingFrame::input(msg)?;
        self.frames
            .lock()
            .unwrap()
            .push(frame)
            .map_err(|full| anyhow::anyhow!("input queue exceeded {} bytes", full.cap))?;
        self.ready.notify_one();
        Ok(())
    }

    async fn next(&self) -> OutgoingFrame {
        loop {
            let notified = self.ready.notified();
            if let Some(frame) = self.frames.lock().unwrap().pop() {
                return frame;
            }
            notified.await;
        }
    }
}

pub struct Session {
    pub state: Arc<Mutex<State>>,
    pub input_failed: AtomicBool,
    pub target: Target,
    pub attachment: Option<String>,
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
            colors: crate::colors::Colors::default(),
            images: felis_client_core::ImageShadow::new(),
            session_id: None,
            error: None,
            connected: false,
            detached: false,
            exited: false,
            clipboard: None,
        }));
        let tx = Arc::new(InputQueue::default());
        let rx = tx.clone();
        let attachment = attach.clone();
        let shared = state.clone();
        let destination = target.clone();
        let task = runtime.spawn(async move {
            let result = run(
                shared.clone(),
                ctx.clone(),
                rx,
                destination,
                attach,
                cwd,
                env,
            )
            .await;
            let mut state = shared.lock().unwrap();
            state.connected = false;
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
            input_failed: AtomicBool::new(false),
            target,
            attachment,
        }
    }

    pub fn send(&self, msg: InputMsg) {
        if self.tx.push(&msg).is_err() {
            self.input_failed.store(true, Ordering::Relaxed);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Closing a pane detaches; it must never kill its shell.
        self.task.abort();
    }
}

async fn run(
    shared: Arc<Mutex<State>>,
    ctx: egui::Context,
    rx: Arc<InputQueue>,
    target: Target,
    attach: Option<String>,
    cwd: Option<String>,
    env: Vec<(String, String)>,
) -> Result<()> {
    let (mut conn, info, _relay) = tokio::time::timeout(Duration::from_secs(20), async {
        let carrier = target.carrier();
        if attach.is_none()
            && let Target::Local(socket) = &target
        {
            // Preserve local daemon auto-start for new shells only.
            drop(connect_or_spawn_daemon(socket, Offer::window(false)).await?);
        }
        let (mut conn, relay): (CarrierConnection, _) = if let Target::Ssh {
            host,
            socket: Some(socket),
        } = &target
        {
            let (conn, child) = crate::ssh::connect(host, socket, Offer::window(false)).await?;
            (conn, Some(child))
        } else {
            (
                connect_carrier(carrier.clone(), Offer::window(false), RemoteSpawn::Refuse).await?,
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
        conn.writer.send(&crate::theme::configure()).await?;
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
        state.session_id = Some(info.id);
        state.connected = true;
    }
    ctx.request_repaint();
    loop {
        tokio::select! {
            command = rx.next() => {
                conn.writer.send_checked(command.frame()).await?;
            }
            frame = conn.next_frame() => {
                let frame = frame?.context("felis daemon closed the connection")?;
                match conn.driver.classify(&frame)? {
                    Incoming::Payload(payload) => match payload.kind {
                        MessageKind::Grid => {
                            if let Delivery::Deliver(delivery) = conn.driver.decode::<GridMsg>(&payload)? {
                                let mut state = shared.lock().unwrap();
                                state.screen.apply(&delivery.msg)?;
                                state.colors.apply_grid(&delivery.msg);
                                if matches!(delivery.msg, GridMsg::RehydrateBegin) {
                                    state.images = felis_client_core::ImageShadow::new();
                                }
                                if let GridMsg::ClipboardSet { write } = &delivery.msg
                                    && write.selection.contains(felis_protocol::messages::ClipboardSelection::CLIPBOARD)
                                    && let Ok(text) = std::str::from_utf8(&write.data) {
                                    state.clipboard = Some(text.to_owned());
                                }
                                ctx.request_repaint();
                            }
                        }
                        MessageKind::Image => {
                            if let Delivery::Deliver(delivery) = conn.driver.decode::<felis_protocol::messages::ImageMsg>(&payload)? {
                                shared.lock().unwrap().images.apply(&delivery.msg)?;
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
        let latest = queue.frames.lock().unwrap().pop().unwrap();
        assert_eq!(latest.frame().body(), expected.frame().body());
        assert!(queue.frames.lock().unwrap().is_empty());
    }

    #[test]
    fn retained_ssh_targets_accept_old_hosts_and_round_trip_remote_sockets() {
        let legacy: Target = serde_json::from_str(r#"{"Ssh":"dev"}"#).unwrap();
        assert_eq!(
            legacy,
            Target::Ssh {
                host: "dev".into(),
                socket: None
            }
        );
        let target = Target::Ssh {
            host: "dev".into(),
            socket: Some("relative.sock".into()),
        };
        let saved = serde_json::to_string(&target).unwrap();
        assert_eq!(serde_json::from_str::<Target>(&saved).unwrap(), target);
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
                let state = session.state.lock().unwrap();
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
    fn live_kitty_pixels_reach_the_frontend_image_store() {
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
        assert_eq!(
            std::env::var("FELIS_SOCKET").unwrap(),
            std::env::var("NEKOMATA_TEST_SOCKET").expect("use scripts/test-live.sh")
        );
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
        let socket = std::env::var("NEKOMATA_TEST_SOCKET").expect("use scripts/test-live.sh");
        assert_eq!(std::env::var("FELIS_SOCKET").unwrap(), socket);
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
        let socket = std::env::var("NEKOMATA_TEST_SOCKET").unwrap();
        assert_eq!(std::env::var("FELIS_SOCKET").unwrap(), socket);
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
                host: "nekomata-test-host".into(),
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
                host: "nekomata-test-host".into(),
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
    fn live_osc52_reaches_the_frontend_clipboard() {
        let socket = std::env::var("NEKOMATA_TEST_SOCKET").expect("use scripts/test-live.sh");
        assert_eq!(std::env::var("FELIS_SOCKET").unwrap(), socket);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let id = runtime.block_on(async {
            let mut ops = connect_or_spawn_daemon(std::path::Path::new(&socket), Offer::ops()).await.unwrap();
            ops.spawn_session(SpawnArgs {
                command: "bash".into(),
                args: vec!["--noprofile".into(), "--norc".into(), "-c".into(),
                    r#"read -r line; printf '\033]52;c;TkVLT01BVEFfQ0xJUEJPQVJECg==\007'; read -r line"#.into()],
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
        wait_for(&session, &ctx, &repaints, |state| state.clipboard.is_some());
        assert_eq!(
            session.state.lock().unwrap().clipboard.take().as_deref(),
            Some("NEKOMATA_CLIPBOARD\n")
        );
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
    fn live_background_query_matches_the_rendered_theme() {
        let socket = std::env::var("NEKOMATA_TEST_SOCKET").expect("use scripts/test-live.sh");
        assert_eq!(std::env::var("FELIS_SOCKET").unwrap(), socket);
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
            "COLOR_REPLY=1414/1717/1f1f"
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
        let socket = std::env::var("NEKOMATA_TEST_SOCKET").expect("use scripts/test-live.sh");
        assert_eq!(std::env::var("FELIS_SOCKET").unwrap(), socket);
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
