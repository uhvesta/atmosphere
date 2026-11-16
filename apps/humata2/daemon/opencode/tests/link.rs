//! The wrapper's link between OpenCode's server and Humata (D201a), with
//! both sides faked: turns are reported as they change, each ask goes to the
//! Inbox and its answer back to OpenCode, an ask the TUI answered first lets
//! go of its card, and a dropped stream or daemon is reconnected.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use humata2_opencode::{Asked, Event, Humata, HumataError, Server, ServerError, Turn, run_link};
use tokio::sync::{mpsc, oneshot};

const RETRY: Duration = Duration::from_millis(500);
/// Long enough for every retry the link would make, on paused time.
const SETTLE: Duration = Duration::from_secs(5);

fn asked(id: &str, session: &str) -> Asked {
    Asked {
        id: id.to_owned(),
        session: session.to_owned(),
        action: "shell".to_owned(),
        resources: vec!["echo hi".to_owned()],
    }
}

/// One `ask` the link made: its payload, and where the test answers it.
struct AskCall {
    payload: serde_json::Value,
    answer: oneshot::Sender<Result<Vec<u8>, HumataError>>,
}

struct FakeHumata {
    turns: mpsc::UnboundedSender<Turn>,
    asks: mpsc::UnboundedSender<AskCall>,
}

impl Humata for FakeHumata {
    async fn turn(&self, turn: Turn) {
        let _ = self.turns.send(turn);
    }

    async fn ask(&self, payload: Vec<u8>) -> Result<Vec<u8>, HumataError> {
        let payload = serde_json::from_slice(&payload).expect("the link sends JSON");
        let (answer, answered) = oneshot::channel();
        let _ = self.asks.send(AskCall { payload, answer });
        match answered.await {
            Ok(result) => result,
            // The test let go without answering: no decision.
            Err(_) => Ok(Vec::new()),
        }
    }
}

/// One reply the link posted.
#[derive(Debug, PartialEq, Eq)]
struct Posted {
    session: String,
    request: String,
    body: serde_json::Value,
}

#[derive(Default)]
struct ServerState {
    /// Each `events` call takes the next stream; none left is an error.
    streams: VecDeque<mpsc::Receiver<Event>>,
    active: Vec<String>,
    pending: Vec<Asked>,
    /// What each reply returns, in order; none left is success.
    replies: VecDeque<Result<(), ServerError>>,
    connects: usize,
}

struct FakeServer {
    state: Mutex<ServerState>,
    posted: mpsc::UnboundedSender<Posted>,
}

impl FakeServer {
    fn with<T>(&self, change: impl FnOnce(&mut ServerState) -> T) -> T {
        let mut state = self.state.lock().expect("the fake's lock");
        change(&mut state)
    }
}

impl Server for FakeServer {
    async fn events(&self) -> Result<mpsc::Receiver<Event>, ServerError> {
        self.with(|state| {
            state.connects += 1;
            state.streams.pop_front().ok_or(ServerError::Status {
                code: 503,
                body: "no stream".to_owned(),
            })
        })
    }

    async fn active(&self) -> Result<Vec<String>, ServerError> {
        Ok(self.with(|state| state.active.clone()))
    }

    async fn pending(&self) -> Result<Vec<Asked>, ServerError> {
        Ok(self.with(|state| state.pending.clone()))
    }

    async fn reply(&self, session: &str, request: &str, body: Vec<u8>) -> Result<(), ServerError> {
        let body = serde_json::from_slice(&body).expect("the link posts JSON");
        let _ = self.posted.send(Posted {
            session: session.to_owned(),
            request: request.to_owned(),
            body,
        });
        self.with(|state| state.replies.pop_front().unwrap_or(Ok(())))
    }
}

struct Rig {
    server: Arc<FakeServer>,
    turns: mpsc::UnboundedReceiver<Turn>,
    asks: mpsc::UnboundedReceiver<AskCall>,
    posted: mpsc::UnboundedReceiver<Posted>,
    link: tokio::task::JoinHandle<()>,
}

