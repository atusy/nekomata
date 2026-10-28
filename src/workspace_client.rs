//! Ordered, asynchronous layout synchronization and a connection-scoped lease.
use std::{future::Future, time::Duration};

use anyhow::{Context, Result, anyhow};
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
    saved: Shared,
    task: tokio::task::JoinHandle<()>,
}

/// Layouts the daemon may hold for a window's workspace.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Saved {
    /// The last layout the daemon acknowledged.
    pub acknowledged: Option<Snapshot>,
    /// Layouts sent after it, oldest first, whose answers were lost with
    /// the connection; each may have landed.
    pub unconfirmed: Vec<Snapshot>,
}

impl Saved {
    /// Unanswered layouts worth remembering; older ones only cost a
    /// duplicate workspace if they turn out to be the retained one.
    const UNCONFIRMED: usize = 4;

    fn holds(&self, snapshot: &Snapshot) -> bool {
        self.acknowledged.as_ref() == Some(snapshot) || self.unconfirmed.contains(snapshot)
    }

    fn acknowledge(&mut self, snapshot: Snapshot) {
        self.acknowledged = Some(snapshot);
        self.unconfirmed.clear();
    }

    fn sending(&mut self, snapshot: &Snapshot) {
        if self.unconfirmed.len() == Self::UNCONFIRMED {
            self.unconfirmed.remove(0);
        }
        self.unconfirmed.push(snapshot.clone());
    }

    fn refused(&mut self, snapshot: &Snapshot) {
        self.unconfirmed
            .retain(|unconfirmed| unconfirmed != snapshot);
    }
}

type Shared = std::sync::Arc<std::sync::Mutex<Saved>>;

/// When a window that lost the daemon tries to reach it again: after 1 s,
/// doubling up to 30 s while attempts keep failing, and never while an
/// attempt is still unanswered or after the daemon refused one.
#[derive(Debug, Default)]
pub struct Reconnect {
    failures: u32,
    due: Option<std::time::Instant>,
    resuming: bool,
}

impl Reconnect {
    const MAX: Duration = Duration::from_secs(30);

    /// Follows an event of a window that holds a workspace.
    pub fn observe(&mut self, operation: Operation, succeeded: bool, now: std::time::Instant) {
        match (operation, succeeded) {
            (Operation::Disconnected, _) => {
                self.resuming = false;
                self.schedule(now);
            }
            (Operation::Resume, true) => *self = Self::default(),
            (Operation::Resume, false) => self.resuming = false,
            _ => {}
        }
    }

    /// Schedules the next attempt after another failure and returns its delay.
    fn schedule(&mut self, now: std::time::Instant) -> Duration {
        let delay = Duration::from_secs(1 << self.failures.min(5)).min(Self::MAX);
        self.failures += 1;
        self.due = Some(now + delay);
        delay
    }

    pub fn retry_now(&mut self) {
        self.due = Some(std::time::Instant::now());
    }

    /// Whether an attempt should start at `now`; it is then in flight.
    pub fn start(&mut self, now: std::time::Instant) -> bool {
        let due = !self.resuming && self.due.is_some_and(|due| due <= now);
        if due {
            self.due = None;
            self.resuming = true;
        }
        due
    }

    /// An attempt was started and has not been answered yet.
    pub fn resuming(&self) -> bool {
        self.resuming
    }
}

/// What a window that lost the daemon needs to lease its workspace again.
#[derive(Debug, Clone)]
pub struct Resume {
    /// The workspace the window held before the connection was lost.
    pub previous: u64,
    /// The layouts the daemon may hold for it.
    pub saved: Saved,
    /// The window's layout now, which the resumed workspace takes.
    pub current: Snapshot,
}

