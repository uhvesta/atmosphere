//! A tab's terminal link over gRPC (D271): the attach read as the terminal
//! component's frames, and its frames sent back.
//!
//! Input is never queued across attaches. A [`Gate`] is open only while an
//! attach is up; every frame the view sends is stamped with the gate's
//! generation, and a frame whose generation is not the current one is
//! dropped. So what is typed while the link reattaches, or after it ended,
//! fails to send, and what was in flight when it dropped is not replayed
//! into the next attach.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use humata2_clients::{DaemonError, LinkSink, SendError, TerminalLink, TerminalNotice};
use crate::humata2_grpc::humata::v1 as pb;
use humata2_terminal_link::{Inbound, Outgoing, open as open_message, outbound};
use terminal_wire::{ClientFrame, ServerFrame};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::connection::Shared;

/// Bytes the daemon may send before the view acknowledges them.
const WINDOW_BYTES: u32 = 4 << 20;
const QUEUE: usize = 256;
/// Lines beside the terminal not yet drawn; past it the oldest wait and a
/// new one is dropped, since a later line says the same.
const NOTICES: usize = 8;

/// Whether frames may be sent now, and to which attach: the generation
/// counts the attaches that ended. Bit 0 is "open": an attach is up. Bit 1
/// is "synced": its screen has arrived. Typing waits for the screen, so
/// nothing is typed into a program the view has not caught up with.
pub(crate) struct Gate {
    state: AtomicU64,
    /// The view's latest resize, which the next attach opens at: one made
    /// while no attach is up is kept here rather than refused.
    resize: Mutex<Option<ClientFrame>>,
    /// The view's latest acknowledgement, with its generation. Each says
    /// how far the view has read, so only the latest needs to go: a flood
    /// acknowledges every frame it draws, and one queued at a time keeps
    /// typing from waiting behind them, or finding the queue full.
    ack: Mutex<Option<(u64, ClientFrame)>>,
    /// Whether one acknowledgement is queued; the link sends the latest
    /// when it reaches it.
    ack_queued: AtomicBool,
}

const OPEN: u64 = 1;
const SYNCED: u64 = 2;
const GENERATION: u32 = 2;

impl Gate {
    /// Closed: no attach is up yet.
    pub(crate) fn closed() -> Self {
        Gate {
            state: AtomicU64::new(0),
            resize: Mutex::new(None),
            ack: Mutex::new(None),
            ack_queued: AtomicBool::new(false),
        }
    }

    /// Keeps `resize`, the view's latest, for the next attach to open at.
    pub(crate) fn remember(&self, resize: &ClientFrame) {
        *self.resize.lock().unwrap_or_else(PoisonError::into_inner) = Some(resize.clone());
    }

    /// The view's latest resize, if it has made one.
    pub(crate) fn last_resize(&self) -> Option<ClientFrame> {
        self.resize
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Keeps `ack`, stamped with `generation`, as the latest. Whether the
    /// caller must queue it: none is queued yet.
    fn note_ack(&self, generation: u64, ack: ClientFrame) -> bool {
        *self.ack.lock().unwrap_or_else(PoisonError::into_inner) = Some((generation, ack));
        !self.ack_queued.swap(true, Ordering::AcqRel)
    }

    /// The latest acknowledgement, for the one queued that the link has
    /// reached; none if an earlier one already took it.
    fn take_ack(&self) -> Option<(u64, ClientFrame)> {
        // Cleared first: one noted from here on is queued again.
        self.ack_queued.store(false, Ordering::Release);
        self.ack.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// A queued acknowledgement could not go: the next one is queued.
    fn ack_unqueued(&self) {
        self.ack_queued.store(false, Ordering::Release);
    }

    /// The generation to stamp a frame with, if frames other than typing
    /// may go now.
    pub(crate) fn admit(&self) -> Result<u64, SendError> {
        let state = self.state.load(Ordering::Acquire);
        if state & OPEN == 0 {
            Err(SendError::Closed)
        } else {
            Ok(state >> GENERATION)
        }
    }

    /// The generation to stamp typing with, if it may go now: the attach is
    /// up and its screen has arrived.
    pub(crate) fn admit_input(&self) -> Result<u64, SendError> {
        let state = self.state.load(Ordering::Acquire);
        if state & (OPEN | SYNCED) == OPEN | SYNCED {
            Ok(state >> GENERATION)
        } else {
            Err(SendError::Closed)
        }
    }

    /// An attach is up.
    pub(crate) fn open(&self) {
        self.state.fetch_or(OPEN, Ordering::AcqRel);
    }

    /// The attach's screen has arrived: typing may go.
    pub(crate) fn synced(&self) {
        let _ = self
            .state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state & OPEN != 0).then_some(state | SYNCED)
            });
    }

    /// The attach ended or is being replaced: frames stamped so far are
    /// stale for good.
    pub(crate) fn close(&self) {
        let _ = self
            .state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                Some(((state >> GENERATION) + 1) << GENERATION)
            });
    }

    /// Whether a frame stamped with `generation` may still go out.
    pub(crate) fn is_current(&self, generation: u64) -> bool {
        self.state.load(Ordering::Acquire) & !SYNCED == (generation << GENERATION) | OPEN
    }
}

