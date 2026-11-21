//! The terminal of a session that lives on another machine (D139, D144,
//! SE-16, TR-4, HO-6): the host answers the app's and the CLI's
//! TerminalService calls by making the same call on that machine's socket
//! daemon, through the peer link's relay.
//!
//! **Access is the caller's job.** Nothing here checks who may do what. The
//! host's handler must run its normal access check for the session (the same
//! check the local path runs) before it branches here, because the machine
//! honours relayed calls as the user's (D144): a call that reaches this file
//! is made with the user's full power there.
//!
//! Every call is a fresh request built from the message alone; the caller's
//! metadata is never copied (see `host_relay`).
//!
//! Unary calls are forwarded as they are, and the machine's answer or error
//! comes back unchanged. With the link down they fail at once with the
//! machine-offline error; nothing waits for a link (TR-6).
//!
//! An Attach is a relay session per client attach. It opens Attach on the
//! machine with the client's `AttachOpen`, then splices the two streams.
//! Output chunks, snapshots, states and notices are forwarded untouched, so
//! nothing is decompressed or re-chunked (TR-11); input, resizes and acks
//! go the other way in order, one at a time, through queues of one message
//! each, so a slow end slows the other and nothing piles up (TR-8). Nothing
//! delays input (TR-10). Nothing sleeps or polls: the relay wakes only for a
//! stream event or a change in the link's state (TR-7).
//!
//! When the link drops, the client's stream stays open. The client is sent
//! a `TerminalState` with `WRITABLE_MACHINE_OFFLINE` and a `DISCONNECTED`
//! notice (SE-16), and its input is dropped, never kept for later ("Typing is
//! disabled"). The machine keeps the holder and the scrollback. When the
//! link returns with a new generation the relay opens Attach again from the
//! offset after the last output it delivered, with the latest size, and
//! goes on. Once the machine has replayed what the client missed it sends a
//! `CAUGHT_UP` notice with the lines that was, which the relay sends on as
//! the `RECONNECTED` notice; a fresh screen gets `RECONNECTED` with no count
//! just before it. The client gets exactly what it missed
//! while the machine still keeps it, or the machine's own fresh snapshot
//! when it doesn't (TR-4), passed through as it is.

use std::collections::VecDeque;
use std::future::{Future, pending};
use std::pin::Pin;
use std::sync::Arc;

use crate::pb;
use pb::attach_client_message::Message as ClientMessage;
use pb::attach_server_message::Message as ServerMessage;
use pb::terminal_service_client::TerminalServiceClient;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::SendError;
use tokio::sync::watch;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tonic::transport::Channel;
use tonic::{Code, Request, Response, Status, Streaming};

use crate::ports::{AttachStream, LinkState, RelayError, RelayLinks, TerminalSessions, now};

/// Messages waiting for the machine before the relay stops reading the
/// client. One keeps the input path a single hop with no stock of old
/// keystrokes (TR-8, TR-10).
const TO_MACHINE: usize = 1;

/// Messages waiting for the client before the relay stops reading the
/// machine.
const TO_CLIENT: usize = 16;

/// Why the relay could not reach an attach's machine.
#[derive(Debug, thiserror::Error)]
enum OpenError {
    #[error(
        "the first message of an attach to {machine} could not be queued, because the request was dropped; open the tab again"
    )]
    Queue {
        machine: String,
        #[source]
        source: SendError<pb::AttachClientMessage>,
    },
}

impl From<OpenError> for Status {
    fn from(error: OpenError) -> Self {
        Status::internal(error.to_string())
    }
}

/// The host's way to a machine's terminals. Cheap to clone.
#[derive(Clone)]
pub struct RemoteTerminal {
    links: Arc<dyn RelayLinks>,
    sessions: Arc<dyn TerminalSessions>,
}

impl RemoteTerminal {
    pub fn new(links: Arc<dyn RelayLinks>, sessions: Arc<dyn TerminalSessions>) -> Self {
        RemoteTerminal { links, sessions }
    }

    /// `ListTabs` on `machine`.
    pub async fn list_tabs(
        &self,
        machine: &str,
        message: pb::ListTabsRequest,
    ) -> Result<Response<pb::ListTabsResponse>, Status> {
        self.forward(machine, message, |mut client, request| async move {
            client.list_tabs(request).await
        })
        .await
    }

    /// `OpenShellTab` on `machine`.
    pub async fn open_shell_tab(
        &self,
        machine: &str,
        message: pb::OpenShellTabRequest,
    ) -> Result<Response<pb::Tab>, Status> {
        self.forward(machine, message, |mut client, request| async move {
            client.open_shell_tab(request).await
        })
        .await
    }

    /// `OpenAgentTab` on `machine`.
    pub async fn open_agent_tab(
        &self,
        machine: &str,
        message: pb::OpenAgentTabRequest,
    ) -> Result<Response<pb::OpenAgentTabResponse>, Status> {
        self.forward(machine, message, |mut client, request| async move {
            client.open_agent_tab(request).await
        })
        .await
    }

    /// `OpenMachineShell` on `machine`.
    pub async fn open_machine_shell(
        &self,
        machine: &str,
        message: pb::OpenMachineShellRequest,
    ) -> Result<Response<pb::Tab>, Status> {
        self.forward(machine, message, |mut client, request| async move {
            client.open_machine_shell(request).await
        })
        .await
    }

    /// `CloseTab` on `machine`.
    pub async fn close_tab(
        &self,
        machine: &str,
        message: pb::CloseTabRequest,
    ) -> Result<Response<pb::CloseTabResponse>, Status> {
        self.forward(machine, message, |mut client, request| async move {
            client.close_tab(request).await
        })
        .await
    }

