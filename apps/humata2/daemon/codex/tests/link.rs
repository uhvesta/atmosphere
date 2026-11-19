//! The wrapper's link between Codex's app-server and Humata (D795), against
//! a fake app-server on a unix socket speaking its websocket JSON-RPC: the
//! link says hello, finds the TUI's thread and subscribes to it; it names
//! the thread as the conversation and again when `/new` starts another;
//! reports turns, an interrupted one as stopped; raises each approval and
//! question in the Inbox and sends the person's answer back; lets go of an
//! ask answered in the TUI first; sends nothing when no one decided; and
//! connects again when the app-server drops it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use humata2_codex::{AskKind, Humata, HumataError, Turn, run_link};
use serde_json::{Value, json};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

const THREAD: &str = "019b2c4d-7e8f-7a01-b234-56789abcdef0";
const NEW_THREAD: &str = "019b2c4f-1111-7000-8000-000000000002";
const RETRY: Duration = Duration::from_millis(100);
const WAIT: Duration = Duration::from_secs(10);

/// One ask the link raised: what kind, its payload, and where the test
/// answers it. Dropped by the link when the ask is withdrawn.
struct AskCall {
    kind: AskKind,
    payload: Value,
    answer: oneshot::Sender<Vec<u8>>,
    withdrawn: oneshot::Receiver<()>,
}

/// What the link told Humata.
#[derive(Debug, PartialEq, Eq)]
enum Told {
    Conversation(String),
    Turn(Turn),
    /// The link to a named thread dropped.
    Lost,
}

struct FakeHumata {
    told: mpsc::UnboundedSender<Told>,
    asks: mpsc::UnboundedSender<AskCall>,
}

/// Fires when the ask's future is dropped before it was answered.
struct OnDrop(Option<oneshot::Sender<()>>);

impl Drop for OnDrop {
    fn drop(&mut self) {
        if let Some(fire) = self.0.take() {
            let _ = fire.send(());
        }
    }
}

impl Humata for FakeHumata {
    async fn turn(&self, turn: Turn) {
        let _ = self.told.send(Told::Turn(turn));
    }

    async fn conversation(&self, thread: String) {
        let _ = self.told.send(Told::Conversation(thread));
    }

    async fn lost(&self) {
        let _ = self.told.send(Told::Lost);
    }

    async fn ask(&self, kind: AskKind, payload: Vec<u8>) -> Result<Vec<u8>, HumataError> {
        let payload = serde_json::from_slice(&payload).expect("the link sends JSON");
        let (answer, answered) = oneshot::channel();
        let (fire, withdrawn) = oneshot::channel();
        let mut guard = OnDrop(Some(fire));
        let _ = self.asks.send(AskCall {
            kind,
            payload,
            answer,
            withdrawn,
        });
        let body = answered.await.unwrap_or_default();
        guard.0 = None;
        Ok(body)
    }
}

/// The fake app-server's side of one connection.
struct Peer {
    ws: WebSocketStream<UnixStream>,
}

impl Peer {
    async fn accept(listener: &UnixListener) -> Self {
        let (stream, _) = tokio::time::timeout(WAIT, listener.accept())
            .await
            .expect("the link connects")
            .expect("accept");
        let ws = tokio_tungstenite::accept_async(stream).await.expect("a websocket handshake");
        Self { ws }
    }

    async fn next(&mut self) -> Value {
        loop {
            let message = tokio::time::timeout(WAIT, self.ws.next())
                .await
                .expect("the link sends something")
                .expect("the link is connected")
                .expect("a frame");
            if let Message::Text(text) = message {
                return serde_json::from_str(&text).expect("the link sends JSON");
            }
        }
    }

    async fn send(&mut self, frame: Value) {
        self.ws.send(Message::text(frame.to_string())).await.expect("send");
    }

