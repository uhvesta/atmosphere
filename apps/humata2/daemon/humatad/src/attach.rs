//! One Attach stream: a tab's output from the client's offset (a snapshot
//! first when it needs one), its state, and its input and resizes passed
//! on to the holder.
//!
//! An attach that names a `checkpoint_engine` gets the daemon's own screen of
//! the tab (D271): a checkpoint of it as the snapshot, then the output after
//! it, and the screen's size in every state, changed in order with the
//! output. An attach that names none (`humata attach`) gets a snapshot that
//! is the raw output the holder still keeps, at most its last
//! `SNAPSHOT_MAX` bytes: replaying it redraws the screen for line-based
//! output, but a full-screen program's screen may start mid-sequence.
//!
//! Snapshots and output are chunks (TR-11, D65): encoded with the codec
//! picked from the client's `accept_codecs`, and a snapshot over
//! `CHUNK_MAX` goes as several pieces to a client that lists any.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_chunk::{CHUNK_MAX, Chunk, ChunkError, Codec, Encoder};
use humata2_grpc::Timestamp;
use humata2_grpc::humata::v1 as pb;
use humata2_holder::{Control, Event, Events, HolderError, Welcome};
use humata2_vt_authority::Resized;
use humata2_store::{TabKind, TabRecord};
use pb::attach_client_message::Message as ClientMessage;
use pb::attach_server_message::Message as ServerMessage;
use pb::invalid_field::Problem;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Status, Streaming};

use crate::holders::{self, Holders, ScreenFailed};
use crate::status;
use crate::terminal::{WriterGuard, Writers};
use crate::terminal_uuid::terminal_uuid;

pub(crate) type AttachStream =
    Pin<Box<dyn Stream<Item = Result<pb::AttachServerMessage, Status>> + Send>>;

/// The most a snapshot carries.
const SNAPSHOT_MAX: usize = 2 << 20;

/// The most a snapshot sent whole carries: under gRPC's default 4 MiB
/// message, with room for its envelope. A bigger one goes in pieces.
const WHOLE_MAX: usize = 3 << 20;

/// Unacknowledged output a client is sent when it doesn't say.
const DEFAULT_WINDOW: u64 = 1 << 20;

/// Messages waiting for the client before the attach stops reading output.
const OUT_QUEUE: usize = 64;

/// The most output already waiting from the holder that is sent as one
/// message: a burst becomes a few big messages, not one per chunk the
/// holder read.
const COALESCE_MAX: usize = 64 << 10;

