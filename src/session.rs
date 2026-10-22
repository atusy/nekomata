//! One ordered, bounded input channel and IPC connection per pane.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, bail};
use eframe::egui;
use felis_client_core::{Offer, ShadowScreen, connect_or_spawn_daemon};
use felis_protocol::{
    MessageKind,
    messages::{GridMsg, InputMsg, SpawnArgs},
};
use felis_transport::{Delivery, Incoming};
use tokio::sync::mpsc;

pub struct State {
    pub screen: ShadowScreen,
    pub session_id: Option<u128>,
    pub error: Option<String>,
    pub connected: bool,
    pub detached: bool,
    pub exited: bool,
}

pub struct Session {
    pub state: Arc<Mutex<State>>,
    pub input_failed: AtomicBool,
    tx: mpsc::Sender<InputMsg>,
    task: tokio::task::JoinHandle<()>,
}

impl Session {
    pub fn new(
        runtime: &tokio::runtime::Runtime,
        ctx: egui::Context,
        attach: Option<String>,
        cwd: Option<String>,
    ) -> Self {
        let state = Arc::new(Mutex::new(State {
            screen: ShadowScreen::new(24, 80),
            session_id: None,
            error: None,
            connected: false,
            detached: false,
            exited: false,
        }));
        let (tx, rx) = mpsc::channel(256);
        let shared = state.clone();
        let task = runtime.spawn(async move {
            let result = run(shared.clone(), ctx.clone(), rx, attach, cwd).await;
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
        }
    }

    pub fn send(&self, msg: InputMsg) {
        if self.tx.try_send(msg).is_err() {
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
    mut rx: mpsc::Receiver<InputMsg>,
    attach: Option<String>,
    cwd: Option<String>,
) -> Result<()> {
    let socket = felis_client_core::local_socket::resolve_local_socket(None)?;
    let mut conn = connect_or_spawn_daemon(&socket, Offer::window(false)).await?;
    let info = if let Some(id) = attach {
        conn.attach_by_prefix(id, felis_client_core::AttachIntent::Deliberate)
            .await?
    } else {
        conn.create_with(SpawnArgs {
            command: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
            args: vec![],
            cwd: match cwd {
                Some(cwd) => cwd,
                None => std::env::current_dir()?.to_string_lossy().into_owned(),
            },
            env: vec![("NEKOMATA_SHELL_INTEGRATION".into(), "1".into())],
            dims: None,
            env_base: felis_client_core::env_base::capture(),
            tags: vec!["nekomata".into()],
        })
        .await
        .context("create felis session")?
    };
    {
        let mut state = shared.lock().unwrap();
        state.screen = ShadowScreen::new(info.dims.rows, info.dims.cols);
        state.session_id = Some(info.id);
        state.connected = true;
    }
    conn.writer.send(&crate::theme::configure()).await?;
    ctx.request_repaint();
    conn.writer
        .send(&InputMsg::ColorScheme { dark: true })
        .await?;
    loop {
        tokio::select! {
            command = rx.recv() => {
                let Some(command) = command else { return Ok(()) };
                conn.writer.send(&command).await?;
            }
            frame = conn.next_frame() => {
                let frame = frame?.context("felis daemon closed the connection")?;
                match conn.driver.classify(&frame)? {
                    Incoming::Payload(payload) => match payload.kind {
                        MessageKind::Grid => {
                            if let Delivery::Deliver(delivery) = conn.driver.decode::<GridMsg>(&payload)? {
                                shared.lock().unwrap().screen.apply(&delivery.msg)?;
                                ctx.request_repaint();
                            }
                        }
                        // This first frontend renders text only; image payloads are validated
                        // and consumed so unsupported graphics do not break the connection.
                        MessageKind::Image => {
                            let _ = conn.driver.decode::<felis_protocol::messages::ImageMsg>(&payload)?;
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
    use felis_protocol::messages::RequestedDims;
    use std::{
        sync::mpsc as sync_mpsc,
        time::{Duration, Instant},
    };

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
            let _ = ctx.run(egui::RawInput::default(), |_| {});
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
        session.send(InputMsg::KeyBytes(
            b"printf '\\033]2;%s\\007' \"$PWD\"\r".to_vec(),
        ));
        wait_for(&session, &ctx, &repaints, |state| {
            state.screen.title() == Some(cwd.as_str())
        });
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
            .args(["sessions", "list", "--format", "json"])
            .output()
            .unwrap();
        assert!(
            listed.status.success(),
            "{}",
            String::from_utf8_lossy(&listed.stderr)
        );
        assert!(String::from_utf8_lossy(&listed.stdout).contains(&format!("{id:032x}")));
        let detached = std::process::Command::new(&executable)
            .args(["sessions", "detach"])
            .env("FELIS_SESSION_ID", format!("{id:032x}"))
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