    /// Answers the link's hello and its look for the loaded threads, and
    /// checks it subscribes to `thread`.
    async fn handshake(&mut self, loaded: &[&str]) {
        let hello = self.next().await;
        assert_eq!(hello["method"], "initialize");
        assert!(hello["params"]["clientInfo"]["name"].is_string());
        self.send(json!({"jsonrpc":"2.0","id":hello["id"],"result":{"userAgent":"codex/0.169.0"}}))
            .await;
        let initialized = self.next().await;
        assert_eq!(initialized["method"], "initialized");
        assert!(initialized.get("id").is_none(), "initialized is a notification");
        let list = self.next().await;
        assert_eq!(list["method"], "thread/loaded/list");
        self.send(json!({"jsonrpc":"2.0","id":list["id"],"result":{"data":loaded,"nextCursor":null}}))
            .await;
        for thread in loaded {
            let resume = self.next().await;
            assert_eq!(resume["method"], "thread/resume");
            assert_eq!(resume["params"]["threadId"], *thread);
            self.send(json!({"jsonrpc":"2.0","id":resume["id"],"result":{"thread":{"id":thread}}}))
                .await;
        }
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    listener: UnixListener,
    told: mpsc::UnboundedReceiver<Told>,
    asks: mpsc::UnboundedReceiver<AskCall>,
    link: tokio::task::JoinHandle<()>,
}

impl Rig {
    fn start() -> Self {
        let dir = tempfile::tempdir().expect("a folder");
        let socket: PathBuf = dir.path().join("codex.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let (told_tx, told) = mpsc::unbounded_channel();
        let (asks_tx, asks) = mpsc::unbounded_channel();
        let humata = Arc::new(FakeHumata {
            told: told_tx,
            asks: asks_tx,
        });
        let link = tokio::spawn(run_link(socket_of(dir.path()), humata, RETRY));
        Self {
            _dir: dir,
            listener,
            told,
            asks,
            link,
        }
    }

    async fn told(&mut self) -> Told {
        tokio::time::timeout(WAIT, self.told.recv())
            .await
            .expect("the link tells Humata something")
            .expect("the link runs")
    }

    async fn ask(&mut self) -> AskCall {
        tokio::time::timeout(WAIT, self.asks.recv())
            .await
            .expect("the link raises an ask")
            .expect("the link runs")
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.link.abort();
    }
}

fn socket_of(dir: &Path) -> PathBuf {
    dir.join("codex.sock")
}

fn command_approval(id: u64, item: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"item/commandExecution/requestApproval","params":{
        "threadId":THREAD,"turnId":"7","itemId":item,"startedAtMs":1,
        "command":"cargo publish --dry-run","cwd":"/work/api","reason":"Check the crate packages"}})
}

#[tokio::test]
async fn the_tuis_thread_is_the_conversation_and_new_moves_it() {
    let mut rig = Rig::start();
    let mut peer = Peer::accept(&rig.listener).await;
    peer.handshake(&[THREAD]).await;
    assert_eq!(rig.told().await, Told::Conversation(THREAD.to_owned()));

    peer.send(json!({"jsonrpc":"2.0","method":"thread/started","params":{"thread":{
        "id":NEW_THREAD,"path":null,"parentThreadId":null,"turns":[]}}}))
        .await;
    assert_eq!(rig.told().await, Told::Conversation(NEW_THREAD.to_owned()));
}

#[tokio::test]
async fn turns_are_reported_and_an_interrupted_one_is_stopped() {
    let mut rig = Rig::start();
    let mut peer = Peer::accept(&rig.listener).await;
    peer.handshake(&[THREAD]).await;
    assert_eq!(rig.told().await, Told::Conversation(THREAD.to_owned()));

    let turn = |method: &str, status: &str| {
        json!({"jsonrpc":"2.0","method":method,"params":{"threadId":THREAD,
            "turn":{"id":"7","status":status,"items":[]}}})
    };
    peer.send(turn("turn/started", "inProgress")).await;
    assert_eq!(rig.told().await, Told::Turn(Turn::Working));
    peer.send(turn("turn/completed", "completed")).await;
    assert_eq!(rig.told().await, Told::Turn(Turn::Idle));
    assert_eq!(rig.told().await, Told::Conversation(THREAD.to_owned()));
    peer.send(turn("turn/started", "inProgress")).await;
    assert_eq!(rig.told().await, Told::Turn(Turn::Working));
    peer.send(turn("turn/completed", "interrupted")).await;
    assert_eq!(rig.told().await, Told::Turn(Turn::Stopped));
}

/// Codex writes the rollout of a thread `/new` started only at its first
/// prompt, after `thread/started` named it: the thread is named again when
/// its turn ends, so Humata can follow it once its rollout is written.
#[tokio::test]
async fn a_new_thread_is_named_again_after_its_first_turn() {
    let mut rig = Rig::start();
    let mut peer = Peer::accept(&rig.listener).await;
    peer.handshake(&[THREAD]).await;
    assert_eq!(rig.told().await, Told::Conversation(THREAD.to_owned()));
    peer.send(json!({"jsonrpc":"2.0","method":"thread/started","params":{"thread":{
        "id":NEW_THREAD,"path":null,"parentThreadId":null,"turns":[]}}}))
        .await;
    assert_eq!(rig.told().await, Told::Conversation(NEW_THREAD.to_owned()));

    let turn = |method: &str, status: &str| {
        json!({"jsonrpc":"2.0","method":method,"params":{"threadId":NEW_THREAD,
            "turn":{"id":"1","status":status,"items":[]}}})
    };
    peer.send(turn("turn/started", "inProgress")).await;
    assert_eq!(rig.told().await, Told::Turn(Turn::Working));
    peer.send(turn("turn/completed", "completed")).await;
    assert_eq!(rig.told().await, Told::Turn(Turn::Idle));
    assert_eq!(rig.told().await, Told::Conversation(NEW_THREAD.to_owned()));
}

