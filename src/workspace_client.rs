//! Ordered, asynchronous layout synchronization and a connection-scoped lease.
use anyhow::{Result, anyhow};
use eframe::egui;
use tokio::sync::{mpsc, oneshot};

use crate::{
    daemon::{self, Payload, Request},
    workspace::Snapshot,
};

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

enum Command {
    Request(Request),
    Prepare {
        snapshot: Snapshot,
        selection: daemon::Selection,
    },
    Close {
        snapshot: Option<Snapshot>,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

pub struct Client {
    tx: mpsc::Sender<Command>,
    pub events: mpsc::UnboundedReceiver<Event>,
    task: tokio::task::JoinHandle<()>,
}

impl Client {
    pub fn start(runtime: &tokio::runtime::Runtime, ctx: egui::Context, window: u32) -> Self {
        let (tx, mut commands) = mpsc::channel(16);
        let (events_tx, events) = mpsc::unbounded_channel();
        let task = runtime.spawn(async move {
            let opened = async {
                let mut stream = daemon::connect().await?;
                let payload = daemon::exchange(&mut stream, Request::Open { window }).await?;
                Ok::<_, anyhow::Error>((stream, payload))
            }
            .await;
            let (mut stream, payload) = match opened {
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
            while let Some(command) = commands.recv().await {
                let (operation, result) = match command {
                    Command::Request(request) => {
                        let operation = match &request {
                            Request::Save(_) => Operation::Save,
                            Request::List => Operation::List,
                            Request::Finish { .. } => Operation::Finish,
                            _ => unreachable!(),
                        };
                        (operation, daemon::exchange(&mut stream, request).await)
                    }
                    Command::Prepare {
                        snapshot,
                        selection,
                    } => (
                        Operation::Prepare,
                        daemon::prepare(&mut stream, snapshot, selection).await,
                    ),
                    Command::Close { snapshot, reply } => {
                        let result = if let Some(snapshot) = snapshot {
                            daemon::exchange(&mut stream, Request::Save(snapshot))
                                .await
                                .map(|_| ())
                                .map_err(|error| format!("{error:#}"))
                        } else {
                            Ok(())
                        };
                        let result = match result {
                            Ok(()) => daemon::exchange(&mut stream, Request::Detach)
                                .await
                                .map(|_| ())
                                .map_err(|error| format!("{error:#}")),
                            error => error,
                        };
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
                    let _ = events_tx.send(Event { operation, result });
                    ctx.request_repaint();
                }
                if disconnected {
                    return;
                }
            }
        });
        Self { tx, events, task }
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    pub fn send(&self, request: Request) -> Result<()> {
        self.tx
            .try_send(Command::Request(request))
            .map_err(|error| anyhow!("nekomata daemon command unavailable: {error}"))
    }

    pub fn prepare(&self, snapshot: Snapshot, selection: daemon::Selection) -> Result<()> {
        self.tx
            .try_send(Command::Prepare {
                snapshot,
                selection,
            })
            .map_err(|error| anyhow!("nekomata daemon command unavailable: {error}"))
    }

    pub fn close(
        &self,
        runtime: &tokio::runtime::Runtime,
        snapshot: Option<Snapshot>,
    ) -> Result<()> {
        runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(6), async {
                let (reply, result) = oneshot::channel();
                self.tx.send(Command::Close { snapshot, reply }).await?;
                result.await?.map_err(anyhow::Error::msg)
            })
            .await?
        })
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}
