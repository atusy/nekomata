//! Cancellable operations on an attached Ops connection, independent of grid pulls.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use eframe::egui;
use felis_client_core::{AttachIntent, CarrierConnection, Offer, RemoteSpawn, ShadowScreen};
use felis_protocol::{
    MessageKind,
    messages::{
        ColSpan, ConnToClientMsg, ConnToDaemonMsg, GridDims, InputMsg, RegionPosition,
        RegionSource, RegionToClientMsg, RegionToDaemonMsg, SearchOptions, SearchSource,
        SearchToClientMsg, SearchToDaemonMsg, StreamId, Subject, ViewportAnchor,
    },
};
use felis_render_core::overlay::{SearchHitSpan, SearchOverlay};
use felis_transport::{Delivery, Incoming};
use tokio::sync::watch;

use crate::session::{State, Target};

const MAX_HITS: usize = 1_000;
const MAX_SEARCH_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Source {
    id: u128,
    generation: u64,
}

fn source(state: &Mutex<State>) -> Option<Source> {
    let state = state.lock().unwrap();
    state.connected.then_some(Source {
        id: state.session_id?,
        generation: state.reconnects,
    })
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub line: i64,
    pub spans: Vec<ColSpan>,
    pub anchor: Option<ViewportAnchor>,
}

#[derive(Default)]
pub struct SearchState {
    pub revision: u64,
    generation: u64,
    source: Option<Source>,
    pub active: bool,
    pub running: bool,
    pub hits: Vec<Hit>,
    pub current: Option<usize>,
    pub error: Option<String>,
    pub limited: bool,
    bytes: usize,
    jump_pending: bool,
}

impl SearchState {
    fn begin(&mut self, from: Option<Source>) -> u64 {
        let revision = self.revision.wrapping_add(1);
        let generation = self.generation.wrapping_add(1);
        *self = Self {
            revision,
            generation,
            source: from,
            active: true,
            running: from.is_some(),
            ..Default::default()
        };
        if from.is_none() {
            self.error = Some("Wait for the session to connect".into());
        }
        generation
    }

