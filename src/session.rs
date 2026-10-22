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
    ) -> Self {
        let state = Arc::new(Mutex::new(State {
            screen: ShadowScreen::new(24, 80),
            session_id: None,
            error: None,
            connected: false,
        }));
        let (tx, rx) = mpsc::channel(256);
        let shared = state.clone();
        let task = runtime.spawn(async move {
            let result = run(shared.clone(), ctx.clone(), rx, attach).await;
            let mut state = shared.lock().unwrap();
            state.connected = false;
            state.error = Some(match result {
                Ok(()) => "Session disconnected. The shell may still be running in felis.".into(),
                Err(err) => format!("{err:#}"),
            });
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
            cwd: std::env::current_dir()?.to_string_lossy().into_owned(),
            env: vec![],
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
                            let _ = conn.driver.decode::<felis_protocol::messages::PushMsg>(&payload)?;
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