/// Re-leases a workspace and gives it `resume.current`, never overwriting
/// another window's retained layout. `Open` claims the most recently used
/// unattached workspace, which need not be this window's own:
///
/// 1. The window's own workspace is the one still holding exactly a layout
///    this window saved under the previous ID; it is reclaimed, whether
///    `Open` offered it or not.
/// 2. Otherwise a workspace without tabs, which is new, takes the layout.
/// 3. Otherwise the layout moves into a new workspace.
async fn resume(
    stream: &mut UnixStream,
    window: u32,
    resume: Resume,
    saved: &Shared,
) -> Result<Payload> {
    let own = |id: u64, retained: &Snapshot| id == resume.previous && resume.saved.holds(retained);
    // Every write below may land even if its answer is lost.
    saved.lock().unwrap().sending(&resume.current);
    let opened = daemon::exchange(stream, Request::Open { window }).await?;
    let instance = match &opened {
        Payload::Workspace { instance, .. } => instance.clone(),
        _ => anyhow::bail!("unexpected nekomata daemon reply"),
    };
    let (id, retained) = workspace(opened)?;
    let commit = Request::Finish {
        snapshot: Some(resume.current.clone()),
    };
    let id = if own(id, &retained) {
        daemon::exchange(stream, Request::Save(resume.current.clone())).await?;
        id
    } else if id != resume.previous
        && let Some(previous) = reserve(stream, daemon::Selection::Id(resume.previous)).await?
    {
        if own(previous.0, &previous.1) {
            daemon::exchange(stream, commit).await?;
            previous.0
        } else {
            daemon::exchange(stream, Request::Finish { snapshot: None }).await?;
            fresh(stream, id, &retained, &resume, commit).await?
        }
    } else {
        fresh(stream, id, &retained, &resume, commit).await?
    };
    Ok(Payload::Workspace {
        id,
        instance,
        snapshot: resume.current,
    })
}

/// Gives the layout to the opened workspace when it is new, else to a new one.
async fn fresh(
    stream: &mut UnixStream,
    id: u64,
    retained: &Snapshot,
    resume: &Resume,
    commit: Request,
) -> Result<u64> {
    if retained.window.tabs.is_empty() {
        daemon::exchange(stream, Request::Save(resume.current.clone())).await?;
        return Ok(id);
    }
    let (id, _) = reserve(stream, daemon::Selection::New)
        .await?
        .context("nekomata daemon refused a new workspace")?;
    daemon::exchange(stream, commit).await?;
    Ok(id)
}

/// Reserves `selection`, or `None` when the daemon refuses it.
async fn reserve(
    stream: &mut UnixStream,
    selection: daemon::Selection,
) -> Result<Option<(u64, Snapshot)>> {
    match daemon::exchange(stream, Request::Prepare(selection)).await {
        Ok(payload) => workspace(payload).map(Some),
        Err(error) if error.downcast_ref::<daemon::Rejection>().is_some() => Ok(None),
        Err(error) => Err(error),
    }
}