/// Starts the attach `open` asks for. Errors that come before the first
/// message are returned; later ones end the stream.
pub(crate) async fn start(
    tab: &TabRecord,
    open: &pb::AttachOpen,
    incoming: Streaming<pb::AttachClientMessage>,
    writers: &Arc<Writers>,
    holders: &Arc<Holders>,
    codec: Codec,
) -> Result<AttachStream, Status> {
    let checkpoint = !open.checkpoint_engine.is_empty();
    if checkpoint && open.checkpoint_engine != ghostty_vt::CHECKPOINT_FORMAT {
        return Err(engine_unsupported(&open.checkpoint_engine));
    }
    let screen = holders
        .link(&tab.id, &tab.holder_socket)
        .await
        .map_err(|error| holder_status(tab, &error))?;
    let from = open.from_offset;
    // A checkpoint attach streams from its checkpoint's offset, so what this
    // connection is for is checking `from` and setting the size.
    let (mut welcome, mut events, mut control) =
        connect(tab, if checkpoint { u64::MAX } else { from }).await?;
    if from > welcome.end {
        return Err(status::invalid_field(
            "open.from_offset",
            Problem::OutOfRange,
            format!(
                "offset {from} is past the end of tab {}'s output, which is at {}",
                tab.id, welcome.end
            ),
            "attach from an offset this tab gave you, or from 0 for a snapshot",
        ));
    }
    let read_only = open.read_only;
    if !read_only && open.cols > 0 && open.rows > 0 {
        resize_tab(
            tab,
            &screen,
            &mut control,
            Size::new(open.cols, open.rows, 0, 0),
        )
        .await?;
    }

    let (out, queued) = mpsc::channel(OUT_QUEUE);
    let pieces = !open.accept_codecs.is_empty();
    let mut first = Vec::new();
    let mut view = None;
    // A resume from an offset replays up to the end the holder had, then
    // says how many lines that was (SE-16).
    let catch_up = (!checkpoint && from != 0 && from >= welcome.earliest).then_some(CatchUp {
        end: welcome.end,
        lines: 0,
    });
    let (cursor, pending) = if checkpoint {
        let taken = screen
            .checkpoint()
            .await
            .map_err(|error| screen_status(tab, &error))?;
        let (snapshot, seen) = checkpoint_parts(tab, taken);
        // The output after the checkpoint comes from the holder, in order.
        (welcome, events, control) = connect(tab, snapshot.offset).await?;
        let cursor = snapshot.offset;
        view = Some(seen);
        first.extend(snapshot_messages(snapshot, codec, pieces).await?);
        (cursor, Vec::new())
    } else if from == 0 || from < welcome.earliest {
        let (snapshot, after) = snapshot(tab, &welcome, &mut events).await?;
        let cursor = snapshot.offset;
        first.extend(snapshot_messages(snapshot, codec, pieces).await?);
        (cursor, after)
    } else {
        (from, Vec::new())
    };

    let writer = (!read_only).then(|| writers.join(&tab.id));
    let mut attached = Attached {
        tab: tab.clone(),
        read_only,
        writer,
        writers: writers.watch(&tab.id),
        watching_writers: true,
        holders: holders.clone(),
        view,
        turns: crate::session_service::turn_changes(),
        moves: crate::session_service::moving_changes(),
        sent: (0, 0),
        link: Link::new(events, control),
        client: Client::new(incoming),
        cursor,
        acked: cursor,
        window: match open.window_bytes {
            0 => DEFAULT_WINDOW,
            bytes => u64::from(bytes),
        },
        exited: welcome.exited,
        pending: pending.into(),
        catch_up,
        out,
        encoder: Encoder::new(codec),
        pieces,
    };
    let state = attached.state();
    attached.sent = (state.status, state.writable);
    first.push(ServerMessage::State(state));
    tokio::spawn(async move {
        if let Err(Ended::Failed(status)) = attached.run(first).await {
            let _ = attached.out.send(Err(status)).await;
        }
    });
    Ok(Box::pin(ReceiverStream::new(queued)))
}

/// What a resume from an offset has still to replay, and how many lines it
/// has replayed so far.
struct CatchUp {
    end: u64,
    lines: u32,
}

/// Why an attach stopped.
enum Ended {
    /// The client went or hung up.
    Client,
    Failed(Status),
}

struct Attached {
    tab: TabRecord,
    read_only: bool,
    /// Held while this attach counts as a writer.
    writer: Option<WriterGuard>,
    writers: watch::Receiver<u32>,
    watching_writers: bool,
    /// Where the tab's link is found, whose screen every resize goes
    /// through. Asked each time: a link kept here would outlive one that was
    /// dropped and replaced, its screen still parsing and answering queries
    /// beside the new one's, and this attach's resizes would miss the new.
    holders: Arc<Holders>,
    /// What a checkpoint attach knows of the screen; none for a raw one.
    view: Option<View>,
    /// Counts every running agent's status change; the agent's tab shows its
    /// session's turn (D169).
    turns: watch::Receiver<u64>,
    /// Changes when a move starts or ends: the tab of a moving session is
    /// read-only (MV-4).
    moves: watch::Receiver<u64>,
    /// The status and writable the last State this attach sent carried.
    sent: (i32, i32),
    link: Link,
    client: Client,
    /// The offset after the last byte sent.
    cursor: u64,
    /// The offset the client has drawn up to.
    acked: u64,
    window: u64,
    exited: Option<i32>,
    /// Holder events read ahead while taking a snapshot, sent next.
    pending: VecDeque<Event>,
    /// Set until a resume from an offset has replayed what it missed.
    catch_up: Option<CatchUp>,
    out: mpsc::Sender<Result<pb::AttachServerMessage, Status>>,
    /// Encodes output: at most `COALESCE_MAX` at a time, so on the runtime.
    encoder: Encoder,
    /// Whether the client takes snapshots in pieces.
    pieces: bool,
}

