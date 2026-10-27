//! The phases of replacing a window's workspace, free of egui and I/O.
//!
//! The window applies what each transition returns: it sends the `Finish`
//! request [`Switch::poll`] asks for, and adopts or discards the destination
//! that [`Switch::finished`] resolves to.
use std::collections::BTreeMap;

use crate::{
    control::Reply,
    layout::Layout,
    session::Target,
    workspace::{PaneSpec, Snapshot, Tab},
};

pub type ReplySender = tokio::sync::oneshot::Sender<Reply>;

/// What a destination pane's felis connection has reached so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneStatus {
    Connecting,
    Connected(Option<u128>),
    Failed(String),
    Ended,
}

enum Phase<P> {
    /// The source snapshot and `Prepare` are on their way to the daemon.
    Preparing,
    /// The destination is reserved; its panes are connecting.
    Connecting {
        id: u64,
        snapshot: Snapshot,
        panes: BTreeMap<u64, P>,
    },
    /// `Finish` with the destination snapshot was sent.
    Committing {
        id: u64,
        snapshot: Snapshot,
        panes: BTreeMap<u64, P>,
    },
    /// The reservation must be released; `sent` once `Finish` without a
    /// snapshot was sent.
    Cancelling { error: String, sent: bool },
}

pub struct Switch<P> {
    phase: Phase<P>,
    reply: Option<ReplySender>,
}

/// The request [`Switch::poll`] wants sent now.
#[derive(Debug, PartialEq)]
pub enum Poll {
    Wait,
    Finish(Option<Snapshot>),
}

pub enum Step<P> {
    Pending(Switch<P>),
    Committed {
        id: u64,
        snapshot: Snapshot,
        panes: BTreeMap<u64, P>,
        reply: Option<ReplySender>,
    },
    Failed {
        error: String,
        reply: Option<ReplySender>,
    },
}

impl<P> Switch<P> {
    pub fn new(reply: Option<ReplySender>) -> Self {
        Self {
            phase: Phase::Preparing,
            reply,
        }
    }

    /// The daemon reserved `id`; `panes` connect the destination `snapshot`.
    pub fn prepared(&mut self, id: u64, snapshot: Snapshot, panes: BTreeMap<u64, P>) {
        if matches!(self.phase, Phase::Preparing) {
            self.phase = Phase::Connecting {
                id,
                snapshot,
                panes,
            };
        }
    }

    /// Abandons the destination, dropping its panes, and releases the
    /// reservation on the next poll.
    pub fn cancel(&mut self, error: String) {
        self.phase = Phase::Cancelling { error, sent: false };
    }

    pub fn poll(&mut self, status: impl Fn(&P) -> PaneStatus, reply_closed: bool) -> Poll {
        let error = match &mut self.phase {
            Phase::Connecting {
                snapshot, panes, ..
            } => {
                let mut sessions = BTreeMap::new();
                let mut error = None;
                for (&id, pane) in panes.iter() {
                    match status(pane) {
                        PaneStatus::Connected(session) => {
                            sessions.insert(id, session);
                        }
                        PaneStatus::Connecting => {}
                        PaneStatus::Failed(failure) => {
                            error = Some(failure);
                            break;
                        }
                        PaneStatus::Ended => {
                            error = Some("destination session ended".into());
                            break;
                        }
                    }
                }
                if reply_closed {
                    error = Some("workspace switch cancelled".into());
                }
                match error {
                    Some(error) => error,
                    None if sessions.len() < panes.len() => return Poll::Wait,
                    None => {
                        for (id, session) in sessions {
                            if let Some(spec) = snapshot.panes.get_mut(&id) {
                                spec.session = session.map(|id| format!("{id:032x}"));
                            }
                        }
                        return Poll::Finish(Some(snapshot.clone()));
                    }
                }
            }
            Phase::Cancelling { sent: false, .. } => return Poll::Finish(None),
            Phase::Preparing | Phase::Committing { .. } | Phase::Cancelling { .. } => {
                return Poll::Wait;
            }
        };
        self.cancel(error);
        Poll::Finish(None)
    }

