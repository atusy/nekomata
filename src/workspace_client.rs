//! Ordered, asynchronous layout synchronization and a connection-scoped lease.
use std::{future::Future, time::Duration};

use anyhow::{Result, anyhow};
use eframe::egui;
use tokio::{
    net::UnixStream,
    sync::{mpsc, oneshot},
    time::Instant,
};

use crate::{
    daemon::{self, Payload, Request},
    workspace::Snapshot,
};

/// How long closing a window may wait for the daemon in total, including
/// commands still queued ahead of the close.
pub const CLOSE_DEADLINE: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Open,
    Resume,
    Disconnected,
    Save,
    List,
    Prepare,
    Finish,
}

pub struct Event {
    pub operation: Operation,
    pub result: Result<Payload, String>,
}

/// Requests a window sends over its lease once the workspace is open.
#[derive(Debug, Clone)]
pub enum Command {
    Save(Snapshot),
    List,
    Finish { snapshot: Option<Snapshot> },
}

impl Command {
    fn operation(&self) -> Operation {
        match self {
            Self::Save(_) => Operation::Save,
            Self::List => Operation::List,
            Self::Finish { .. } => Operation::Finish,
        }
    }

    fn request(self) -> Request {
        match self {
            Self::Save(snapshot) => Request::Save(snapshot),
            Self::List => Request::List,
            Self::Finish { snapshot } => Request::Finish { snapshot },
        }
    }
}