impl Attached {
    async fn run(&mut self, first: Vec<ServerMessage>) -> Result<(), Ended> {
        for message in first {
            self.send(message).await?;
        }
        self.caught_up().await?;
        loop {
            while let Some(event) = self.pending.pop_front() {
                self.event(event).await?;
            }
            let room = self.cursor.saturating_sub(self.acked) < self.window;
            tokio::select! {
                () = self.out.closed() => return Err(Ended::Client),
                event = self.link.events.recv(), if room => match event {
                    Some(Ok(event)) => {
                        let in_flight = self.cursor.saturating_sub(self.acked);
                        let room = usize::try_from(self.window.saturating_sub(in_flight))
                            .unwrap_or(usize::MAX)
                            .min(COALESCE_MAX);
                        let (event, held) =
                            coalesce(event, room, || self.link.events.try_recv().ok());
                        self.event(event).await?;
                        match held {
                            Some(Ok(event)) => self.pending.push_back(event),
                            Some(Err(error)) => {
                                return Err(Ended::Failed(holder_status(&self.tab, &error)));
                            }
                            None => {}
                        }
                    }
                    Some(Err(error)) => return Err(Ended::Failed(holder_status(&self.tab, &error))),
                    None => return Err(Ended::Failed(crate::terminal::no_tab(&self.tab.id))),
                },
                message = self.client.messages.recv() => match message {
                    Some(Ok(message)) => self.message(message).await?,
                    Some(Err(_)) | None => return Err(Ended::Client),
                },
                changed = self.writers.changed(), if self.watching_writers => match changed {
                    Ok(()) => self.send_state().await?,
                    Err(_) => self.watching_writers = false,
                },
                change = next_change(&mut self.view), if self.view.is_some() => {
                    self.size_changed(change).await?;
                }
                Ok(()) = self.turns.changed() => self.send_changed_state().await?,
                Ok(()) = self.moves.changed() => self.send_changed_state().await?,
            }
        }
    }

    async fn event(&mut self, event: Event) -> Result<(), Ended> {
        match event {
            Event::Output { offset, data } => {
                let end = offset + data.len() as u64;
                if end <= self.cursor {
                    return Ok(());
                }
                let skip = usize::try_from(self.cursor.saturating_sub(offset)).unwrap_or(0);
                let data = data.get(skip..).unwrap_or_default();
                let mut at = offset.max(self.cursor);
                if let Some(catch_up) = &mut self.catch_up {
                    let replayed = usize::try_from(catch_up.end.saturating_sub(at))
                        .unwrap_or(usize::MAX)
                        .min(data.len());
                    let lines = data
                        .get(..replayed)
                        .unwrap_or_default()
                        .iter()
                        .filter(|&&byte| byte == b'\n')
                        .count();
                    catch_up.lines = catch_up
                        .lines
                        .saturating_add(u32::try_from(lines).unwrap_or(u32::MAX));
                }
                for raw in data.chunks(CHUNK_MAX) {
                    let chunk = self.encoder.encode(raw.to_vec()).map_err(|error| Ended::Failed(encode_failed(&error)))?;
                    self.send(ServerMessage::Output(pb::TerminalOutput {
                        offset: at,
                        codec: chunk.header.codec.wire(),
                        raw_size: chunk.header.raw_len,
                        crc32: chunk.header.crc32,
                        data: chunk.data,
                    }))
                    .await?;
                    at += raw.len() as u64;
                }
                self.cursor = end;
                self.caught_up().await?;
                self.flush_sizes().await?;
            }
            // Output this client hadn't been sent is gone: start it over
            // from a snapshot rather than leave a gap.
            Event::Trimmed { .. } => self.resync().await?,
            Event::Exited { code } => {
                self.exited = Some(code);
                self.send_state().await?;
                self.send(ServerMessage::Notice(pb::TerminalNotice {
                    kind: pb::terminal_notice::Kind::Exited as i32,
                    text: format!("exited with code {code}"),
                    at_offset: self.cursor,
                    caught_up_lines: 0,
                    at: Some(now()),
                }))
                .await?;
            }
            Event::InputWritten { seq } => {
                self.send(ServerMessage::InputAccepted(pb::InputAccepted { input_seq: seq }))
                    .await?;
            }
            Event::Refused { message } => {
                crate::log::say!("humatad2: tab {}'s holder refused a request: {message}", self.tab.id);
            }
        }
        Ok(())
    }

