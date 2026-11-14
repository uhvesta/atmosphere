//! Agents on a host reach the board and review threads, which are the
//! daemon's, through the host: `humatad mcp` calls this relay on a socket in
//! the host's state folder, and the relay hands each call to the daemon
//! watching over its connection, then the daemon's answer back.
//!
//! While no daemon watches, as when the Mac is asleep, an agent's questions
//! and move requests wait in a file in the host's state folder, and the next
//! daemon to watch gets them in order.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use humata_wire::{Reply, Service, ServiceFuture, describe, error, pb};
use prost::Message;
use tokio::sync::{mpsc, oneshot};

use crate::error::{HostError, RelayError};

/// How long a call waits for the daemon's answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

/// The most questions and move requests of one session that wait for a
/// daemon. The file is written whole for each, and an agent that keeps
/// asking while the Mac is away would otherwise fill it without end.
const MOST_QUEUED_OF_A_SESSION: usize = 8;

/// The session whose question or move request `request` is; None for any
/// other call.
fn asking_session(request: &pb::request::Kind) -> Option<&str> {
    match request {
        pb::request::Kind::AskPerson(ask) => Some(&ask.session_id),
        pb::request::Kind::RequestMove(asked) => Some(&asked.session_id),
        _ => None,
    }
}

pub struct Relay {
    state: Mutex<State>,
    /// Where the calls made while no daemon watched wait for the next one.
    queue_file: PathBuf,
}

#[derive(Default)]
struct State {
    next: u64,
    /// The daemon's stream of calls, and which watch it is.
    watcher: Option<(u64, mpsc::Sender<Result<pb::StreamItem, pb::Error>>)>,
    waiting: HashMap<u64, oneshot::Sender<pb::response::Kind>>,
    /// The daemon's copy of the specs each session reads, by session, so
    /// its agent reads them while the Mac is away. Only in memory: the
    /// daemon copies them again when it next watches the session.
    specs: HashMap<String, Vec<pb::Spec>>,
    /// The calls waiting for a daemon, oldest first, each by its key.
    queued: Vec<(u64, pb::Request)>,
}

impl Relay {
    /// The relay whose queue is `queue_file`, with the calls already in it.
    /// A file it can't read is moved aside, so the host still starts.
    pub fn open(queue_file: PathBuf) -> Self {
        let mut state = State::default();
        match read_queue(&queue_file) {
            Ok(requests) => {
                for request in requests {
                    state.next += 1;
                    state.queued.push((state.next, request));
                }
            }
            Err(failure) => {
                let aside = queue_file.with_extension("unreadable");
                let _ = std::fs::rename(&queue_file, &aside);
                eprintln!(
                    "humatad: {}; moved it to {}, whose questions and move requests no one will see unless asked again",
                    describe(&failure),
                    aside.display()
                );
            }
        }
        Self {
            state: Mutex::new(state),
            queue_file,
        }
    }