    fn push(&mut self, generation: u64, message: SearchToClientMsg) -> bool {
        if self.generation != generation || !self.active {
            return false;
        }
        let SearchToClientMsg::Match {
            line_index,
            text,
            byte_spans,
            col_spans,
            anchor,
        } = message;
        let size = text
            .len()
            .saturating_add(
                byte_spans
                    .len()
                    .saturating_mul(std::mem::size_of::<felis_protocol::messages::ByteSpan>()),
            )
            .saturating_add(
                col_spans
                    .len()
                    .saturating_mul(std::mem::size_of::<ColSpan>()),
            );
        if self.hits.len() >= MAX_HITS || size > MAX_SEARCH_BYTES.saturating_sub(self.bytes) {
            self.limited = true;
            return false;
        }
        self.bytes += size;
        self.hits.push(Hit {
            line: line_index,
            spans: col_spans,
            anchor,
        });
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn advance(&mut self, older: bool) {
        if self.hits.is_empty() {
            return;
        }
        self.current = Some(match self.current {
            None => 0,
            Some(index) if older => (index + 1) % self.hits.len(),
            Some(index) => (index + self.hits.len() - 1) % self.hits.len(),
        });
        self.jump_pending = true;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn take_jump(&mut self, shadow: &ShadowScreen) -> Option<InputMsg> {
        if !std::mem::take(&mut self.jump_pending) {
            return None;
        }
        let hit = self.hits.get(self.current?)?;
        let line = hit.spans.first().map_or(hit.line, |span| span.line_index);
        let shown_anchor = shadow.viewport_anchor();
        let shift = hit
            .anchor
            .zip(shown_anchor)
            .map_or(Some(0), |(old, shown)| old.rows_until(shown));
        // Match felis's navigation frame: never project a fresh hit into an
        // older viewport, or a history hit may incorrectly snap to live.
        let (line, anchor, max) = match shift {
            Some(shift) if hit.anchor.is_none() || (shift >= 0 && shown_anchor.is_some()) => (
                line.saturating_sub(shift),
                shown_anchor,
                shadow
                    .viewport_max()
                    .saturating_sub(u32::from(shadow.screen().rows())),
            ),
            _ => (line, hit.anchor, u32::MAX),
        };
        let offset =
            felis_client_core::viewport::viewport_for_hit(line, shadow.screen().rows(), max);
        Some(InputMsg::Viewport {
            lines_from_bottom: offset,
            anchor: anchor.filter(|_| offset > 0),
        })
    }

    pub fn overlay(&self, shadow: &ShadowScreen) -> Option<SearchOverlay> {
        if !self.active {
            return None;
        }
        let mut visible_hits = Vec::new();
        for (index, hit) in self.hits.iter().enumerate() {
            let shift = match hit.anchor.zip(shadow.viewport_anchor()) {
                Some((old, shown)) => old.rows_until(shown),
                None => Some(0),
            };
            let Some(shift) = shift else {
                continue;
            };
            for span in &hit.spans {
                if span.col_end <= span.col_start {
                    continue;
                }
                if let Some(row) = felis_client_core::viewport::composed_row_for_line(
                    span.line_index.saturating_sub(shift),
                    shadow.viewport(),
                    shadow.screen().rows(),
                ) {
                    visible_hits.push(SearchHitSpan {
                        row,
                        col_start: span.col_start,
                        col_end: span.col_end,
                        is_current: self.current == Some(index),
                    });
                }
            }
        }
        Some(SearchOverlay {
            label: String::new(),
            visible_hits,
        })
    }
}

struct Job {
    cancel: watch::Sender<bool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

pub struct Output {
    pub data: Vec<u8>,
    pub position: Option<RegionPosition>,
    pub origin: felis_client_core::pipe::Origin,
    pub dims: GridDims,
    pub target: Target,
    pub edit: bool,
}
#[derive(Default)]
pub struct OutputState {
    generation: u64,
    source: Option<Source>,
    pub running: bool,
    pub result: Option<Result<Output, String>>,
}

pub struct Tools {
    pub output: Arc<Mutex<OutputState>>,
    output_task: Mutex<Option<Job>>,
    pub search: Arc<Mutex<SearchState>>,
    task: Mutex<Option<Job>>,

    runtime: tokio::runtime::Handle,
    ctx: egui::Context,
    target: Target,
    session: Arc<Mutex<State>>,
}

impl Tools {
    pub fn new(
        runtime: &tokio::runtime::Runtime,
        ctx: egui::Context,
        target: Target,
        session: Arc<Mutex<State>>,
    ) -> Self {
        Self {
            output: Arc::default(),
            output_task: Mutex::default(),
            search: Arc::default(),
            task: Mutex::default(),

            runtime: runtime.handle().clone(),
            ctx,
            target,
            session,
        }
    }

    pub fn take_output(&self) -> Option<Result<Output, String>> {
        let live = source(&self.session);
        let mut state = self.output.lock().unwrap();
        let result = state.result.take()?;
        Some(if state.source == live {
            result
        } else {
            Err("The session changed; request output again".into())
        })
    }

    pub fn command_output(&self, edit: bool) {
        self.output_task.lock().unwrap().take();
        let from = source(&self.session);
        let generation = {
            let mut state = self.output.lock().unwrap();
            state.generation = state.generation.wrapping_add(1);
            state.source = from;
            state.running = from.is_some();
            state.result = from
                .is_none()
                .then(|| Err("Wait for the session to connect".into()));
            state.generation
        };
        let Some(from) = from else {
            return;
        };
        let (origin, dims) = {
            let state = self.session.lock().unwrap();
            (
                felis_client_core::pipe::Origin {
                    session_id: from.id,
                    host: match &self.target {
                        Target::Ssh { host, .. } => Some(host.clone()),
                        _ => None,
                    },
                    osc7: state.screen.cwd().map(str::to_owned),
                },
                GridDims {
                    rows: state.screen.screen().rows(),
                    cols: state.screen.screen().cols(),
                    pixel_w: 0,
                    pixel_h: 0,
                },
            )
        };
        let (cancel, mut cancelled) = watch::channel(false);
        *self.output_task.lock().unwrap() = Some(Job { cancel });
        let target = self.target.clone();
        let shared = self.output.clone();
        let session = self.session.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let result = region_request(&target, from.id, &mut cancelled)
                .await
                .map(|(data, position)| Output {
                    data,
                    position,
                    origin,
                    dims,
                    target,
                    edit,
                })
                .map_err(|e| format!("{e:#}"));
            let live = source(&session) == Some(from);
            let mut state = shared.lock().unwrap();
            if state.generation != generation {
                return;
            }
            state.running = false;
            state.result = Some(if live {
                result
            } else {
                Err("The session changed; request output again".into())
            });
            ctx.request_repaint();
        });
    }

    pub fn cancel_search(&self) {
        self.task.lock().unwrap().take();
        let mut state = self.search.lock().unwrap();
        state.begin(None);
        state.active = false;
        state.error = None;
        self.ctx.request_repaint();
    }

    pub fn search(&self, query: String, options: SearchOptions) {
        self.task.lock().unwrap().take();
        let from = source(&self.session);
        let generation = self.search.lock().unwrap().begin(from);
        let Some(from) = from else {
            return;
        };
        let (cancel, mut cancelled) = watch::channel(false);
        *self.task.lock().unwrap() = Some(Job { cancel });
        let target = self.target.clone();
        let shared = self.search.clone();
        let session = self.session.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let result = search_request(
                &target,
                from,
                &query,
                options,
                generation,
                &shared,
                &session,
                &ctx,
                &mut cancelled,
            )
            .await;
            let live = source(&session) == Some(from);
            let mut state = shared.lock().unwrap();
            if state.generation != generation {
                return;
            }
            state.running = false;
            if !live {
                state.hits.clear();
                state.current = None;
                state.error = Some("The session changed; search again".into());
            } else if let Err(error) = result {
                state.error = Some(format!("{error:#}"));
            } else if !state.hits.is_empty() {
                state.current = Some(0);
                state.jump_pending = true;
            }
            state.revision = state.revision.wrapping_add(1);
            ctx.request_repaint();
        });
    }

    pub fn invalidate_changed_session(&self) {
        let live = source(&self.session);
        let stale = {
            let search = self.search.lock().unwrap();
            search.active && search.source != live
        };
        if stale {
            self.cancel_search();
        }
        let mut output = self.output.lock().unwrap();
        if output.source != live && output.running {
            self.output_task.lock().unwrap().take();
            output.generation = output.generation.wrapping_add(1);
            output.running = false;
            output.result = Some(Err("The session changed; request output again".into()));
        }
    }
}