    async fn message(&mut self, message: pb::AttachClientMessage) -> Result<(), Ended> {
        match message.message {
            Some(ClientMessage::Input(input)) => {
                if self.read_only {
                    return Err(Ended::Failed(status::invalid_field(
                        "input",
                        Problem::NotAllowedHere,
                        "this attach is read-only, so it can't type",
                        "attach again without read_only to type into the tab",
                    )));
                }
                // Typing is disabled while the session moves; nothing is
                // kept for later, as with a machine offline (MV-4, SE-16).
                if self.moving() {
                    return Ok(());
                }
                crate::read_marks::tab_typed(&self.tab);
                self.link
                    .control
                    .input(input.input_seq, &input.data)
                    .await
                    .map_err(|error| Ended::Failed(holder_status(&self.tab, &error)))?;
            }
            Some(ClientMessage::Resize(resize)) => {
                if self.read_only || resize.cols == 0 || resize.rows == 0 {
                    return Ok(());
                }
                // D14: every attach that can type may resize.
                let screen = self.screen().await?;
                resize_tab(
                    &self.tab,
                    &screen,
                    &mut self.link.control,
                    Size::new(resize.cols, resize.rows, resize.cell_width, resize.cell_height),
                )
                .await
                .map_err(Ended::Failed)?;
            }
            Some(ClientMessage::Ack(ack)) => {
                self.acked = self.acked.max(ack.offset.min(self.cursor));
            }
            Some(ClientMessage::Open(_)) => {
                return Err(Ended::Failed(status::invalid_field(
                    "open",
                    Problem::NotAllowedHere,
                    "this attach is already open",
                    "send Open once, as the first message; start a new Attach for another tab",
                )));
            }
            None => {}
        }
        Ok(())
    }

    /// Applies a size change the daemon's screen made, once the output
    /// before it has been sent. A subscriber that missed some starts over
    /// from a new checkpoint, so it never draws a size it wasn't told.
    async fn size_changed(&mut self, change: Result<Resized, RecvError>) -> Result<(), Ended> {
        match change {
            Ok(resized) => {
                if let Some(view) = &mut self.view {
                    view.waiting.push_back(resized);
                }
                self.flush_sizes().await
            }
            // The screen it watched was dropped (its link was replaced or
            // the tab closed): start over from the one the tab has now. A
            // tab that is gone fails there.
            Err(RecvError::Lagged(_) | RecvError::Closed) => self.resync().await,
        }
    }

    /// The tab's link as it is now, connecting a new one if the last was
    /// dropped.
    async fn screen(&self) -> Result<Arc<holders::Link>, Ended> {
        self.holders
            .link(&self.tab.id, &self.tab.holder_socket)
            .await
            .map_err(|error| Ended::Failed(holder_status(&self.tab, &error)))
    }

    /// Sends the state for each waiting size change the output has reached.
    async fn flush_sizes(&mut self) -> Result<(), Ended> {
        let mut changed = false;
        if let Some(view) = &mut self.view {
            while let Some(next) = view.waiting.front().copied()
                && next.at <= self.cursor
            {
                view.waiting.pop_front();
                view.size = Size::new(
                    u32::from(next.cols),
                    u32::from(next.rows),
                    next.cell_width,
                    next.cell_height,
                );
                changed = true;
            }
        }
        if changed {
            self.send_state().await
        } else {
            Ok(())
        }
    }

    /// Says how many lines a resume from an offset replayed, once it has
    /// replayed them all.
    async fn caught_up(&mut self) -> Result<(), Ended> {
        let cursor = self.cursor;
        let Some(catch_up) = self.catch_up.take_if(|catch_up| catch_up.end <= cursor) else {
            return Ok(());
        };
        self.send(ServerMessage::Notice(pb::TerminalNotice {
            kind: pb::terminal_notice::Kind::CaughtUp as i32,
            text: format!("caught up {} lines of output", catch_up.lines),
            at_offset: catch_up.end,
            caught_up_lines: catch_up.lines,
            at: Some(now()),
        }))
        .await
    }

    /// Starts a checkpoint attach over from a new checkpoint.
    async fn resync_checkpoint(&mut self) -> Result<(), Ended> {
        let taken = self
            .screen()
            .await?
            .checkpoint()
            .await
            .map_err(|error| Ended::Failed(screen_status(&self.tab, &error)))?;
        let (snapshot, view) = checkpoint_parts(&self.tab, taken);
        let (welcome, events, control) = connect(&self.tab, snapshot.offset)
            .await
            .map_err(Ended::Failed)?;
        self.link = Link::new(events, control);
        self.cursor = snapshot.offset;
        self.acked = self.cursor;
        self.exited = welcome.exited;
        self.pending.clear();
        self.view = Some(view);
        for message in snapshot_messages(snapshot, self.encoder.codec(), self.pieces)
            .await
            .map_err(Ended::Failed)?
        {
            self.send(message).await?;
        }
        self.send_state().await
    }