/// The sink a view sends its frames through: stamps each with the gate's
/// generation, or refuses it while the gate is closed (typing: until the
/// attach's screen has arrived). Opening and taking control send nothing on
/// an attach, so they are accepted before it is up: a view binds as soon as
/// it has the link, while the attach is still on its way. A resize is kept
/// for the next attach to open at, so one made before an attach is up is
/// accepted too.
pub(crate) fn sink(gate: Arc<Gate>, to: mpsc::Sender<(u64, ClientFrame)>) -> LinkSink {
    LinkSink::new(move |frame| {
        let generation = match frame {
            ClientFrame::Open { .. } | ClientFrame::TakeControl { .. } => return Ok(()),
            ClientFrame::Resize { .. } => {
                // Kept before the gate is read, so an attach that opens
                // after this finds it.
                gate.remember(&frame);
                match gate.admit() {
                    Ok(generation) => generation,
                    // No attach is up: the next one opens at this size.
                    Err(_) => return Ok(()),
                }
            }
            ClientFrame::Input { .. } => gate.admit_input()?,
            ClientFrame::Ack { .. } => {
                let generation = gate.admit()?;
                if !gate.note_ack(generation, frame.clone()) {
                    return Ok(());
                }
                return to.try_send((generation, frame)).map_err(|error| {
                    gate.ack_unqueued();
                    SendError::from(error)
                });
            }
            _ => gate.admit()?,
        };
        to.try_send((generation, frame)).map_err(SendError::from)
    })
}

/// Starts the link for tab `tab` of the session. The returned link's
/// frames end when the attach does.
pub(crate) fn start(
    shared: &Arc<Shared>,
    session: pb::SessionId,
    tab: pb::TabId,
    route: u64,
    size: (u16, u16),
) -> TerminalLink {
    let gate = Arc::new(Gate::closed());
    let (input_to, input) = mpsc::channel(64);
    let (frames, received) = mpsc::channel(QUEUE);
    let (notices, notices_received) = mpsc::channel(NOTICES);
    let pump = Pump {
        shared: shared.clone(),
        session,
        tab,
        route,
        size,
        gate: gate.clone(),
        input,
        frames,
        notices,
        identity: Identity::default(),
    };
    shared.spawn(pump.run());
    TerminalLink {
        frames: received,
        notices: notices_received,
        sink: sink(gate, input_to),
    }
}

/// The line a daemon's notice asks the app to draw beside the terminal, if
/// any. The app draws a lost connection from the session's state and an
/// exit from the screen's, so only a reconnect comes from here.
fn notice_of(message: &pb::AttachServerMessage) -> Option<TerminalNotice> {
    match &message.message {
        Some(pb::attach_server_message::Message::Notice(notice))
            if notice.kind() == pb::terminal_notice::Kind::Reconnected =>
        {
            Some(TerminalNotice::Reconnected {
                caught_up_lines: notice.caught_up_lines,
            })
        }
        _ => None,
    }
}

/// How many others can type into the tab, from a state message (SE-24).
fn writers_of(message: &pb::AttachServerMessage) -> Option<u32> {
    match &message.message {
        Some(pb::attach_server_message::Message::State(state)) => Some(state.other_writers),
        _ => None,
    }
}

