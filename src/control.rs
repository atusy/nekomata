//! Per-window control socket. Only the GUI thread mutates panes.
use std::{
    ffi::OsString,
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
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

pub(crate) fn directory() -> Result<PathBuf> {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let owner = unsafe { libc::geteuid() };
    let path = default_directory(
        |name| std::env::var_os(name),
        owner,
        user_temp_directory(),
        |base| is_private_directory(base, owner),
    );
    match std::fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => {
            return Err(err)
                .with_context(|| format!("create control directory {}", path.display()));
        }
    }
    if !is_private_directory(&path, owner) {
        bail!(
            "control directory {} must be private and owned by the current user",
            path.display()
        );
    }
    Ok(path)
}

/// Longest name bound in the directory: `{u32::MAX}.sock`.
const LONGEST_SOCKET_NAME: usize = "4294967295.sock".len();
/// `sun_path` (104 bytes on macOS, 108 on Linux) less its NUL terminator.
const MAX_SOCKET_PATH: usize = std::mem::size_of::<libc::sockaddr_un>()
    - std::mem::offset_of!(libc::sockaddr_un, sun_path)
    - 1;

/// Every process of one user must agree on this path whatever its build or
/// environment, so it uses no hash and no per-shell variable like TMPDIR.
/// `XDG_RUNTIME_DIR` is read but also implied: logind's `/run/user/<uid>` is
/// probed for processes started without it.
fn default_directory(
    env: impl Fn(&str) -> Option<OsString>,
    owner: u32,
    user_temp: Option<PathBuf>,
    private: impl Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(path) = env("NEKOMATA_RUNTIME_DIR") {
        return PathBuf::from(path);
    }
    let xdg = env("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    user_temp
        .into_iter()
        .chain(xdg)
        .chain([PathBuf::from(format!("/run/user/{owner}"))])
        .filter(|base| private(base))
        .map(|base| base.join("nekomata"))
        .find(|path| path.as_os_str().len() + 1 + LONGEST_SOCKET_NAME <= MAX_SOCKET_PATH)
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/nekomata-{owner}")))
}

/// Whether `path` is an existing directory only `owner` can use, as the XDG
/// specification requires of `XDG_RUNTIME_DIR`.
fn is_private_directory(path: &Path, owner: u32) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_dir() && metadata.uid() == owner && metadata.permissions().mode() & 0o077 == 0
    })
}

/// The per-user temporary directory macOS derives from the uid, unlike TMPDIR.
#[cfg(target_os = "macos")]
fn user_temp_directory() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut buffer = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: the buffer is writable for its full length.
    let length = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if length == 0 || length > buffer.len() {
        return None;
    }
    let path = std::ffi::CStr::from_bytes_until_nul(&buffer).ok()?;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes())))
}

#[cfg(not(target_os = "macos"))]
fn user_temp_directory() -> Option<PathBuf> {
    None
}

impl Server {
    pub fn bind(runtime: &tokio::runtime::Runtime, ctx: egui::Context) -> Result<Self> {
        let window = std::process::id();
        let path = directory()?.join(format!("{window}.sock"));
        let _guard = runtime.enter();
        let listener = listen(&path).context("bind nekomata control socket")?;
        listener.set_nonblocking(true)?;
        let listener = std::sync::Arc::new(UnixListener::from_std(listener)?);
        let (tx, requests) = mpsc::channel::<Message>(16);
        let task = runtime.spawn(accept_each(
            move || {
                let listener = listener.clone();
                async move { listener.accept().await.map(|(stream, _)| stream) }
            },
            move |stream| {
                let tx = tx.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let _ = tokio::time::timeout(DEADLINE, serve(stream, tx, ctx)).await;
                });
            },
        ));
        Ok(Self {
            window,
            requests,
            task,
            path,
        })
    }
}