enum Message {
    Command(Command),
    Prepare {
        snapshot: Snapshot,
        selection: daemon::Selection,
    },
    Close {
        snapshot: Option<Snapshot>,
        deadline: Instant,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

pub struct Client {
    tx: mpsc::Sender<Message>,
    pub events: mpsc::UnboundedReceiver<Event>,
    task: tokio::task::JoinHandle<()>,
}

/// When a window that lost the daemon tries to reach it again: after 1 s,
/// doubling up to 30 s while attempts keep failing.
#[derive(Debug, Default)]
pub struct Backoff {
    failures: u32,
    due: Option<std::time::Instant>,
}

impl Backoff {
    const MAX: Duration = Duration::from_secs(30);

    /// Schedules the next attempt after another failure and returns its delay.
    pub fn schedule(&mut self, now: std::time::Instant) -> Duration {
        let delay = Duration::from_secs(1 << self.failures.min(5)).min(Self::MAX);
        self.failures += 1;
        self.due = Some(now + delay);
        delay
    }

    pub fn retry_now(&mut self) {
        self.due = Some(std::time::Instant::now());
    }

    /// Whether an attempt is due at `now`; it is then no longer scheduled.
    pub fn take_due(&mut self, now: std::time::Instant) -> bool {
        let due = self.due.is_some_and(|due| due <= now);
        if due {
            self.due = None;
        }
        due
    }
}

/// What a window that lost the daemon needs to lease its workspace again.
#[derive(Debug, Clone)]
pub struct Resume {
    /// The workspace the window held before the connection was lost.
    pub previous: u64,
    /// The last snapshot the window sent for it.
    pub sent: Option<Snapshot>,
    /// The window's layout now, which the resumed workspace takes.
    pub current: Snapshot,
}

/// Whether the workspace a fresh `Open` claimed may take the window's
/// layout: one without tabs is new, and one still holding exactly what this
/// window last sent under its ID is the window's own. Any other is somebody
/// else's retained layout, which must not be overwritten.
fn reclaimable(resume: &Resume, id: u64, retained: &Snapshot) -> bool {
    retained.tabs.is_empty() || (id == resume.previous && resume.sent.as_ref() == Some(retained))
}

async fn resume(stream: &mut UnixStream, window: u32, resume: Resume) -> Result<Payload> {
    let Payload::Workspace { id, snapshot } =
        daemon::exchange(stream, Request::Open { window }).await?
    else {
        anyhow::bail!("unexpected nekomata daemon reply");
    };
    let id = if reclaimable(&resume, id, &snapshot) {
        daemon::exchange(stream, Request::Save(resume.current.clone())).await?;
        id
    } else {
        let Payload::Workspace { id, .. } =
            daemon::exchange(stream, Request::Prepare(daemon::Selection::New)).await?
        else {
            anyhow::bail!("unexpected nekomata daemon reply");
        };
        daemon::exchange(
            stream,
            Request::Finish {
                snapshot: Some(resume.current.clone()),
            },
        )
        .await?;
        id
    };
    Ok(Payload::Workspace {
        id,
        snapshot: resume.current,
    })
}

impl Client {
    pub fn start(runtime: &tokio::runtime::Runtime, ctx: egui::Context, window: u32) -> Self {
        Self::open_with(runtime.handle(), ctx, window, daemon::connect())
    }

    /// Leases a workspace for a window that lost its daemon connection,
    /// starting the daemon if needed, and gives it the window's layout.
    pub fn resume(
        runtime: &tokio::runtime::Runtime,
        ctx: egui::Context,
        window: u32,
        state: Resume,
    ) -> Self {
        Self::resume_with(runtime.handle(), ctx, window, state, daemon::connect())
    }

    fn open_with(
        runtime: &tokio::runtime::Handle,
        ctx: egui::Context,
        window: u32,
        connect: impl Future<Output = Result<UnixStream>> + Send + 'static,
    ) -> Self {
        Self::launch(runtime, ctx, async move {
            let opened = async {
                let mut stream = connect.await?;
                let payload = daemon::exchange(&mut stream, Request::Open { window }).await?;
                Ok::<_, anyhow::Error>((stream, payload))
            }
            .await;
            match opened {
                Ok((stream, payload)) => (Some(stream), Operation::Open, Ok(payload)),
                Err(error) => (None, Operation::Open, Err(format!("{error:#}"))),
            }
        })
    }

    fn resume_with(
        runtime: &tokio::runtime::Handle,
        ctx: egui::Context,
        window: u32,
        state: Resume,
        connect: impl Future<Output = Result<UnixStream>> + Send + 'static,
    ) -> Self {
        Self::launch(runtime, ctx, async move {
            let resumed = async {
                let mut stream = connect.await?;
                let payload = resume(&mut stream, window, state).await?;
                Ok::<_, anyhow::Error>((stream, payload))
            }
            .await;
            match resumed {
                Ok((stream, payload)) => (Some(stream), Operation::Resume, Ok(payload)),
                // A refusal will not change by itself; a lost or absent daemon may.
                Err(error) if error.downcast_ref::<daemon::Rejection>().is_some() => {
                    (None, Operation::Resume, Err(format!("{error:#}")))
                }
                Err(error) => (None, Operation::Disconnected, Err(format!("{error:#}"))),
            }
        })
    }

    fn launch(
        runtime: &tokio::runtime::Handle,
        ctx: egui::Context,
        open: impl Future<Output = (Option<UnixStream>, Operation, Result<Payload, String>)>
        + Send
        + 'static,
    ) -> Self {
        let (tx, commands) = mpsc::channel(16);
        let (events_tx, events) = mpsc::unbounded_channel();
        let task = runtime.spawn(async move {
            let (stream, operation, result) = open.await;
            let sent = events_tx.send(Event { operation, result }).is_ok();
            ctx.request_repaint();
            if let (Some(stream), true) = (stream, sent) {
                serve(stream, commands, events_tx, ctx).await;
            }
        });
        Self { tx, events, task }
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    pub fn send(&self, command: Command) -> Result<()> {
        self.tx
            .try_send(Message::Command(command))
            .map_err(|error| anyhow!("nekomata daemon command unavailable: {error}"))
    }

    pub fn prepare(&self, snapshot: Snapshot, selection: daemon::Selection) -> Result<()> {
        self.tx
            .try_send(Message::Prepare {
                snapshot,
                selection,
            })
            .map_err(|error| anyhow!("nekomata daemon command unavailable: {error}"))
    }

    /// Saves `snapshot` when given, then detaches, blocking the calling thread
    /// for at most [`CLOSE_DEADLINE`].
    pub fn close(
        &self,
        runtime: &tokio::runtime::Runtime,
        snapshot: Option<Snapshot>,
    ) -> Result<()> {
        runtime.block_on(self.close_by(snapshot, Instant::now() + CLOSE_DEADLINE))
    }

    async fn close_by(&self, snapshot: Option<Snapshot>, deadline: Instant) -> Result<()> {
        tokio::time::timeout_at(deadline, async {
            let (reply, result) = oneshot::channel();
            self.tx
                .send(Message::Close {
                    snapshot,
                    deadline,
                    reply,
                })
                .await?;
            result.await?.map_err(anyhow::Error::msg)
        })
        .await
        .map_err(|_| anyhow!("nekomata daemon did not acknowledge in time"))?
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(
    mut stream: UnixStream,
    mut commands: mpsc::Receiver<Message>,
    events: mpsc::UnboundedSender<Event>,
    ctx: egui::Context,
) {
    loop {
        let message = tokio::select! {
            message = commands.recv() => match message {
                Some(message) => message,
                None => return,
            },
            // The daemon only ever answers, so anything readable while no
            // request is outstanding is the connection ending.
            _ = stream.readable() => {
                let mut byte = [0; 1];
                match stream.try_read(&mut byte) {
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
                    Ok(0) => {
                        disconnected(&events, &ctx, "nekomata daemon closed the connection".into());
                    }
                    Ok(_) => disconnected(&events, &ctx, "unexpected nekomata daemon message".into()),
                    Err(error) => disconnected(&events, &ctx, format!("{error:#}")),
                }
                return;
            }
        };
        let (operation, result) = match message {
            Message::Command(command) => (
                command.operation(),
                daemon::exchange(&mut stream, command.request()).await,
            ),
            Message::Prepare {
                snapshot,
                selection,
            } => (
                Operation::Prepare,
                daemon::prepare(&mut stream, snapshot, selection).await,
            ),
            Message::Close {
                snapshot,
                deadline,
                reply,
            } => {
                let result = tokio::time::timeout_at(deadline, close(&mut stream, snapshot))
                    .await
                    .unwrap_or_else(|_| Err("nekomata daemon did not acknowledge in time".into()));
                drop(stream);
                let _ = reply.send(result);
                return;
            }
        };
        let disconnected = result
            .as_ref()
            .is_err_and(|error| error.downcast_ref::<daemon::Rejection>().is_none());
        let operation = if disconnected {
            Operation::Disconnected
        } else {
            operation
        };
        let result = result.map_err(|error| format!("{error:#}"));
        if operation != Operation::Save || result.is_err() {
            let _ = events.send(Event { operation, result });
            ctx.request_repaint();
        }
        if disconnected {
            return;
        }
    }
}

fn disconnected(events: &mpsc::UnboundedSender<Event>, ctx: &egui::Context, error: String) {
    let _ = events.send(Event {
        operation: Operation::Disconnected,
        result: Err(error),
    });
    ctx.request_repaint();
}

async fn close(stream: &mut UnixStream, snapshot: Option<Snapshot>) -> Result<(), String> {
    if let Some(snapshot) = snapshot {
        daemon::exchange(stream, Request::Save(snapshot))
            .await
            .map_err(|error| format!("{error:#}"))?;
    }
    daemon::exchange(stream, Request::Detach)
        .await
        .map(|_| ())
        .map_err(|error| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One framed request as the daemon sees it.
    async fn receive(stream: &mut UnixStream) -> Option<Request> {
        let len = stream.read_u32().await.ok()? as usize;
        let mut bytes = vec![0; len];
        stream.read_exact(&mut bytes).await.ok()?;
        let envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        Some(serde_json::from_value(envelope["request"].clone()).unwrap())
    }

    async fn respond(stream: &mut UnixStream, response: Result<Payload, String>) {
        let bytes = serde_json::to_vec(&response).unwrap();
        stream.write_u32(bytes.len() as u32).await.unwrap();
        stream.write_all(&bytes).await.unwrap();
    }

    /// A client whose Open the fake daemon has already answered.
    async fn opened() -> (Client, UnixStream) {
        let (client, mut daemon) = UnixStream::pair().unwrap();
        let mut client = Client::open_with(
            &tokio::runtime::Handle::current(),
            egui::Context::default(),
            7,
            async move { Ok(client) },
        );
        assert!(matches!(
            receive(&mut daemon).await,
            Some(Request::Open { window: 7 })
        ));
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                id: 1,
                snapshot: Snapshot::default(),
            }),
        )
        .await;
        let event = client.events.recv().await.unwrap();
        assert_eq!(event.operation, Operation::Open);
        (client, daemon)
    }

    #[test]
    fn reconnect_attempts_back_off_up_to_thirty_seconds() {
        let now = std::time::Instant::now();
        let mut backoff = Backoff::default();
        assert!(!backoff.take_due(now));
        let delays: Vec<_> = (0..8).map(|_| backoff.schedule(now).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30, 30]);
        assert!(!backoff.take_due(now));
        assert!(backoff.take_due(now + Duration::from_secs(30)));
        assert!(!backoff.take_due(now + Duration::from_secs(30)), "once");
        backoff.retry_now();
        assert!(backoff.take_due(std::time::Instant::now()));
    }

    #[tokio::test]
    async fn a_rejected_command_keeps_the_connection() {
        let (mut client, mut daemon) = opened().await;
        client.send(Command::List).unwrap();
        assert!(matches!(receive(&mut daemon).await, Some(Request::List)));
        respond(&mut daemon, Err("no".into())).await;
        let event = client.events.recv().await.unwrap();
        assert_eq!(event.operation, Operation::List);
        assert_eq!(event.result.unwrap_err(), "no");
        client.send(Command::List).unwrap();
        assert!(matches!(receive(&mut daemon).await, Some(Request::List)));
    }

    #[tokio::test]
    async fn a_lost_daemon_reports_disconnection_and_stops() {
        let (mut client, mut daemon) = opened().await;
        client.send(Command::Finish { snapshot: None }).unwrap();
        assert!(receive(&mut daemon).await.is_some());
        drop(daemon);
        let event = client.events.recv().await.unwrap();
        assert_eq!(event.operation, Operation::Disconnected);
        assert!(event.result.is_err());
        assert!(client.events.recv().await.is_none());
        assert!(client.is_closed());
    }

    #[tokio::test]
    async fn a_daemon_lost_while_idle_is_noticed_without_a_command() {
        let (mut client, daemon) = opened().await;
        drop(daemon);
        let event = tokio::time::timeout(Duration::from_secs(5), client.events.recv())
            .await
            .expect("the loss is reported while no command is pending")
            .unwrap();
        assert_eq!(event.operation, Operation::Disconnected);
        assert!(client.events.recv().await.is_none());
    }

    #[tokio::test]
    async fn successful_saves_are_silent() {
        let (mut client, mut daemon) = opened().await;
        client.send(Command::Save(Snapshot::default())).unwrap();
        client.send(Command::List).unwrap();
        assert!(matches!(receive(&mut daemon).await, Some(Request::Save(_))));
        respond(&mut daemon, Ok(Payload::Done)).await;
        assert!(matches!(receive(&mut daemon).await, Some(Request::List)));
        respond(&mut daemon, Ok(Payload::Workspaces(vec![]))).await;
        assert_eq!(
            client.events.recv().await.unwrap().operation,
            Operation::List
        );
    }

    #[tokio::test]
    async fn closing_saves_before_detaching() {
        let (client, mut daemon) = opened().await;
        let fake = tokio::spawn(async move {
            let mut seen = vec![];
            while let Some(request) = receive(&mut daemon).await {
                seen.push(request);
                respond(&mut daemon, Ok(Payload::Done)).await;
            }
            seen
        });
        client
            .close_by(
                Some(Snapshot::default()),
                Instant::now() + Duration::from_secs(5),
            )
            .await
            .unwrap();
        let seen = fake.await.unwrap();
        assert!(matches!(
            seen.as_slice(),
            [Request::Save(_), Request::Detach]
        ));
    }

    #[tokio::test]
    async fn a_rejected_close_save_does_not_detach() {
        let (client, mut daemon) = opened().await;
        let fake = tokio::spawn(async move {
            let mut seen = vec![];
            while let Some(request) = receive(&mut daemon).await {
                seen.push(request);
                respond(&mut daemon, Err("invalid".into())).await;
            }
            seen
        });
        let error = client
            .close_by(
                Some(Snapshot::default()),
                Instant::now() + Duration::from_secs(5),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("invalid"), "{error:#}");
        let seen = fake.await.unwrap();
        assert!(matches!(seen.as_slice(), [Request::Save(_)]));
    }

    #[tokio::test]
    async fn closing_gives_up_at_one_shared_deadline() {
        let (client, mut daemon) = opened().await;
        // The daemon reads requests but never answers.
        let _fake = tokio::spawn(async move { while receive(&mut daemon).await.is_some() {} });
        let started = Instant::now();
        let error = client
            .close_by(
                Some(Snapshot::default()),
                started + Duration::from_millis(200),
            )
            .await
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1), "{error:#}");
    }

