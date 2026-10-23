//! SSH relay adapter for explicit remote sockets, which Carrier::Ssh cannot carry.
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

fn relay_command(host: &str, socket: &str) -> Command {
    // SSH joins remote argv into shell text. Quote the path for that shell,
    // including embedded apostrophes, rather than treating argv as an escape.
    let quoted = format!("'{}'", socket.replace('\'', "'\\''"));
    let mut command = Command::new("ssh");
    command
        .arg("-oBatchMode=yes")
        .arg(host)
        .arg(format!(
            "exec felis-daemon relay --no-spawn --socket {quoted}"
        ))
        .kill_on_drop(true);
    command
}

pub async fn connect(host: &str, socket: &str, offer: Offer) -> Result<(CarrierConnection, Child)> {
    let session = felis_transport::spawn_command(relay_command(host, socket))?;
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
    fn remote_shell_receives_the_socket_as_literal_data() {
        let socket = "relative dir/a'b $(printf expanded); \"socket\"\n.sock";
        let command = relay_command("dev", socket);
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