    /// Reconnects from the start and sends a fresh snapshot.
    async fn resync(&mut self) -> Result<(), Ended> {
        // A fresh screen replays nothing; the snapshot says what is there.
        self.catch_up = None;
        if self.view.is_some() {
            return self.resync_checkpoint().await;
        }
        let (welcome, mut events, control) =
            connect(&self.tab, 0).await.map_err(Ended::Failed)?;
        let (snapshot, pending) = snapshot(&self.tab, &welcome, &mut events)
            .await
            .map_err(Ended::Failed)?;
        self.link = Link::new(events, control);
        self.cursor = snapshot.offset;
        self.acked = self.cursor;
        self.exited = welcome.exited;
        for message in snapshot_messages(snapshot, self.encoder.codec(), self.pieces)
            .await
            .map_err(Ended::Failed)?
        {
            self.send(message).await?;
        }
        self.pending = pending.into();
        self.send_state().await?;
        Ok(())
    }

    /// Sends the tab's state, and remembers the status it showed.
    async fn send_state(&mut self) -> Result<(), Ended> {
        let state = self.state();
        self.sent = (state.status, state.writable);
        self.send(ServerMessage::State(state)).await
    }

    /// Sends the tab's state if it isn't what the last one said.
    async fn send_changed_state(&mut self) -> Result<(), Ended> {
        let state = self.state();
        if (state.status, state.writable) == self.sent {
            return Ok(());
        }
        self.send_state().await
    }

    /// Whether this tab's session is being moved: it takes no input until
    /// the agent resumes on the new machine (MV-4).
    fn moving(&self) -> bool {
        self.tab.session.as_deref().is_some_and(crate::session_service::is_moving)
    }

    fn state(&self) -> pb::TerminalState {
        let writers = *self.writers.borrow();
        let others = if self.writer.is_some() {
            writers.saturating_sub(1)
        } else {
            writers
        };
        let moving = self.moving();
        let writable = match (self.exited, self.read_only) {
            _ if moving => pb::terminal_state::Writable::Moving,
            (Some(_), _) => pb::terminal_state::Writable::Exited,
            (None, true) => pb::terminal_state::Writable::ReadOnlyAttach,
            (None, false) => pb::terminal_state::Writable::Yes,
        };
        let status = match (self.exited, &self.tab.session) {
            _ if moving => pb::SessionStatus::Moving,
            (Some(_), _) => pb::SessionStatus::Exited,
            // The agent's tab shows the turn its session shows (D169; none
            // for an agent with no turns); a shell has none.
            (None, Some(session)) if matches!(self.tab.kind, TabKind::Agent) => {
                crate::session_service::running_turn(session, self.tab.holder_pid)
            }
            (None, _) => pb::SessionStatus::RunningUnknownTurn,
        };
        let size = self.view.as_ref().map(|view| view.size).unwrap_or_default();
        pb::TerminalState {
            writable: writable as i32,
            status: status as i32,
            exit_code: self.exited.unwrap_or(0),
            other_writers: others,
            cols: size.cols,
            rows: size.rows,
            cell_width: size.cell_width,
            cell_height: size.cell_height,
            ..Default::default()
        }
    }

    async fn send(&self, message: ServerMessage) -> Result<(), Ended> {
        self.out
            .send(Ok(pb::AttachServerMessage {
                message: Some(message),
            }))
            .await
            .map_err(|_| Ended::Client)
    }
}

/// A screen size in cells, and a cell's in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Size {
    cols: u32,
    rows: u32,
    cell_width: u32,
    cell_height: u32,
}

impl Size {
    fn new(cols: u32, rows: u32, cell_width: u32, cell_height: u32) -> Self {
        Self {
            cols,
            rows,
            cell_width,
            cell_height,
        }
    }
}

/// What a checkpoint attach has of the daemon's screen.
struct View {
    /// The size the last state said.
    size: Size,
    changes: broadcast::Receiver<Resized>,
    /// Size changes the output hasn't reached yet, oldest first.
    waiting: VecDeque<Resized>,
}

