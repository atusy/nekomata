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

impl Client {
    pub fn start(runtime: &tokio::runtime::Runtime, ctx: egui::Context, window: u32) -> Self {
        Self::launch(runtime.handle(), ctx, window, daemon::connect())
    }

    fn launch(
        runtime: &tokio::runtime::Handle,
        ctx: egui::Context,
        window: u32,
        connect: impl Future<Output = Result<UnixStream>> + Send + 'static,
    ) -> Self {
        let (tx, commands) = mpsc::channel(16);
        let (events_tx, events) = mpsc::unbounded_channel();
        let task = runtime.spawn(async move {
            let opened = async {
                let mut stream = connect.await?;
                let payload = daemon::exchange(&mut stream, Request::Open { window }).await?;
                Ok::<_, anyhow::Error>((stream, payload))
            }
            .await;
            let (stream, payload) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    let _ = events_tx.send(Event {
                        operation: Operation::Open,
                        result: Err(format!("{error:#}")),
                    });
                    ctx.request_repaint();
                    return;
                }
            };
            if events_tx
                .send(Event {
                    operation: Operation::Open,
                    result: Ok(payload),
                })
                .is_err()
            {
                return;
            }
            ctx.request_repaint();
            serve(stream, commands, events_tx, ctx).await;
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
    while let Some(message) = commands.recv().await {
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
        let mut client = Client::launch(
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
}