fn workspace(payload: Payload) -> Result<(u64, Snapshot)> {
    match payload {
        Payload::Workspace { id, snapshot, .. } => Ok((id, snapshot)),
        _ => anyhow::bail!("unexpected nekomata daemon reply"),
    }
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
        Self::launch(runtime, ctx, Saved::default(), |_| async move {
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
        // Until the resume lands, the daemon may still hold what the previous
        // connection saved, which the next attempt must still recognize.
        Self::launch(runtime, ctx, state.saved.clone(), |saved| async move {
            let resumed = async {
                let mut stream = connect.await?;
                let payload = resume(&mut stream, window, state, &saved).await?;
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

    fn launch<F>(
        runtime: &tokio::runtime::Handle,
        ctx: egui::Context,
        saved: Saved,
        open: impl FnOnce(Shared) -> F,
    ) -> Self
    where
        F: Future<Output = (Option<UnixStream>, Operation, Result<Payload, String>)>
            + Send
            + 'static,
    {
        let (tx, commands) = mpsc::channel(16);
        let (events_tx, events) = mpsc::unbounded_channel();
        let saved = Shared::new(saved.into());
        let acknowledged = saved.clone();
        let open = open(saved.clone());
        let task = runtime.spawn(async move {
            let (stream, operation, result) = open.await;
            if let Ok(Payload::Workspace { snapshot, .. }) = &result {
                acknowledged.lock().unwrap().acknowledge(snapshot.clone());
            }
            let sent = events_tx.send(Event { operation, result }).is_ok();
            ctx.request_repaint();
            if let (Some(stream), true) = (stream, sent) {
                serve(stream, commands, events_tx, acknowledged, ctx).await;
            }
        });
        Self {
            tx,
            events,
            saved,
            task,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// The layouts the daemon may hold, unlike ones merely queued.
    pub fn saved(&self) -> Saved {
        self.saved.lock().unwrap().clone()
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
    saved: Shared,
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
            Message::Command(command) => {
                let operation = command.operation();
                let result = match command {
                    Command::Save(snapshot) => {
                        saved.lock().unwrap().sending(&snapshot);
                        let result =
                            daemon::exchange(&mut stream, Request::Save(snapshot.clone())).await;
                        let mut saved = saved.lock().unwrap();
                        match &result {
                            Ok(_) => saved.acknowledge(snapshot),
                            // A refused save stored nothing.
                            Err(error) if error.downcast_ref::<daemon::Rejection>().is_some() => {
                                saved.refused(&snapshot);
                            }
                            Err(_) => {}
                        }
                        result
                    }
                    // A commit moves the window to another workspace; one
                    // whose answer was lost leaves the window on its source.
                    Command::Finish {
                        snapshot: Some(snapshot),
                    } => {
                        let request = Request::Finish {
                            snapshot: Some(snapshot.clone()),
                        };
                        let result = daemon::exchange(&mut stream, request).await;
                        if result.is_ok() {
                            *saved.lock().unwrap() = Saved {
                                acknowledged: Some(snapshot),
                                unconfirmed: vec![],
                            };
                        }
                        result
                    }
                    command => daemon::exchange(&mut stream, command.request()).await,
                };
                (operation, result)
            }
            Message::Prepare {
                snapshot,
                selection,
            } => {
                // The source save may land even when the reservation fails.
                saved.lock().unwrap().sending(&snapshot);
                let result = daemon::prepare(&mut stream, snapshot.clone(), selection).await;
                match &result {
                    Ok(_) => saved.lock().unwrap().acknowledge(snapshot),
                    // The window only sends layouts it built and the daemon
                    // accepts, so a refusal comes from the reservation, after
                    // the source save landed.
                    Err(error) if error.downcast_ref::<daemon::Rejection>().is_some() => {
                        saved.lock().unwrap().acknowledge(snapshot);
                    }
                    Err(_) => {}
                }
                (Operation::Prepare, result)
            }
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
                instance: "test-daemon".into(),
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
        let mut reconnect = Reconnect::default();
        assert!(!reconnect.start(now));
        let delays: Vec<_> = (0..8).map(|_| reconnect.schedule(now).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30, 30]);
        assert!(!reconnect.start(now));
        assert!(reconnect.start(now + Duration::from_secs(30)));
        assert!(reconnect.resuming());
    }

    #[test]
    fn a_lost_connection_schedules_one_attempt_at_a_time() {
        let now = std::time::Instant::now();
        let later = now + Duration::from_secs(60);
        let mut reconnect = Reconnect::default();
        reconnect.observe(Operation::Disconnected, false, now);
        assert!(!reconnect.start(now));
        assert!(reconnect.start(later));
        reconnect.retry_now();
        assert!(!reconnect.start(later), "an attempt is still unanswered");
        // The attempt could not reach a daemon: try again later.
        reconnect.observe(Operation::Disconnected, false, later);
        assert!(!reconnect.resuming());
        assert!(reconnect.start(later + Duration::from_secs(2)));
        reconnect.observe(Operation::Resume, true, later);
        assert!(!reconnect.resuming());
        assert!(!reconnect.start(later + Duration::from_secs(60)));
        // Backoff restarts from 1 s after a success.
        reconnect.observe(Operation::Disconnected, false, now);
        assert!(reconnect.start(now + Duration::from_secs(1)));
    }

    #[test]
    fn a_refused_attempt_is_not_retried_automatically() {
        let now = std::time::Instant::now();
        let mut reconnect = Reconnect::default();
        reconnect.observe(Operation::Disconnected, false, now);
        assert!(reconnect.start(now + Duration::from_secs(1)));
        reconnect.observe(Operation::Resume, false, now);
        assert!(!reconnect.resuming());
        assert!(!reconnect.start(now + Duration::from_secs(3600)));
        reconnect.retry_now();
        assert!(reconnect.start(std::time::Instant::now()));
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
    async fn only_acknowledged_layouts_count_as_saved() {
        let (mut client, mut daemon) = opened().await;
        assert_eq!(
            client.saved(),
            acknowledged(Snapshot::default()),
            "Open's layout"
        );
        let first = layout();
        let mut second = layout();
        second.window.font_size += 1.0;
        client.send(Command::Save(first.clone())).unwrap();
        assert!(receive(&mut daemon).await.is_some());
        respond(&mut daemon, Ok(Payload::Done)).await;
        client.send(Command::Save(second.clone())).unwrap();
        assert!(receive(&mut daemon).await.is_some());
        respond(&mut daemon, Err("invalid".into())).await;
        let mut third = layout();
        third.window.font_size += 2.0;
        client.send(Command::Save(third.clone())).unwrap();
        assert!(receive(&mut daemon).await.is_some());
        drop(daemon);
        while let Some(event) = client.events.recv().await {
            if event.operation == Operation::Disconnected {
                break;
            }
        }
        assert_eq!(
            client.saved(),
            Saved {
                acknowledged: Some(first),
                unconfirmed: vec![third],
            },
            "the refused save is forgotten; the lost one may have landed"
        );
    }

    #[tokio::test]
    async fn a_source_save_may_have_landed_when_the_reservation_is_lost() {
        let (mut client, mut daemon) = opened().await;
        let source = layout();
        client
            .prepare(source.clone(), daemon::Selection::New)
            .unwrap();
        assert!(matches!(receive(&mut daemon).await, Some(Request::Save(_))));
        respond(&mut daemon, Ok(Payload::Done)).await;
        assert!(matches!(
            receive(&mut daemon).await,
            Some(Request::Prepare(_))
        ));
        drop(daemon);
        assert_eq!(
            client.events.recv().await.unwrap().operation,
            Operation::Disconnected
        );
        assert!(client.saved().holds(&source));
    }

    #[tokio::test]
    async fn a_refused_reservation_still_saved_the_source() {
        let (mut client, mut daemon) = opened().await;
        let source = layout();
        client
            .prepare(source.clone(), daemon::Selection::Id(9))
            .unwrap();
        receive(&mut daemon).await.unwrap();
        respond(&mut daemon, Ok(Payload::Done)).await;
        receive(&mut daemon).await.unwrap();
        respond(&mut daemon, Err("workspace not found".into())).await;
        assert_eq!(
            client.events.recv().await.unwrap().operation,
            Operation::Prepare
        );
        assert_eq!(client.saved(), acknowledged(source));
    }

    #[tokio::test]
    async fn a_resume_whose_answer_is_lost_may_have_saved_its_layout() {
        let before = acknowledged(layout());
        let mut current = layout();
        current.window.font_size += 1.0;
        let (mut client, mut daemon) = resuming(Resume {
            previous: 3,
            saved: before.clone(),
            current: current.clone(),
        });
        receive(&mut daemon).await.unwrap();
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                instance: "test-daemon".into(),
                id: 3,
                snapshot: before.acknowledged.clone().unwrap(),
            }),
        )
        .await;
        assert!(matches!(receive(&mut daemon).await, Some(Request::Save(_))));
        drop(daemon);
        assert_eq!(
            client.events.recv().await.unwrap().operation,
            Operation::Disconnected
        );
        assert!(client.saved().holds(&before.acknowledged.unwrap()));
        assert!(client.saved().holds(&current));
    }

    #[tokio::test]
    async fn a_failed_resume_keeps_what_the_previous_connection_saved() {
        let saved = acknowledged(layout());
        let mut client = Client::resume_with(
            &tokio::runtime::Handle::current(),
            egui::Context::default(),
            7,
            Resume {
                previous: 3,
                saved: saved.clone(),
                current: Snapshot::default(),
            },
            async { Err(anyhow!("no daemon")) },
        );
        assert!(client.events.recv().await.is_some());
        assert_eq!(client.saved(), saved);
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
    async fn closing_gives_up_at_its_deadline_including_queued_saves() {
        let (client, mut daemon) = opened().await;
        // The daemon answers the queued save late and never answers the close.
        let _fake = tokio::spawn(async move {
            receive(&mut daemon).await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
            respond(&mut daemon, Ok(Payload::Done)).await;
            while receive(&mut daemon).await.is_some() {}
        });
        client.send(Command::Save(layout())).unwrap();
        let started = Instant::now();
        let deadline = started + Duration::from_millis(300);
        let error = client
            .close_by(Some(Snapshot::default()), deadline)
            .await
            .unwrap_err();
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(300) && elapsed < Duration::from_secs(2),
            "{elapsed:?}: {error:#}"
        );
    }

    fn acknowledged(snapshot: Snapshot) -> Saved {
        Saved {
            acknowledged: Some(snapshot),
            unconfirmed: vec![],
        }
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
        let Ok(Payload::Workspace { id, instance, .. }) = event.result else {
            panic!("expected a resumed workspace: {:?}", event.result.err())
        };
        assert_eq!(instance, "test-daemon");
        id
    }

    #[tokio::test]
    async fn resuming_fills_a_new_workspace_with_the_current_layout() {
        let current = layout();
        let (mut client, mut daemon) = resuming(Resume {
            previous: 3,
            saved: Saved::default(),
            current: current.clone(),
        });
        assert!(matches!(
            receive(&mut daemon).await,
            Some(Request::Open { window: 7 })
        ));
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                instance: "test-daemon".into(),
                id: 1,
                snapshot: Snapshot::default(),
            }),
        )
        .await;
        // A restarted daemon no longer has the previous workspace.
        assert!(matches!(
            receive(&mut daemon).await,
            Some(Request::Prepare(daemon::Selection::Id(3)))
        ));
        respond(&mut daemon, Err("workspace not found".into())).await;
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
        current.window.font_size += 1.0;
        let (mut client, mut daemon) = resuming(Resume {
            previous: 3,
            saved: acknowledged(sent.clone()),
            current: current.clone(),
        });
        receive(&mut daemon).await.unwrap();
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                instance: "test-daemon".into(),
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
            saved: acknowledged(current.clone()),
            current: current.clone(),
        });
        receive(&mut daemon).await.unwrap();
        let mut other = layout();
        other.window.font_size += 2.0;
        // Same ID, different layout: a restarted daemon's workspace.
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                instance: "test-daemon".into(),
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
                instance: "test-daemon".into(),
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
    async fn resuming_finds_the_windows_workspace_behind_a_more_recent_one() {
        let sent = layout();
        let mut current = sent.clone();
        current.window.font_size += 1.0;
        let (mut client, mut daemon) = resuming(Resume {
            previous: 3,
            saved: acknowledged(sent.clone()),
            current: current.clone(),
        });
        receive(&mut daemon).await.unwrap();
        let mut other = layout();
        other.window.font_size += 2.0;
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                instance: "test-daemon".into(),
                id: 5,
                snapshot: other,
            }),
        )
        .await;
        assert!(matches!(
            receive(&mut daemon).await,
            Some(Request::Prepare(daemon::Selection::Id(3)))
        ));
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                instance: "test-daemon".into(),
                id: 3,
                snapshot: sent,
            }),
        )
        .await;
        let Some(Request::Finish {
            snapshot: Some(saved),
        }) = receive(&mut daemon).await
        else {
            panic!("expected the layout to commit into the previous workspace")
        };
        assert_eq!(saved, current);
        respond(&mut daemon, Ok(Payload::Done)).await;
        assert_eq!(resumed(&mut client).await, 3);
    }

    #[tokio::test]
    async fn resuming_never_takes_an_identical_layout_under_another_id() {
        let current = layout();
        let (mut client, mut daemon) = resuming(Resume {
            previous: 3,
            saved: acknowledged(current.clone()),
            current: current.clone(),
        });
        receive(&mut daemon).await.unwrap();
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                instance: "test-daemon".into(),
                id: 4,
                snapshot: current.clone(),
            }),
        )
        .await;
        assert!(matches!(
            receive(&mut daemon).await,
            Some(Request::Prepare(daemon::Selection::Id(3)))
        ));
        // A restarted daemon's workspace 3 holds another window's layout.
        let mut other = layout();
        other.window.font_size += 2.0;
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                instance: "test-daemon".into(),
                id: 3,
                snapshot: other,
            }),
        )
        .await;
        assert!(matches!(
            receive(&mut daemon).await,
            Some(Request::Finish { snapshot: None })
        ));
        respond(&mut daemon, Ok(Payload::Done)).await;
        assert!(matches!(
            receive(&mut daemon).await,
            Some(Request::Prepare(daemon::Selection::New))
        ));
        respond(
            &mut daemon,
            Ok(Payload::Workspace {
                instance: "test-daemon".into(),
                id: 6,
                snapshot: Snapshot::default(),
            }),
        )
        .await;
        assert!(matches!(
            receive(&mut daemon).await,
            Some(Request::Finish { snapshot: Some(_) })
        ));
        respond(&mut daemon, Ok(Payload::Done)).await;
        assert_eq!(resumed(&mut client).await, 6);
    }

    #[tokio::test]
    async fn resuming_without_a_daemon_reports_disconnection() {
        let mut client = Client::resume_with(
            &tokio::runtime::Handle::current(),
            egui::Context::default(),
            7,
            Resume {
                previous: 3,
                saved: Saved::default(),
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
            saved: Saved::default(),
            current: layout(),
        });
        receive(&mut daemon).await.unwrap();
        respond(&mut daemon, Err("incompatible nekomata protocol".into())).await;
        let event = client.events.recv().await.unwrap();
        assert_eq!(event.operation, Operation::Resume);
        assert!(event.result.is_err());
    }
}