    /// A layout with one tab, unlike the empty one a new workspace has.
    fn layout() -> Snapshot {
        let mut snapshot = Snapshot::default();
        crate::workspace_switch::add_first_tab(
            &mut snapshot,
            14.0,
            crate::session::Target::Local("/felis.sock".into()),
        );
        snapshot
    }

    fn resuming(state: Resume) -> (Client, UnixStream) {
        let (client, daemon) = UnixStream::pair().unwrap();
        let client = Client::resume_with(
            &tokio::runtime::Handle::current(),
            egui::Context::default(),
            7,
            state,
            async move { Ok(client) },
        );
        (client, daemon)
    }

    async fn resumed(client: &mut Client) -> u64 {
        let event = client.events.recv().await.unwrap();
        assert_eq!(event.operation, Operation::Resume);
        let Ok(Payload::Workspace { id, .. }) = event.result else {
            panic!("expected a resumed workspace: {:?}", event.result.err())
        };
        id
    }

    #[tokio::test]
    async fn resuming_fills_a_new_workspace_with_the_current_layout() {
        let current = layout();
        let (mut client, mut daemon) = resuming(Resume {
            previous: 3,
            sent: None,
            current: current.clone(),
        });
        assert!(matches!(
            receive(&mut daemon).await,
            Some(Request::Open { window: 7 })
        ));
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                id: 1,
                snapshot: Snapshot::default(),
            }),
        )
        .await;
        let Some(Request::Save(saved)) = receive(&mut daemon).await else {
            panic!("expected the layout to be saved")
        };
        assert_eq!(saved, current);
        respond(&mut daemon, Ok(Payload::Done)).await;
        assert_eq!(resumed(&mut client).await, 1);
        // The resumed connection carries later commands.
        client.send(Command::List).unwrap();
        assert!(matches!(receive(&mut daemon).await, Some(Request::List)));
    }

    #[tokio::test]
    async fn resuming_reclaims_the_windows_own_retained_workspace() {
        let sent = layout();
        let mut current = sent.clone();
        current.font_size += 1.0;
        let (mut client, mut daemon) = resuming(Resume {
            previous: 3,
            sent: Some(sent.clone()),
            current: current.clone(),
        });
        receive(&mut daemon).await.unwrap();
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                id: 3,
                snapshot: sent,
            }),
        )
        .await;
        let Some(Request::Save(saved)) = receive(&mut daemon).await else {
            panic!("expected the layout to be saved")
        };
        assert_eq!(saved, current);
        respond(&mut daemon, Ok(Payload::Done)).await;
        assert_eq!(resumed(&mut client).await, 3);
    }

    #[tokio::test]
    async fn resuming_never_overwrites_another_retained_workspace() {
        let current = layout();
        let (mut client, mut daemon) = resuming(Resume {
            previous: 3,
            sent: Some(current.clone()),
            current: current.clone(),
        });
        receive(&mut daemon).await.unwrap();
        let mut other = layout();
        other.font_size += 2.0;
        // Same ID, different layout: a restarted daemon's workspace.
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                id: 3,
                snapshot: other,
            }),
        )
        .await;
        assert!(matches!(
            receive(&mut daemon).await,
            Some(Request::Prepare(daemon::Selection::New))
        ));
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                id: 4,
                snapshot: Snapshot::default(),
            }),
        )
        .await;
        let Some(Request::Finish {
            snapshot: Some(saved),
        }) = receive(&mut daemon).await
        else {
            panic!("expected the layout to commit into the new workspace")
        };
        assert_eq!(saved, current);
        respond(&mut daemon, Ok(Payload::Done)).await;
        assert_eq!(resumed(&mut client).await, 4);
    }

    #[tokio::test]
    async fn resuming_without_a_daemon_reports_disconnection() {
        let mut client = Client::resume_with(
            &tokio::runtime::Handle::current(),
            egui::Context::default(),
            7,
            Resume {
                previous: 3,
                sent: None,
                current: layout(),
            },
            async { Err(anyhow!("no daemon")) },
        );
        let event = client.events.recv().await.unwrap();
        assert_eq!(event.operation, Operation::Disconnected);
        assert!(client.events.recv().await.is_none());
    }

    #[tokio::test]
    async fn a_refused_resume_is_not_a_disconnection() {
        let (mut client, mut daemon) = resuming(Resume {
            previous: 3,
            sent: None,
            current: layout(),
        });
        receive(&mut daemon).await.unwrap();
        respond(&mut daemon, Err("incompatible nekomata protocol".into())).await;
        let event = client.events.recv().await.unwrap();
        assert_eq!(event.operation, Operation::Resume);
        assert!(event.result.is_err());
    }
}
