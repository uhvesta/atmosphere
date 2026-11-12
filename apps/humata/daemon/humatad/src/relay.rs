//! Agents on a host reach the board and review threads, which are the
//! daemon's, through the host: `humatad mcp` calls this relay on a socket in
//! the host's state folder, and the relay hands each call to the daemon
//! watching over its connection, then the daemon's answer back.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use humata_wire::{Reply, Service, ServiceFuture, describe, error, pb};
use tokio::sync::{mpsc, oneshot};

use crate::error::{HostError, RelayError};

/// How long a call waits for the daemon's answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Relay {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    next: u64,
    /// The daemon's stream of calls, and which watch it is.
    watcher: Option<(u64, mpsc::Sender<Result<pb::StreamItem, pb::Error>>)>,
    waiting: HashMap<u64, oneshot::Sender<pb::response::Kind>>,
}

impl Relay {
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

    async fn call(&self, request: pb::request::Kind) -> Result<pb::response::Kind, RelayError> {
        if !matches!(
            request,
            pb::request::Kind::ListSessions(_)
                | pb::request::Kind::ListItems(_)
                | pb::request::Kind::CreateItem(_)
                | pb::request::Kind::UpdateItem(_)
                | pb::request::Kind::ListThreads(_)
        ) {
            return Err(RelayError::NotATool);
        }
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
            Ok(Err(_)) => Err(RelayError::NotWatched),
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
