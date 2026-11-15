//! One Attach stream: a tab's output from the client's offset (a snapshot
//! first when it needs one), its state, and its input and resizes passed
//! on to the holder.
//!
//! Until the VT engine is wired in, a snapshot is the raw output the holder
//! still keeps, at most its last `SNAPSHOT_MAX` bytes: replaying it redraws
//! the screen for line-based output, but a full-screen program's screen
//! may start mid-sequence.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_grpc::Timestamp;
use humata2_grpc::humata::v1 as pb;
use humata2_holder::{Control, Event, Events, HolderError, Welcome};
use humata2_store::TabRecord;
use pb::attach_client_message::Message as ClientMessage;
use pb::attach_server_message::Message as ServerMessage;
use pb::invalid_field::Problem;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Status, Streaming};

use crate::status;
use crate::terminal::{WriterGuard, Writers};

pub(crate) type AttachStream =
    Pin<Box<dyn Stream<Item = Result<pb::AttachServerMessage, Status>> + Send>>;

/// The most a snapshot carries.
const SNAPSHOT_MAX: usize = 2 << 20;

/// Unacknowledged output a client is sent when it doesn't say.
const DEFAULT_WINDOW: u64 = 1 << 20;

/// Messages waiting for the client before the attach stops reading output.
const OUT_QUEUE: usize = 64;

/// Starts the attach `open` asks for. Errors that come before the first
/// message are returned; later ones end the stream.
pub(crate) async fn start(
    tab: &TabRecord,
    open: &pb::AttachOpen,
    incoming: Streaming<pb::AttachClientMessage>,
    writers: &Arc<Writers>,
) -> Result<AttachStream, Status> {
    let from = open.from_offset;
    let (welcome, mut events, mut control) = connect(tab, from).await?;
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
        control
            .resize(clamp(open.cols), clamp(open.rows))
            .await
            .map_err(|error| holder_status(tab, &error))?;
    }

    let (out, queued) = mpsc::channel(OUT_QUEUE);
    let mut first = Vec::new();
    let (cursor, pending) = if from == 0 || from < welcome.earliest {
        let (snapshot, after) = snapshot(tab, &welcome, &mut events).await?;
        let cursor = snapshot.offset;
        first.push(ServerMessage::Snapshot(snapshot));
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
        out,
    };
    first.push(ServerMessage::State(attached.state()));
    tokio::spawn(async move {
        if let Err(Ended::Failed(status)) = attached.run(first).await {
            let _ = attached.out.send(Err(status)).await;
        }
    });
    Ok(Box::pin(ReceiverStream::new(queued)))
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
    out: mpsc::Sender<Result<pb::AttachServerMessage, Status>>,
}

impl Attached {
    async fn run(&mut self, first: Vec<ServerMessage>) -> Result<(), Ended> {
        for message in first {
            self.send(message).await?;
        }
        loop {
            while let Some(event) = self.pending.pop_front() {
                self.event(event).await?;
            }
            let room = self.cursor.saturating_sub(self.acked) < self.window;
            tokio::select! {
                () = self.out.closed() => return Err(Ended::Client),
                event = self.link.events.recv(), if room => match event {
                    Some(Ok(event)) => self.event(event).await?,
                    Some(Err(error)) => return Err(Ended::Failed(holder_status(&self.tab, &error))),
                    None => return Err(Ended::Failed(crate::terminal::no_tab(&self.tab.id))),
                },
                message = self.client.messages.recv() => match message {
                    Some(Ok(message)) => self.message(message).await?,
                    Some(Err(_)) | None => return Err(Ended::Client),
                },
                changed = self.writers.changed(), if self.watching_writers => match changed {
                    Ok(()) => self.send(ServerMessage::State(self.state())).await?,
                    Err(_) => self.watching_writers = false,
                },
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
                let data = data.get(skip..).map(<[u8]>::to_vec).unwrap_or_default();
                self.send(ServerMessage::Output(pb::TerminalOutput {
                    offset: offset.max(self.cursor),
                    data,
                }))
                .await?;
                self.cursor = end;
            }
            // Output this client hadn't been sent is gone: start it over
            // from a snapshot rather than leave a gap.
            Event::Trimmed { .. } => self.resync().await?,
            Event::Exited { code } => {
                self.exited = Some(code);
                self.send(ServerMessage::State(self.state())).await?;
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
                eprintln!("humatad2: tab {}'s holder refused a request: {message}", self.tab.id);
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
                self.link
                    .control
                    .resize(clamp(resize.cols), clamp(resize.rows))
                    .await
                    .map_err(|error| Ended::Failed(holder_status(&self.tab, &error)))?;
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

    /// Reconnects from the start and sends a fresh snapshot.
    async fn resync(&mut self) -> Result<(), Ended> {
        let (welcome, mut events, control) =
            connect(&self.tab, 0).await.map_err(Ended::Failed)?;
        let (snapshot, pending) = snapshot(&self.tab, &welcome, &mut events)
            .await
            .map_err(Ended::Failed)?;
        self.link = Link::new(events, control);
        self.cursor = snapshot.offset;
        self.acked = self.cursor;
        self.exited = welcome.exited;
        self.send(ServerMessage::Snapshot(snapshot)).await?;
        self.pending = pending.into();
        self.send(ServerMessage::State(self.state())).await?;
        Ok(())
    }

    fn state(&self) -> pb::TerminalState {
        let writers = *self.writers.borrow();
        let others = if self.writer.is_some() {
            writers.saturating_sub(1)
        } else {
            writers
        };
        let writable = match (self.exited, self.read_only) {
            (Some(_), _) => pb::terminal_state::Writable::Exited,
            (None, true) => pb::terminal_state::Writable::ReadOnlyAttach,
            (None, false) => pb::terminal_state::Writable::Yes,
        };
        let status = match self.exited {
            Some(_) => pb::SessionStatus::Exited,
            None => pb::SessionStatus::RunningUnknownTurn,
        };
        pb::TerminalState {
            writable: writable as i32,
            status: status as i32,
            exit_code: self.exited.unwrap_or(0),
            other_writers: others,
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
        },
        after,
    ))
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

fn now() -> Timestamp {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp {
        seconds: i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(elapsed.subsec_nanos()).unwrap_or(0),
    }
}
