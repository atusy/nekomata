//! Cancellable operations on an attached Ops connection, independent of grid pulls.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use eframe::egui;
use felis_client_core::{AttachIntent, CarrierConnection, Offer, RemoteSpawn};
use felis_protocol::{
    MessageKind,
    messages::{
        ConnToClientMsg, GridDims, RegionPosition, RegionSource, RegionToClientMsg,
        RegionToDaemonMsg, Subject,
    },
};
use felis_transport::{Delivery, Incoming};
use tokio::sync::watch;

use crate::session::{State, Target};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Source {
    id: u128,
    generation: u64,
}

fn source(state: &Mutex<State>) -> Option<Source> {
    let state = state.lock().unwrap();
    state.connected.then_some(Source {
        id: state.session_id?,
        generation: state.reconnects,
    })
}

struct Job {
    cancel: watch::Sender<bool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

pub struct Output {
    pub data: Vec<u8>,
    pub position: Option<RegionPosition>,
    pub origin: felis_client_core::pipe::Origin,
    pub dims: GridDims,
    pub target: Target,
    pub edit: bool,
}
#[derive(Default)]
pub struct OutputState {
    generation: u64,
    source: Option<Source>,
    pub running: bool,
    pub result: Option<Result<Output, String>>,
}

pub struct Tools {
    pub output: Arc<Mutex<OutputState>>,
    output_task: Mutex<Option<Job>>,

    runtime: tokio::runtime::Handle,
    ctx: egui::Context,
    target: Target,
    session: Arc<Mutex<State>>,
}

impl Tools {
    pub fn new(
        runtime: &tokio::runtime::Runtime,
        ctx: egui::Context,
        target: Target,
        session: Arc<Mutex<State>>,
    ) -> Self {
        Self {
            output: Arc::default(),
            output_task: Mutex::default(),

            runtime: runtime.handle().clone(),
            ctx,
            target,
            session,
        }
    }

    pub fn cancel_output(&self) {
        self.output_task.lock().unwrap().take();
        let mut state = self.output.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
        state.running = false;
        state.result = None;
    }

    pub fn take_output(&self) -> Option<Result<Output, String>> {
        let live = source(&self.session);
        let mut state = self.output.lock().unwrap();
        let result = state.result.take()?;
        Some(if state.source == live {
            result
        } else {
            Err("The session changed; request output again".into())
        })
    }

    pub fn command_output(&self, edit: bool) {
        self.output_task.lock().unwrap().take();
        let from = source(&self.session);
        let generation = {
            let mut state = self.output.lock().unwrap();
            state.generation = state.generation.wrapping_add(1);
            state.source = from;
            state.running = from.is_some();
            state.result = from
                .is_none()
                .then(|| Err("Wait for the session to connect".into()));
            state.generation
        };
        let Some(from) = from else {
            return;
        };
        let (origin, dims) = {
            let state = self.session.lock().unwrap();
            (
                felis_client_core::pipe::Origin {
                    session_id: from.id,
                    host: match &self.target {
                        Target::Ssh { host, .. } => Some(host.clone()),
                        _ => None,
                    },
                    osc7: state.screen.cwd().map(str::to_owned),
                },
                GridDims {
                    rows: state.screen.screen().rows(),
                    cols: state.screen.screen().cols(),
                    pixel_w: 0,
                    pixel_h: 0,
                },
            )
        };
        let (cancel, mut cancelled) = watch::channel(false);
        *self.output_task.lock().unwrap() = Some(Job { cancel });
        let target = self.target.clone();
        let shared = self.output.clone();
        let session = self.session.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let result = region_request(&target, from.id, &mut cancelled)
                .await
                .map(|(data, position)| Output {
                    data,
                    position,
                    origin,
                    dims,
                    target,
                    edit,
                })
                .map_err(|e| format!("{e:#}"));
            let live = source(&session) == Some(from);
            let mut state = shared.lock().unwrap();
            if state.generation != generation {
                return;
            }
            state.running = false;
            state.result = Some(if live {
                result
            } else {
                Err("The session changed; request output again".into())
            });
            ctx.request_repaint();
        });
    }

    pub fn invalidate_changed_session(&self) {
        let live = source(&self.session);
        let mut output = self.output.lock().unwrap();
        if output.source != live && output.running {
            self.output_task.lock().unwrap().take();
            output.generation = output.generation.wrapping_add(1);
            output.running = false;
            output.result = Some(Err("The session changed; request output again".into()));
        }
    }
}

struct Ops {
    conn: CarrierConnection,
    _relay: Option<tokio::process::Child>,
}
async fn attached_ops(target: &Target, id: u128) -> Result<Ops> {
    let mut ops = if let Target::Ssh { host, socket } = target {
        let (conn, relay) = crate::ssh::connect(host, socket.as_deref(), Offer::ops()).await?;
        Ops {
            conn,
            _relay: Some(relay),
        }
    } else {
        Ops {
            conn: felis_client_core::connect_carrier(
                target.carrier(),
                Offer::ops(),
                RemoteSpawn::Refuse,
            )
            .await?,
            _relay: None,
        }
    };
    ops.conn
        .attach_by_prefix(format!("{id:032x}"), AttachIntent::Automatic)
        .await?;
    Ok(ops)
}