impl Rig {
    /// A link whose server gives `streams` in turn, set up by `setup`.
    fn start(streams: Vec<mpsc::Receiver<Event>>, setup: impl FnOnce(&mut ServerState)) -> Self {
        let (posted_tx, posted) = mpsc::unbounded_channel();
        let server = Arc::new(FakeServer {
            state: Mutex::new(ServerState {
                streams: streams.into(),
                ..ServerState::default()
            }),
            posted: posted_tx,
        });
        server.with(setup);
        let (turns_tx, turns) = mpsc::unbounded_channel();
        let (asks_tx, asks) = mpsc::unbounded_channel();
        let humata = Arc::new(FakeHumata {
            turns: turns_tx,
            asks: asks_tx,
        });
        let link = tokio::spawn(run_link(Arc::clone(&server), humata, RETRY));
        Self {
            server,
            turns,
            asks,
            posted,
            link,
        }
    }

    async fn next_turn(&mut self) -> Turn {
        self.turns.recv().await.expect("a turn report")
    }

    async fn next_ask(&mut self) -> AskCall {
        self.asks.recv().await.expect("an ask")
    }

    async fn next_post(&mut self) -> Posted {
        self.posted.recv().await.expect("a reply")
    }

    /// Lets the link run until it has nothing left to do.
    async fn settle(&self) {
        tokio::time::sleep(SETTLE).await;
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.link.abort();
    }
}

fn stream() -> (mpsc::Sender<Event>, mpsc::Receiver<Event>) {
    mpsc::channel(16)
}

async fn send(events: &mpsc::Sender<Event>, event: Event) {
    events.send(event).await.expect("the link reads its stream");
}

#[tokio::test(start_paused = true)]
async fn turns_are_reported_as_the_server_runs_and_stops() {
    let (events, receiver) = stream();
    let mut rig = Rig::start(vec![receiver], |_| {});
    // What the server says on connecting comes first.
    assert_eq!(rig.next_turn().await, Turn::Idle);
    send(&events, Event::Started { session: "a".to_owned() }).await;
    assert_eq!(rig.next_turn().await, Turn::Working);
    send(&events, Event::Ended { session: "a".to_owned() }).await;
    assert_eq!(rig.next_turn().await, Turn::Idle);
}

#[tokio::test(start_paused = true)]
async fn an_ask_goes_to_the_inbox_and_its_answer_back_to_its_own_session() {
    let (events, receiver) = stream();
    let mut rig = Rig::start(vec![receiver], |_| {});
    rig.next_turn().await;
    send(&events, Event::Started { session: "parent".to_owned() }).await;
    // Asks can come from any session on the server.
    send(&events, Event::Asked(asked("per_1", "child"))).await;
    let call = rig.next_ask().await;
    assert_eq!(call.payload["id"], "per_1");
    assert_eq!(call.payload["action"], "shell");
    assert_eq!(call.payload["resources"][0], "echo hi");
    let body = serde_json::json!({"decision": "once"});
    let bytes = serde_json::to_vec(&body).expect("JSON");
    call.answer.send(Ok(bytes)).expect("the link waits");
    assert_eq!(
        rig.next_post().await,
        Posted {
            session: "child".to_owned(),
            request: "per_1".to_owned(),
            body,
        }
    );
}

