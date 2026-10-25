//! Versioned session control. Terminal traffic stays between the GUI and felis.
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::Notify,
};

use crate::workspace::Snapshot;

const VERSION: u32 = 1;
const MAX_MESSAGE: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Selection {
    Latest,
    New,
    Id(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Open { window: u32 },
    Save(Snapshot),
    List,
    Prepare(Selection),
    Finish { snapshot: Option<Snapshot> },
    Stop,
    Detach,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub id: u64,
    pub window: Option<u32>,
    pub tabs: usize,
    pub panes: usize,
    pub last_used: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Payload {
    Session { id: u64, snapshot: Snapshot },
    Sessions(Vec<Summary>),
    Done,
}

#[derive(Debug)]
pub struct Rejection(String);

impl std::fmt::Display for Rejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Rejection {}

type Response = Result<Payload, String>;

#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u32,
    request: Request,
}

struct Record {
    snapshot: Snapshot,
    owner: Option<(u64, u32)>,
    last_used: u64,
}

#[derive(Default)]
struct Registry {
    sessions: BTreeMap<u64, Record>,
    next_id: u64,
    clock: u64,
}

impl Registry {
    fn claim(&mut self, owner: u64, window: u32, selection: Selection) -> Result<u64> {
        let selected = match selection {
            Selection::Latest => self
                .sessions
                .iter()
                .filter(|(_, record)| record.owner.is_none())
                .max_by_key(|(id, record)| (record.last_used, **id))
                .map(|(&id, _)| id),
            Selection::New => None,
            Selection::Id(id) => {
                let record = self
                    .sessions
                    .get(&id)
                    .context("nekomata session not found")?;
                ensure!(
                    record.owner.is_none(),
                    "session is already attached to a window"
                );
                Some(id)
            }
        };
        let id = selected.unwrap_or_else(|| {
            self.next_id += 1;
            let id = self.next_id;
            self.sessions.insert(
                id,
                Record {
                    snapshot: Snapshot::default(),
                    owner: None,
                    last_used: 0,
                },
            );
            id
        });
        self.sessions.get_mut(&id).unwrap().owner = Some((owner, window));
        Ok(id)
    }

    fn touch(&mut self, id: u64) {
        self.clock += 1;
        self.sessions.get_mut(&id).unwrap().last_used = self.clock;
    }

    fn snapshot(&self, id: u64) -> Payload {
        Payload::Session {
            id,
            snapshot: self.sessions[&id].snapshot.clone(),
        }
    }

    fn release(&mut self, owner: u64, current: Option<u64>) {
        if let Some(id) = current {
            self.touch(id);
        }
        for record in self.sessions.values_mut() {
            if record.owner.is_some_and(|(id, _)| id == owner) {
                record.owner = None;
            }
        }
    }

    fn list(&self) -> Vec<Summary> {
        let mut sessions: Vec<_> = self
            .sessions
            .iter()
            .map(|(&id, record)| Summary {
                id,
                window: record.owner.map(|(_, window)| window),
                tabs: record.snapshot.tabs.len(),
                panes: record.snapshot.panes.len(),
                last_used: record.last_used,
            })
            .collect();
        sessions.sort_by_key(|s| std::cmp::Reverse((s.last_used, s.id)));
        sessions
    }
}

pub fn socket() -> Result<PathBuf> {
    Ok(crate::control::directory()?.join("daemon.sock"))
}

async fn read_json<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> Result<T> {
    let len = stream.read_u32().await? as usize;
    ensure!(len <= MAX_MESSAGE, "daemon message is too large");
    let mut bytes = vec![0; len];
    stream.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn write_json<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= MAX_MESSAGE, "daemon message is too large");
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}

pub async fn exchange(stream: &mut UnixStream, request: Request) -> Result<Payload> {
    tokio::time::timeout(DEADLINE, async {
        write_json(
            stream,
            &Envelope {
                version: VERSION,
                request,
            },
        )
        .await?;
        let response: Response = read_json(stream).await?;
        response.map_err(|error| Rejection(error).into())
    })
    .await
    .context("nekomata daemon request timed out")?
}

pub async fn prepare(
    stream: &mut UnixStream,
    snapshot: Snapshot,
    selection: Selection,
) -> Result<Payload> {
    exchange(stream, Request::Save(snapshot)).await?;
    exchange(stream, Request::Prepare(selection)).await
}

