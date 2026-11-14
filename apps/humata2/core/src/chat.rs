//! A session's chat: its history read once, then each event as it happens,
//! with the reply that's streaming kept apart so only what it gained
//! crosses to Swift.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tca::{Clock, Effect, Identified, IdentifiedVec, Reducer};

use crate::daemon::Daemon;

/// One row of the chat. Rows are shared, not copied: comparing two that
/// are the same row is a pointer check, and a row changes by being
/// replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: Arc<str>,
    pub body: Arc<Body>,
}

impl Entry {
    pub fn new(id: impl Into<Arc<str>>, body: Body) -> Self {
        Self {
            id: id.into(),
            body: Arc::new(body),
        }
    }
}

impl Identified for Entry {
    type Id = Arc<str>;

    fn id(&self) -> Arc<str> {
        self.id.clone()
    }
}

/// What a row shows.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Body {
    /// The person's message, and how many images came with it.
    Asked {
        text: String,
        images: u32,
    },
    Replied {
        text: String,
    },
    Thought {
        text: String,
    },
    Tool {
        call_id: String,
        tool: String,
        /// Its arguments, as JSON.
        arguments: String,
        /// None while it runs.
        outcome: Option<ToolOutcome>,
    },
    Permission {
        request_id: String,
        summary: String,
        /// None while it waits for an answer.
        answer: Option<PermissionAnswer>,
    },
    Notice {
        text: String,
    },
    Failed {
        message: String,
    },
    /// The turn was stopped before it finished.
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ToolOutcome {
    pub success: bool,
    pub output: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PermissionAnswer {
    pub approved: bool,
    /// Answered by the session's approval mode, not by a person.
    pub automatic: bool,
}

/// What the daemon tells a watch of a session.
#[derive(Clone, Debug, PartialEq)]
pub enum ChatItem {
    Event {
        id: String,
        kind: EventKind,
    },
    /// How much of the history is read; a total of 0 isn't known.
    HistoryLoading {
        read: u64,
        total: u64,
    },
    /// Everything before was the history; everything after is new.
    HistoryLoaded,
}

/// One thing that happened in a session, as the chat needs it.
#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    Asked {
        text: String,
        images: u32,
    },
    TurnStarted,
    /// Part of the reply as it streams; the `Replied` that follows holds
    /// all of it.
    Delta {
        text: String,
    },
    Replied {
        text: String,
    },
    Thought {
        text: String,
    },
    ToolStarted {
        call_id: String,
        tool: String,
        arguments: String,
    },
    ToolCompleted {
        call_id: String,
        success: bool,
        output: String,
    },
    PermissionRequested {
        request_id: String,
        summary: String,
    },
    PermissionCompleted {
        request_id: String,
        approved: bool,
        automatic: bool,
    },
    TurnEnded {
        aborted: bool,
    },
    Idle,
    Failed {
        message: String,
    },
    Queued {
        queue_id: String,
        text: String,
    },
    Dequeued {
        queue_id: String,
    },
    Waiting {
        reason: String,
    },
    Notice {
        text: String,
    },
    /// The chat was cut back or put back: it's read again.
    Rewritten,
    /// Something the chat doesn't show.
    Other,
}

/// The reply as it streams.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Streaming {
    pub id: Arc<str>,
    pub text: String,
}

/// A message waiting for the running turn to end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Queued {
    pub id: String,
    pub text: String,
}

/// What a chat holds: its rows, the reply streaming, and the turn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcript {
    pub entries: IdentifiedVec<Entry>,
    pub streaming: Option<Streaming>,
    pub working: bool,
    /// What the turn waits for, until whatever comes next.
    pub waiting: Option<String>,
    pub queued: Vec<Queued>,
}