#[tokio::test(start_paused = true)]
async fn no_decision_posts_nothing_and_asks_no_more() {
    let (events, receiver) = stream();
    let mut rig = Rig::start(vec![receiver], |_| {});
    send(&events, Event::Asked(asked("per_1", "a"))).await;
    let call = rig.next_ask().await;
    call.answer.send(Ok(Vec::new())).expect("the link waits");
    rig.settle().await;
    // The TUI's own prompt is left to ask; the card isn't raised again.
    assert!(rig.posted.try_recv().is_err());
    assert!(rig.asks.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn losing_the_daemon_mid_ask_asks_again_until_it_answers() {
    let (events, receiver) = stream();
    let mut rig = Rig::start(vec![receiver], |_| {});
    send(&events, Event::Asked(asked("per_1", "a"))).await;
    let lost = rig.next_ask().await;
    lost.answer
        .send(Err(HumataError::Lost { reason: "daemon restarted".to_owned() }))
        .expect("the link waits");
    let again = rig.next_ask().await;
    assert_eq!(again.payload["id"], "per_1");
    let bytes = serde_json::to_vec(&serde_json::json!({"decision": "once"})).expect("JSON");
    again.answer.send(Ok(bytes)).expect("the link waits");
    assert_eq!(rig.next_post().await.request, "per_1");
}

#[tokio::test(start_paused = true)]
async fn the_tui_answering_first_lets_go_of_the_card() {
    let (events, receiver) = stream();
    let mut rig = Rig::start(vec![receiver], |_| {});
    send(&events, Event::Started { session: "a".to_owned() }).await;
    send(&events, Event::Asked(asked("per_1", "a"))).await;
    let mut call = rig.next_ask().await;
    send(
        &events,
        Event::Replied {
            session: "a".to_owned(),
            request: "per_1".to_owned(),
        },
    )
    .await;
    // Letting go of the ask is what withdraws its card.
    tokio::time::timeout(SETTLE, call.answer.closed())
        .await
        .expect("the link let go of the ask");
    rig.settle().await;
    assert!(rig.posted.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn a_run_ending_lets_go_of_its_asks() {
    let (events, receiver) = stream();
    let mut rig = Rig::start(vec![receiver], |_| {});
    send(&events, Event::Started { session: "a".to_owned() }).await;
    send(&events, Event::Asked(asked("per_1", "a"))).await;
    let mut call = rig.next_ask().await;
    send(&events, Event::Ended { session: "a".to_owned() }).await;
    tokio::time::timeout(SETTLE, call.answer.closed())
        .await
        .expect("the link let go of the ask");
}

#[tokio::test(start_paused = true)]
async fn an_answer_the_tui_beat_is_not_retried() {
    let (events, receiver) = stream();
    let mut rig = Rig::start(vec![receiver], |state| {
        state.replies.push_back(Err(ServerError::Gone));
    });
    send(&events, Event::Asked(asked("per_1", "a"))).await;
    let call = rig.next_ask().await;
    let bytes = serde_json::to_vec(&serde_json::json!({"decision": "once"})).expect("JSON");
    call.answer.send(Ok(bytes)).expect("the link waits");
    rig.next_post().await;
    rig.settle().await;
    assert!(rig.posted.try_recv().is_err());
    assert!(rig.asks.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn a_dropped_stream_reconnects_and_takes_up_what_it_missed() {
    let (first, receiver) = stream();
    let (_second, again) = stream();
    let mut rig = Rig::start(vec![receiver, again], |_| {});
    assert_eq!(rig.next_turn().await, Turn::Idle);
    send(&first, Event::Started { session: "a".to_owned() }).await;
    assert_eq!(rig.next_turn().await, Turn::Working);
    // While the stream is down, "a" ends and "b" asks.
    rig.server.with(|state| {
        state.active = vec!["b".to_owned()];
        state.pending = vec![asked("per_9", "b")];
    });
    drop(first);
    assert_eq!(rig.next_turn().await, Turn::NeedsInput);
    let call = rig.next_ask().await;
    assert_eq!(call.payload["id"], "per_9");
    assert_eq!(rig.server.with(|state| state.connects), 2);
}

#[tokio::test(start_paused = true)]
async fn an_ask_still_open_across_a_reconnect_is_raised_once() {
    let (first, receiver) = stream();
    let (_second, again) = stream();
    let mut rig = Rig::start(vec![receiver, again], |_| {});
    send(&first, Event::Asked(asked("per_1", "a"))).await;
    let call = rig.next_ask().await;
    rig.server.with(|state| {
        state.active = vec!["a".to_owned()];
        state.pending = vec![asked("per_1", "a")];
    });
    drop(first);
    rig.settle().await;
    assert!(rig.asks.try_recv().is_err(), "the same ask was raised twice");
    // The first hold is still the one answering.
    assert!(!call.answer.is_closed());
}

#[tokio::test(start_paused = true)]
async fn a_server_that_isnt_up_yet_is_tried_again() {
    let mut rig = Rig::start(Vec::new(), |_| {});
    tokio::time::sleep(RETRY * 3).await;
    // Nothing is reported while there's no server to say what it's doing.
    assert!(rig.turns.try_recv().is_err());
    let (_events, receiver) = stream();
    rig.server.with(|state| state.streams.push_back(receiver));
    assert_eq!(rig.next_turn().await, Turn::Idle);
    assert!(rig.server.with(|state| state.connects) >= 2);
}

#[tokio::test(start_paused = true)]
async fn a_stream_that_ends_at_once_is_opened_again() {
    let (_events, receiver) = stream();
    let ended = mpsc::channel::<Event>(1).1;
    let mut rig = Rig::start(vec![ended, receiver], |_| {});
    rig.next_turn().await;
    rig.settle().await;
    assert_eq!(rig.server.with(|state| state.connects), 2);
}