const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
async fn region_request(
    target: &Target,
    id: u128,
    cancelled: &mut watch::Receiver<bool>,
) -> Result<(Vec<u8>, Option<RegionPosition>)> {
    let mut ops = tokio::select! {
        _ = cancelled.changed() => bail!("output request cancelled"),
        connected = tokio::time::timeout(Duration::from_secs(5), attached_ops(target, id)) => connected.context("output connection timed out")??,
    };
    let request = ops.conn.driver.issue_request()?;
    tokio::time::timeout(
        Duration::from_secs(5),
        ops.conn.writer.send_correlated(
            &RegionToDaemonMsg::Request {
                source: RegionSource::CommandOutput,
                ansi: false,
            },
            felis_protocol::messages::Correlation::request(request),
        ),
    )
    .await
    .context("output request timed out")??;
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    loop {
        let frame = tokio::select! {
            _ = cancelled.changed() => { bail!("output request cancelled"); },
            _ = tokio::time::sleep_until(deadline) => { bail!("output request timed out"); },
            frame = ops.conn.next_frame() => frame?.context("output connection closed")?,
        };
        match ops.conn.driver.classify(&frame)? {
            Incoming::Payload(payload) if payload.kind == MessageKind::Region => {
                if let Delivery::Deliver(delivered) =
                    ops.conn.driver.decode::<RegionToClientMsg>(&payload)?
                {
                    let RegionToClientMsg::Reply { data, position, .. } = delivered.msg else {
                        bail!("Unexpected streamed output response");
                    };
                    if data.is_empty() {
                        bail!("No command output is available (shell integration required)");
                    }
                    if data.len() > MAX_OUTPUT_BYTES {
                        bail!("Command output exceeds the 8 MiB limit");
                    }
                    return Ok((data, position));
                }
            }
            Incoming::Control(ConnToClientMsg::Error {
                subject: Subject::Request(id),
                detail,
                ..
            }) if id == request => bail!("{detail}"),
            Incoming::Payload(payload) if payload.correlation.is_none() => {}
            Incoming::CancelIgnored { .. } => {}
            other => bail!("Unexpected output response: {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use felis_protocol::messages::InputMsg;
    #[test]
    #[ignore = "requires private felis daemon; run scripts/test-live.sh"]
    fn live_command_output_preserves_the_window_attachment() {
        let socket = std::env::var("NEKOMATA_TEST_SOCKET").expect("private test socket required");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let session = crate::session::Session::start(
            &runtime,
            ctx,
            Target::Local(socket.into()),
            None,
            None,
            vec![],
        );
        let wait = |predicate: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !predicate() {
                assert!(std::time::Instant::now() < deadline, "operation timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait(&|| session.state.lock().unwrap().connected);
        session.send(InputMsg::KeyBytes(b"printf '\\033]133;A\\007prompt> \\033]133;B\\007fake-command\\n\\033]133;C\\007OUTPUT_ONLY\\n\\033]133;D;0\\007'\r".to_vec()));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            session.output.command_output(false);
            wait(&|| !session.output.output.lock().unwrap().running);
            let result = session.output.take_output();
            if let Some(Ok(output)) = result {
                let text = String::from_utf8(output.data).unwrap();
                if text.contains("OUTPUT_ONLY") {
                    assert!(!text.contains("fake-command"), "{text}");
                    assert!(!text.contains('\u{1b}'), "{text:?}");
                    assert_eq!(
                        output.origin.session_id,
                        session.state.lock().unwrap().session_id.unwrap()
                    );
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "command output did not resolve"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(session.state.lock().unwrap().connected);
        assert!(!session.state.lock().unwrap().detached);
    }
    #[test]
    #[ignore = "requires private fake SSH relay; run scripts/test-live.sh"]
    fn live_ssh_carrier_output_preserves_explicit_socket_and_origin() {
        assert_eq!(std::env::var("NEKOMATA_TEST_SSH").as_deref(), Ok("1"));
        let socket = std::env::var("NEKOMATA_TEST_SOCKET").expect("private socket required");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let session = crate::session::Session::start(
            &runtime,
            ctx.clone(),
            Target::Local(socket.clone().into()),
            None,
            None,
            vec![],
        );
        let wait = |predicate: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !predicate() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "SSH operation timed out"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait(&|| session.state.lock().unwrap().connected);
        let quoted_socket = std::path::Path::new(&socket)
            .parent()
            .unwrap()
            .join("ops 'socket' $(printf unused).sock");
        std::os::unix::fs::symlink(&socket, &quoted_socket).unwrap();
        let tools = Tools::new(
            &runtime,
            ctx,
            Target::Ssh {
                host: "nekomata-test-host".into(),
                socket: Some(quoted_socket.to_str().unwrap().into()),
            },
            session.state.clone(),
        );
        session.send(InputMsg::KeyBytes(b"printf '\\033]133;A\\007prompt> \\033]133;B\\007fake-command\\n\\033]133;C\\007SSH_OUTPUT_ONLY\\n\\033]133;D;0\\007'\r".to_vec()));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            tools.command_output(false);
            wait(&|| !tools.output.lock().unwrap().running);
            if let Some(Ok(output)) = tools.take_output() {
                assert_eq!(output.origin.host.as_deref(), Some("nekomata-test-host"));
                if String::from_utf8(output.data)
                    .unwrap()
                    .contains("SSH_OUTPUT_ONLY")
                {
                    break;
                }
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(session.state.lock().unwrap().connected);
        assert!(!session.state.lock().unwrap().detached);
        std::fs::remove_file(quoted_socket).unwrap();
    }
}