/// The next size change of the screen a checkpoint attach watches; never
/// ready for an attach with none.
async fn next_change(view: &mut Option<View>) -> Result<Resized, RecvError> {
    match view {
        Some(view) => view.changes.recv().await,
        None => std::future::pending().await,
    }
}

/// The snapshot a checkpoint is sent as, and what the attach keeps of the
/// screen after it.
fn checkpoint_parts(
    tab: &TabRecord,
    taken: holders::Checkpoint,
) -> (pb::TerminalSnapshot, View) {
    let holders::Checkpoint {
        snapshot,
        cell,
        changes,
    } = taken;
    let size = Size::new(
        u32::from(snapshot.cols),
        u32::from(snapshot.rows),
        cell.0,
        cell.1,
    );
    let message = pb::TerminalSnapshot {
        offset: snapshot.offset,
        vt: snapshot.bytes,
        cols: size.cols,
        rows: size.rows,
        engine: snapshot.engine.to_owned(),
        terminal_uuid: terminal_uuid(&tab.id, tab.holder_generation),
        ..Default::default()
    };
    let view = View {
        size,
        changes,
        waiting: VecDeque::new(),
    };
    (message, view)
}

/// Resizes the daemon's screen of the tab, then the tab's holder, so the
/// output the new size brings is parsed at it. Every attach of the tab
/// hears of it through the screen. A size the screen refuses leaves both
/// as they were.
async fn resize_tab(
    tab: &TabRecord,
    screen: &holders::Link,
    control: &mut Control,
    size: Size,
) -> Result<(), Status> {
    let (cols, rows) = (clamp(size.cols), clamp(size.rows));
    // The program repaints at its new size; that is not new output (D393).
    crate::read_marks::tab_resized(tab);
    match screen.resize(cols, rows, size.cell_width, size.cell_height) {
        Ok(()) => {}
        Err(ScreenFailed::Unavailable) => {}
        Err(error) => {
            crate::log::say!(
                "humatad2: tab {} wasn't resized to {cols}x{rows}: {}",
                tab.id,
                status::chain(&error)
            );
            return Ok(());
        }
    }
    control
        .resize(cols, rows)
        .await
        .map_err(|error| holder_status(tab, &error))
}

fn screen_status(tab: &TabRecord, error: &ScreenFailed) -> Status {
    status::internal(
        format!("tab {}'s screen failed: {}", tab.id, status::chain(error)),
        "",
    )
}

/// The app asked for a checkpoint format this daemon doesn't make: the two
/// ends are on different versions, which a machine upgrade ends.
fn engine_unsupported(asked: &str) -> Status {
    let ours = ghostty_vt::CHECKPOINT_FORMAT;
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!(
                "the app draws terminals with {asked} but this machine's humatad2 makes {ours}: they are on different terminal engine versions, and the machine is being upgraded to match"
            ),
            "wait for the upgrade to finish and open the tab again; if it never starts, update Humata on this machine and in the app to the same version",
            pb::error::Kind::PeerVersionUnsupported(pb::PeerVersionUnsupported {
                peer_version: asked.to_owned(),
                supported_min: ours.to_owned(),
                supported_max: ours.to_owned(),
                ..Default::default()
            }),
        ),
    )
}

/// A connection to the tab's holder, its events read by a task of their
/// own because reading them can't be cancelled midway.
struct Link {
    events: mpsc::Receiver<Result<Event, HolderError>>,
    control: Control,
    reader: JoinHandle<()>,
}