    /// Records that the `Finish` request [`Self::poll`] asked for was sent.
    pub fn finish_sent(&mut self) {
        self.phase = match std::mem::replace(&mut self.phase, Phase::Preparing) {
            Phase::Connecting {
                id,
                snapshot,
                panes,
            } => Phase::Committing {
                id,
                snapshot,
                panes,
            },
            Phase::Cancelling { error, .. } => Phase::Cancelling { error, sent: true },
            phase => phase,
        };
    }

    /// The daemon answered `Finish`.
    pub fn finished(mut self, result: Result<(), String>) -> Step<P> {
        match (std::mem::replace(&mut self.phase, Phase::Preparing), result) {
            (Phase::Cancelling { error, .. }, Ok(())) | (Phase::Cancelling { .. }, Err(error)) => {
                Step::Failed {
                    error,
                    reply: self.reply,
                }
            }
            (
                Phase::Committing {
                    id,
                    snapshot,
                    panes,
                },
                Ok(()),
            ) => Step::Committed {
                id,
                snapshot,
                panes,
                reply: self.reply,
            },
            (_, Ok(())) => Step::Failed {
                error: "workspace switch ended unexpectedly".into(),
                reply: self.reply,
            },
            (_, Err(error)) => {
                self.cancel(error);
                Step::Pending(self)
            }
        }
    }

    /// Why the switch is being cancelled, once it is.
    pub fn error(&self) -> Option<&str> {
        match &self.phase {
            Phase::Cancelling { error, .. } => Some(error),
            _ => None,
        }
    }

    /// Whether the CLI that requested the switch stopped waiting for it.
    pub fn reply_closed(&self) -> bool {
        self.reply.as_ref().is_some_and(|reply| reply.is_closed())
    }

    /// The daemon refused the switch or the connection to it was lost; there
    /// is no reservation left to release.
    pub fn abort(self) -> Option<ReplySender> {
        self.reply
    }
}