/// The columns and rows a resize asks for.
fn size_of(frame: &ClientFrame) -> Option<(u16, u16)> {
    match frame {
        ClientFrame::Resize { cols, rows, .. } => Some((*cols, *rows)),
        _ => None,
    }
}

/// What a screen's UUID means for the link: the daemon gives each holder's
/// screen its own UUID, so a new holder shows up as a new one. An empty UUID
/// is no identity at all: it matches only another empty one.
#[derive(Debug, Default)]
struct Identity {
    first: Option<String>,
}

/// Whether a screen belongs to the link.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// The screen is the link's own, or the first one seen.
    Keep,
    /// The screen belongs to another holder: the link must end.
    Replaced,
}

impl Identity {
    /// Checks the UUID of a screen an attach installed. The first one seen
    /// becomes the link's identity.
    fn check(&mut self, uuid: &str) -> Verdict {
        match &self.first {
            None => {
                self.first = Some(uuid.to_owned());
                Verdict::Keep
            }
            Some(first) if first == uuid => Verdict::Keep,
            Some(_) => Verdict::Replaced,
        }
    }
}

/// How one attach ended.
enum End {
    /// Attach again from the start, for a fresh screen.
    Reopen,
    /// The link is over.
    Over,
}

struct Pump {
    shared: Arc<Shared>,
    session: pb::SessionId,
    tab: pb::TabId,
    route: u64,
    size: (u16, u16),
    gate: Arc<Gate>,
    input: mpsc::Receiver<(u64, ClientFrame)>,
    frames: mpsc::Sender<ServerFrame>,
    notices: mpsc::Sender<TerminalNotice>,
    /// The screen the link started with: every later attach must show it.
    identity: Identity,
}

impl Pump {
    async fn run(mut self) {
        while let End::Reopen = self.attach_once().await {}
    }

    /// Reports why the link ended to the view and closes the gate.
    async fn fail(&self, message: String) -> End {
        self.gate.close();
        let _ = self
            .frames
            .send(ServerFrame::Error {
                id: Some(self.route),
                code: "attach".to_owned(),
                message,
            })
            .await;
        End::Over
    }

    /// Ends the link for a refusal that says what to migrate first: the view
    /// is told it as a notice, so it can say it beside the session. Any
    /// other error ends it as `fail` does.
    async fn refuse(&self, error: DaemonError) -> End {
        if matches!(error, DaemonError::MigrationNeeded { .. }) {
            let _ = self.notices.try_send(TerminalNotice::Refused(error.clone()));
        }
        self.fail(error.to_string()).await
    }

    /// Hands the view the line a notice asks for, then gives the message
    /// back to be read as frames.
    /// Also tells the view how many others can type when that changes;
    /// `writers` is what this attach told last.
    fn tell(
        &self,
        message: pb::AttachServerMessage,
        writers: &mut Option<u32>,
    ) -> pb::AttachServerMessage {
        if let Some(notice) = notice_of(&message) {
            let _ = self.notices.try_send(notice);
        }
        if let Some(others) = writers_of(&message)
            && *writers != Some(others)
            && self
                .notices
                .try_send(TerminalNotice::Writers { others })
                .is_ok()
        {
            *writers = Some(others);
        }
        message
    }

    /// Drops what is queued: none of it was typed for the next attach.
    fn discard_input(&mut self) {
        while self.input.try_recv().is_ok() {}
        let _ = self.gate.take_ack();
    }

