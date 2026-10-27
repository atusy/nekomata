//! Versioned workspace control. Terminal traffic stays between the GUI and felis.
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    os::unix::{
        fs::{FileTypeExt, OpenOptionsExt},
        process::CommandExt,
    },
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
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

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
    /// Identifies the daemon that issued `id`; empty from older daemons.
    #[serde(default)]
    pub instance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Payload {
    #[serde(rename = "Session")]
    Workspace {
        id: u64,
        snapshot: Snapshot,
        /// Identifies the daemon that issued `id`; empty from older daemons.
        #[serde(default)]
        instance: String,
    },
    #[serde(rename = "Sessions")]
    Workspaces(Vec<Summary>),
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

/// An envelope of any protocol version. The version is checked before the
/// request is decoded, so a newer request shape still gets a readable reply.
/// Any JSON type, so that a client changing even the version's type gets the
/// rejection instead of a closed connection.
#[derive(Deserialize)]
struct Frame {
    #[serde(default)]
    version: serde_json::Value,
    #[serde(default)]
    request: serde_json::Value,
}

impl Frame {
    fn decode(self) -> Result<Request, String> {
        if self.request == "Stop" {
            return Ok(Request::Stop);
        }
        if self.version == VERSION {
            serde_json::from_value(self.request)
                .map_err(|error| format!("malformed nekomata daemon request: {error}"))
        } else {
            Err(format!(
                "incompatible nekomata protocol: client {}, daemon {VERSION}",
                match self.version {
                    serde_json::Value::Null => "unknown".into(),
                    version => version.to_string(),
                }
            ))
        }
    }
}

struct Record {
    snapshot: Snapshot,
    owner: Option<(u64, u32)>,
    last_used: u64,
}

#[derive(Default)]
struct Registry {
    workspaces: BTreeMap<u64, Record>,
    next_id: u64,
    clock: u64,
    /// Distinguishes this daemon from earlier ones, whose IDs restarted at 1.
    instance: String,
    /// Set under the lock by an accepted Stop, so no window attaches between
    /// the owner check and the daemon exiting.
    stopping: bool,
}

impl Registry {
    fn claim(&mut self, owner: u64, window: u32, selection: Selection) -> Result<u64> {
        let selected = match selection {
            Selection::Latest => self
                .workspaces
                .iter()
                .filter(|(_, record)| record.owner.is_none())
                .max_by_key(|(id, record)| (record.last_used, **id))
                .map(|(&id, _)| id),
            Selection::New => None,
            Selection::Id(id) => {
                let record = self.workspaces.get(&id).context("workspace not found")?;
                ensure!(
                    record.owner.is_none(),
                    "workspace is already attached to a window"
                );
                Some(id)
            }
        };
        let id = selected.unwrap_or_else(|| {
            self.next_id += 1;
            let id = self.next_id;
            self.workspaces.insert(
                id,
                Record {
                    snapshot: Snapshot::default(),
                    owner: None,
                    last_used: 0,
                },
            );
            id
        });
        self.workspaces.get_mut(&id).unwrap().owner = Some((owner, window));
        Ok(id)
    }

    fn touch(&mut self, id: u64) {
        self.clock += 1;
        self.workspaces.get_mut(&id).unwrap().last_used = self.clock;
    }

    fn snapshot(&self, id: u64) -> Payload {
        Payload::Workspace {
            id,
            snapshot: self.workspaces[&id].snapshot.clone(),
            instance: self.instance.clone(),
        }
    }

    fn release(&mut self, owner: u64, current: Option<u64>) {
        if let Some(id) = current {
            self.touch(id);
        }
        let leased: Vec<_> = self
            .workspaces
            .iter()
            .filter(|(_, record)| record.owner.is_some_and(|(id, _)| id == owner))
            .map(|(&id, _)| id)
            .collect();
        for id in leased {
            self.unlease(id);
        }
    }

    /// Ends the lease on a workspace. One without tabs has nothing to restore,
    /// so it is discarded rather than listed forever.
    fn unlease(&mut self, id: u64) {
        let record = self.workspaces.get_mut(&id).unwrap();
        record.owner = None;
        if record.snapshot.tabs.is_empty() {
            self.workspaces.remove(&id);
        }
    }

    fn list(&self) -> Vec<Summary> {
        let mut workspaces: Vec<_> = self
            .workspaces
            .iter()
            .map(|(&id, record)| Summary {
                id,
                window: record.owner.map(|(_, window)| window),
                tabs: record.snapshot.tabs.len(),
                panes: record.snapshot.panes.len(),
                last_used: record.last_used,
                instance: self.instance.clone(),
            })
            .collect();
        workspaces.sort_by_key(|s| std::cmp::Reverse((s.last_used, s.id)));
        workspaces
    }
}

/// RandomState is seeded from the OS; mixing in the PID and start time keeps
/// two daemons apart even if that seed repeats.
fn instance_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    format!("{:016x}", hasher.finish())
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

/// The one request whose wire form never changes, so that any client can
/// retire a daemon of another protocol version. Version 1 daemons accept Stop
/// only at their own version; later daemons accept it at any version.
#[derive(Serialize)]
struct StopFrame {
    version: u32,
    request: &'static str,
}

fn stop_frame() -> StopFrame {
    StopFrame {
        version: 1,
        request: "Stop",
    }
}

async fn request_stop(stream: &mut UnixStream) -> Result<()> {
    tokio::time::timeout(DEADLINE, async {
        write_json(stream, &stop_frame()).await?;
        // Only the Ok/Err tag is frozen; the success payload may differ.
        let response: Result<serde_json::Value, String> = read_json(stream).await?;
        response.map(drop).map_err(|error| Rejection(error).into())
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
    // The spawned daemon would refuse this path with its stderr discarded.
    socket_at(&path)?;
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
    remove_stale_socket(&path)?;
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = SocketGuard {
        path: path.clone(),
        _lock: lock,
    };
    runtime.block_on(serve(&path))
}

/// Only a socket is ours to remove; a symlink or regular file in its place is
/// left for the user to inspect. Returns whether a socket is there.
fn socket_at(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => Ok(true),
        Ok(_) => bail!(
            "{} exists and is not a socket; remove it to start the nekomata daemon",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("inspect nekomata daemon socket"),
    }
}

fn remove_stale_socket(path: &Path) -> Result<()> {
    if socket_at(path)? {
        std::fs::remove_file(path).context("remove stale nekomata daemon socket")?;
    }
    Ok(())
}

/// Running out of descriptors or memory, or a client resetting before accept,
/// passes; giving up there would discard every retained workspace.
fn accept_error_is_transient(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        error.kind(),
        ErrorKind::ConnectionAborted | ErrorKind::Interrupted | ErrorKind::OutOfMemory
    ) || matches!(
        error.raw_os_error(),
        // std maps these to ErrorKind::Uncategorized.
        Some(libc::EMFILE | libc::ENFILE | libc::ENOBUFS)
    )
}

async fn serve(path: &Path) -> Result<()> {
    let listener = UnixListener::bind(path)?;
    let registry = Arc::new(Mutex::new(Registry {
        instance: instance_token(),
        ..Default::default()
    }));
    let stop = Arc::new(Notify::new());
    let mut owner = 0;
    loop {
        tokio::select! {
            result = listener.accept() => {
                let stream = match result {
                    Ok((stream, _)) => stream,
                    Err(error) if accept_error_is_transient(&error) => {
                        eprintln!("nekomata daemon: accept failed, retrying: {error}");
                        tokio::time::sleep(ACCEPT_BACKOFF).await;
                        continue;
                    }
                    Err(error) => return Err(error).context("accept nekomata daemon client"),
                };
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
            let request = read_json::<Frame>(&mut stream).await?.decode();
            let stopping = matches!(request, Ok(Request::Stop));
            let response = request.and_then(|request| {
                handle(
                    &mut registry.lock().unwrap(),
                    owner,
                    &mut window,
                    &mut current,
                    &mut pending,
                    request,
                )
                .map_err(|error| error.to_string())
            });
            let written = write_json(&mut stream, &response).await;
            // The registry already refuses attachments; exit even if the
            // stopping client hung up before reading the reply.
            if stopping && response.is_ok() {
                stop.notify_one();
                return written;
            }
            written?;
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
            ensure!(!registry.stopping, "nekomata daemon is stopping");
            ensure!(current.is_none(), "window already has a workspace");
            ensure!(id != 0, "invalid window ID");
            let workspace = registry.claim(owner, id, Selection::Latest)?;
            registry.touch(workspace);
            *current = Some(workspace);
            *window = Some(id);
            Ok(registry.snapshot(workspace))
        }
        Request::List => Ok(Payload::Workspaces(registry.list())),
        Request::Save(snapshot) => {
            let id = current.context("window has no workspace")?;
            snapshot.validate()?;
            registry.workspaces.get_mut(&id).unwrap().snapshot = snapshot;
            Ok(Payload::Done)
        }
        Request::Prepare(selection) => {
            ensure!(!registry.stopping, "nekomata daemon is stopping");
            ensure!(pending.is_none(), "workspace switch already in progress");
            let id =
                registry.claim(owner, window.context("window has no workspace")?, selection)?;
            *pending = Some(id);
            Ok(registry.snapshot(id))
        }
        Request::Finish { snapshot } => {
            let id = pending.context("no workspace switch in progress")?;
            if let Some(snapshot) = snapshot {
                snapshot.validate()?;
                let source = current.context("window has no workspace")?;
                registry.touch(source);
                registry.unlease(source);
                registry.workspaces.get_mut(&id).unwrap().snapshot = snapshot;
                registry.touch(id);
                *current = Some(id);
            } else {
                registry.unlease(id);
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
                registry.workspaces.values().all(|s| s.owner.is_none()),
                "cannot stop nekomata daemon while windows are attached"
            );
            registry.stopping = true;
            Ok(Payload::Done)
        }
    }
}

pub fn command(stop: bool) -> Result<String> {
    tokio::runtime::Runtime::new()?.block_on(async {
        if stop {
            request_stop(&mut UnixStream::connect(socket()?).await?).await?;
            return Ok("Stopped nekomata daemon".into());
        }
        match exchange(&mut connect().await?, Request::List).await? {
            Payload::Workspaces(workspaces) => {
                let mut output = String::from("WORKSPACE\tWINDOW\tTABS\tPANES\n");
                for workspace in workspaces {
                    output.push_str(&format!(
                        "{}\t{}\t{}\t{}\n",
                        workspace.id,
                        workspace
                            .window
                            .map(|w| w.to_string())
                            .unwrap_or_else(|| "-".into()),
                        workspace.tabs,
                        workspace.panes
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
        let workspace = Payload::Workspace {
            id: 3,
            snapshot: Snapshot::default(),
            instance: String::new(),
        };
        let json = serde_json::to_value(&workspace).unwrap();
        assert_eq!(json["Session"]["id"], 3);
        assert_eq!(
            serde_json::to_value(Payload::Workspaces(vec![])).unwrap(),
            serde_json::json!({"Sessions": []})
        );
    }

    #[test]
    fn stale_daemon_sockets_are_removed_but_other_files_are_kept() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("daemon.sock");
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
        remove_stale_socket(&socket).unwrap();
        assert!(std::fs::symlink_metadata(&socket).is_err());
        remove_stale_socket(&socket).unwrap();

        let file = directory.path().join("precious");
        std::fs::write(&file, "keep").unwrap();
        std::os::unix::fs::symlink(&file, &socket).unwrap();
        assert!(remove_stale_socket(&socket).is_err());
        assert!(std::fs::symlink_metadata(&socket).unwrap().is_symlink());
        std::fs::remove_file(&socket).unwrap();

        std::fs::write(&socket, "keep").unwrap();
        assert!(remove_stale_socket(&socket).is_err());
        assert_eq!(std::fs::read_to_string(&socket).unwrap(), "keep");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep");
    }

    #[test]
    fn only_a_socket_or_nothing_may_occupy_the_daemon_socket_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("daemon.sock");
        assert!(!socket_at(&path).unwrap());
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(socket_at(&path).unwrap());
        drop(listener);
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, "keep").unwrap();
        assert!(
            socket_at(&path)
                .unwrap_err()
                .to_string()
                .contains("is not a socket")
        );
    }

    #[test]
    fn accept_survives_resource_exhaustion_but_not_a_broken_listener() {
        use std::io::{Error, ErrorKind};
        for errno in [libc::EMFILE, libc::ENFILE, libc::ENOBUFS, libc::ENOMEM] {
            assert!(accept_error_is_transient(&Error::from_raw_os_error(errno)));
        }
        for kind in [ErrorKind::ConnectionAborted, ErrorKind::Interrupted] {
            assert!(accept_error_is_transient(&Error::from(kind)));
        }
        for errno in [libc::EBADF, libc::EINVAL, libc::ENOTSOCK] {
            assert!(!accept_error_is_transient(&Error::from_raw_os_error(errno)));
        }
    }

    #[test]
    fn each_daemon_gets_a_distinct_instance_token() {
        let (first, second) = (instance_token(), instance_token());
        assert_eq!(first.len(), 16);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn workspaces_name_the_daemon_instance_that_issued_their_ids() {
        let registry = Arc::new(Mutex::new(Registry {
            instance: "feed".into(),
            ..Default::default()
        }));
        let (mut client, task) = connection(&registry, 1);
        let Payload::Workspace { instance, .. } =
            exchange(&mut client, Request::Open { window: 11 })
                .await
                .unwrap()
        else {
            panic!("expected workspace")
        };
        assert_eq!(instance, "feed");
        let Payload::Workspaces(workspaces) = exchange(&mut client, Request::List).await.unwrap()
        else {
            panic!("expected workspaces")
        };
        assert_eq!(workspaces[0].instance, "feed");
        drop(client);
        let _ = task.await;
        // Daemons predating the token omit it.
        let Payload::Workspace { instance, .. } = serde_json::from_value(
            serde_json::to_value(Payload::Workspace {
                id: 1,
                snapshot: Snapshot::default(),
                instance: String::new(),
            })
            .map(|mut value| {
                value["Session"].as_object_mut().unwrap().remove("instance");
                value
            })
            .unwrap(),
        )
        .unwrap() else {
            panic!("expected workspace")
        };
        assert!(instance.is_empty());
    }

    #[test]
    fn startup_resumes_the_latest_unattached_workspace() {
        let mut registry = Registry::default();
        let older = registry.claim(1, 101, Selection::New).unwrap();
        registry.workspaces.get_mut(&older).unwrap().snapshot = one_pane(15.0);
        registry.touch(older);
        registry.release(1, Some(older));
        let newer = registry.claim(2, 102, Selection::New).unwrap();
        registry.workspaces.get_mut(&newer).unwrap().snapshot = one_pane(15.0);
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

    fn one_pane(font_size: f32) -> Snapshot {
        Snapshot {
            tabs: vec![crate::workspace::Tab {
                name: "shell".into(),
                layout: crate::layout::Layout::Pane(1),
                focused: 1,
                focus_history: vec![1],
            }],
            panes: [(
                1,
                crate::workspace::PaneSpec {
                    target: crate::session::Target::Local("/felis.sock".into()),
                    session: None,
                },
            )]
            .into(),
            next_id: 2,
            font_size,
            ..Default::default()
        }
    }

    fn workspace(payload: Payload) -> (u64, Snapshot) {
        let Payload::Workspace { id, snapshot, .. } = payload else {
            panic!("expected workspace")
        };
        (id, snapshot)
    }

    #[tokio::test]
    async fn disconnected_window_restores_its_saved_layout_and_releases_reservations() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut first, task) = connection(&registry, 1);
        let (id, _) = workspace(
            exchange(&mut first, Request::Open { window: 11 })
                .await
                .unwrap(),
        );
        let snapshot = one_pane(21.0);
        exchange(&mut first, Request::Save(snapshot.clone()))
            .await
            .unwrap();
        let (reserved, _) = workspace(
            exchange(&mut first, Request::Prepare(Selection::New))
                .await
                .unwrap(),
        );
        drop(first);
        assert!(task.await.unwrap().is_err());
        assert!(!registry.lock().unwrap().workspaces.contains_key(&reserved));
        let (mut replacement, task) = connection(&registry, 2);
        let restored = workspace(
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
        let (source, _) = workspace(
            exchange(&mut client, Request::Open { window: 11 })
                .await
                .unwrap(),
        );
        let (mut other, other_task) = connection(&registry, 2);
        let (target, _) = workspace(
            exchange(&mut other, Request::Open { window: 22 })
                .await
                .unwrap(),
        );
        exchange(&mut other, Request::Save(one_pane(17.0)))
            .await
            .unwrap();
        exchange(&mut other, Request::Detach).await.unwrap();
        exchange(&mut client, Request::Save(one_pane(15.0)))
            .await
            .unwrap();
        exchange(&mut client, Request::Prepare(Selection::Id(target)))
            .await
            .unwrap();
        assert!(registry.lock().unwrap().workspaces[&source].owner.is_some());
        exchange(&mut client, Request::Finish { snapshot: None })
            .await
            .unwrap();
        assert!(registry.lock().unwrap().workspaces[&source].owner.is_some());
        assert!(registry.lock().unwrap().workspaces[&target].owner.is_none());
        exchange(&mut client, Request::Prepare(Selection::Id(target)))
            .await
            .unwrap();
        let snapshot = one_pane(19.0);
        exchange(
            &mut client,
            Request::Finish {
                snapshot: Some(snapshot.clone()),
            },
        )
        .await
        .unwrap();
        assert!(registry.lock().unwrap().workspaces[&source].owner.is_none());
        assert_eq!(
            registry.lock().unwrap().workspaces[&target].snapshot,
            snapshot
        );
        drop(client);
        drop(other);
        let _ = task.await;
        let _ = other_task.await;
    }

    #[tokio::test]
    async fn workspaces_without_tabs_are_discarded_once_no_window_holds_them() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let workspaces = || registry.lock().unwrap().workspaces.len();
        let (mut client, task) = connection(&registry, 1);
        exchange(&mut client, Request::Open { window: 11 })
            .await
            .unwrap();
        exchange(&mut client, Request::Save(one_pane(15.0)))
            .await
            .unwrap();
        // A cancelled switch to a new workspace leaves nothing behind.
        exchange(&mut client, Request::Prepare(Selection::New))
            .await
            .unwrap();
        assert_eq!(workspaces(), 2);
        exchange(&mut client, Request::Finish { snapshot: None })
            .await
            .unwrap();
        assert_eq!(workspaces(), 1);
        // Closing the last pane saves a layout without tabs.
        exchange(&mut client, Request::Save(Snapshot::default()))
            .await
            .unwrap();
        exchange(&mut client, Request::Detach).await.unwrap();
        assert_eq!(workspaces(), 0);
        // A window that disconnects before saving leaves nothing behind.
        exchange(&mut client, Request::Open { window: 11 })
            .await
            .unwrap();
        drop(client);
        let _ = task.await;
        assert_eq!(workspaces(), 0);
    }

    #[tokio::test]
    async fn occupied_workspaces_cannot_be_claimed_or_stopped_by_another_client() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut first, first_task) = connection(&registry, 1);
        let (id, _) = workspace(
            exchange(&mut first, Request::Open { window: 11 })
                .await
                .unwrap(),
        );
        let (mut second, second_task) = connection(&registry, 2);
        let (other, _) = workspace(
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
    async fn a_stopping_daemon_refuses_new_attachments() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut stopper, stopper_task) = connection(&registry, 1);
        let (mut late, late_task) = connection(&registry, 2);
        exchange(&mut stopper, Request::Stop).await.unwrap();
        assert!(
            exchange(&mut late, Request::Open { window: 22 })
                .await
                .unwrap_err()
                .to_string()
                .contains("stopping")
        );
        assert!(registry.lock().unwrap().workspaces.is_empty());
        drop(late);
        let _ = stopper_task.await;
        let _ = late_task.await;
    }

    #[tokio::test]
    async fn stop_completes_even_if_its_client_hangs_up_before_the_reply() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let stop = Arc::new(Notify::new());
        let (mut client, server) = UnixStream::pair().unwrap();
        let task = tokio::spawn(serve_connection(server, registry, stop.clone(), 1));
        write_json(
            &mut client,
            &Envelope {
                version: VERSION,
                request: Request::Stop,
            },
        )
        .await
        .unwrap();
        drop(client);
        tokio::time::timeout(Duration::from_secs(1), stop.notified())
            .await
            .unwrap();
        let _ = task.await;
    }

    #[tokio::test]
    async fn incompatible_clients_do_not_claim_a_workspace() {
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
        assert!(registry.lock().unwrap().workspaces.is_empty());
        drop(client);
        let _ = task.await;
    }

    async fn raw_exchange(client: &mut UnixStream, frame: serde_json::Value) -> Response {
        write_json(client, &frame).await.unwrap();
        read_json(client).await.unwrap()
    }

    #[tokio::test]
    async fn requests_of_another_version_get_a_readable_rejection_whatever_their_shape() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        let response = raw_exchange(
            &mut client,
            serde_json::json!({"version": VERSION + 1, "request": {"Brand": {"new": 1}}}),
        )
        .await;
        assert!(
            response
                .unwrap_err()
                .contains("incompatible nekomata protocol")
        );
        for frame in [
            serde_json::json!({"request": "List"}),
            serde_json::json!({"version": "2", "request": "List"}),
            serde_json::json!({"version": 2.5, "request": "List"}),
            serde_json::json!({"version": -1, "request": "List"}),
        ] {
            let response = raw_exchange(&mut client, frame).await;
            assert!(response.unwrap_err().contains("incompatible"));
        }
        drop(client);
        let _ = task.await;
    }

    #[tokio::test]
    async fn malformed_requests_are_rejected_without_dropping_the_connection() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        let response = raw_exchange(
            &mut client,
            serde_json::json!({"version": VERSION, "request": {"Open": {}}}),
        )
        .await;
        assert!(response.unwrap_err().contains("malformed"));
        assert!(matches!(
            exchange(&mut client, Request::List).await.unwrap(),
            Payload::Workspaces(_)
        ));
        drop(client);
        let _ = task.await;
    }

    #[tokio::test]
    async fn rejections_are_readable_whatever_the_success_payload_looks_like() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let reply = tokio::spawn(async move {
            let _: serde_json::Value = read_json(&mut server).await.unwrap();
            write_json(&mut server, &serde_json::json!({"Err": "refused"}))
                .await
                .unwrap();
            let _: serde_json::Value = read_json(&mut server).await.unwrap();
            write_json(&mut server, &serde_json::json!({"Ok": {"Future": 1}}))
                .await
                .unwrap();
        });
        let error = exchange(&mut client, Request::List).await.unwrap_err();
        assert_eq!(error.downcast_ref::<Rejection>().unwrap().0, "refused");
        let error = exchange(&mut client, Request::List).await.unwrap_err();
        assert!(error.downcast_ref::<Rejection>().is_none());
        reply.await.unwrap();
    }

    #[test]
    fn the_stop_frame_is_frozen_so_any_daemon_version_accepts_it() {
        assert_eq!(
            serde_json::to_string(&stop_frame()).unwrap(),
            r#"{"version":1,"request":"Stop"}"#
        );
        assert_eq!(
            serde_json::to_string(&Response::Ok(Payload::Done)).unwrap(),
            r#"{"Ok":"Done"}"#
        );
    }

    #[tokio::test]
    async fn daemons_accept_a_stop_from_any_protocol_version() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        let response = raw_exchange(
            &mut client,
            serde_json::json!({"version": VERSION + 1, "request": "Stop"}),
        )
        .await;
        assert!(response.is_ok());
        let _ = task.await;
    }

    #[tokio::test]
    async fn stopping_succeeds_whatever_the_daemon_returns_on_success() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let reply = tokio::spawn(async move {
            let _: serde_json::Value = read_json(&mut server).await.unwrap();
            write_json(&mut server, &serde_json::json!({"Ok": {"Future": 1}}))
                .await
                .unwrap();
            let _: serde_json::Value = read_json(&mut server).await.unwrap();
            write_json(&mut server, &serde_json::json!({"Err": "busy"}))
                .await
                .unwrap();
        });
        request_stop(&mut client).await.unwrap();
        let error = request_stop(&mut client).await.unwrap_err();
        assert_eq!(error.downcast_ref::<Rejection>().unwrap().0, "busy");
        reply.await.unwrap();
    }

    #[tokio::test]
    async fn stop_requests_reach_a_daemon_through_the_frozen_frame() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        request_stop(&mut client).await.unwrap();
        assert!(registry.lock().unwrap().stopping);
        let _ = task.await;
    }

    #[tokio::test]
    async fn malformed_snapshots_preserve_the_last_saved_layout() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        let (id, _) = workspace(
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
            registry.lock().unwrap().workspaces[&id].snapshot,
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
        assert!(registry.lock().unwrap().workspaces.is_empty());
    }
    #[tokio::test]
    async fn acknowledged_detach_makes_the_workspace_available_before_the_connection_closes() {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (mut client, task) = connection(&registry, 1);
        let (id, _) = workspace(
            exchange(&mut client, Request::Open { window: 11 })
                .await
                .unwrap(),
        );
        exchange(&mut client, Request::Save(one_pane(15.0)))
            .await
            .unwrap();
        exchange(&mut client, Request::Detach).await.unwrap();
        let (mut replacement, replacement_task) = connection(&registry, 2);
        let (restored, _) = workspace(
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
        assert_eq!(registry.lock().unwrap().workspaces.len(), 1);
        drop(client);
        let _ = task.await;
    }
}
