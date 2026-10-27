//! SSH relay adapter retaining child ownership for cancellation and reconnects.
use anyhow::{Context, Result, bail};
use felis_client_core::{CarrierConnection, CarrierReader, CarrierWriter, Connection, Offer};
use felis_protocol::{
    messages::{ConnToClientMsg, ConnToDaemonMsg},
    preface::{self, ClientPreface, DaemonPreface},
};
use felis_transport::{ConnectionDriver, FrameReader, FrameWriter, Incoming};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    process::{Child, Command},
};

fn relay_command(host: &str, socket: Option<&str>) -> Command {
    // SSH joins remote argv into shell text. Quote an explicit socket for that
    // shell, including embedded apostrophes. None keeps the remote default.
    let mut remote = "exec felis-daemon relay --no-spawn".to_owned();
    if let Some(socket) = socket {
        remote.push_str(&format!(" --socket '{}'", socket.replace('\'', "'\\''")));
    }
    let mut command = Command::new("ssh");
    command.args(["-oBatchMode=yes", "--", host, &remote]);
    command
}

pub async fn connect(
    host: &str,
    socket: Option<&str>,
    offer: Offer,
) -> Result<(CarrierConnection, Child)> {
    connect_command(relay_command(host, socket), offer).await
}

async fn connect_command(mut command: Command, offer: Offer) -> Result<(CarrierConnection, Child)> {
    command.kill_on_drop(true);
    let session = felis_transport::spawn_command(command)?;
    // Retain the child for the lifetime of the connection so failed handshakes,
    // cancellation, and closing the pane also reap the relay process.
    let child = session.child;
    let connection = handshake(
        CarrierReader::Ssh(session.reader.into_inner()),
        CarrierWriter::Ssh(session.writer.into_inner()),
        offer,
    )
    .await?;
    Ok((connection, child))
}

async fn handshake<R, W>(mut reader: R, mut writer: W, offer: Offer) -> Result<Connection<R, W>>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    // Bootstrap uses the public protocol/transport codecs and driver. The
    // client-core connector resumes after Welcome without discarding read-ahead.
    let offered = ClientPreface::CURRENT;
    felis_transport::preface::write_client_preface(&mut writer, offered).await?;
    let reply = felis_transport::preface::read_daemon_preface(&mut reader).await?;
    let minor = preface::confirm_accept(offered, reply).context("negotiate SSH daemon protocol")?;
    let DaemonPreface::Accept {
        major,
        minor: advertised,
    } = reply
    else {
        bail!("SSH daemon refused protocol");
    };
    if minor < offer.mode.since_minor() {
        bail!("SSH daemon protocol is too old for this connection mode");
    }
    let mut reader = FrameReader::new(reader);
    let mut writer = FrameWriter::new(writer, minor);
    let mut driver = ConnectionDriver::client(offer.mode);
    driver.preface_done();
    writer
        .send(&ConnToDaemonMsg::Hello {
            mode: offer.mode,
            pull_paced: offer.pull_paced,
        })
        .await?;
    let frame = reader
        .next_frame()
        .await?
        .context("SSH daemon closed before Welcome")?;
    let identity = match driver.classify(&frame)? {
        Incoming::Control(ConnToClientMsg::Welcome { identity }) => identity,
        Incoming::Control(ConnToClientMsg::Refused { reason, detail }) => {
            bail!("SSH daemon refused connection: {reason:?}: {detail:?}")
        }
        other => bail!("expected SSH daemon Welcome, received {other:?}"),
    };
    driver.handshake_done(offer.mode);
    let mut connection = Connection::from_halves(reader, writer, offer.mode);
    connection.driver = driver;
    connection.accepted_major = major;
    connection.daemon_minor = advertised;
    connection.daemon_identity = identity;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_cannot_be_read_as_an_ssh_option() {
        let command = relay_command("-oProxyCommand=false", None);
        let args: Vec<_> = command.as_std().get_args().collect();
        assert_eq!(args[..3], ["-oBatchMode=yes", "--", "-oProxyCommand=false"]);
    }

    #[test]
    fn default_socket_is_resolved_on_the_remote_host() {
        let command = relay_command("dev", None);
        assert_eq!(
            command.as_std().get_args().last().unwrap(),
            "exec felis-daemon relay --no-spawn"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_handshake_reaps_the_relay_process() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("pid");
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "echo $$ > \"$1\"; exec sleep 60", "relay"])
            .arg(&marker);
        let task = tokio::spawn(connect_command(command, Offer::ops()));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() {
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let pid = std::fs::read_to_string(marker).unwrap();
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        loop {
            // The shell builtin: the Nix build sandbox on Linux has no /bin/kill.
            let alive = Command::new("/bin/sh")
                .args(["-c", "kill -0 \"$1\"", "kill", pid.trim()])
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .unwrap()
                .success();
            if !alive {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "cancelled relay survived"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[test]
    fn remote_shell_receives_the_socket_as_literal_data() {
        let socket = "relative dir/a'b $(printf expanded); \"socket\"\n.sock";
        let command = relay_command("dev", Some(socket));
        let remote = command
            .as_std()
            .get_args()
            .last()
            .unwrap()
            .to_str()
            .unwrap();
        let quoted = remote
            .strip_prefix("exec felis-daemon relay --no-spawn --socket ")
            .unwrap();
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &format!("printf '%s' {quoted}")])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, socket.as_bytes());
    }

    #[tokio::test]
    async fn handshake_negotiates_versions_without_losing_buffered_frames() {
        let (client, server) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move {
            let (mut read, mut write) = tokio::io::split(server);
            felis_transport::preface::read_client_preface(&mut read)
                .await
                .unwrap();
            felis_transport::preface::write_daemon_preface(
                &mut write,
                DaemonPreface::Accept {
                    major: preface::PROTOCOL_MAJOR,
                    minor: preface::PROTOCOL_MINOR + 1,
                },
            )
            .await
            .unwrap();
            let mut reader = FrameReader::new(read);
            assert!(reader.next_frame().await.unwrap().is_some());
            let mut writer = FrameWriter::new(write, preface::PROTOCOL_MINOR);
            writer
                .send(&ConnToClientMsg::Welcome { identity: None })
                .await
                .unwrap();
            writer
                .send(&ConnToClientMsg::Welcome { identity: None })
                .await
                .unwrap();
        });
        let (read, write) = tokio::io::split(client);
        let mut conn = handshake(read, write, Offer::window(false)).await.unwrap();
        assert_eq!(conn.effective_minor, preface::PROTOCOL_MINOR);
        assert_eq!(conn.daemon_minor, preface::PROTOCOL_MINOR + 1);
        assert!(conn.next_frame().await.unwrap().is_some());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn handshake_rejects_an_incompatible_daemon() {
        let (client, server) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move {
            let (mut read, mut write) = tokio::io::split(server);
            felis_transport::preface::read_client_preface(&mut read)
                .await
                .unwrap();
            felis_transport::preface::write_daemon_preface(
                &mut write,
                DaemonPreface::Refuse {
                    min_major: preface::PROTOCOL_MAJOR + 1,
                    max_major: preface::PROTOCOL_MAJOR + 1,
                },
            )
            .await
            .unwrap();
        });
        let (read, write) = tokio::io::split(client);
        let result = handshake(read, write, Offer::window(false)).await;
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("negotiate SSH daemon protocol")
        );
        task.await.unwrap();
    }
}