    /// The stream of calls for the daemon, replacing any earlier one, whose
    /// calls waiting are dropped since that daemon can no longer answer them.
    pub fn watch(self: &Arc<Self>) -> Reply {
        let (items, receiver) = mpsc::channel(64);
        let watch = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.next += 1;
            let watch = state.next;
            state.watcher = Some((watch, items.clone()));
            state.waiting.clear();
            watch
        };
        let replaying = self.clone();
        tokio::spawn(async move { replaying.replay().await });
        let relay = Arc::downgrade(self);
        tokio::spawn(async move {
            items.closed().await;
            if let Some(relay) = relay.upgrade() {
                let mut state = relay.state.lock().unwrap_or_else(PoisonError::into_inner);
                if state.watcher.as_ref().is_some_and(|(id, _)| *id == watch) {
                    state.watcher = None;
                    state.waiting.clear();
                }
            }
        });
        Reply::Stream {
            response: pb::response::Kind::ToolCallsWatched(pb::ToolCallsWatched {}),
            items: receiver,
        }
    }

    /// Hands the daemon's answer to the call waiting for it.
    pub fn answer(&self, answer: pb::AnswerToolCall) -> Result<(), HostError> {
        let waiting = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .waiting
            .remove(&answer.id);
        let (Some(waiting), Some(response)) = (waiting, answer.response.and_then(|r| r.kind))
        else {
            return Err(HostError::UnknownToolCall { id: answer.id });
        };
        // A call that gave up waiting has nothing left to answer.
        let _ = waiting.send(response);
        Ok(())
    }

    pub fn set_specs(&self, set: pb::SetSpecs) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if set.specs.is_empty() {
            state.specs.remove(&set.session_id);
        } else {
            state.specs.insert(set.session_id, set.specs);
        }
    }

    /// The host's copy of a spec its session reads; None if it has none.
    fn copied_spec(&self, read: &pb::ReadSpec) -> Option<pb::Spec> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .specs
            .get(&read.session_id)?
            .iter()
            .find(|spec| spec.item_id == read.item_id)
            .cloned()
    }

    /// Hands the queued calls to the daemon now watching, in order, each
    /// taken off the queue once the daemon answers it. One unanswered stops
    /// the replay, and the next watch starts over from it.
    async fn replay(&self) {
        loop {
            let next = {
                let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.queued.first().cloned()
            };
            let Some((key, request)) = next else {
                return;
            };
            let Some(kind) = request.kind else {
                self.unqueue(key);
                continue;
            };
            if self.send(kind).await.is_err() {
                return;
            }
            self.unqueue(key);
        }
    }

    /// Adds `request` to the end of the queue, kept in the file, unless its
    /// session already has the most that may wait.
    fn enqueue(&self, request: pb::request::Kind) -> Result<(), RelayError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(session) = asking_session(&request) {
            let waiting = state
                .queued
                .iter()
                .filter_map(|(_, queued)| queued.kind.as_ref().and_then(asking_session))
                .filter(|queued| *queued == session)
                .count();
            if waiting >= MOST_QUEUED_OF_A_SESSION {
                return Err(RelayError::QueueFull { waiting });
            }
        }
        state.next += 1;
        let key = state.next;
        state.queued.push((
            key,
            pb::Request {
                kind: Some(request),
            },
        ));
        if let Err(failure) = write_queue(&self.queue_file, &state.queued) {
            state.queued.retain(|(queued, _)| *queued != key);
            return Err(failure);
        }
        Ok(())
    }

    fn unqueue(&self, key: u64) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.queued.retain(|(queued, _)| *queued != key);
        // Left in the file, the daemon would get it again from the next
        // watch: a repeated question, not a lost one.
        if let Err(failure) = write_queue(&self.queue_file, &state.queued) {
            eprintln!("humatad: {}", describe(&failure));
        }
    }

    async fn call(&self, request: pb::request::Kind) -> Result<pb::response::Kind, RelayError> {
        if !matches!(
            request,
            pb::request::Kind::ListSessions(_)
                | pb::request::Kind::ListItems(_)
                | pb::request::Kind::ListThreads(_)
                | pb::request::Kind::AskPerson(_)
                | pb::request::Kind::RequestMove(_)
                | pb::request::Kind::AgentStartSession(_)
                | pb::request::Kind::AgentSendMessage(_)
                | pb::request::Kind::ReadSpec(_)
                | pb::request::Kind::ReadSkill(_)
                | pb::request::Kind::ReadHistory(_)
                | pb::request::Kind::Search(_)
        ) {
            return Err(RelayError::NotATool);
        }
        match request {
            // The daemon's spec is the live one; the copy is for while the
            // Mac is away.
            pb::request::Kind::ReadSpec(read) => {
                let asked = self.send(pb::request::Kind::ReadSpec(read.clone())).await;
                match (asked, self.copied_spec(&read)) {
                    (
                        Err(
                            RelayError::NotWatched
                            | RelayError::AnswerLost
                            | RelayError::TimedOut { .. },
                        ),
                        Some(spec),
                    ) => Ok(pb::response::Kind::Spec(spec)),
                    // Nothing was changed by a read, so it is simply not
                    // connected.
                    (Err(RelayError::AnswerLost), None) => Err(RelayError::NotWatched),
                    (asked, _) => asked,
                }
            }
            // Made while the Mac is away, they wait in the queue, and the
            // agent hears it was asked with no request yet.
            // One id for every try, so the inbox keeps it once however
            // often it's handed over.
            mut request @ (pb::request::Kind::AskPerson(_) | pb::request::Kind::RequestMove(_)) => {
                let client_id = uuid::Uuid::new_v4().to_string();
                match &mut request {
                    pb::request::Kind::AskPerson(ask) => ask.client_id = client_id,
                    pb::request::Kind::RequestMove(asked) => asked.client_id = client_id,
                    _ => {}
                }
                match self.send(request.clone()).await {
                    // One whose answer was lost may be in the inbox already;
                    // its id keeps it there once when it's handed over again.
                    Err(
                        RelayError::NotWatched
                        | RelayError::AnswerLost
                        | RelayError::TimedOut { .. },
                    ) => {
                        self.enqueue(request)?;
                        Ok(pb::response::Kind::RequestMade(pb::RequestMade {
                            request: None,
                        }))
                    }
                    asked => asked,
                }
            }
            // A start or a message the daemon didn't answer may have been
            // made all the same, so the agent is told to look before it
            // asks again. One whose answer was lost with the connection
            // says so by itself.
            request @ (pb::request::Kind::AgentStartSession(_)
            | pb::request::Kind::AgentSendMessage(_)) => match self.send(request).await {
                Err(RelayError::TimedOut { seconds }) => Err(RelayError::Unanswered { seconds }),
                asked => asked,
            },
            // A read whose answer was lost changed nothing: the Mac isn't
            // connected, and asking again later is right.
            request => match self.send(request).await {
                Err(RelayError::AnswerLost) => Err(RelayError::NotWatched),
                asked => asked,
            },
        }
    }

    /// Hands `request` to the daemon watching and waits for its answer.
    /// `NotWatched` is a call that was never handed over; `AnswerLost` is
    /// one the daemon got and may have acted on before its watch ended.
    async fn send(&self, request: pb::request::Kind) -> Result<pb::response::Kind, RelayError> {
        let (id, watcher, answered) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let Some((_, watcher)) = state.watcher.clone() else {
                return Err(RelayError::NotWatched);
            };
            state.next += 1;
            let id = state.next;
            let (answer, answered) = oneshot::channel();
            state.waiting.insert(id, answer);
            (id, watcher, answered)
        };
        let call = pb::StreamItem {
            kind: Some(pb::stream_item::Kind::ToolCall(pb::ToolCall {
                id,
                request: Some(pb::Request {
                    kind: Some(request),
                }),
            })),
        };
        if watcher.send(Ok(call)).await.is_err() {
            self.forget(id);
            return Err(RelayError::NotWatched);
        }
        match tokio::time::timeout(ANSWER_TIMEOUT, answered).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(RelayError::AnswerLost),
            Err(_) => {
                self.forget(id);
                Err(RelayError::TimedOut {
                    seconds: ANSWER_TIMEOUT.as_secs(),
                })
            }
        }
    }

    fn forget(&self, id: u64) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .waiting
            .remove(&id);
    }
}