    /// `Type` on `machine`.
    pub async fn type_text(
        &self,
        machine: &str,
        message: pb::TypeRequest,
    ) -> Result<Response<pb::TypeResponse>, Status> {
        self.forward(machine, message, |mut client, request| async move {
            client.r#type(request).await
        })
        .await
    }

    /// `CancelQueuedTyping` on `machine`.
    pub async fn cancel_queued_typing(
        &self,
        machine: &str,
        message: pb::CancelQueuedTypingRequest,
    ) -> Result<Response<pb::CancelQueuedTypingResponse>, Status> {
        self.forward(machine, message, |mut client, request| async move {
            client.cancel_queued_typing(request).await
        })
        .await
    }

    /// `ListQueued` on `machine`.
    pub async fn list_queued(
        &self,
        machine: &str,
        message: pb::ListQueuedRequest,
    ) -> Result<Response<pb::ListQueuedResponse>, Status> {
        self.forward(machine, message, |mut client, request| async move {
            client.list_queued(request).await
        })
        .await
    }

    /// `SendQueuedNow` on `machine`.
    pub async fn send_queued_now(
        &self,
        machine: &str,
        message: pb::SendQueuedNowRequest,
    ) -> Result<Response<pb::SendQueuedNowResponse>, Status> {
        self.forward(machine, message, |mut client, request| async move {
            client.send_queued_now(request).await
        })
        .await
    }

    /// `PasteImage` on `machine`.
    pub async fn paste_image(
        &self,
        machine: &str,
        message: pb::PasteImageRequest,
    ) -> Result<Response<pb::PasteImageResponse>, Status> {
        self.forward(machine, message, |mut client, request| async move {
            client.paste_image(request).await
        })
        .await
    }

    /// A fresh request for `message` on the machine's live link; the
    /// machine's answer or error comes back as it is.
    async fn forward<Req, Res, Call, Fut>(
        &self,
        machine: &str,
        message: Req,
        call: Call,
    ) -> Result<Response<Res>, Status>
    where
        Call: FnOnce(TerminalServiceClient<Channel>, Request<Req>) -> Fut,
        Fut: Future<Output = Result<Response<Res>, Status>>,
    {
        let channel = self.links.channel(machine)?;
        call(TerminalServiceClient::new(channel), Request::new(message)).await
    }

    /// Attaches the client to `open`'s tab on `machine`. `open` is the first
    /// message the handler already read from `incoming`, which carries the
    /// rest. Fails at once when the machine is offline or refuses the
    /// attach; later link drops keep the returned stream open (SE-16).
    pub async fn attach<S>(
        &self,
        machine: &str,
        open: pb::AttachOpen,
        incoming: S,
    ) -> Result<AttachStream, Status>
    where
        S: Stream<Item = Result<pb::AttachClientMessage, Status>> + Send + 'static,
    {
        let mut link = self.links.watch(machine);
        let first = open_machine_side(self.links.as_ref(), machine, &open).await?;
        // Changes before this point are already reflected in `first`.
        let current = *link.borrow_and_update();
        let relay = Relay {
            links: self.links.clone(),
            sessions: self.sessions.clone(),
            machine: machine.to_owned(),
            session: open.session.as_ref().map(|session| session.value.clone()),
            moves: self.sessions.moving_changes(),
            showed_moving: false,
            tracker: Tracker::new(open),
        };
        let (out, queued) = mpsc::channel(TO_CLIENT);
        let client: Pin<Box<dyn Stream<Item = Result<pb::AttachClientMessage, Status>> + Send>> =
            Box::pin(incoming);
        tokio::spawn(relay.run(first, current, link, client, out));
        Ok(Box::pin(ReceiverStream::new(queued)))
    }
}

/// What the relay remembers to open the machine's attach again.
struct Tracker {
    open: pb::AttachOpen,
    /// The offset after the last output or snapshot handed to the client.
    delivered: Option<u64>,
    /// The latest size the client asked for.
    size: Option<(u32, u32)>,
    /// The machine's last state, shown as offline while the link is down.
    state: Option<pb::TerminalState>,
}

impl Tracker {
    fn new(open: pb::AttachOpen) -> Self {
        let size = (open.cols > 0 && open.rows > 0).then_some((open.cols, open.rows));
        Tracker {
            open,
            delivered: None,
            size,
            state: None,
        }
    }

    /// Notes what a message from the machine moves the client to.
    fn delivered(&mut self, message: &pb::AttachServerMessage) {
        match &message.message {
            // The offset after a snapshot counts once its last piece is out.
            Some(ServerMessage::Snapshot(snapshot)) if !snapshot.more => {
                self.delivered = Some(snapshot.offset);
            }
            Some(ServerMessage::Output(output)) => {
                self.delivered = Some(output.offset.saturating_add(decoded_len(output)));
            }
            Some(ServerMessage::State(state)) => self.state = Some(state.clone()),
            _ => {}
        }
    }

    /// Notes what the client asked for.
    fn asked(&mut self, message: &pb::AttachClientMessage) {
        if let Some(ClientMessage::Resize(resize)) = &message.message {
            if resize.cols > 0 && resize.rows > 0 {
                self.size = Some((resize.cols, resize.rows));
            }
        }
    }

    /// The open that continues where the client is.
    fn reopen(&self) -> pb::AttachOpen {
        let mut open = self.open.clone();
        if let Some(offset) = self.delivered {
            open.from_offset = offset;
        }
        if let Some((cols, rows)) = self.size {
            open.cols = cols;
            open.rows = rows;
        }
        open
    }
}

/// The decoded length of an output chunk: offsets count decoded bytes.
fn decoded_len(output: &pb::TerminalOutput) -> u64 {
    if output.raw_size == 0 && output.codec == pb::ChunkCodec::None as i32 {
        output.data.len() as u64
    } else {
        u64::from(output.raw_size)
    }
}

