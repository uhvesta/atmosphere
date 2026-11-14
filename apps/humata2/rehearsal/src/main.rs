//! A stand-in humatad with one session, `rehearsal`. Its history is
//! `--rows` messages, alternately the person's and the agent's, and each
//! message sent to it starts a turn whose reply streams in `--deltas`
//! parts. Serves on `--socket` until it's stopped.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use humata_wire::{Reply, Service, ServiceFuture, WireError, pb, serve};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

const SESSION: &str = "rehearsal";

/// Each part of a streamed reply.
const PART: &str = "token ";

#[derive(Debug, thiserror::Error)]
enum RehearsalError {
    #[error("{0}; run it as rehearsal --socket <path> --rows <n> --deltas <n>")]
    Usage(String),
    #[error("couldn't listen on {path}; remove a stale socket there or pick another path")]
    Listen {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't start the async runtime")]
    Runtime(#[source] std::io::Error),
}

struct Options {
    socket: PathBuf,
    rows: u64,
    deltas: u64,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, RehearsalError> {
        let (mut socket, mut rows, mut deltas) = (None, 0, 0);
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| RehearsalError::Usage(format!("{flag} needs a value")))?;
            let count = || {
                value.parse::<u64>().map_err(|_| {
                    RehearsalError::Usage(format!("{flag} takes a count, not {value}"))
                })
            };
            match flag.as_str() {
                "--socket" => socket = Some(PathBuf::from(&value)),
                "--rows" => rows = count()?,
                "--deltas" => deltas = count()?,
                _ => return Err(RehearsalError::Usage(format!("unknown flag {flag}"))),
            }
        }
        let socket = socket.ok_or_else(|| RehearsalError::Usage("no --socket given".into()))?;
        Ok(Self {
            socket,
            rows,
            deltas,
        })
    }
}

type Items = mpsc::Sender<Result<pb::StreamItem, pb::Error>>;

struct Rehearsal {
    rows: u64,
    deltas: u64,
    /// Every open watch.
    watchers: Mutex<Vec<Items>>,
    turns: Mutex<u64>,
}

fn event(id: String, kind: pb::session_event::Kind) -> pb::StreamItem {
    pb::StreamItem {
        kind: Some(pb::stream_item::Kind::SessionEvent(pb::SessionEvent {
            id,
            time_unix_ms: 0,
            kind: Some(kind),
        })),
    }
}

impl Rehearsal {
    fn history(&self) -> Vec<pb::StreamItem> {
        use pb::session_event::Kind;
        let mut items = vec![pb::StreamItem {
            kind: Some(pb::stream_item::Kind::HistoryLoading(pb::HistoryLoading {
                read: 0,
                total: self.rows,
            })),
        }];
        for row in 0..self.rows {
            let text = format!("message {row}");
            let kind = if row % 2 == 0 {
                Kind::UserMessage(pb::UserMessage {
                    text,
                    images: Vec::new(),
                })
            } else {
                Kind::AssistantMessage(pb::AssistantMessage { text })
            };
            items.push(event(format!("h{row}"), kind));
        }
        items.push(pb::StreamItem {
            kind: Some(pb::stream_item::Kind::HistoryLoaded(pb::HistoryLoaded {})),
        });
        items
    }

    /// The turn a message starts, as each watch sees it.
    fn turn(&self, text: String) -> Vec<pb::StreamItem> {
        use pb::session_event::Kind;
        let turn = {
            let mut turns = self.turns.lock().unwrap_or_else(PoisonError::into_inner);
            *turns += 1;
            *turns
        };
        let id = |part: &str| format!("t{turn}-{part}");
        let mut items = vec![
            event(
                id("asked"),
                Kind::UserMessage(pb::UserMessage {
                    text,
                    images: Vec::new(),
                }),
            ),
            event(id("started"), Kind::TurnStarted(pb::TurnStarted {})),
        ];
        for delta in 0..self.deltas {
            items.push(event(
                id(&format!("d{delta}")),
                Kind::AssistantDelta(pb::AssistantDelta { text: PART.into() }),
            ));
        }
        items.push(event(
            id("replied"),
            Kind::AssistantMessage(pb::AssistantMessage {
                text: PART.repeat(self.deltas as usize),
            }),
        ));
        items.push(event(
            id("ended"),
            Kind::TurnEnded(pb::TurnEnded { aborted: false }),
        ));
        items.push(event(id("idle"), Kind::Idle(pb::SessionIdle {})));
        items
    }

    fn session() -> pb::SessionInfo {
        pb::SessionInfo {
            id: SESSION.into(),
            name: "Rehearsal".into(),
            state: pb::SessionState::Idle.into(),
            ..pb::SessionInfo::default()
        }
    }
}

fn refuse(message: &str) -> pb::response::Kind {
    pb::response::Kind::Error(pb::Error {
        code: pb::ErrorCode::InvalidRequest.into(),
        message: message.into(),
    })
}

impl Service for Rehearsal {
    fn handle(self: Arc<Self>, request: pb::request::Kind) -> ServiceFuture {
        Box::pin(async move {
            use pb::request::Kind;
            match request {
                Kind::ListSessions(_) => {
                    Reply::Response(pb::response::Kind::Sessions(pb::Sessions {
                        sessions: vec![Rehearsal::session()],
                    }))
                }
                Kind::WatchSession(watch) if watch.session_id == SESSION => {
                    let (items, received) = mpsc::channel(1024);
                    let history = self.history();
                    let watchers = self.clone();
                    tokio::spawn(async move {
                        for item in history {
                            if items.send(Ok(item)).await.is_err() {
                                return;
                            }
                        }
                        watchers
                            .watchers
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(items);
                    });
                    Reply::Stream {
                        response: pb::response::Kind::Watching(pb::Watching {}),
                        items: received,
                    }
                }
                Kind::SendMessage(message) if message.session_id == SESSION => {
                    let turn = self.turn(message.text);
                    let watchers = self
                        .watchers
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    tokio::spawn(async move {
                        for watcher in watchers {
                            for item in turn.iter().cloned() {
                                if watcher.send(Ok(item)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    });
                    Reply::Response(pb::response::Kind::MessageSent(pb::MessageSent {}))
                }
                Kind::AbortTurn(_) => {
                    Reply::Response(pb::response::Kind::TurnAborted(pb::TurnAborted {}))
                }
                _ => Reply::Response(refuse("the rehearsal knows only its one session")),
            }
        })
    }
}

fn main() -> Result<(), RehearsalError> {
    let options = Options::parse(std::env::args().skip(1))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(RehearsalError::Runtime)?;
    runtime.block_on(async {
        let listener =
            UnixListener::bind(&options.socket).map_err(|source| RehearsalError::Listen {
                path: options.socket.clone(),
                source,
            })?;
        let rehearsal = Arc::new(Rehearsal {
            rows: options.rows,
            deltas: options.deltas,
            watchers: Mutex::default(),
            turns: Mutex::default(),
        });
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let rehearsal = rehearsal.clone();
            tokio::spawn(async move {
                let (reader, writer) = stream.into_split();
                let served: Result<_, WireError> =
                    serve(reader, writer, pb::Role::Daemon, "rehearsal", rehearsal).await;
                if let Err(error) = served {
                    eprintln!("rehearsal: a connection ended with {error}");
                }
            });
        }
    })
}