pub async fn connect() -> Result<UnixStream> {
    let path = socket()?;
    if let Ok(stream) = UnixStream::connect(&path).await {
        return Ok(stream);
    }
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .args(["daemon", "run"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .context("start nekomata daemon")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    tokio::time::timeout(DEADLINE, async {
        loop {
            if let Ok(stream) = UnixStream::connect(&path).await {
                return stream;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .context("nekomata daemon did not start")
}

struct SocketGuard {
    path: PathBuf,
    _lock: File,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn run() -> Result<()> {
    let path = socket()?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path.with_extension("lock"))?;
    if lock.try_lock().is_err() {
        return Ok(());
    }
    if path.exists() {
        std::fs::remove_file(&path).context("remove stale nekomata daemon socket")?;
    }
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = SocketGuard {
        path: path.clone(),
        _lock: lock,
    };
    runtime.block_on(serve(&path))
}

async fn serve(path: &Path) -> Result<()> {
    let listener = UnixListener::bind(path)?;
    let registry = Arc::new(Mutex::new(Registry::default()));
    let stop = Arc::new(Notify::new());
    let mut owner = 0;
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result?;
                owner += 1;
                let registry = registry.clone();
                let stop = stop.clone();
                tokio::spawn(async move {
                    let _ = serve_connection(stream, registry, stop, owner).await;
                });
            }
            _ = stop.notified() => return Ok(()),
        }
    }
}

async fn serve_connection(
    mut stream: UnixStream,
    registry: Arc<Mutex<Registry>>,
    stop: Arc<Notify>,
    owner: u64,
) -> Result<()> {
    let mut current = None;
    let mut pending = None;
    let mut window = None;
    let result = async {
        loop {
            let envelope: Envelope = read_json(&mut stream).await?;
            let stopping = matches!(envelope.request, Request::Stop);
            let response = if envelope.version != VERSION {
                Err(format!(
                    "incompatible nekomata protocol: client {}, daemon {VERSION}",
                    envelope.version
                ))
            } else {
                handle(
                    &mut registry.lock().unwrap(),
                    owner,
                    &mut window,
                    &mut current,
                    &mut pending,
                    envelope.request,
                )
                .map_err(|error| error.to_string())
            };
            write_json(&mut stream, &response).await?;
            if stopping && response.is_ok() {
                stop.notify_one();
                return Ok(());
            }
        }
    }
    .await;
    registry.lock().unwrap().release(owner, current);
    result
}

fn handle(
    registry: &mut Registry,
    owner: u64,
    window: &mut Option<u32>,
    current: &mut Option<u64>,
    pending: &mut Option<u64>,
    request: Request,
) -> Result<Payload> {
    match request {
        Request::Open { window: id } => {
            ensure!(current.is_none(), "window already has a session");
            ensure!(id != 0, "invalid window ID");
            let session = registry.claim(owner, id, Selection::Latest)?;
            registry.touch(session);
            *current = Some(session);
            *window = Some(id);
            Ok(registry.snapshot(session))
        }
        Request::List => Ok(Payload::Sessions(registry.list())),
        Request::Save(snapshot) => {
            let id = current.context("window has no session")?;
            snapshot.validate()?;
            registry.sessions.get_mut(&id).unwrap().snapshot = snapshot;
            Ok(Payload::Done)
        }
        Request::Prepare(selection) => {
            ensure!(pending.is_none(), "session switch already in progress");
            let id = registry.claim(owner, window.context("window has no session")?, selection)?;
            *pending = Some(id);
            Ok(registry.snapshot(id))
        }
        Request::Finish { snapshot } => {
            let id = pending.context("no session switch in progress")?;
            if let Some(snapshot) = snapshot {
                snapshot.validate()?;
                let source = current.context("window has no session")?;
                registry.sessions.get_mut(&source).unwrap().owner = None;
                registry.touch(source);
                registry.sessions.get_mut(&id).unwrap().snapshot = snapshot;
                registry.touch(id);
                *current = Some(id);
            } else {
                registry.sessions.get_mut(&id).unwrap().owner = None;
            }
            *pending = None;
            Ok(Payload::Done)
        }
        Request::Detach => {
            registry.release(owner, current.take());
            *pending = None;
            *window = None;
            Ok(Payload::Done)
        }
        Request::Stop => {
            ensure!(
                registry.sessions.values().all(|s| s.owner.is_none()),
                "cannot stop nekomata daemon while windows are attached"
            );
            Ok(Payload::Done)
        }
    }
}

pub fn command(stop: bool) -> Result<String> {
    tokio::runtime::Runtime::new()?.block_on(async {
        let mut stream = if stop {
            UnixStream::connect(socket()?).await?
        } else {
            connect().await?
        };
        match exchange(
            &mut stream,
            if stop { Request::Stop } else { Request::List },
        )
        .await?
        {
            Payload::Done => Ok("Stopped nekomata daemon".into()),
            Payload::Sessions(sessions) => {
                let mut output = String::from("SESSION\tWINDOW\tTABS\tPANES\n");
                for session in sessions {
                    output.push_str(&format!(
                        "{}\t{}\t{}\t{}\n",
                        session.id,
                        session
                            .window
                            .map(|w| w.to_string())
                            .unwrap_or_else(|| "-".into()),
                        session.tabs,
                        session.panes
                    ));
                }
                Ok(output.trim_end().into())
            }
            _ => bail!("unexpected nekomata daemon response"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_payloads_keep_their_wire_names_for_running_daemons() {
        let workspace = Payload::Session {
            id: 3,
            snapshot: Snapshot::default(),
        };
        let json = serde_json::to_value(&workspace).unwrap();
        assert_eq!(json["Session"]["id"], 3);
        assert_eq!(
            serde_json::to_value(Payload::Sessions(vec![])).unwrap(),
            serde_json::json!({"Sessions": []})
        );
    }

    #[test]
    fn startup_resumes_the_latest_unattached_session() {
        let mut registry = Registry::default();
        let older = registry.claim(1, 101, Selection::New).unwrap();
        registry.touch(older);
        registry.release(1, Some(older));
        let newer = registry.claim(2, 102, Selection::New).unwrap();
        registry.touch(newer);
        registry.release(2, Some(newer));
        assert_eq!(registry.claim(3, 103, Selection::Latest).unwrap(), newer);
        assert_eq!(registry.claim(4, 104, Selection::Latest).unwrap(), older);
    }
    fn connection(
        registry: &Arc<Mutex<Registry>>,
        owner: u64,
    ) -> (UnixStream, tokio::task::JoinHandle<Result<()>>) {
        let (client, server) = UnixStream::pair().unwrap();
        let registry = registry.clone();
        let task = tokio::spawn(serve_connection(
            server,
            registry,
            Arc::new(Notify::new()),
            owner,
        ));
        (client, task)
    }

    fn session(payload: Payload) -> (u64, Snapshot) {
        let Payload::Session { id, snapshot } = payload else {
            panic!("expected session")
        };
        (id, snapshot)
    }

    #[tokio::test]
    async fn disconnected_window_restores_its_saved_layout_and_releases_reservations() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut first, task) = connection(&registry, 1);
        let (id, mut snapshot) = session(
            exchange(&mut first, Request::Open { window: 11 })
                .await
                .unwrap(),
        );
        snapshot.font_size = 21.0;
        exchange(&mut first, Request::Save(snapshot.clone()))
            .await
            .unwrap();
        let (reserved, _) = session(
            exchange(&mut first, Request::Prepare(Selection::New))
                .await
                .unwrap(),
        );
        drop(first);
        assert!(task.await.unwrap().is_err());
        assert!(registry.lock().unwrap().sessions[&reserved].owner.is_none());
        let (mut replacement, task) = connection(&registry, 2);
        let restored = session(
            exchange(&mut replacement, Request::Open { window: 22 })
                .await
                .unwrap(),
        );
        assert_eq!(restored, (id, snapshot));
        drop(replacement);
        let _ = task.await;
    }

    #[tokio::test]
    async fn reserved_switch_preserves_source_until_commit_and_cancel_keeps_it_attached() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        let (source, _) = session(
            exchange(&mut client, Request::Open { window: 11 })
                .await
                .unwrap(),
        );
        let (target, _) = session(
            exchange(&mut client, Request::Prepare(Selection::New))
                .await
                .unwrap(),
        );
        assert!(registry.lock().unwrap().sessions[&source].owner.is_some());
        exchange(&mut client, Request::Finish { snapshot: None })
            .await
            .unwrap();
        assert!(registry.lock().unwrap().sessions[&source].owner.is_some());
        assert!(registry.lock().unwrap().sessions[&target].owner.is_none());
        exchange(&mut client, Request::Prepare(Selection::Id(target)))
            .await
            .unwrap();
        let snapshot = Snapshot {
            font_size: 19.0,
            ..Default::default()
        };
        exchange(
            &mut client,
            Request::Finish {
                snapshot: Some(snapshot.clone()),
            },
        )
        .await
        .unwrap();
        assert!(registry.lock().unwrap().sessions[&source].owner.is_none());
        assert_eq!(
            registry.lock().unwrap().sessions[&target].snapshot,
            snapshot
        );
        drop(client);
        let _ = task.await;
    }

    #[tokio::test]
    async fn occupied_sessions_cannot_be_claimed_or_stopped_by_another_client() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut first, first_task) = connection(&registry, 1);
        let (id, _) = session(
            exchange(&mut first, Request::Open { window: 11 })
                .await
                .unwrap(),
        );
        let (mut second, second_task) = connection(&registry, 2);
        let (other, _) = session(
            exchange(&mut second, Request::Open { window: 22 })
                .await
                .unwrap(),
        );
        assert_ne!(id, other);
        assert!(
            exchange(&mut second, Request::Prepare(Selection::Id(id)))
                .await
                .unwrap_err()
                .to_string()
                .contains("already attached")
        );
        assert!(exchange(&mut second, Request::Stop).await.is_err());
        drop(first);
        drop(second);
        let _ = first_task.await;
        let _ = second_task.await;
    }

    #[tokio::test]
    async fn incompatible_clients_do_not_claim_a_session() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        write_json(
            &mut client,
            &Envelope {
                version: VERSION + 1,
                request: Request::Open { window: 11 },
            },
        )
        .await
        .unwrap();
        let response: Response = read_json(&mut client).await.unwrap();
        assert!(
            response
                .unwrap_err()
                .contains("incompatible nekomata protocol")
        );
        assert!(registry.lock().unwrap().sessions.is_empty());
        drop(client);
        let _ = task.await;
    }

    #[tokio::test]
    async fn malformed_snapshots_preserve_the_last_saved_layout() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        let (id, _) = session(
            exchange(&mut client, Request::Open { window: 11 })
                .await
                .unwrap(),
        );
        let invalid = Snapshot {
            active: 9,
            ..Default::default()
        };
        assert!(exchange(&mut client, Request::Save(invalid)).await.is_err());
        assert_eq!(
            registry.lock().unwrap().sessions[&id].snapshot,
            Snapshot::default()
        );
        drop(client);
        let _ = task.await;
    }

    #[tokio::test]
    async fn oversized_frames_are_rejected_before_reading_the_body() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        client.write_u32(MAX_MESSAGE as u32 + 1).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert!(result.unwrap_err().to_string().contains("too large"));
        assert!(registry.lock().unwrap().sessions.is_empty());
    }
    #[tokio::test]
    async fn acknowledged_detach_makes_the_session_available_before_the_connection_closes() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        let (id, _) = session(
            exchange(&mut client, Request::Open { window: 11 })
                .await
                .unwrap(),
        );
        exchange(&mut client, Request::Detach).await.unwrap();
        let (mut replacement, replacement_task) = connection(&registry, 2);
        let (restored, _) = session(
            exchange(&mut replacement, Request::Open { window: 22 })
                .await
                .unwrap(),
        );
        assert_eq!(restored, id);
        drop(client);
        drop(replacement);
        let _ = task.await;
        let _ = replacement_task.await;
    }
    #[tokio::test]
    async fn rejected_source_save_prevents_destination_reservation() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        exchange(&mut client, Request::Open { window: 11 })
            .await
            .unwrap();
        let invalid = Snapshot {
            active: 7,
            ..Default::default()
        };
        assert!(prepare(&mut client, invalid, Selection::New).await.is_err());
        assert_eq!(registry.lock().unwrap().sessions.len(), 1);
        drop(client);
        let _ = task.await;
    }
}