/// The machine's end of one attach.
struct Upstream {
    to: mpsc::Sender<pb::AttachClientMessage>,
    from: Streaming<pb::AttachServerMessage>,
    generation: u64,
}

/// Opens Attach on `machine` with `open`. A resume from an offset the
/// machine can't use (out of range) starts again from a fresh screen.
async fn open_machine_side(
    links: &dyn RelayLinks,
    machine: &str,
    open: &pb::AttachOpen,
) -> Result<Upstream, Status> {
    match open_once(links, machine, open).await {
        Err(status)
            if open.from_offset > 0
                && matches!(status.code(), Code::OutOfRange | Code::InvalidArgument) =>
        {
            let mut fresh = open.clone();
            fresh.from_offset = 0;
            open_once(links, machine, &fresh).await
        }
        other => other,
    }
}

async fn open_once(
    links: &dyn RelayLinks,
    machine: &str,
    open: &pb::AttachOpen,
) -> Result<Upstream, Status> {
    let channel = links.channel(machine)?;
    let generation = match links.state(machine) {
        LinkState::Up(generation) => generation,
        LinkState::Down => 0,
    };
    let mut client = TerminalServiceClient::new(channel);
    let (to, requests) = mpsc::channel(TO_MACHINE);
    to.send(pb::AttachClientMessage {
        message: Some(ClientMessage::Open(open.clone())),
    })
    .await
    .map_err(|source| OpenError::Queue {
        machine: machine.to_owned(),
        source,
    })?;
    let from = client
        .attach(Request::new(ReceiverStream::new(requests)))
        .await?
        .into_inner();
    Ok(Upstream {
        to,
        from,
        generation,
    })
}

struct Relay {
    links: Arc<dyn RelayLinks>,
    sessions: Arc<dyn TerminalSessions>,
    machine: String,
    /// The session whose terminal this is. While this host moves it, the
    /// terminal is shown read-only and typing is dropped (MV-4).
    session: Option<String>,
    moves: watch::Receiver<u64>,
    /// Whether the last state the client was sent said Moving.
    showed_moving: bool,
    tracker: Tracker,
}

/// What the machine's stream says when it fails: the link, not the tab.
fn link_failure(status: &Status) -> bool {
    matches!(
        status.code(),
        Code::Unavailable | Code::Cancelled | Code::Unknown | Code::Aborted
    )
}

impl Relay {
    fn moving(&self) -> bool {
        self.session
            .as_deref()
            .is_some_and(|id| self.sessions.is_moving(id))
    }

    /// Shows `state` as Moving while this host moves the session (MV-4).
    fn show_moving(&mut self, state: &mut pb::TerminalState) {
        self.showed_moving = self.moving();
        if self.showed_moving {
            state.writable = pb::terminal_state::Writable::Moving as i32;
            state.status = pb::SessionStatus::Moving as i32;
        }
    }