    async fn attach_once(&mut self) -> End {
        let (to, outgoing) = mpsc::channel(64);
        // At the view's latest size: it may have resized since the link
        // started, or while no attach was up.
        let opened = self
            .gate
            .last_resize()
            .as_ref()
            .and_then(size_of)
            .unwrap_or(self.size);
        let first = open_message(
            Some(self.session.clone()),
            self.tab.clone(),
            opened.0,
            opened.1,
            false,
            WINDOW_BYTES,
        );
        if to.send(first).await.is_err() {
            return self
                .fail("The terminal's connection closed before it opened. Open the tab again".into())
                .await;
        }
        let mut stream = match self
            .shared
            .terminal()
            .attach(ReceiverStream::new(outgoing))
            .await
        {
            Ok(response) => response.into_inner(),
            Err(status) => {
                let error = self.shared.error(&status, "The terminal");
                return self.refuse(error).await;
            }
        };
        self.gate.open();
        // A resize made while this attach was opening was kept, not sent.
        if let Some(resize) = self.gate.last_resize() {
            if size_of(&resize) != Some(opened) {
                if let Ok(Outgoing::Send(message)) = outbound(resize) {
                    let _ = to.send(message).await;
                }
            }
        }
        let mut inbound = Inbound::new(self.route);
        // Each attach tells the count afresh: it may have changed while
        // none was up.
        let mut writers = None;
        loop {
            tokio::select! {
                message = stream.message() => match message {
                    Ok(Some(message)) => match inbound.map(self.tell(message, &mut writers)) {
                        Ok(frames) => {
                            if let Some(uuid) = inbound.terminal_uuid() {
                                if self.identity.check(uuid) == Verdict::Replaced {
                                    return self
                                        .fail("The tab's program was replaced on the machine, so this screen no longer shows it. Nothing typed here was sent. Open the tab again to see the new program".into())
                                        .await;
                                }
                            }
                            for frame in frames {
                                // Typing may go once the screen is on its way
                                // to the view: open the gate before handing it
                                // over, so a view that types as soon as it has
                                // the screen is never refused.
                                if matches!(frame, ServerFrame::Snapshot { .. }) {
                                    self.gate.synced();
                                }
                                if self.frames.send(frame).await.is_err() {
                                    self.gate.close();
                                    return End::Over;
                                }
                            }
                        }
                        Err(error) => return self.fail(error.to_string()).await,
                    },
                    // The daemon ended the attach: nothing typed after this
                    // is sent, and the app opens the link again.
                    Ok(None) => {
                        return self
                            .fail("The terminal's connection ended. Open the tab again".into())
                            .await;
                    }
                    Err(status) => {
                        let error = self.shared.error(&status, "The terminal");
                        if matches!(error, DaemonError::MigrationNeeded { .. }) {
                            return self.refuse(error).await;
                        }
                        return self
                            .fail("The terminal's connection ended. Open the tab again".into())
                            .await;
                    }
                },
                stamped = self.input.recv() => {
                    let Some((generation, frame)) = stamped else {
                        self.gate.close();
                        return End::Over;
                    };
                    // The queued acknowledgement goes as the latest one.
                    let (generation, frame) = if matches!(frame, ClientFrame::Ack { .. }) {
                        match self.gate.take_ack() {
                            Some(latest) => latest,
                            None => continue,
                        }
                    } else {
                        (generation, frame)
                    };
                    if !self.gate.is_current(generation) {
                        continue;
                    }
                    match outbound(frame) {
                        Ok(Outgoing::Send(message)) => {
                            let _ = to.send(message).await;
                        }
                        Ok(Outgoing::Reopen) => {
                            self.gate.close();
                            self.discard_input();
                            return End::Reopen;
                        }
                        Ok(Outgoing::Close) => {
                            self.gate.close();
                            let _ = self
                                .frames
                                .send(ServerFrame::Detached { id: self.route })
                                .await;
                            return End::Over;
                        }
                        Ok(Outgoing::Nothing) => {}
                        Err(error) => {
                            let _ = self.frames.send(ServerFrame::Error {
                                id: Some(self.route),
                                code: "link".to_owned(),
                                message: error.to_string(),
                            }).await;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(kind: pb::terminal_notice::Kind, caught_up_lines: u32) -> pb::AttachServerMessage {
        pb::AttachServerMessage {
            message: Some(pb::attach_server_message::Message::Notice(pb::TerminalNotice {
                kind: kind as i32,
                text: String::new(),
                at_offset: 0,
                caught_up_lines,
                at: None,
            })),
        }
    }

    #[test]
    fn a_reconnect_notice_becomes_a_line_beside_the_terminal() {
        assert_eq!(
            notice_of(&notice(pb::terminal_notice::Kind::Reconnected, 12)),
            Some(TerminalNotice::Reconnected {
                caught_up_lines: 12
            })
        );
    }

    #[test]
    fn notices_the_app_draws_from_state_become_no_line() {
        for kind in [
            pb::terminal_notice::Kind::Disconnected,
            pb::terminal_notice::Kind::Exited,
        ] {
            assert_eq!(notice_of(&notice(kind, 0)), None);
        }
    }

    #[test]
    fn a_state_message_says_how_many_others_can_type() {
        let state = |other_writers| pb::AttachServerMessage {
            message: Some(pb::attach_server_message::Message::State(pb::TerminalState {
                other_writers,
                ..Default::default()
            })),
        };
        assert_eq!(writers_of(&state(2)), Some(2));
        assert_eq!(writers_of(&state(0)), Some(0));
        assert_eq!(
            writers_of(&notice(pb::terminal_notice::Kind::Reconnected, 3)),
            None
        );
    }

    fn typed() -> ClientFrame {
        ClientFrame::Input {
            id: 0,
            seq: 1,
            bytes: b"x".to_vec(),
        }
    }

    fn link() -> (Arc<Gate>, LinkSink, mpsc::Receiver<(u64, ClientFrame)>) {
        let gate = Arc::new(Gate::closed());
        let (to, from) = mpsc::channel(8);
        (gate.clone(), sink(gate, to), from)
    }

    #[test]
    fn opening_before_an_attach_is_up_is_accepted_and_sends_nothing() {
        let (_gate, sink, mut from) = link();
        sink.send(ClientFrame::Open { id: 0 }).expect("an open needs no attach to be up");
        assert!(from.try_recv().is_err());
    }

    #[test]
    fn input_before_an_attach_is_up_fails_as_closed() {
        let (_gate, sink, mut from) = link();
        assert_eq!(sink.send(typed()), Err(SendError::Closed));
        assert!(from.try_recv().is_err());
    }

    fn resized() -> ClientFrame {
        ClientFrame::Resize {
            id: 0,
            cols: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        }
    }

    fn resized_to(cols: u16, rows: u16) -> ClientFrame {
        ClientFrame::Resize {
            id: 0,
            cols,
            rows,
            cell_width: 8,
            cell_height: 16,
        }
    }

    #[test]
    fn a_resize_before_an_attach_is_up_is_accepted_and_kept_for_it() {
        let (gate, sink, mut from) = link();
        sink.send(resized_to(97, 31)).expect("a resize needs no attach to be up");
        assert!(from.try_recv().is_err(), "nothing is sent while no attach is up");
        let kept = gate.last_resize().as_ref().and_then(size_of);
        assert_eq!(kept, Some((97, 31)));
    }

    #[test]
    fn a_resize_while_reattaching_is_kept_for_the_next_attach() {
        let (gate, sink, mut from) = link();
        up(&gate);
        sink.send(resized_to(80, 24)).expect("a resize is sent");
        let _ = from.try_recv().expect("it is queued");
        gate.close();
        sink.send(resized_to(120, 40)).expect("a resize while reattaching is accepted");
        assert!(from.try_recv().is_err());
        let kept = gate.last_resize().as_ref().and_then(size_of);
        assert_eq!(kept, Some((120, 40)), "the latest size is the one kept");
    }

    /// An attach whose screen has arrived.
    fn up(gate: &Gate) {
        gate.open();
        gate.synced();
    }

    #[test]
    fn input_goes_out_while_an_attach_is_up_and_synced() {
        let (gate, sink, mut from) = link();
        up(&gate);
        sink.send(typed()).expect("input is sent");
        let (generation, _) = from.try_recv().expect("it is queued");
        assert!(gate.is_current(generation));
    }

    #[test]
    fn input_waits_for_the_screen_but_a_resize_does_not() {
        let (gate, sink, mut from) = link();
        gate.open();
        assert_eq!(sink.send(typed()), Err(SendError::Closed));
        assert!(from.try_recv().is_err());
        sink.send(resized()).expect("a resize is sent");
        let (generation, _) = from.try_recv().expect("it is queued");
        assert!(gate.is_current(generation));
        gate.synced();
        sink.send(typed()).expect("input is sent once synced");
    }

    #[test]
    fn a_screen_before_any_attach_is_up_does_not_open_the_gate() {
        let (gate, sink, _from) = link();
        gate.synced();
        assert_eq!(sink.send(typed()), Err(SendError::Closed));
    }

    #[test]
    fn input_while_reattaching_fails_and_is_never_queued() {
        let (gate, sink, mut from) = link();
        up(&gate);
        gate.close();
        assert_eq!(sink.send(typed()), Err(SendError::Closed));
        assert!(from.try_recv().is_err());
    }

    #[test]
    fn input_in_flight_when_the_attach_dropped_is_stale_in_the_next_one() {
        let (gate, sink, mut from) = link();
        up(&gate);
        sink.send(typed()).expect("input is sent");
        let (stale, _) = from.try_recv().expect("it is queued");
        gate.close();
        up(&gate);
        assert!(!gate.is_current(stale));
    }

    #[test]
    fn input_in_the_next_attach_waits_for_its_own_screen() {
        let (gate, sink, _from) = link();
        up(&gate);
        gate.close();
        gate.open();
        assert_eq!(sink.send(typed()), Err(SendError::Closed));
        gate.synced();
        sink.send(typed()).expect("input is sent");
    }

    #[test]
    fn input_after_the_new_attach_is_up_goes_out_again() {
        let (gate, sink, mut from) = link();
        up(&gate);
        gate.close();
        up(&gate);
        sink.send(typed()).expect("input is sent");
        let (generation, _) = from.try_recv().expect("it is queued");
        assert!(gate.is_current(generation));
    }

    #[test]
    fn input_after_the_pump_is_gone_fails_as_closed() {
        let (gate, sink, from) = link();
        up(&gate);
        drop(from);
        assert_eq!(sink.send(typed()), Err(SendError::Closed));
    }

    #[test]
    fn the_first_screen_seen_is_the_links_identity() {
        let mut identity = Identity::default();
        assert_eq!(identity.check("holder-a"), Verdict::Keep);
    }

    fn acked(offset: u64) -> ClientFrame {
        ClientFrame::Ack { id: 0, offset }
    }

    #[test]
    fn a_flood_of_acknowledgements_queues_one_that_goes_as_the_latest() {
        let (gate, sink, mut from) = link();
        gate.open();
        gate.synced();
        for offset in 1..=100 {
            sink.send(acked(offset)).expect("an acknowledgement is accepted");
        }
        sink.send(typed()).expect("typing still finds room");

        let (_, queued) = from.try_recv().expect("one acknowledgement is queued");
        assert!(matches!(queued, ClientFrame::Ack { .. }));
        assert!(matches!(from.try_recv(), Ok((_, ClientFrame::Input { .. }))));
        assert!(from.try_recv().is_err(), "nothing else is queued");
        assert!(matches!(gate.take_ack(), Some((_, ClientFrame::Ack { offset: 100, .. }))));
    }

    #[test]
    fn an_acknowledgement_after_the_queued_one_was_taken_is_queued_again() {
        let (gate, sink, mut from) = link();
        gate.open();
        sink.send(acked(1)).expect("an acknowledgement is accepted");
        assert!(from.try_recv().is_ok());
        assert!(gate.take_ack().is_some());

        sink.send(acked(2)).expect("an acknowledgement is accepted");
        assert!(from.try_recv().is_ok(), "the next one is queued");
        assert!(matches!(gate.take_ack(), Some((_, ClientFrame::Ack { offset: 2, .. }))));
    }

    #[test]
    fn a_reattach_showing_the_same_screen_is_kept() {
        let mut identity = Identity::default();
        identity.check("holder-a");
        assert_eq!(identity.check("holder-a"), Verdict::Keep);
    }

    #[test]
    fn a_screen_from_another_holder_replaces_the_link() {
        let mut identity = Identity::default();
        identity.check("holder-a");
        assert_eq!(identity.check("holder-b"), Verdict::Replaced);
    }

    #[test]
    fn a_screen_without_a_uuid_after_one_with_a_uuid_replaces_the_link() {
        let mut identity = Identity::default();
        identity.check("holder-a");
        assert_eq!(identity.check(""), Verdict::Replaced);
    }

    #[test]
    fn a_screen_with_a_uuid_after_one_without_a_uuid_replaces_the_link() {
        let mut identity = Identity::default();
        identity.check("");
        assert_eq!(identity.check("holder-a"), Verdict::Replaced);
    }
}