/// How far reading a session's history has got.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    pub read: u64,
    /// 0 when it isn't known.
    pub total: u64,
    /// The history read so far, shown in one go once it's all read.
    pub transcript: Transcript,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chat {
    /// The session shown; None when none is.
    pub session: Option<String>,
    /// What shows.
    pub shown: Transcript,
    /// Set while the history is read, first or again.
    pub reading: Option<Reading>,
    pub sending: bool,
    pub stopping: bool,
    /// A message that couldn't be sent, offered back to send again.
    pub unsent: Option<String>,
    /// Why the last thing asked failed, until something succeeds.
    pub error: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum ChatAction {
    /// Show this session's chat.
    Open(String),
    /// Show none.
    Close,
    Watched(ChatItem),
    /// The watch ended; with why, when it failed. It's asked for again.
    WatchEnded(Option<String>),
    /// Watch again, after a watch ended.
    Rewatch,
    Send(String),
    Sent,
    SendFailed {
        text: String,
        error: String,
    },
    /// The unsent message was taken back into the message field.
    TookUnsent,
    Stop,
    StopFailed(String),
}

/// The chat's reducer.
pub struct ChatFeature {
    daemon: Arc<dyn Daemon>,
    clock: Arc<dyn Clock>,
    /// How long to wait before watching again after a watch ends.
    retry: Duration,
    sends: AtomicU64,
}

impl ChatFeature {
    pub fn new(daemon: Arc<dyn Daemon>, clock: Arc<dyn Clock>, retry: Duration) -> Self {
        Self {
            daemon,
            clock,
            retry,
            sends: AtomicU64::new(0),
        }
    }

    fn watch(&self, session: String) -> Effect<ChatAction> {
        let daemon = self.daemon.clone();
        Effect::run(move |send| async move {
            let mut items = daemon.watch(session);
            while let Some(item) = items.recv().await {
                match item {
                    Ok(item) => send.send(ChatAction::Watched(item)),
                    Err(error) => {
                        send.send(ChatAction::WatchEnded(Some(error.to_string())));
                        return;
                    }
                }
            }
            send.send(ChatAction::WatchEnded(None));
        })
        .cancellable(WATCH, true)
    }

    /// Names a send, so one sent again after its answer was lost runs once.
    fn client_id(&self) -> String {
        let n = self.sends.fetch_add(1, Ordering::Relaxed);
        format!("humata2-{}-{n}", std::process::id())
    }
}

const WATCH: &str = "chat.watch";
const SEND: &str = "chat.send";
const STOP: &str = "chat.stop";

impl Reducer for ChatFeature {
    type State = Chat;
    type Action = ChatAction;

    fn reduce(&self, state: &mut Chat, action: ChatAction) -> Effect<ChatAction> {
        match action {
            ChatAction::Open(session) => {
                if state.session.as_deref() == Some(session.as_str()) {
                    return Effect::none();
                }
                *state = Chat {
                    session: Some(session.clone()),
                    reading: Some(Reading::default()),
                    ..Chat::default()
                };
                Effect::cancel(SEND)
                    .and(Effect::cancel(STOP))
                    .and(self.watch(session))
            }
            ChatAction::Close => {
                *state = Chat::default();
                Effect::cancel(WATCH)
                    .and(Effect::cancel(SEND))
                    .and(Effect::cancel(STOP))
            }
            ChatAction::Watched(item) => {
                state.error = None;
                match item {
                    ChatItem::HistoryLoading { read, total } => {
                        let reading = state.reading.get_or_insert_with(Reading::default);
                        reading.read = read;
                        reading.total = total;
                        Effect::none()
                    }
                    ChatItem::HistoryLoaded => {
                        if let Some(reading) = state.reading.take() {
                            state.shown = reading.transcript;
                        }
                        Effect::none()
                    }
                    ChatItem::Event { id, kind } => {
                        if matches!(kind, EventKind::Rewritten) {
                            state.reading = Some(Reading::default());
                            let Some(session) = state.session.clone() else {
                                return Effect::none();
                            };
                            return self.watch(session);
                        }
                        if matches!(kind, EventKind::TurnEnded { .. } | EventKind::Idle) {
                            state.stopping = false;
                        }
                        let transcript = match &mut state.reading {
                            Some(reading) => &mut reading.transcript,
                            None => &mut state.shown,
                        };
                        apply(transcript, id, kind);
                        Effect::none()
                    }
                }
            }
            ChatAction::WatchEnded(error) => {
                if let Some(error) = error {
                    state.error = Some(error);
                }
                let clock = self.clock.clone();
                let retry = self.retry;
                Effect::run(move |send| async move {
                    clock.sleep(retry).await;
                    send.send(ChatAction::Rewatch);
                })
                .cancellable(WATCH, true)
            }
            ChatAction::Rewatch => match state.session.clone() {
                Some(session) => {
                    state.reading = Some(Reading::default());
                    self.watch(session)
                }
                None => Effect::none(),
            },
            ChatAction::Send(text) => {
                let Some(session) = state.session.clone() else {
                    return Effect::none();
                };
                if text.trim().is_empty() {
                    return Effect::none();
                }
                state.sending = true;
                let daemon = self.daemon.clone();
                let client_id = self.client_id();
                Effect::future(async move {
                    match daemon.send_message(session, text.clone(), client_id).await {
                        Ok(()) => ChatAction::Sent,
                        Err(error) => ChatAction::SendFailed {
                            text,
                            error: error.to_string(),
                        },
                    }
                })
                .cancellable(SEND, false)
            }
            ChatAction::Sent => {
                state.sending = false;
                state.error = None;
                Effect::none()
            }
            ChatAction::SendFailed { text, error } => {
                state.sending = false;
                state.error = Some(error);
                state.unsent = Some(match state.unsent.take() {
                    Some(earlier) => format!("{earlier}\n\n{text}"),
                    None => text,
                });
                Effect::none()
            }
            ChatAction::TookUnsent => {
                state.unsent = None;
                Effect::none()
            }
            ChatAction::Stop => {
                let Some(session) = state.session.clone() else {
                    return Effect::none();
                };
                if state.stopping || !state.shown.working {
                    return Effect::none();
                }
                state.stopping = true;
                let daemon = self.daemon.clone();
                Effect::run(move |send| async move {
                    if let Err(error) = daemon.abort(session).await {
                        send.send(ChatAction::StopFailed(error.to_string()));
                    }
                })
                .cancellable(STOP, true)
            }
            ChatAction::StopFailed(error) => {
                state.stopping = false;
                state.error = Some(error);
                Effect::none()
            }
        }
    }
}

/// Changes `transcript` by one event.
fn apply(transcript: &mut Transcript, id: String, kind: EventKind) {
    if !matches!(kind, EventKind::Waiting { .. } | EventKind::Other) {
        transcript.waiting = None;
    }
    match kind {
        EventKind::Asked { text, images } => {
            transcript
                .entries
                .push(Entry::new(id, Body::Asked { text, images }));
        }
        EventKind::TurnStarted => transcript.working = true,
        EventKind::Delta { text } => match &mut transcript.streaming {
            Some(streaming) => streaming.text.push_str(&text),
            None => {
                transcript.streaming = Some(Streaming {
                    id: id.into(),
                    text,
                });
            }
        },
        EventKind::Replied { text } => {
            transcript.streaming = None;
            transcript
                .entries
                .push(Entry::new(id, Body::Replied { text }));
        }
        EventKind::Thought { text } => {
            transcript
                .entries
                .push(Entry::new(id, Body::Thought { text }));
        }
        EventKind::ToolStarted {
            call_id,
            tool,
            arguments,
        } => {
            transcript.entries.push(Entry::new(
                id,
                Body::Tool {
                    call_id,
                    tool,
                    arguments,
                    outcome: None,
                },
            ));
        }
        EventKind::ToolCompleted {
            call_id,
            success,
            output,
        } => change_last(
            transcript,
            |body| matches!(body, Body::Tool { call_id: started, .. } if *started == call_id),
            |body| {
                if let Body::Tool { outcome, .. } = body {
                    *outcome = Some(ToolOutcome { success, output });
                }
            },
        ),
        EventKind::PermissionRequested {
            request_id,
            summary,
        } => {
            transcript.entries.push(Entry::new(
                id,
                Body::Permission {
                    request_id,
                    summary,
                    answer: None,
                },
            ));
        }
        EventKind::PermissionCompleted {
            request_id,
            approved,
            automatic,
        } => change_last(
            transcript,
            |body| matches!(body, Body::Permission { request_id: asked, .. } if *asked == request_id),
            |body| {
                if let Body::Permission { answer, .. } = body {
                    *answer = Some(PermissionAnswer {
                        approved,
                        automatic,
                    });
                }
            },
        ),
        EventKind::TurnEnded { aborted } => {
            end_turn(transcript);
            if aborted {
                transcript.entries.push(Entry::new(id, Body::Stopped));
            }
        }
        EventKind::Idle => end_turn(transcript),
        EventKind::Failed { message } => {
            end_turn(transcript);
            transcript
                .entries
                .push(Entry::new(id, Body::Failed { message }));
        }
        EventKind::Queued { queue_id, text } => {
            transcript.queued.retain(|queued| queued.id != queue_id);
            transcript.queued.push(Queued { id: queue_id, text });
        }
        EventKind::Dequeued { queue_id } => {
            transcript.queued.retain(|queued| queued.id != queue_id);
        }
        EventKind::Waiting { reason } => transcript.waiting = Some(reason),
        EventKind::Notice { text } => {
            transcript
                .entries
                .push(Entry::new(id, Body::Notice { text }));
        }
        EventKind::Rewritten | EventKind::Other => {}
    }
}

/// The turn is over: what streamed stays as the reply it got to.
fn end_turn(transcript: &mut Transcript) {
    transcript.working = false;
    transcript.waiting = None;
    if let Some(streaming) = transcript.streaming.take()
        && !streaming.text.is_empty()
    {
        transcript.entries.push(Entry {
            id: streaming.id,
            body: Arc::new(Body::Replied {
                text: streaming.text,
            }),
        });
    }
}

/// Changes the latest row `is` picks, searching from the end, where what
/// an event completes almost always is. The row is replaced, not changed
/// in place, as the feed may still hold it.
fn change_last(
    transcript: &mut Transcript,
    is: impl Fn(&Body) -> bool,
    change: impl FnOnce(&mut Body),
) {
    let Some(id) = transcript
        .entries
        .iter()
        .rev()
        .find(|entry| is(&entry.body))
        .map(|entry| entry.id.clone())
    else {
        return;
    };
    transcript.entries.update(&id, |entry| {
        let mut body = Body::clone(&entry.body);
        change(&mut body);
        entry.body = Arc::new(body);
    });
}