    async fn run(
        mut self,
        first: Upstream,
        current: LinkState,
        mut link: watch::Receiver<LinkState>,
        mut client: Pin<Box<dyn Stream<Item = Result<pb::AttachClientMessage, Status>> + Send>>,
        out: mpsc::Sender<Result<pb::AttachServerMessage, Status>>,
    ) {
        let mut generation = Some(first.generation);
        let mut to = Some(first.to);
        let mut from = Some(first.from);
        // At most the notices of one drop and one message from the machine.
        let mut outbox: VecDeque<Result<pb::AttachServerMessage, Status>> = VecDeque::new();
        let mut waiting: Option<pb::AttachClientMessage> = None;
        let mut finishing = false;
        let mut was_down = false;
        // Owed the reconnected line, which waits for the machine's replay.
        let mut catching_up = false;

        // The link may have changed while the first attach was opening.
        let mut pending_change = match current {
            LinkState::Up(g) => Some(g) != generation,
            LinkState::Down => true,
        };

        loop {
            if pending_change {
                pending_change = false;
                let state = *link.borrow_and_update();
                match state {
                    LinkState::Down => {
                        if !was_down || from.is_some() {
                            self.go_down(&mut to, &mut from, &mut generation, &mut waiting, &mut outbox, &mut was_down);
                        }
                    }
                    LinkState::Up(g) if generation != Some(g) => {
                        self.go_down_quietly(&mut to, &mut from, &mut generation, &mut waiting);
                        let open = self.tracker.reopen();
                        let opened = tokio::select! {
                            () = out.closed() => return,
                            opened = open_machine_side(self.links.as_ref(), &self.machine, &open) => opened,
                        };
                        match opened {
                            Ok(upstream) => {
                                if was_down {
                                    was_down = false;
                                    catching_up = true;
                                }
                                generation = Some(upstream.generation);
                                to = Some(upstream.to);
                                from = Some(upstream.from);
                            }
                            Err(status) if link_failure(&status) => {
                                // Not reachable yet: stay disconnected until the link changes.
                                if !was_down {
                                    self.go_down(&mut to, &mut from, &mut generation, &mut waiting, &mut outbox, &mut was_down);
                                }
                            }
                            Err(status) => {
                                outbox.push_back(Err(status));
                                finishing = true;
                            }
                        }
                    }
                    LinkState::Up(_) => {}
                }
            }
            if finishing && outbox.is_empty() {
                return;
            }

            enum Event {
                ClientGone,
                OutRoom(bool),
                Machine(Option<Result<pb::AttachServerMessage, Status>>),
                ToRoom(bool),
                Client(Option<Result<pb::AttachClientMessage, Status>>),
                Link(bool),
                Moved,
            }

            let event = tokio::select! {
                () = out.closed() => Event::ClientGone,
                room = out.reserve(), if !outbox.is_empty() => match room {
                    Ok(permit) => {
                        if let Some(message) = outbox.pop_front() {
                            permit.send(message);
                        }
                        Event::OutRoom(true)
                    }
                    Err(_) => Event::OutRoom(false),
                },
                message = next_from(&mut from), if from.is_some() && outbox.is_empty() => Event::Machine(message),
                room = reserve_to(&to), if waiting.is_some() && to.is_some() => match room {
                    Ok(permit) => {
                        if let Some(message) = waiting.take() {
                            permit.send(message);
                        }
                        Event::ToRoom(true)
                    }
                    Err(_) => Event::ToRoom(false),
                },
                message = client.next(), if waiting.is_none() => Event::Client(message),
                changed = link.changed() => Event::Link(changed.is_ok()),
                Ok(()) = self.moves.changed() => Event::Moved,
            };

            match event {
                Event::ClientGone | Event::OutRoom(false) => return,
                Event::OutRoom(true) | Event::ToRoom(true) => {}
                // The request side closed: the stream's end or error says why.
                Event::ToRoom(false) => {
                    waiting = None;
                    to = None;
                }
                Event::Machine(Some(Ok(mut message))) => {
                    self.tracker.delivered(&message);
                    let counted = match &message.message {
                        Some(ServerMessage::Notice(notice))
                            if notice.kind() == pb::terminal_notice::Kind::CaughtUp =>
                        {
                            Some(notice.caught_up_lines)
                        }
                        _ => None,
                    };
                    match counted {
                        // The machine's count goes out as the reconnected
                        // line, never as a line of its own.
                        Some(lines) => {
                            if std::mem::take(&mut catching_up) {
                                outbox.push_back(Ok(self.reconnected(lines)));
                            }
                        }
                        None => {
                            // A fresh screen replays nothing: the machine is
                            // back all the same.
                            if catching_up
                                && matches!(message.message, Some(ServerMessage::Snapshot(_)))
                            {
                                catching_up = false;
                                outbox.push_back(Ok(self.reconnected(0)));
                            }
                            if let Some(ServerMessage::State(state)) = &mut message.message {
                                self.show_moving(state);
                            }
                            outbox.push_back(Ok(message));
                        }
                    }
                }
                Event::Machine(Some(Err(status))) if link_failure(&status) => {
                    self.go_down(&mut to, &mut from, &mut generation, &mut waiting, &mut outbox, &mut was_down);
                }
                Event::Machine(Some(Err(status))) => {
                    outbox.push_back(Err(status));
                    finishing = true;
                    to = None;
                    from = None;
                }
                // The machine ended the terminal's stream.
                Event::Machine(None) => {
                    finishing = true;
                    to = None;
                    from = None;
                }
                Event::Client(Some(Ok(message))) => {
                    self.tracker.asked(&message);
                    // Down or moving: typing is disabled; nothing is kept for
                    // later (SE-16, MV-4).
                    let typing = matches!(message.message, Some(ClientMessage::Input(_)));
                    if to.is_some() && !(typing && self.moving()) {
                        waiting = Some(message);
                    }
                }
                Event::Client(Some(Err(_)) | None) => return,
                Event::Link(true) => pending_change = true,
                Event::Moved => {
                    if self.moving() != self.showed_moving
                        && let Some(mut state) = self.tracker.state.clone()
                    {
                        self.show_moving(&mut state);
                        outbox.push_back(Ok(pb::AttachServerMessage {
                            message: Some(ServerMessage::State(state)),
                        }));
                    }
                }
                Event::Link(false) => {
                    outbox.push_back(Err(RelayError::Offline {
                        machine: self.machine.clone(),
                    }
                    .into()));
                    finishing = true;
                    to = None;
                    from = None;
                }
            }
        }
    }

    /// The machine's side is gone; the client hears it once (SE-16).
    fn go_down(
        &self,
        to: &mut Option<mpsc::Sender<pb::AttachClientMessage>>,
        from: &mut Option<Streaming<pb::AttachServerMessage>>,
        generation: &mut Option<u64>,
        waiting: &mut Option<pb::AttachClientMessage>,
        outbox: &mut VecDeque<Result<pb::AttachServerMessage, Status>>,
        was_down: &mut bool,
    ) {
        self.go_down_quietly(to, from, generation, waiting);
        if *was_down {
            return;
        }
        *was_down = true;
        let mut state = self.tracker.state.clone().unwrap_or_default();
        state.writable = pb::terminal_state::Writable::MachineOffline as i32;
        outbox.push_back(Ok(pb::AttachServerMessage {
            message: Some(ServerMessage::State(state)),
        }));
        outbox.push_back(Ok(pb::AttachServerMessage {
            message: Some(ServerMessage::Notice(pb::TerminalNotice {
                kind: pb::terminal_notice::Kind::Disconnected as i32,
                text: format!(
                    "lost the connection to {}; the session keeps running there and this view will catch up",
                    self.machine
                ),
                at_offset: self.tracker.delivered.unwrap_or(0),
                caught_up_lines: 0,
                at: Some(now()),
            })),
        }));
    }

    /// Drops the machine's side: the machine closes its attach and the
    /// terminal keeps running (PER-4). Input waiting for it is dropped.
    fn go_down_quietly(
        &self,
        to: &mut Option<mpsc::Sender<pb::AttachClientMessage>>,
        from: &mut Option<Streaming<pb::AttachServerMessage>>,
        generation: &mut Option<u64>,
        waiting: &mut Option<pb::AttachClientMessage>,
    ) {
        *to = None;
        *from = None;
        *generation = None;
        *waiting = None;
    }

