//! Per-window control socket. Only the GUI thread mutates panes.
use std::{
    collections::hash_map::DefaultHasher,
    ffi::OsString,
    hash::{Hash, Hasher},
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use eframe::egui;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{mpsc, oneshot},
};

use crate::cli::PaneAction;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    List,
    Switch {
        #[serde(rename = "session")]
        workspace: Option<u64>,
    },
    Attach {
        pane: Option<u64>,
        daemon: Option<String>,
        ssh: Option<String>,
        session: String,
    },
}

pub type Reply = Result<String, String>;
pub struct Message {
    pub request: Request,
    pub reply: oneshot::Sender<Reply>,
}

pub struct Server {
    pub window: u32,
    pub requests: mpsc::Receiver<Message>,
    task: tokio::task::JoinHandle<()>,
    path: PathBuf,
}

const DEADLINE: Duration = Duration::from_secs(30);
const MAX_MESSAGE: u64 = 65536;

pub(crate) fn directory() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is required for GUI discovery")?;
    let owner = std::fs::metadata(&home)?.uid();
    let path = default_directory(|name| std::env::var_os(name), owner);
    match std::fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err.into()),
    }
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.uid() != owner || metadata.permissions().mode() & 0o077 != 0 {
        bail!("control directory must be private and owned by the current user");
    }
    Ok(path)
}

fn default_directory(env: impl Fn(&str) -> Option<OsString>, owner: u32) -> PathBuf {
    if let Some(path) = env("NEKOMATA_RUNTIME_DIR") {
        return PathBuf::from(path);
    }
    let home = env("HOME").unwrap_or_default();
    let mut hash = DefaultHasher::new();
    home.hash(&mut hash);
    // Keep Unix socket paths short, including on macOS.
    PathBuf::from("/tmp").join(format!("nekomata-{owner}-{:x}", hash.finish()))
}