#[tokio::test]
async fn an_approval_goes_to_the_inbox_and_its_answer_back_to_codex() {
    let mut rig = Rig::start();
    let mut peer = Peer::accept(&rig.listener).await;
    peer.handshake(&[THREAD]).await;
    let _ = rig.told().await;

    peer.send(command_approval(0, "call_a1")).await;
    let ask = rig.ask().await;
    assert_eq!(ask.kind, AskKind::Command);
    // The Inbox reads it as Codex's own permission hook would send it.
    assert_eq!(ask.payload["session_id"], THREAD);
    assert_eq!(ask.payload["tool_input"]["command"], "cargo publish --dry-run");
    // Humata answers as it answers Codex's hook: allow.
    let allow = json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest",
        "decision":{"behavior":"allow"}}});
    ask.answer.send(allow.to_string().into_bytes()).expect("the link waits");
    let reply = peer.next().await;
    assert_eq!(reply["id"], 0);
    assert_eq!(reply["result"]["decision"], "accept");

    peer.send(command_approval(1, "call_a2")).await;
    let ask = rig.ask().await;
    let deny = json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest",
        "decision":{"behavior":"deny","message":"not now"}}});
    ask.answer.send(deny.to_string().into_bytes()).expect("the link waits");
    let reply = peer.next().await;
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["result"]["decision"], "decline");
}

#[tokio::test]
async fn a_question_goes_to_the_inbox_and_its_answers_back_to_codex() {
    let mut rig = Rig::start();
    let mut peer = Peer::accept(&rig.listener).await;
    peer.handshake(&[THREAD]).await;
    let _ = rig.told().await;

    peer.send(json!({"jsonrpc":"2.0","id":"q-1","method":"item/tool/requestUserInput","params":{
        "threadId":THREAD,"turnId":"7","itemId":"call_9f8e","isBlocking":true,
        "questions":[{"id":"deploy_target","header":"Deploy","question":"Which environment?",
            "isOther":true,"isSecret":false,"options":[{"label":"staging","description":"x"}]}]}}))
        .await;
    let ask = rig.ask().await;
    assert_eq!(ask.kind, AskKind::Question);
    assert_eq!(ask.payload["questions"][0]["id"], "deploy_target");
    let answers = json!({"answers":{"deploy_target":{"answers":["staging"]}}});
    ask.answer.send(answers.to_string().into_bytes()).expect("the link waits");
    let reply = peer.next().await;
    assert_eq!(reply["id"], "q-1");
    assert_eq!(reply["result"], answers);
}

#[tokio::test]
async fn an_ask_answered_in_the_tui_first_is_withdrawn_from_the_inbox() {
    let mut rig = Rig::start();
    let mut peer = Peer::accept(&rig.listener).await;
    peer.handshake(&[THREAD]).await;
    let _ = rig.told().await;

    peer.send(command_approval(4, "call_a4")).await;
    let ask = rig.ask().await;
    peer.send(json!({"jsonrpc":"2.0","method":"serverRequest/resolved",
        "params":{"threadId":THREAD,"requestId":4}}))
        .await;
    tokio::time::timeout(WAIT, ask.withdrawn)
        .await
        .expect("the ask is withdrawn")
        .expect("withdrawn, not answered");
}

#[tokio::test]
async fn no_decision_sends_codex_nothing_so_its_popup_answers() {
    let mut rig = Rig::start();
    let mut peer = Peer::accept(&rig.listener).await;
    peer.handshake(&[THREAD]).await;
    let _ = rig.told().await;

    peer.send(command_approval(5, "call_a5")).await;
    let ask = rig.ask().await;
    ask.answer.send(Vec::new()).expect("the link waits");
    // The next thing the link sends is about the next ask, not this one.
    peer.send(command_approval(6, "call_a6")).await;
    let next = rig.ask().await;
    next.answer
        .send(json!({"hookSpecificOutput":{"decision":{"behavior":"allow"}}}).to_string().into_bytes())
        .expect("the link waits");
    let reply = peer.next().await;
    assert_eq!(reply["id"], 6, "the undecided ask was answered: {reply}");
}

#[tokio::test]
async fn a_dropped_app_server_is_told_connected_again_and_its_thread_named_again() {
    let mut rig = Rig::start();
    let mut peer = Peer::accept(&rig.listener).await;
    peer.handshake(&[THREAD]).await;
    assert_eq!(rig.told().await, Told::Conversation(THREAD.to_owned()));
    drop(peer);
    // D798: Humata hears of the drop, so its screen fallback is back.
    assert_eq!(rig.told().await, Told::Lost);

    let mut again = Peer::accept(&rig.listener).await;
    again.handshake(&[THREAD]).await;
    // Named again, so Humata hears the session once more.
    assert_eq!(rig.told().await, Told::Conversation(THREAD.to_owned()));
    again
        .send(json!({"jsonrpc":"2.0","method":"turn/started","params":{"threadId":THREAD,
            "turn":{"id":"9","status":"inProgress","items":[]}}}))
        .await;
    assert_eq!(rig.told().await, Told::Turn(Turn::Working));
}

#[tokio::test]
async fn a_thread_started_before_any_prompt_is_named_once_loaded() {
    // A TUI whose thread isn't loaded yet: the link waits for thread/started.
    let mut rig = Rig::start();
    let mut peer = Peer::accept(&rig.listener).await;
    peer.handshake(&[]).await;
    peer.send(json!({"jsonrpc":"2.0","method":"thread/started","params":{"thread":{
        "id":THREAD,"path":null,"parentThreadId":null,"turns":[]}}}))
        .await;
    assert_eq!(rig.told().await, Told::Conversation(THREAD.to_owned()));
}