    /// The line shown when the machine is back (SE-16), with the lines the
    /// machine counted as it replayed what was missed: the relay never
    /// decodes chunks to count them (TR-11). 0 before a fresh screen.
    fn reconnected(&self, caught_up_lines: u32) -> pb::AttachServerMessage {
        pb::AttachServerMessage {
            message: Some(ServerMessage::Notice(pb::TerminalNotice {
                kind: pb::terminal_notice::Kind::Reconnected as i32,
                text: format!("reconnected to {}", self.machine),
                at_offset: self.tracker.delivered.unwrap_or(0),
                caught_up_lines,
                at: Some(now()),
            })),
        }
    }
}

/// The next message from the machine; never ready while there is no
/// machine side.
async fn next_from(
    from: &mut Option<Streaming<pb::AttachServerMessage>>,
) -> Option<Result<pb::AttachServerMessage, Status>> {
    match from {
        Some(stream) => stream.next().await,
        None => pending().await,
    }
}

/// Room for one more message to the machine; never ready while there is no
/// machine side.
async fn reserve_to(
    to: &Option<mpsc::Sender<pb::AttachClientMessage>>,
) -> Result<mpsc::Permit<'_, pb::AttachClientMessage>, mpsc::error::SendError<()>> {
    match to {
        Some(sender) => sender.reserve().await,
        None => pending().await,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};
    use std::time::Duration;

    use humata2_rpc_kit::lock;
    use hyper_util::rt::TokioIo;
    use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
    use tonic::transport::server::Connected;
    use tonic::transport::{Endpoint, Server, Uri};

    use super::*;

    #[derive(Default)]
    struct FakeLinks {
        inner: Mutex<HashMap<String, FakeLink>>,
    }

    struct FakeLink {
        channel: Option<Channel>,
        generation: u64,
        tx: watch::Sender<LinkState>,
    }

    impl FakeLinks {
        fn up(&self, machine: &str, channel: Channel) {
            let mut map = lock(&self.inner);
            if let Some(entry) = map.get_mut(machine) {
                entry.generation += 1;
                entry.channel = Some(channel);
                let _ = entry.tx.send(LinkState::Up(entry.generation));
            } else {
                let (tx, _rx) = watch::channel(LinkState::Up(1));
                map.insert(
                    machine.to_owned(),
                    FakeLink {
                        channel: Some(channel),
                        generation: 1,
                        tx,
                    },
                );
            }
        }

        fn down(&self, machine: &str) {
            let mut map = lock(&self.inner);
            if let Some(entry) = map.get_mut(machine) {
                entry.channel = None;
                let _ = entry.tx.send(LinkState::Down);
            }
        }
    }

    impl RelayLinks for FakeLinks {
        fn channel(&self, machine: &str) -> Result<Channel, RelayError> {
            let map = lock(&self.inner);
            match map.get(machine) {
                Some(entry) => entry.channel.clone().ok_or_else(|| RelayError::Offline {
                    machine: machine.to_owned(),
                }),
                None => Err(RelayError::Unknown {
                    machine: machine.to_owned(),
                }),
            }
        }

        fn state(&self, machine: &str) -> LinkState {
            let map = lock(&self.inner);
            map.get(machine)
                .map_or(LinkState::Down, |entry| *entry.tx.borrow())
        }

        fn watch(&self, machine: &str) -> watch::Receiver<LinkState> {
            let mut map = lock(&self.inner);
            let entry = map.entry(machine.to_owned()).or_insert_with(|| {
                let (tx, _rx) = watch::channel(LinkState::Down);
                FakeLink {
                    channel: None,
                    generation: 0,
                    tx,
                }
            });
            entry.tx.subscribe()
        }
    }

    struct NoopSessions {
        moves: watch::Sender<u64>,
    }

    impl Default for NoopSessions {
        fn default() -> Self {
            let (moves, _) = watch::channel(0);
            Self { moves }
        }
    }

    #[tonic::async_trait]
    impl TerminalSessions for NoopSessions {
        fn is_moving(&self, _session: &str) -> bool {
            false
        }

        fn moving_changes(&self) -> watch::Receiver<u64> {
            self.moves.subscribe()
        }

        fn paused_for_move(&self, _socket: &std::path::Path, _pid: u32) -> bool {
            false
        }

        fn refuse_input_while_moving(&self, _session: &str) -> Result<(), Status> {
            Ok(())
        }

        async fn resume_shell(
            &self,
            _caller: &humata2_daemon_ports::Caller,
            _tab: &str,
        ) -> Result<(), Status> {
            Ok(())
        }
    }

    /// The machine's end of an in-memory link.
    struct Served(DuplexStream);

    impl Connected for Served {
        type ConnectInfo = ();

        fn connect_info(&self) -> Self::ConnectInfo {}
    }

    impl AsyncRead for Served {
        fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for Served {
        fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.0).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_shutdown(cx)
        }
    }

    /// The service the in-memory server runs (the trait is foreign, so a newtype).
    struct Machine(Arc<Fake>);

    type Feed = mpsc::Sender<Result<pb::AttachServerMessage, Status>>;

    /// A machine's terminal service: records what it is asked and lets the
    /// test say what each attach sends.
    #[derive(Default)]
    struct Fake {
        opens: Mutex<Vec<pb::AttachOpen>>,
        inputs: Mutex<Vec<Vec<u8>>>,
        feeds: Mutex<Vec<Feed>>,
        closed: AtomicUsize,
    }

    impl Fake {
        async fn say(&self, attach: usize, message: ServerMessage) {
            let feed = lock(&self.feeds)[attach].clone();
            let _ = feed
                .send(Ok(pb::AttachServerMessage {
                    message: Some(message),
                }))
                .await;
        }

        /// The machine drops attach `attach`'s stream, as a link cut does.
        async fn cut(&self, attach: usize) {
            let feed = lock(&self.feeds)[attach].clone();
            let _ = feed.send(Err(Status::unavailable("link lost"))).await;
        }

        /// The machine ends attach `attach`'s stream cleanly.
        fn end(&self, attach: usize) {
            let feed = lock(&self.feeds)[attach].clone();
            drop(feed);
            lock(&self.feeds)[attach] = mpsc::channel(1).0;
        }

        fn inputs(&self) -> Vec<Vec<u8>> {
            lock(&self.inputs).clone()
        }

        fn opens(&self) -> Vec<pb::AttachOpen> {
            lock(&self.opens).clone()
        }
    }

    #[tonic::async_trait]
    impl pb::terminal_service_server::TerminalService for Machine {
        async fn list_tabs(
            &self,
            request: Request<pb::ListTabsRequest>,
        ) -> Result<Response<pb::ListTabsResponse>, Status> {
            let tab = pb::Tab {
                title: format!("tab of {:?}", request.into_inner().session.map(|s| s.value)),
                end_offset: 42,
                ..Default::default()
            };
            Ok(Response::new(pb::ListTabsResponse { tabs: vec![tab] }))
        }

        async fn open_shell_tab(&self, _: Request<pb::OpenShellTabRequest>) -> Result<Response<pb::Tab>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn open_agent_tab(
            &self,
            request: Request<pb::OpenAgentTabRequest>,
        ) -> Result<Response<pb::OpenAgentTabResponse>, Status> {
            let req = request.into_inner();
            let agent = req.agent.as_ref().map(|a| a.value.clone()).unwrap_or_default();
            let tab = pb::Tab {
                id: Some(pb::TabId { value: "tb_agent_remote".to_owned() }),
                session: req.session,
                kind: pb::tab::Kind::Shell as i32,
                title: agent.clone(),
                agent: (!agent.is_empty()).then(|| pb::AgentId { value: agent }),
                ..Default::default()
            };
            Ok(Response::new(pb::OpenAgentTabResponse {
                tab: Some(tab),
                index: 2,
            }))
        }

        async fn open_machine_shell(&self, _: Request<pb::OpenMachineShellRequest>) -> Result<Response<pb::Tab>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn close_tab(&self, _: Request<pb::CloseTabRequest>) -> Result<Response<pb::CloseTabResponse>, Status> {
            Err(Status::not_found("no such tab"))
        }

        type AttachStream = Pin<Box<dyn Stream<Item = Result<pb::AttachServerMessage, Status>> + Send>>;

        async fn attach(
            &self,
            request: Request<Streaming<pb::AttachClientMessage>>,
        ) -> Result<Response<Self::AttachStream>, Status> {
            let mut incoming = request.into_inner();
            let (feed, replies) = mpsc::channel(32);
            lock(&self.0.feeds).push(feed);
            let fake = self.0.clone();
            tokio::spawn(async move {
                while let Some(Ok(message)) = incoming.next().await {
                    match message.message {
                        Some(ClientMessage::Open(open)) => lock(&fake.opens).push(open),
                        Some(ClientMessage::Input(input)) => lock(&fake.inputs).push(input.data.to_vec()),
                        _ => {}
                    }
                }
                fake.closed.fetch_add(1, Ordering::SeqCst);
            });
            Ok(Response::new(Box::pin(ReceiverStream::new(replies))))
        }

        async fn r#type(&self, _: Request<pb::TypeRequest>) -> Result<Response<pb::TypeResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn cancel_queued_typing(
            &self,
            _: Request<pb::CancelQueuedTypingRequest>,
        ) -> Result<Response<pb::CancelQueuedTypingResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn list_queued(&self, _: Request<pb::ListQueuedRequest>) -> Result<Response<pb::ListQueuedResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn send_queued_now(
            &self,
            _: Request<pb::SendQueuedNowRequest>,
        ) -> Result<Response<pb::SendQueuedNowResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn paste_image(&self, _: Request<pb::PasteImageRequest>) -> Result<Response<pb::PasteImageResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }
    }

    /// A channel to an in-memory machine running `fake`.
    async fn connect(fake: Arc<Fake>) -> Channel {
        let (host, machine) = tokio::io::duplex(256 * 1024);
        tokio::spawn(
            Server::builder()
                .add_service(pb::terminal_service_server::TerminalServiceServer::new(Machine(fake)))
                .serve_with_incoming(tokio_stream::once(Ok::<_, std::io::Error>(Served(machine)))),
        );
        let pipe = Arc::new(Mutex::new(Some(host)));
        let connected = Endpoint::from_static("http://relay.test")
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let pipe = lock(&pipe).take();
                async move { pipe.map(TokioIo::new).ok_or_else(|| std::io::Error::other("used")) }
            }))
            .await;
        connected.expect("the in-memory link connects")
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    fn output(offset: u64, data: &[u8]) -> ServerMessage {
        ServerMessage::Output(pb::TerminalOutput {
            offset,
            data: data.to_vec().into(),
            codec: pb::ChunkCodec::None as i32,
            raw_size: data.len() as u32,
            crc32: None,
        })
    }

    fn input(seq: u64, data: &[u8]) -> pb::AttachClientMessage {
        pb::AttachClientMessage {
            message: Some(ClientMessage::Input(pb::TerminalInput {
                data: data.to_vec().into(),
                input_seq: seq,
            })),
        }
    }

    /// The machine's count of what an attach from an offset replayed.
    fn caught_up(at_offset: u64, lines: u32) -> ServerMessage {
        ServerMessage::Notice(pb::TerminalNotice {
            kind: pb::terminal_notice::Kind::CaughtUp as i32,
            at_offset,
            caught_up_lines: lines,
            ..Default::default()
        })
    }

    fn open(from: u64) -> pb::AttachOpen {
        pb::AttachOpen {
            tab: Some(pb::TabId { value: "tb_1".into() }),
            from_offset: from,
            cols: 80,
            rows: 24,
            ..Default::default()
        }
    }

    type Typed = mpsc::Sender<Result<pb::AttachClientMessage, Status>>;

    struct Rig {
        links: Arc<FakeLinks>,
        fake: Arc<Fake>,
        relay: RemoteTerminal,
    }

    async fn rig() -> Rig {
        let links = Arc::new(FakeLinks::default());
        let fake = Arc::new(Fake::default());
        links.up("devbox", connect(fake.clone()).await);
        let relay = RemoteTerminal::new(links.clone(), Arc::new(NoopSessions::default()));
        Rig { links, fake, relay }
    }

    async fn attached(rig: &Rig, from: u64) -> (Typed, AttachStream) {
        let (typed, incoming) = mpsc::channel(8);
        let stream = rig
            .relay
            .attach("devbox", open(from), ReceiverStream::new(incoming))
            .await
            .expect("the attach opens");
        settle().await;
        (typed, stream)
    }

    async fn next(stream: &mut AttachStream) -> ServerMessage {
        tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("a message arrives")
            .expect("the stream is open")
            .expect("the message is not an error")
            .message
            .expect("the message has a body")
    }

    async fn quiet(stream: &mut AttachStream) -> bool {
        tokio::time::timeout(Duration::from_secs(60), stream.next()).await.is_err()
    }

    #[tokio::test(start_paused = true)]
    async fn a_unary_call_is_forwarded_and_the_machines_error_comes_back_as_it_is() {
        let rig = rig().await;

        let tabs = rig
            .relay
            .list_tabs(
                "devbox",
                pb::ListTabsRequest {
                    session: Some(pb::SessionId { value: "se_1".into() }),
                },
            )
            .await
            .expect("the machine answers")
            .into_inner();
        let refused = rig.relay.close_tab("devbox", pb::CloseTabRequest::default()).await;

        assert_eq!(tabs.tabs.len(), 1);
        assert_eq!(tabs.tabs[0].end_offset, 42);
        assert!(tabs.tabs[0].title.contains("se_1"));
        assert_eq!(refused.expect_err("the machine refuses").code(), Code::NotFound);
    }

    #[tokio::test(start_paused = true)]
    async fn with_the_link_down_calls_fail_as_offline_at_once() {
        let rig = rig().await;
        rig.links.down("devbox");

        let call = tokio::time::timeout(
            Duration::from_secs(1),
            rig.relay.list_tabs("devbox", pb::ListTabsRequest::default()),
        )
        .await
        .expect("it answers at once");
        let (_typed, incoming) = mpsc::channel(1);
        let attach = tokio::time::timeout(
            Duration::from_secs(1),
            rig.relay.attach("devbox", open(0), ReceiverStream::new(incoming)),
        )
        .await
        .expect("it answers at once");

        assert_eq!(call.expect_err("offline").code(), Code::Unavailable);
        assert!(attach.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn output_and_input_pass_through_in_order_with_offsets_intact() {
        let rig = rig().await;
        let (typed, mut stream) = attached(&rig, 0).await;

        rig.fake.say(0, output(0, b"hello ")).await;
        // A compressed chunk is forwarded untouched, whatever its bytes are.
        let chunk = pb::TerminalOutput {
            offset: 6,
            data: vec![9, 8, 7].into(),
            codec: pb::ChunkCodec::Lz4 as i32,
            raw_size: 20,
            crc32: Some(5),
        };
        rig.fake.say(0, ServerMessage::Output(chunk.clone())).await;
        for (seq, key) in [(1, b"a"), (2, b"b"), (3, b"c")] {
            typed.send(Ok(input(seq, key))).await.expect("typed");
        }
        settle().await;

        assert_eq!(next(&mut stream).await, output(0, b"hello "));
        assert_eq!(next(&mut stream).await, ServerMessage::Output(chunk));
        assert_eq!(rig.fake.inputs(), vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
        assert_eq!(rig.fake.opens()[0].from_offset, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_cut_link_keeps_the_client_open_and_typing_is_not_delivered() {
        let rig = rig().await;
        let (typed, mut stream) = attached(&rig, 0).await;
        rig.fake.say(0, output(0, b"abc")).await;
        assert_eq!(next(&mut stream).await, output(0, b"abc"));

        rig.fake.cut(0).await;
        rig.links.down("devbox");
        settle().await;
        let state = next(&mut stream).await;
        let notice = next(&mut stream).await;
        typed.send(Ok(input(1, b"lost"))).await.expect("typed");
        settle().await;

        match state {
            ServerMessage::State(state) => {
                assert_eq!(state.writable, pb::terminal_state::Writable::MachineOffline as i32);
            }
            other => panic!("expected a state, got {other:?}"),
        }
        match notice {
            ServerMessage::Notice(notice) => {
                assert_eq!(notice.kind, pb::terminal_notice::Kind::Disconnected as i32);
            }
            other => panic!("expected a notice, got {other:?}"),
        }
        assert!(quiet(&mut stream).await, "the stream stays open and silent");
        assert!(rig.fake.inputs().is_empty());

        // What was typed while down is not sent when the link returns.
        let returned = connect(rig.fake.clone()).await;
        rig.links.up("devbox", returned);
        settle().await;
        assert!(rig.fake.inputs().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_returning_link_resumes_from_the_last_delivered_offset() {
        let rig = rig().await;
        let (typed, mut stream) = attached(&rig, 0).await;
        rig.fake.say(0, output(0, b"abc")).await;
        rig.fake.say(0, output(3, b"de")).await;
        next(&mut stream).await;
        next(&mut stream).await;
        typed
            .send(Ok(pb::AttachClientMessage {
                message: Some(ClientMessage::Resize(pb::TerminalResize { cols: 100, rows: 30, ..Default::default() })),
            }))
            .await
            .expect("resized");
        settle().await;

        rig.fake.cut(0).await;
        rig.links.down("devbox");
        settle().await;
        next(&mut stream).await;
        next(&mut stream).await;
        let returned = connect(rig.fake.clone()).await;
        rig.links.up("devbox", returned);
        settle().await;

        let opens = rig.fake.opens();
        assert_eq!(opens.len(), 2);
        assert_eq!(opens[1].from_offset, 5);
        assert_eq!((opens[1].cols, opens[1].rows), (100, 30));
        rig.fake.say(1, caught_up(5, 0)).await;
        match next(&mut stream).await {
            ServerMessage::Notice(notice) => {
                assert_eq!(notice.kind, pb::terminal_notice::Kind::Reconnected as i32);
                assert_eq!(notice.at_offset, 5);
            }
            other => panic!("expected a notice, got {other:?}"),
        }
        rig.fake.say(1, output(5, b"fg")).await;
        typed.send(Ok(input(2, b"x"))).await.expect("typed");
        settle().await;
        assert_eq!(next(&mut stream).await, output(5, b"fg"));
        assert_eq!(rig.fake.inputs(), vec![b"x".to_vec()]);
        assert!(quiet(&mut stream).await, "nothing is repeated");
    }

    #[tokio::test(start_paused = true)]
    async fn a_fresh_screen_from_the_machine_is_passed_through() {
        let rig = rig().await;
        let (_typed, mut stream) = attached(&rig, 0).await;
        rig.fake.say(0, output(0, b"abc")).await;
        next(&mut stream).await;
        rig.fake.cut(0).await;
        rig.links.down("devbox");
        settle().await;
        next(&mut stream).await;
        next(&mut stream).await;

        rig.links.up("devbox", connect(rig.fake.clone()).await);
        settle().await;
        let snapshot = pb::TerminalSnapshot {
            offset: 900,
            vt: vec![1, 2, 3].into(),
            cols: 80,
            rows: 24,
            ..Default::default()
        };
        rig.fake.say(1, ServerMessage::Snapshot(snapshot.clone())).await;
        rig.fake.say(1, output(900, b"z")).await;

        assert!(matches!(
            next(&mut stream).await,
            ServerMessage::Notice(notice)
                if notice.kind == pb::terminal_notice::Kind::Reconnected as i32
        ));
        assert_eq!(next(&mut stream).await, ServerMessage::Snapshot(snapshot));
        assert_eq!(next(&mut stream).await, output(900, b"z"));
    }

    /// SE-16: the line saying the machine is back comes after what it
    /// replayed and counts its lines; the machine's own count is not passed
    /// on as a second line.
    #[tokio::test(start_paused = true)]
    async fn the_reconnected_line_counts_what_the_machine_replayed() {
        let rig = rig().await;
        let (_typed, mut stream) = attached(&rig, 0).await;
        rig.fake.say(0, output(0, b"abc")).await;
        next(&mut stream).await;
        rig.fake.cut(0).await;
        rig.links.down("devbox");
        settle().await;
        next(&mut stream).await;
        next(&mut stream).await;

        rig.links.up("devbox", connect(rig.fake.clone()).await);
        settle().await;
        rig.fake.say(1, output(3, b"d\r\ne\r\n")).await;
        rig.fake.say(1, caught_up(9, 2)).await;

        assert_eq!(next(&mut stream).await, output(3, b"d\r\ne\r\n"));
        match next(&mut stream).await {
            ServerMessage::Notice(notice) => {
                assert_eq!(notice.kind, pb::terminal_notice::Kind::Reconnected as i32);
                assert_eq!(notice.caught_up_lines, 2);
                assert_eq!(notice.at_offset, 9);
            }
            other => panic!("expected a notice, got {other:?}"),
        }
        assert!(quiet(&mut stream).await, "the machine's count is not a second line");
    }

    #[tokio::test(start_paused = true)]
    async fn the_client_leaving_closes_the_machine_side_attach() {
        let rig = rig().await;
        let (typed, stream) = attached(&rig, 0).await;
        assert_eq!(rig.fake.closed.load(Ordering::SeqCst), 0);

        drop(stream);
        drop(typed);
        settle().await;

        assert_eq!(rig.fake.closed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_machine_ending_the_terminal_ends_the_client_stream() {
        let rig = rig().await;
        let (_typed, mut stream) = attached(&rig, 0).await;
        rig.fake.say(0, output(0, b"bye")).await;
        assert_eq!(next(&mut stream).await, output(0, b"bye"));

        rig.fake.end(0);
        settle().await;

        let ended = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("it ends at once");
        assert!(ended.is_none());
    }

    #[tokio::test]
    async fn open_agent_tab_forwards_request_and_returns_tab_and_index() {
        let rig = rig().await;
        let reply = rig
            .relay
            .open_agent_tab(
                "devbox",
                pb::OpenAgentTabRequest {
                    idempotency_key: Some(pb::IdempotencyKey { value: "k1".to_owned() }),
                    session: Some(pb::SessionId { value: "se_remote".to_owned() }),
                    agent: Some(pb::AgentId { value: "codex".to_owned() }),
                    prompt: "Review the diff".to_owned(),
                },
            )
            .await
            .expect("relayed open_agent_tab succeeds")
            .into_inner();
        assert_eq!(reply.index, 2);
        let tab = reply.tab.expect("response carries tab");
        assert_eq!(tab.id.as_ref().map(|id| id.value.as_str()), Some("tb_agent_remote"));
        assert_eq!(tab.title, "codex");
        assert_eq!(tab.agent.as_ref().map(|a| a.value.as_str()), Some("codex"));
    }
}