impl Server {
    pub fn bind(runtime: &tokio::runtime::Runtime, ctx: egui::Context) -> Result<Self> {
        let window = std::process::id();
        let path = directory()?.join(format!("{window}.sock"));
        let _guard = runtime.enter();
        let listener = UnixListener::bind(&path).context("bind nekomata control socket")?;
        let (tx, requests) = mpsc::channel::<Message>(16);
        let task = runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let tx = tx.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let _ = tokio::time::timeout(DEADLINE, serve(stream, tx, ctx)).await;
                });
            }
        });
        Ok(Self {
            window,
            requests,
            task,
            path,
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn serve(
    mut stream: UnixStream,
    tx: mpsc::Sender<Message>,
    ctx: egui::Context,
) -> Result<()> {
    let mut bytes = Vec::new();
    (&mut stream)
        .take(MAX_MESSAGE + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > MAX_MESSAGE {
        bail!("control request is too large");
    }
    let request: Request = serde_json::from_slice(&bytes)?;
    let (reply, result) = oneshot::channel();
    tx.send(Message { request, reply }).await?;
    ctx.request_repaint();
    let response = result.await.unwrap_or_else(|_| Err("window closed".into()));
    stream.write_all(&serde_json::to_vec(&response)?).await?;
    Ok(())
}

async fn exchange(path: &Path, request: &Request) -> Result<String> {
    let mut stream = UnixStream::connect(path).await?;
    stream.write_all(&serde_json::to_vec(request)?).await?;
    stream.shutdown().await?;
    let mut bytes = Vec::new();
    stream.take(MAX_MESSAGE + 1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > MAX_MESSAGE {
        bail!("control response is too large");
    }
    let response: Reply = serde_json::from_slice(&bytes)?;
    response.map_err(anyhow::Error::msg)
}

pub fn run(window: Option<u32>, action: PaneAction) -> Result<String> {
    let request = match action {
        PaneAction::List => Request::List,
        PaneAction::Attach {
            pane,
            daemon,
            ssh,
            session,
        } => {
            let daemon = daemon
                .map(|socket| -> Result<PathBuf> {
                    let path = PathBuf::from(socket);
                    if ssh.is_some() || path.is_absolute() {
                        Ok(path)
                    } else {
                        Ok(std::env::current_dir()?.join(path))
                    }
                })
                .transpose()?
                .map(|path: PathBuf| path.to_string_lossy().into_owned());
            Request::Attach {
                pane,
                daemon,
                ssh,
                session,
            }
        }
    };
    run_request(window, request)
}

/// Exported to local shells so CLI calls follow their workspace across GUI restarts.
pub(crate) const WORKSPACE_ENV: &str = "NEKOMATA_WORKSPACE_ID";
/// Name exported by builds before the workspace rename; their shells still carry it.
const LEGACY_WORKSPACE_ENV: &str = "NEKOMATA_SESSION_ID";

fn workspace_from_env(get: impl Fn(&str) -> Option<String>) -> Option<u64> {
    [WORKSPACE_ENV, LEGACY_WORKSPACE_ENV]
        .into_iter()
        .find_map(|name| get(name)?.parse().ok())
}

pub fn run_request(window: Option<u32>, request: Request) -> Result<String> {
    let workspace = window
        .is_none()
        .then(|| workspace_from_env(|name| std::env::var(name).ok()))
        .flatten();
    let window = window.or_else(|| {
        std::env::var("NEKOMATA_WINDOW_ID")
            .ok()
            .and_then(|id| id.parse().ok())
    });
    tokio::runtime::Runtime::new()?.block_on(async {
        let window = if let Some(id) = workspace {
            let mut stream = UnixStream::connect(crate::daemon::socket()?)
                .await
                .context("connect to the shell's nekomata workspace daemon")?;
            let crate::daemon::Payload::Workspaces(workspaces) =
                crate::daemon::exchange(&mut stream, crate::daemon::Request::List).await?
            else {
                bail!("unexpected workspace list response");
            };
            Some(
                workspaces
                    .into_iter()
                    .find(|workspace| workspace.id == id)
                    .and_then(|workspace| workspace.window)
                    .context("the shell's workspace is not attached to a window")?,
            )
        } else {
            window
        };
        let directory = directory()?;
        let path = if let Some(id) = window {
            directory.join(format!("{id}.sock"))
        } else {
            let mut candidates = Vec::new();
            for entry in std::fs::read_dir(&directory)? {
                let path = entry?.path();
                if path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.parse::<u32>().is_ok())
                    && path.extension().is_some_and(|s| s == "sock")
                    && matches!(
                        tokio::time::timeout(
                            Duration::from_millis(250),
                            UnixStream::connect(&path)
                        )
                        .await,
                        Ok(Ok(_))
                    )
                {
                    candidates.push(path);
                }
            }
            match candidates.len() {
                0 => bail!("no running nekomata window"),
                1 => candidates.remove(0),
                _ => bail!(
                    "multiple nekomata windows; specify --window WINDOW_ID (socket names in {})",
                    directory.display()
                ),
            }
        };
        tokio::time::timeout(DEADLINE, exchange(&path, &request))
            .await
            .context("GUI command timed out")?
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn control_socket_returns_the_gui_reply() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let (tx, mut rx) = mpsc::channel(1);
        let task = tokio::spawn(serve(server, tx, egui::Context::default()));
        client
            .write_all(
                &serde_json::to_vec(&Request::Attach {
                    pane: Some(7),
                    daemon: None,
                    ssh: Some("dev".into()),
                    session: "abc".into(),
                })
                .unwrap(),
            )
            .await
            .unwrap();
        client.shutdown().await.unwrap();
        let message = rx.recv().await.unwrap();
        assert!(matches!(
            message.request,
            Request::Attach { pane: Some(7), .. }
        ));
        message.reply.send(Err("session not found".into())).unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Reply>(&response).unwrap(),
            Err("session not found".into())
        );
        task.await.unwrap().unwrap();
    }

    #[test]
    fn shells_name_their_workspace_with_a_legacy_fallback() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        assert_eq!(workspace_from_env(env(&[])), None);
        assert_eq!(
            workspace_from_env(env(&[("NEKOMATA_WORKSPACE_ID", "7")])),
            Some(7)
        );
        // Shells started by builds before the rename only carry the old name.
        assert_eq!(
            workspace_from_env(env(&[("NEKOMATA_SESSION_ID", "5")])),
            Some(5)
        );
        assert_eq!(
            workspace_from_env(env(&[
                ("NEKOMATA_WORKSPACE_ID", "7"),
                ("NEKOMATA_SESSION_ID", "5")
            ])),
            Some(7)
        );
        assert_eq!(
            workspace_from_env(env(&[
                ("NEKOMATA_WORKSPACE_ID", "bad"),
                ("NEKOMATA_SESSION_ID", "5")
            ])),
            Some(5)
        );
    }

    #[test]
    fn workspace_switch_keeps_its_wire_name_for_running_windows() {
        let request = Request::Switch { workspace: Some(7) };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({"Switch": {"session": 7}})
        );
    }

    #[tokio::test]
    async fn oversized_requests_never_reach_the_gui() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let (tx, mut rx) = mpsc::channel(1);
        let task = tokio::spawn(serve(server, tx, egui::Context::default()));
        client
            .write_all(&vec![b'x'; MAX_MESSAGE as usize + 1])
            .await
            .unwrap();
        client.shutdown().await.unwrap();
        assert!(
            task.await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("too large")
        );
        assert!(rx.recv().await.is_none());
    }
}