impl Link {
    fn new(mut events: Events, control: Control) -> Self {
        let (sender, receiver) = mpsc::channel(OUT_QUEUE);
        let reader = tokio::spawn(async move {
            loop {
                let next = events.next().await.transpose();
                let failed = matches!(next, Some(Err(_)));
                let Some(next) = next else { return };
                if sender.send(next).await.is_err() || failed {
                    return;
                }
            }
        });
        Self {
            events: receiver,
            control,
            reader,
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// The client's messages after Open, read by a task of their own.
struct Client {
    messages: mpsc::Receiver<Result<pb::AttachClientMessage, Status>>,
    reader: JoinHandle<()>,
}

impl Client {
    fn new(mut incoming: Streaming<pb::AttachClientMessage>) -> Self {
        let (sender, messages) = mpsc::channel(16);
        let reader = tokio::spawn(async move {
            loop {
                let next = incoming.message().await.transpose();
                let failed = matches!(next, Some(Err(_)));
                let Some(next) = next else { return };
                if sender.send(next).await.is_err() || failed {
                    return;
                }
            }
        });
        Self { messages, reader }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// Joins to `first` the output that continues it among what `next` has
/// ready, up to `limit` bytes in all (but never less than `first`). Returns
/// the joined event and the first one that couldn't join, if any was taken.
fn coalesce<E>(
    first: Event,
    limit: usize,
    mut next: impl FnMut() -> Option<Result<Event, E>>,
) -> (Event, Option<Result<Event, E>>) {
    let Event::Output { offset, mut data } = first else {
        return (first, None);
    };
    while data.len() < limit {
        match next() {
            Some(Ok(Event::Output {
                offset: more_at,
                data: more,
            })) if more_at == offset + data.len() as u64 && data.len() + more.len() <= limit => {
                data.extend_from_slice(&more);
            }
            None => break,
            held => return (Event::Output { offset, data }, held),
        }
    }
    (Event::Output { offset, data }, None)
}

async fn connect(tab: &TabRecord, from: u64) -> Result<(Welcome, Events, Control), Status> {
    let attachment = humata2_holder::attach(&tab.holder_socket, from)
        .await
        .map_err(|error| holder_status(tab, &error))?;
    Ok((attachment.welcome, attachment.events, attachment.control))
}

/// Reads the output the holder keeps, which it sends first, and returns it
/// as a snapshot with whatever came after it.
async fn snapshot(
    tab: &TabRecord,
    welcome: &Welcome,
    events: &mut Events,
) -> Result<(pb::TerminalSnapshot, Vec<Event>), Status> {
    let mut kept = Vec::new();
    let mut after = Vec::new();
    let mut at = welcome.earliest;
    while at < welcome.end {
        let event = events
            .next()
            .await
            .map_err(|error| holder_status(tab, &error))?
            .ok_or_else(|| crate::terminal::no_tab(&tab.id))?;
        match event {
            Event::Output { offset, data } => {
                let end = offset + data.len() as u64;
                let split = usize::try_from(welcome.end.saturating_sub(offset))
                    .unwrap_or(usize::MAX)
                    .min(data.len());
                let (inside, beyond) = data.split_at(split);
                kept.extend_from_slice(inside);
                if !beyond.is_empty() {
                    after.push(Event::Output {
                        offset: welcome.end,
                        data: beyond.to_vec(),
                    });
                }
                at = end;
            }
            // Said before the kept output when the attach asked for older.
            Event::Trimmed { resume_at } => at = at.max(resume_at),
            other => after.push(other),
        }
    }
    let tail = kept.len().saturating_sub(SNAPSHOT_MAX);
    kept.drain(..tail);
    Ok((
        pb::TerminalSnapshot {
            offset: welcome.end,
            vt: kept,
            cols: u32::from(welcome.cols),
            rows: u32::from(welcome.rows),
            ..Default::default()
        },
        after,
    ))
}

/// The messages that carry `snapshot`: as it is to a client that lists no
/// codecs, else in pieces of at most `CHUNK_MAX`, each encoded on a
/// blocking thread.
async fn snapshot_messages(
    snapshot: pb::TerminalSnapshot,
    codec: Codec,
    pieces: bool,
) -> Result<Vec<ServerMessage>, Status> {
    if !pieces {
        if snapshot.vt.len() > WHOLE_MAX {
            return Err(status::invalid_field(
                "open.accept_codecs",
                Problem::Missing,
                format!(
                    "the tab's screen is {} MiB, more than one message carries, and this attach lists no codecs, so it can't take it in pieces",
                    snapshot.vt.len().div_ceil(1 << 20)
                ),
                "list a codec in accept_codecs (CHUNK_CODEC_NONE is enough) and attach again",
            ));
        }
        return Ok(vec![ServerMessage::Snapshot(snapshot)]);
    }
    let chunks = tokio::task::spawn_blocking(move || {
        let mut encoder = Encoder::new(codec);
        let mut chunks = Vec::new();
        if snapshot.vt.is_empty() {
            chunks.push(encoder.encode(Vec::new())?);
        }
        for raw in snapshot.vt.chunks(CHUNK_MAX) {
            chunks.push(encoder.encode(raw.to_vec())?);
        }
        Ok::<_, ChunkError>((snapshot, chunks))
    })
    .await
    .map_err(|error| status::internal(format!("a snapshot encoder stopped: {error}"), ""))?;
    let (snapshot, chunks) = chunks.map_err(|error| encode_failed(&error))?;
    let last = chunks.len().saturating_sub(1);
    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(index, Chunk { header, data })| {
            ServerMessage::Snapshot(pb::TerminalSnapshot {
                offset: snapshot.offset,
                vt: data,
                cols: snapshot.cols,
                rows: snapshot.rows,
                engine: snapshot.engine.clone(),
                terminal_uuid: snapshot.terminal_uuid.clone(),
                codec: header.codec.wire(),
                raw_size: header.raw_len,
                crc32: header.crc32,
                more: index < last,
                ..Default::default()
            })
        })
        .collect())
}

/// Encoding fails only when the codec can't start (zstd out of memory).
fn encode_failed(error: &ChunkError) -> Status {
    status::internal(
        format!("humatad2 couldn't compress a terminal's output: {}", status::chain(error)),
        "",
    )
}

fn holder_status(tab: &TabRecord, error: &HolderError) -> Status {
    match error {
        HolderError::Connect { .. } | HolderError::Lost { .. } => status::not_found(
            pb::EntityKind::Tab,
            &tab.id,
            format!("tab {}'s shell isn't running any more: its holder is gone", tab.id),
            "close the tab and open a new one; what it showed can't be recovered",
        ),
        other => status::internal(
            format!("tab {}'s holder failed: {}", tab.id, status::chain(other)),
            "",
        ),
    }
}

fn clamp(value: u32) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

pub(crate) fn now() -> Timestamp {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp {
        seconds: i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(elapsed.subsec_nanos()).unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(offset: u64, data: &[u8]) -> Event {
        Event::Output {
            offset,
            data: data.to_vec(),
        }
    }

    fn ready(events: Vec<Event>) -> impl FnMut() -> Option<Result<Event, ()>> {
        let mut events = events.into_iter();
        move || events.next().map(Ok)
    }

    #[test]
    fn output_waiting_behind_output_goes_as_one_message() {
        let (joined, held) = coalesce(
            output(10, b"ab"),
            100,
            ready(vec![output(12, b"cd"), output(14, b"ef")]),
        );

        assert_eq!(joined, output(10, b"abcdef"));
        assert!(held.is_none());
    }

    #[test]
    fn what_cannot_join_is_handed_back_in_order() {
        let mut ready = ready(vec![
            output(12, b"cd"),
            Event::Exited { code: 0 },
            output(14, b"ef"),
        ]);
        let (joined, held) = coalesce(output(10, b"ab"), 100, &mut ready);

        assert_eq!(joined, output(10, b"abcd"));
        assert_eq!(held, Some(Ok(Event::Exited { code: 0 })));
        assert_eq!(ready(), Some(Ok(output(14, b"ef"))));
    }

    #[test]
    fn output_after_a_gap_is_not_joined() {
        let (joined, held) = coalesce(output(10, b"ab"), 100, ready(vec![output(20, b"cd")]));

        assert_eq!(joined, output(10, b"ab"));
        assert_eq!(held, Some(Ok(output(20, b"cd"))));
    }

    #[test]
    fn joining_stops_at_the_limit() {
        let (joined, held) = coalesce(
            output(0, b"abc"),
            5,
            ready(vec![output(3, b"de"), output(5, b"f")]),
        );

        assert_eq!(joined, output(0, b"abcde"));
        assert!(held.is_none());
    }

    #[test]
    fn a_chunk_bigger_than_the_room_left_is_held() {
        let (joined, held) = coalesce(output(0, b"abc"), 4, ready(vec![output(3, b"de")]));

        assert_eq!(joined, output(0, b"abc"));
        assert_eq!(held, Some(Ok(output(3, b"de"))));
    }

    #[test]
    fn anything_but_output_is_passed_through() {
        let mut taken = false;
        let (event, held) = coalesce(Event::Exited { code: 1 }, 100, || {
            taken = true;
            None::<Result<Event, ()>>
        });

        assert_eq!(event, Event::Exited { code: 1 });
        assert!(held.is_none());
        assert!(!taken);
    }
}