/// The calls in a queue file, none if there's no file.
fn read_queue(path: &Path) -> Result<Vec<pb::Request>, RelayError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(failure) if failure.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(RelayError::ReadQueue {
                path: path.to_owned(),
                source,
            });
        }
    };
    let mut rest = bytes.as_slice();
    let mut requests = Vec::new();
    while !rest.is_empty() {
        let request = pb::Request::decode_length_delimited(&mut rest).map_err(|source| {
            RelayError::BadQueue {
                path: path.to_owned(),
                source,
            }
        })?;
        requests.push(request);
    }
    Ok(requests)
}

/// Replaces the queue file with `queued`, whole or not at all.
fn write_queue(path: &Path, queued: &[(u64, pb::Request)]) -> Result<(), RelayError> {
    let mut bytes = Vec::new();
    for (_, request) in queued {
        // Encoding into a Vec has room for anything.
        let _ = request.encode_length_delimited(&mut bytes);
    }
    let written = path.with_extension("new");
    std::fs::write(&written, bytes)
        .and_then(|()| std::fs::rename(&written, path))
        .map_err(|source| RelayError::WriteQueue {
            path: path.to_owned(),
            source,
        })
}

impl Service for Relay {
    fn handle(self: Arc<Self>, request: pb::request::Kind) -> ServiceFuture {
        Box::pin(async move {
            Reply::Response(match self.call(request).await {
                Ok(response) => response,
                Err(failure) => error(failure.code(), describe(&failure)),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(text: &str) -> pb::request::Kind {
        pb::request::Kind::AskPerson(pb::AskPerson {
            session_id: "s1".to_owned(),
            text: text.to_owned(),
            client_id: String::new(),
        })
    }

    /// The question `call` asks, and the id it's asked under.
    fn asked(call: &pb::request::Kind) -> (&str, &str) {
        match call {
            pb::request::Kind::AskPerson(ask) => (&ask.text, &ask.client_id),
            other => panic!("expected a question, got {other:?}"),
        }
    }

    fn calls(reply: Reply) -> mpsc::Receiver<Result<pb::StreamItem, pb::Error>> {
        match reply {
            Reply::Stream { items, .. } => items,
            Reply::Response(other) => panic!("expected a stream, got {other:?}"),
        }
    }

    /// The next call the daemon gets, with its id.
    async fn next_call(
        calls: &mut mpsc::Receiver<Result<pb::StreamItem, pb::Error>>,
    ) -> (u64, pb::request::Kind) {
        let item = tokio::time::timeout(Duration::from_secs(5), calls.recv())
            .await
            .unwrap_or_else(|_| panic!("no call came"));
        match item {
            Some(Ok(pb::StreamItem {
                kind:
                    Some(pb::stream_item::Kind::ToolCall(pb::ToolCall {
                        id,
                        request: Some(pb::Request { kind: Some(kind) }),
                    })),
            })) => (id, kind),
            other => panic!("expected a tool call, got {other:?}"),
        }
    }

    fn answer(relay: &Relay, id: u64, response: pb::response::Kind) {
        relay
            .answer(pb::AnswerToolCall {
                id,
                response: Some(pb::Response {
                    kind: Some(response),
                }),
            })
            .unwrap_or_else(|error| panic!("{error}"));
    }

    async fn until_empty(path: &Path) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !read_queue(path)
                .unwrap_or_else(|error| panic!("{error}"))
                .is_empty()
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the queue never emptied"));
    }

    #[tokio::test]
    async fn a_question_asked_while_no_one_watches_reaches_the_next_watcher_in_order() {
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let file = state.path().join("queued");
        let first = Relay::open(file.clone());
        for text in ["one", "two"] {
            let made = first.call(ask(text)).await;
            assert!(
                matches!(
                    &made,
                    Ok(pb::response::Kind::RequestMade(pb::RequestMade {
                        request: None
                    }))
                ),
                "{made:?}"
            );
        }
        drop(first);

        // A host started again still has them.
        let relay = Arc::new(Relay::open(file.clone()));
        let mut daemon = calls(relay.watch());
        for text in ["one", "two"] {
            let (id, call) = next_call(&mut daemon).await;
            assert_eq!(asked(&call).0, text);
            answer(
                &relay,
                id,
                pb::response::Kind::RequestMade(pb::RequestMade {
                    request: Some(pb::AgentRequest::default()),
                }),
            );
        }
        until_empty(&file).await;
        assert!(
            Relay::open(file)
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .queued
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_question_unanswered_stays_for_the_next_watch() {
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let file = state.path().join("queued");
        let relay = Arc::new(Relay::open(file.clone()));
        let made = relay.call(ask("one")).await;
        assert!(made.is_ok(), "{made:?}");

        let mut gone = calls(relay.watch());
        let (_, tried) = next_call(&mut gone).await;
        drop(gone);

        // Handed over again under the same id, so the inbox keeps it once.
        let mut daemon = calls(relay.watch());
        let (id, call) = next_call(&mut daemon).await;
        assert_eq!(asked(&call), asked(&tried));
        assert!(!asked(&call).1.is_empty());
        answer(
            &relay,
            id,
            pb::response::Kind::RequestMade(pb::RequestMade::default()),
        );
        until_empty(&file).await;
    }

    /// `request` called on its own task, and the daemon's stream once the
    /// call has reached it.
    async fn handed_over(
        relay: &Arc<Relay>,
        request: pb::request::Kind,
    ) -> (
        tokio::task::JoinHandle<Result<pb::response::Kind, RelayError>>,
        mpsc::Receiver<Result<pb::StreamItem, pb::Error>>,
        pb::request::Kind,
    ) {
        let mut daemon = calls(relay.watch());
        let calling = tokio::spawn({
            let relay = relay.clone();
            async move { relay.call(request).await }
        });
        let (_, call) = next_call(&mut daemon).await;
        (calling, daemon, call)
    }

    #[tokio::test]
    async fn a_start_whose_answer_is_lost_with_the_watch_says_it_may_have_acted() {
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let relay = Arc::new(Relay::open(state.path().join("queued")));
        let start = pb::request::Kind::AgentStartSession(pb::AgentStartSession {
            session_id: "s1".to_owned(),
            name: "child".to_owned(),
            text: "go".to_owned(),
        });

        let (starting, daemon, _) = handed_over(&relay, start.clone()).await;
        drop(daemon);
        let lost = starting.await.unwrap_or_else(|error| panic!("{error}"));
        // With no one watching it was never handed over.
        let never = relay.call(start).await;

        assert!(matches!(lost, Err(RelayError::AnswerLost)), "{lost:?}");
        assert!(matches!(never, Err(RelayError::NotWatched)), "{never:?}");
    }

    #[tokio::test]
    async fn a_question_whose_answer_is_lost_with_the_watch_waits_for_the_next_watcher() {
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let file = state.path().join("queued");
        let relay = Arc::new(Relay::open(file.clone()));

        let (asking, daemon, tried) = handed_over(&relay, ask("one")).await;
        drop(daemon);
        let made = asking.await.unwrap_or_else(|error| panic!("{error}"));

        assert!(
            matches!(
                &made,
                Ok(pb::response::Kind::RequestMade(pb::RequestMade {
                    request: None
                }))
            ),
            "{made:?}"
        );
        // Handed over again under the same id, so the inbox keeps it once.
        let mut daemon = calls(relay.watch());
        let (id, call) = next_call(&mut daemon).await;
        assert_eq!(asked(&call), asked(&tried));
        answer(
            &relay,
            id,
            pb::response::Kind::RequestMade(pb::RequestMade::default()),
        );
        until_empty(&file).await;
    }

    #[tokio::test]
    async fn a_read_whose_answer_is_lost_is_told_the_mac_isnt_connected() {
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let relay = Arc::new(Relay::open(state.path().join("queued")));
        let list = pb::request::Kind::ListItems(pb::ListItems {
            project_id: "p1".to_owned(),
        });
        let read = pb::request::Kind::ReadSpec(pb::ReadSpec {
            session_id: "s1".to_owned(),
            item_id: "i1".to_owned(),
        });

        let (listing, daemon, _) = handed_over(&relay, list).await;
        drop(daemon);
        let listed = listing.await.unwrap_or_else(|error| panic!("{error}"));
        // A spec the host has no copy of.
        let (reading, daemon, _) = handed_over(&relay, read).await;
        drop(daemon);
        let spec = reading.await.unwrap_or_else(|error| panic!("{error}"));

        assert!(matches!(listed, Err(RelayError::NotWatched)), "{listed:?}");
        assert!(matches!(spec, Err(RelayError::NotWatched)), "{spec:?}");
    }

    #[tokio::test]
    async fn a_ninth_question_of_one_session_isnt_kept_and_another_sessions_is() {
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let file = state.path().join("queued");
        let relay = Relay::open(file.clone());
        let move_of = |session: &str| {
            pb::request::Kind::RequestMove(pb::RequestMove {
                session_id: session.to_owned(),
                ..Default::default()
            })
        };
        for number in 0..4 {
            let made = relay.call(ask(&format!("question {number}"))).await;
            assert!(made.is_ok(), "{made:?}");
            let made = relay.call(move_of("s1")).await;
            assert!(made.is_ok(), "{made:?}");
        }

        let ninth = relay.call(ask("one too many")).await;
        let ninth_move = relay.call(move_of("s1")).await;
        let other = relay.call(move_of("s2")).await;

        assert!(
            matches!(ninth, Err(RelayError::QueueFull { .. })),
            "{ninth:?}"
        );
        assert!(
            matches!(ninth_move, Err(RelayError::QueueFull { .. })),
            "{ninth_move:?}"
        );
        assert!(other.is_ok(), "{other:?}");
        let kept = read_queue(&file).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(kept.len(), 9);
    }

    #[tokio::test]
    async fn a_spec_is_the_daemons_while_it_watches_and_the_copy_while_it_doesnt() {
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let relay = Arc::new(Relay::open(state.path().join("queued")));
        let spec = |text: &str| pb::Spec {
            item_id: "i1".to_owned(),
            spec: text.to_owned(),
            ..Default::default()
        };
        relay.set_specs(pb::SetSpecs {
            session_id: "s1".to_owned(),
            specs: vec![spec("copied")],
        });
        let read = pb::request::Kind::ReadSpec(pb::ReadSpec {
            session_id: "s1".to_owned(),
            item_id: "i1".to_owned(),
        });

        let away = relay.call(read.clone()).await;
        assert_eq!(away.ok(), Some(pb::response::Kind::Spec(spec("copied"))));

        let mut daemon = calls(relay.watch());
        let reading = tokio::spawn({
            let relay = relay.clone();
            let read = read.clone();
            async move { relay.call(read).await }
        });
        let (id, call) = next_call(&mut daemon).await;
        assert_eq!(call, read);
        answer(&relay, id, pb::response::Kind::Spec(spec("live")));
        let live = reading.await.unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(live.ok(), Some(pb::response::Kind::Spec(spec("live"))));
    }
}