struct Ops {
    conn: CarrierConnection,
    _relay: Option<tokio::process::Child>,
}
async fn attached_ops(target: &Target, id: u128) -> Result<Ops> {
    let mut ops = if let Target::Ssh {
        host,
        socket: Some(socket),
    } = target
    {
        let (conn, relay) = crate::ssh::connect(host, socket, Offer::ops()).await?;
        Ops {
            conn,
            _relay: Some(relay),
        }
    } else {
        Ops {
            conn: felis_client_core::connect_carrier(
                target.carrier(),
                Offer::ops(),
                RemoteSpawn::Refuse,
            )
            .await?,
            _relay: None,
        }
    };
    ops.conn
        .attach_by_prefix(format!("{id:032x}"), AttachIntent::Automatic)
        .await?;
    Ok(ops)
}

async fn cancel(conn: &mut CarrierConnection, stream_id: StreamId) {
    conn.driver.cancel_stream(stream_id);
    // The connection is private to this operation. On a stalled Cancel,
    // closing it still releases the daemon's stream and the SSH relay.
    let _ = tokio::time::timeout(
        Duration::from_millis(250),
        conn.writer.send(&ConnToDaemonMsg::Cancel { stream_id }),
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn search_request(
    target: &Target,
    from: Source,
    query: &str,
    options: SearchOptions,
    generation: u64,
    shared: &Mutex<SearchState>,
    session: &Mutex<State>,
    ctx: &egui::Context,
    cancelled: &mut watch::Receiver<bool>,
) -> Result<()> {
    let mut ops = tokio::select! {
        _ = cancelled.changed() => return Ok(()),
        connected = tokio::time::timeout(Duration::from_secs(5), attached_ops(target, from.id)) => connected.context("search connection timed out")??,
    };
    let stream = tokio::time::timeout(
        Duration::from_secs(5),
        ops.conn.open_stream(&SearchToDaemonMsg::Query {
            query: query.into(),
            options,
            source: SearchSource::Scrollback,
        }),
    )
    .await
    .context("search request timed out")??;
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    loop {
        let frame = tokio::select! {
            _ = cancelled.changed() => { cancel(&mut ops.conn, stream).await; return Ok(()); },
            _ = tokio::time::sleep_until(deadline) => { cancel(&mut ops.conn, stream).await; bail!("search timed out"); },
            frame = ops.conn.next_frame() => frame?.context("search connection closed")?,
        };
        match ops.conn.driver.classify(&frame)? {
            Incoming::Payload(payload) if payload.kind == MessageKind::Search => {
                if let Delivery::Deliver(delivered) =
                    ops.conn.driver.decode::<SearchToClientMsg>(&payload)?
                {
                    if source(session) != Some(from) {
                        cancel(&mut ops.conn, stream).await;
                        return Ok(());
                    }
                    let accepted = shared.lock().unwrap().push(generation, delivered.msg);
                    ctx.request_repaint();
                    if !accepted {
                        cancel(&mut ops.conn, stream).await;
                        return Ok(());
                    }
                }
            }
            Incoming::Control(ConnToClientMsg::End { stream_id, .. }) if stream_id == stream => {
                return Ok(());
            }
            Incoming::Control(ConnToClientMsg::Error {
                subject: Subject::Stream(id),
                detail,
                ..
            }) if id == stream => bail!("{detail}"),
            // Ops attachments can receive unsolicited attach/state notices.
            // Presentation remains owned by the window connection.
            Incoming::Payload(payload) if payload.correlation.is_none() => {}
            Incoming::CancelIgnored { .. } => {}
            other => bail!("Unexpected search response: {other:?}"),
        }
    }
}

const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
async fn region_request(
    target: &Target,
    id: u128,
    cancelled: &mut watch::Receiver<bool>,
) -> Result<(Vec<u8>, Option<RegionPosition>)> {
    let mut ops = tokio::select! {
        _ = cancelled.changed() => bail!("output request cancelled"),
        connected = tokio::time::timeout(Duration::from_secs(5), attached_ops(target, id)) => connected.context("output connection timed out")??,
    };
    let request = ops.conn.driver.issue_request()?;
    tokio::time::timeout(
        Duration::from_secs(5),
        ops.conn.writer.send_correlated(
            &RegionToDaemonMsg::Request {
                source: RegionSource::CommandOutput,
                ansi: false,
            },
            felis_protocol::messages::Correlation::request(request),
        ),
    )
    .await
    .context("output request timed out")??;
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    loop {
        let frame = tokio::select! {
            _ = cancelled.changed() => { bail!("output request cancelled"); },
            _ = tokio::time::sleep_until(deadline) => { bail!("output request timed out"); },
            frame = ops.conn.next_frame() => frame?.context("output connection closed")?,
        };
        match ops.conn.driver.classify(&frame)? {
            Incoming::Payload(payload) if payload.kind == MessageKind::Region => {
                if let Delivery::Deliver(delivered) =
                    ops.conn.driver.decode::<RegionToClientMsg>(&payload)?
                {
                    let RegionToClientMsg::Reply { data, position, .. } = delivered.msg else {
                        bail!("Unexpected streamed output response");
                    };
                    if data.is_empty() {
                        bail!("No command output is available (shell integration required)");
                    }
                    if data.len() > MAX_OUTPUT_BYTES {
                        bail!("Command output exceeds the 8 MiB limit");
                    }
                    return Ok((data, position));
                }
            }
            Incoming::Control(ConnToClientMsg::Error {
                subject: Subject::Request(id),
                detail,
                ..
            }) if id == request => bail!("{detail}"),
            Incoming::Payload(payload) if payload.correlation.is_none() => {}
            Incoming::CancelIgnored { .. } => {}
            other => bail!("Unexpected output response: {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hit(text: String) -> SearchToClientMsg {
        SearchToClientMsg::Match {
            line_index: -1,
            text,
            byte_spans: vec![],
            col_spans: vec![ColSpan {
                line_index: -1,
                col_start: 0,
                col_end: 3,
            }],
            anchor: None,
        }
    }
    #[test]
    fn search_navigation_rebases_history_and_drops_invalid_overlay_generations() {
        let old = ViewportAnchor {
            pushed: 100,
            generation: 1,
        };
        let shown = ViewportAnchor {
            pushed: 110,
            generation: 1,
        };
        let mut screen = ShadowScreen::new(24, 80);
        screen
            .apply(&felis_protocol::messages::GridMsg::ViewportState {
                lines_from_bottom: 20,
                max: 124,
                anchor: Some(shown),
            })
            .unwrap();
        let mut state = SearchState {
            active: true,
            current: Some(0),
            jump_pending: true,
            hits: vec![Hit {
                line: -10,
                spans: vec![ColSpan {
                    line_index: -10,
                    col_start: 1,
                    col_end: 3,
                }],
                anchor: Some(old),
            }],
            ..Default::default()
        };
        assert_eq!(
            state.take_jump(&screen),
            Some(InputMsg::Viewport {
                lines_from_bottom: 32,
                anchor: Some(shown)
            })
        );
        assert_eq!(state.overlay(&screen).unwrap().visible_hits[0].row, 0);
        screen
            .apply(&felis_protocol::messages::GridMsg::ViewportState {
                lines_from_bottom: 20,
                max: 124,
                anchor: Some(ViewportAnchor {
                    generation: 2,
                    ..shown
                }),
            })
            .unwrap();
        assert!(state.overlay(&screen).unwrap().visible_hits.is_empty());
        state.jump_pending = true;
        assert_eq!(
            state.take_jump(&screen),
            Some(InputMsg::Viewport {
                lines_from_bottom: 22,
                anchor: Some(old)
            })
        );
    }

    #[test]
    fn superseded_search_results_cannot_replace_the_current_query() {
        let mut state = SearchState::default();
        let old = state.begin(Some(Source {
            id: 1,
            generation: 0,
        }));
        let current = state.begin(Some(Source {
            id: 1,
            generation: 0,
        }));
        assert!(!state.push(old, hit("old".into())));
        assert!(state.push(current, hit("new".into())));
        assert_eq!(state.hits.len(), 1);
    }
    #[test]
    fn search_limits_aggregate_bytes_and_wraps_navigation() {
        let mut state = SearchState::default();
        let generation = state.begin(None);
        assert!(state.push(generation, hit("one".into())));
        assert!(state.push(generation, hit("two".into())));
        state.advance(true);
        assert_eq!(state.current, Some(0));
        state.advance(false);
        assert_eq!(state.current, Some(1));
        assert!(!state.push(generation, hit("x".repeat(MAX_SEARCH_BYTES))));
        assert!(state.limited);
        assert_eq!(state.hits.len(), 2);
    }
    #[test]
    #[ignore = "requires private felis daemon; run scripts/test-live.sh"]
    fn live_scrollback_search_does_not_evict_the_window_attachment() {
        let socket = std::env::var("NEKOMATA_TEST_SOCKET").expect("private test socket required");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let session = crate::session::Session::start(
            &runtime,
            ctx,
            Target::Local(socket.into()),
            None,
            None,
            vec![],
        );
        let wait = |predicate: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !predicate() {
                assert!(std::time::Instant::now() < deadline, "operation timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait(&|| session.state.lock().unwrap().connected);
        session.send(InputMsg::KeyBytes(
            b"printf 'NEKOMATA_SEARCH_MARKER\\n'\r".to_vec(),
        ));
        // A fence on the window input queue is not available from the Ops
        // connection; retry until the shell's persistent output is searchable.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            session
                .tools
                .search("NEKOMATA_SEARCH_MARKER".into(), SearchOptions::default());
            wait(&|| !session.tools.search.lock().unwrap().running);
            let search = session.tools.search.lock().unwrap();
            assert!(search.error.is_none(), "{:?}", search.error);
            if !search.hits.is_empty() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            drop(search);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(session.state.lock().unwrap().connected);
        assert!(!session.state.lock().unwrap().detached);
        session.tools.search(
            "(".into(),
            SearchOptions {
                regex: true,
                case_insensitive: false,
            },
        );
        wait(&|| !session.tools.search.lock().unwrap().running);
        assert!(session.tools.search.lock().unwrap().error.is_some());
        assert!(session.state.lock().unwrap().connected);
        session.tools.cancel_search();
        assert!(!session.tools.search.lock().unwrap().active);
        assert!(
            session
                .tools
                .search
                .lock()
                .unwrap()
                .overlay(&session.state.lock().unwrap().screen)
                .is_none()
        );
        session.send(InputMsg::KeyBytes(b"printf '\\033]133;A\\007prompt> \\033]133;B\\007fake-command\\n\\033]133;C\\007OUTPUT_ONLY\\n\\033]133;D;0\\007'\r".to_vec()));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            session.tools.command_output(false);
            wait(&|| !session.tools.output.lock().unwrap().running);
            let result = session.tools.take_output();
            if let Some(Ok(output)) = result {
                let text = String::from_utf8(output.data).unwrap();
                if text.contains("OUTPUT_ONLY") {
                    assert!(!text.contains("fake-command"), "{text}");
                    assert!(!text.contains('\u{1b}'), "{text:?}");
                    assert_eq!(
                        output.origin.session_id,
                        session.state.lock().unwrap().session_id.unwrap()
                    );
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "command output did not resolve"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(session.state.lock().unwrap().connected);
        assert!(!session.state.lock().unwrap().detached);
    }
}