async fn accept_each<S, F>(mut accept: impl FnMut() -> F, mut handle: impl FnMut(S))
where
    F: Future<Output = std::io::Result<S>>,
{
    loop {
        match accept().await {
            Ok(stream) => handle(stream),
            // Errors such as EMFILE pass once connections close; retrying
            // without a pause would spin while the condition lasts.
            Err(err) => {
                eprintln!("nekomata: accept control connection: {err}");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
        }
    }
}

/// Window sockets are named by PID and removed only on a clean exit, so a
/// crashed window's socket can occupy the name of a later process.
fn listen(path: &Path) -> std::io::Result<std::os::unix::net::UnixListener> {
    use std::os::unix::net::{UnixListener, UnixStream};
    match UnixListener::bind(path) {
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
            match UnixStream::connect(path) {
                Err(refused) if refused.kind() == std::io::ErrorKind::ConnectionRefused => {
                    std::fs::remove_file(path)?;
                    UnixListener::bind(path)
                }
                _ => Err(err),
            }
        }
        result => result,
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Answers queued requests at once, so CLI calls fail fast instead of
/// waiting out [`DEADLINE`] while the window cannot act on them.
pub fn reject_all(requests: &mut mpsc::Receiver<Message>, reason: &str) {
    while let Ok(message) = requests.try_recv() {
        let _ = message.reply.send(Err(reason.into()));
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

    fn vars(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name: &str| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    /// Treats exactly `bases` as existing private directories of the user.
    fn private(bases: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |path| bases.iter().any(|base| path == Path::new(base))
    }

    #[test]
    fn runtime_directory_does_not_depend_on_home_or_build() {
        // A hash of HOME could change between Rust releases; the path must not.
        assert_eq!(
            default_directory(vars(&[("HOME", "/home/a")]), 501, None, private(&[])),
            PathBuf::from("/tmp/nekomata-501")
        );
        assert_eq!(
            default_directory(vars(&[("HOME", "/home/b")]), 501, None, private(&[])),
            PathBuf::from("/tmp/nekomata-501")
        );
    }

    #[test]
    fn runtime_directory_prefers_the_override_then_per_user_locations() {
        let darwin = Some(PathBuf::from("/var/folders/ab/cd/T"));
        let all = private(&["/var/folders/ab/cd/T", "/run/user/501", "/xdg"]);
        assert_eq!(
            default_directory(
                vars(&[
                    ("NEKOMATA_RUNTIME_DIR", "/x/gui"),
                    ("XDG_RUNTIME_DIR", "/xdg")
                ]),
                501,
                darwin.clone(),
                &all
            ),
            PathBuf::from("/x/gui")
        );
        // TMPDIR differs per environment (nix develop sets its own), so the
        // macOS directory comes from confstr, not the variable.
        assert_eq!(
            default_directory(
                vars(&[("TMPDIR", "/tmp/nix-shell.x"), ("XDG_RUNTIME_DIR", "/xdg")]),
                501,
                darwin,
                &all
            ),
            PathBuf::from("/var/folders/ab/cd/T/nekomata")
        );
        assert_eq!(
            default_directory(vars(&[("XDG_RUNTIME_DIR", "/xdg")]), 501, None, &all),
            PathBuf::from("/xdg/nekomata")
        );
    }

    #[test]
    fn runtime_directory_on_linux_does_not_need_xdg_runtime_dir() {
        // Shells started by felis, cron or a bare ssh login may lack the
        // variable; they must still meet a desktop-launched GUI.
        let logind = private(&["/run/user/501"]);
        assert_eq!(
            default_directory(vars(&[]), 501, None, &logind),
            PathBuf::from("/run/user/501/nekomata")
        );
        // The XDG specification ignores relative paths.
        assert_eq!(
            default_directory(vars(&[("XDG_RUNTIME_DIR", "run")]), 501, None, &logind),
            PathBuf::from("/run/user/501/nekomata")
        );
    }

    #[test]
    fn runtime_directory_skips_bases_that_are_missing_or_not_private() {
        // `sudo -E` keeps another user's XDG_RUNTIME_DIR; a logout removes it.
        assert_eq!(
            default_directory(
                vars(&[("XDG_RUNTIME_DIR", "/run/user/1000")]),
                0,
                Some(PathBuf::from("/var/folders/ab/cd/T")),
                private(&[])
            ),
            PathBuf::from("/tmp/nekomata-0")
        );
    }

    #[test]
    fn runtime_directory_skips_bases_too_long_for_socket_paths() {
        // `{base}/nekomata/4294967295.sock` must fit in sun_path.
        let longest = MAX_SOCKET_PATH - "/nekomata/4294967295.sock".len();
        let base = |len: usize| PathBuf::from(format!("/{}", "d".repeat(len - 1)));
        assert_eq!(
            default_directory(vars(&[]), 501, Some(base(longest)), |_| true),
            base(longest).join("nekomata")
        );
        assert_eq!(
            default_directory(vars(&[]), 501, Some(base(longest + 1)), |_| true),
            PathBuf::from("/run/user/501/nekomata")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_user_temp_directory_is_owned_by_the_user() {
        let path = user_temp_directory().unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    }

    #[test]
    fn a_crashed_window_socket_does_not_block_its_pid() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("7.sock");
        // A crash skips Drop, leaving the bound path behind with no listener.
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let listener = listen(&path).unwrap();
        std::os::unix::net::UnixStream::connect(&path).unwrap();
        drop(listener);
    }

    #[test]
    fn a_live_window_socket_is_not_taken_over() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("7.sock");
        let _live = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert_eq!(
            listen(&path).unwrap_err().kind(),
            std::io::ErrorKind::AddrInUse
        );
        std::os::unix::net::UnixStream::connect(&path).unwrap();
    }

    #[tokio::test]
    async fn transient_accept_errors_do_not_stop_the_control_socket() {
        let mut results = std::collections::VecDeque::from([
            Err(std::io::Error::from_raw_os_error(libc::EMFILE)),
            Ok(1),
        ]);
        let (tx, mut handled) = mpsc::unbounded_channel();
        let task = tokio::spawn(accept_each(
            move || {
                let next = results.pop_front();
                async move {
                    match next {
                        Some(result) => result,
                        None => std::future::pending().await,
                    }
                }
            },
            move |stream| tx.send(stream).unwrap(),
        ));
        assert_eq!(handled.recv().await, Some(1));
        task.abort();
    }

    #[test]
    fn pending_requests_are_answered_when_the_window_cannot_serve_them() {
        let (tx, mut requests) = mpsc::channel(2);
        let mut replies = Vec::new();
        for request in [Request::List, Request::Switch { workspace: None }] {
            let (reply, result) = oneshot::channel();
            tx.try_send(Message { request, reply }).unwrap();
            replies.push(result);
        }
        reject_all(&mut requests, "workspace is not ready");
        for mut result in replies {
            assert_eq!(
                result.try_recv().unwrap(),
                Err("workspace is not ready".into())
            );
        }
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