/// Gives a destination without tabs, such as a new workspace, its first pane.
pub fn add_first_tab(snapshot: &mut Snapshot, font_size: f32, target: Target) {
    snapshot.font_size = font_size;
    let pane = snapshot.next_id;
    snapshot.next_id += 1;
    snapshot.tabs.push(Tab {
        name: format!("Terminal {pane}"),
        layout: Layout::Pane(pane),
        focused: pane,
        focus_history: vec![pane],
    });
    snapshot.panes.insert(
        pane,
        PaneSpec {
            target,
            session: None,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> Target {
        Target::Local("/felis.sock".into())
    }

    fn destination(sessions: &[Option<&str>]) -> Snapshot {
        let mut snapshot = Snapshot::default();
        for (index, session) in sessions.iter().enumerate() {
            let id = index as u64 + 1;
            snapshot.tabs.push(Tab {
                name: format!("Terminal {id}"),
                layout: Layout::Pane(id),
                focused: id,
                focus_history: vec![id],
            });
            snapshot.panes.insert(
                id,
                PaneSpec {
                    target: target(),
                    session: session.map(str::to_owned),
                },
            );
        }
        snapshot.next_id = sessions.len() as u64 + 1;
        snapshot
    }

    /// Test panes are just the status they report.
    type Status = std::cell::RefCell<PaneStatus>;

    fn connecting(
        sessions: &[Option<&str>],
    ) -> (
        Switch<std::rc::Rc<Status>>,
        BTreeMap<u64, std::rc::Rc<Status>>,
    ) {
        let snapshot = destination(sessions);
        let panes: BTreeMap<_, _> = snapshot
            .panes
            .keys()
            .map(|&id| (id, std::rc::Rc::new(Status::new(PaneStatus::Connecting))))
            .collect();
        let mut switch = Switch::new(None);
        switch.prepared(9, snapshot, panes.clone());
        (switch, panes)
    }

    fn status(pane: &std::rc::Rc<Status>) -> PaneStatus {
        pane.borrow().clone()
    }

    #[test]
    fn nothing_is_sent_before_the_destination_is_reserved() {
        let mut switch = Switch::<()>::new(None);
        assert_eq!(switch.poll(|_| unreachable!(), false), Poll::Wait);
    }

    #[test]
    fn the_switch_commits_once_every_destination_pane_connects() {
        let (mut switch, panes) = connecting(&[Some(&"a".repeat(32)), None]);
        *panes[&1].borrow_mut() = PaneStatus::Connected(Some(0xa));
        assert_eq!(switch.poll(status, false), Poll::Wait);
        *panes[&2].borrow_mut() = PaneStatus::Connected(Some(0xb));
        let Poll::Finish(Some(snapshot)) = switch.poll(status, false) else {
            panic!("expected a commit")
        };
        assert_eq!(
            snapshot.panes[&2].session.as_deref(),
            Some(format!("{:032x}", 0xb).as_str()),
            "the commit records the sessions the panes reached"
        );
        switch.finish_sent();
        assert_eq!(switch.poll(status, false), Poll::Wait);
        let Step::Committed { id, panes, .. } = switch.finished(Ok(())) else {
            panic!("expected a committed switch")
        };
        assert_eq!(id, 9);
        assert_eq!(panes.len(), 2);
    }

    #[test]
    fn an_unsent_commit_is_retried() {
        let (mut switch, panes) = connecting(&[None]);
        *panes[&1].borrow_mut() = PaneStatus::Connected(Some(1));
        assert!(matches!(switch.poll(status, false), Poll::Finish(Some(_))));
        assert!(matches!(switch.poll(status, false), Poll::Finish(Some(_))));
    }

    #[test]
    fn a_failing_destination_pane_cancels_the_switch() {
        let (mut switch, panes) = connecting(&[None, None]);
        *panes[&1].borrow_mut() = PaneStatus::Connected(Some(1));
        *panes[&2].borrow_mut() = PaneStatus::Failed("unreachable".into());
        assert_eq!(switch.poll(status, false), Poll::Finish(None));
        switch.finish_sent();
        assert_eq!(switch.poll(status, false), Poll::Wait);
        let Step::Failed { error, .. } = switch.finished(Ok(())) else {
            panic!("expected a failed switch")
        };
        assert_eq!(error, "unreachable");
    }

    #[test]
    fn an_ending_destination_session_cancels_the_switch() {
        let (mut switch, panes) = connecting(&[None]);
        *panes[&1].borrow_mut() = PaneStatus::Ended;
        assert_eq!(switch.poll(status, false), Poll::Finish(None));
        switch.finish_sent();
        let Step::Failed { error, .. } = switch.finished(Ok(())) else {
            panic!("expected a failed switch")
        };
        assert_eq!(error, "destination session ended");
    }

    #[test]
    fn a_requester_that_gave_up_cancels_the_switch() {
        let (mut switch, _panes) = connecting(&[None]);
        assert_eq!(switch.poll(status, true), Poll::Finish(None));
    }

    #[test]
    fn a_rejected_commit_releases_the_reservation() {
        let (mut switch, panes) = connecting(&[None]);
        *panes[&1].borrow_mut() = PaneStatus::Connected(Some(1));
        assert!(matches!(switch.poll(status, false), Poll::Finish(Some(_))));
        switch.finish_sent();
        let Step::Pending(mut switch) = switch.finished(Err("invalid".into())) else {
            panic!("expected the switch to keep cancelling")
        };
        assert_eq!(switch.poll(status, false), Poll::Finish(None));
        switch.finish_sent();
        let Step::Failed { error, .. } = switch.finished(Ok(())) else {
            panic!("expected a failed switch")
        };
        assert_eq!(error, "invalid");
    }

    #[test]
    fn a_rejected_release_reports_its_own_error() {
        let (mut switch, _panes) = connecting(&[None]);
        switch.cancel("first".into());
        switch.finish_sent();
        let Step::Failed { error, .. } = switch.finished(Err("second".into())) else {
            panic!("expected a failed switch")
        };
        assert_eq!(error, "second");
    }

    #[test]
    fn a_reservation_arriving_after_cancellation_is_released() {
        let mut switch = Switch::new(None);
        switch.cancel("gone".into());
        switch.prepared(9, destination(&[None]), BTreeMap::from([(1, ())]));
        assert_eq!(switch.poll(|_| unreachable!(), false), Poll::Finish(None));
    }

    #[test]
    fn a_new_workspace_gets_one_pane_with_a_fresh_shell() {
        let mut snapshot = Snapshot::default();
        add_first_tab(&mut snapshot, 13.0, target());
        snapshot.validate().unwrap();
        assert_eq!(snapshot.font_size, 13.0);
        assert_eq!(snapshot.tabs.len(), 1);
        assert_eq!(snapshot.panes[&1].session, None);
        assert_eq!(snapshot.next_id, 2);
    }
}
