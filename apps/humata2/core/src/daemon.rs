//! What the features ask of `humatad`, and the live way to ask it.

use std::path::PathBuf;
use std::sync::Arc;

use humata_wire::{Client, ClientError, WireError, pb};
use tca::{BoxFuture, Identified};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, mpsc};

use crate::chat::{ChatItem, EventKind};

/// Humata 2's version, given to the daemon in the hello.
const VERSION: &str = "humata2-0.0.0";

/// A session as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Session {
    pub id: String,
    pub name: String,
    pub state: SessionState,
    pub project_id: String,
    /// The host it runs on; empty for this Mac.
    pub host: String,
    /// When its last event happened; 0 before it has any.
    pub last_active_unix_ms: i64,
    pub archived: bool,
    /// It finished a turn the person hasn't seen.
    pub unread: bool,
}

impl Identified for Session {
    type Id = String;

    fn id(&self) -> String {
        self.id.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SessionState {
    Idle,
    Working,
    WaitingForApproval,
    Failed,
}

impl From<&pb::SessionInfo> for Session {
    fn from(info: &pb::SessionInfo) -> Self {
        let state = match info.state() {
            pb::SessionState::Working => SessionState::Working,
            pb::SessionState::WaitingForApproval => SessionState::WaitingForApproval,
            pb::SessionState::Failed => SessionState::Failed,
            pb::SessionState::Idle | pb::SessionState::Unspecified => SessionState::Idle,
        };
        Self {
            id: info.id.clone(),
            name: info.name.clone(),
            state,
            project_id: info.project_id.clone(),
            host: info.host.clone(),
            last_active_unix_ms: info.last_active_unix_ms,
            archived: info.archived,
            unread: info.turns > info.seen_turns,
        }
    }
}

/// Why the daemon couldn't answer.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error(
        "couldn't find where Humata's daemon listens: there's no home folder. Set HUMATA_SOCKET to the daemon's socket."
    )]
    NoSocket,
    #[error(
        "couldn't reach Humata's daemon at {}. Open Humata so it starts the daemon, or set HUMATA_SOCKET to where it listens.",
        path.display()
    )]
    Connect {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "Humata's daemon at {} didn't finish saying hello. If it's from another version of Humata, update Humata and Humata 2 together.",
        path.display()
    )]
    Handshake {
        path: PathBuf,
        #[source]
        source: WireError,
    },
    #[error(
        "Humata's daemon didn't list the sessions. It tries again shortly; if it keeps failing, restart Humata."
    )]
    List {
        #[source]
        source: ClientError,
    },
    #[error(
        "Humata's daemon answered the session list with something else. Update Humata and Humata 2 to the same version."
    )]
    Unexpected,
    #[error(
        "couldn't follow the session's chat: {source}. Humata 2 tries again shortly; if it keeps failing, restart Humata."
    )]
    Watch {
        #[source]
        source: ClientError,
    },
    #[error(
        "the session's chat stopped: {message}. Humata 2 tries again shortly; if it keeps failing, restart Humata."
    )]
    WatchStopped { message: String },
    #[error("couldn't send the message: {source}. It's kept in the message field to send again.")]
    Send {
        #[source]
        source: ClientError,
    },
    #[error(
        "couldn't stop the turn: {source}. Try Stop again; if it keeps failing, restart Humata."
    )]
    Abort {
        #[source]
        source: ClientError,
    },
}

/// What a watch of a session tells, until it ends.
pub type ChatItems = mpsc::UnboundedReceiver<Result<ChatItem, DaemonError>>;

/// What the features ask of the daemon. Tests give the features a fake.
pub trait Daemon: Send + Sync + 'static {
    /// Every session, archived ones too.
    fn sessions(&self) -> BoxFuture<Result<Vec<Session>, DaemonError>>;

    /// The session's history, then each new event, until the receiver is
    /// dropped or the watch ends. An error ends it.
    fn watch(&self, session: String) -> ChatItems;

    /// Sends the person's message: it starts a turn, or waits for the one
    /// running. `client_id` names it, so it runs once if sent again.
    fn send_message(
        &self,
        session: String,
        text: String,
        client_id: String,
    ) -> BoxFuture<Result<(), DaemonError>>;

    /// Stops the session's turn.
    fn abort(&self, session: String) -> BoxFuture<Result<(), DaemonError>>;
}

/// The daemon on this Mac, reached on its socket. Connects on first use and
/// again whenever the connection has dropped.
#[derive(Clone)]
pub struct LiveDaemon {
    inner: Arc<Inner>,
}

struct Inner {
    /// None for the daemon's usual place.
    socket: Option<PathBuf>,
    client: Mutex<Option<Arc<Client>>>,
}

impl LiveDaemon {
    /// The daemon listening at `socket`, or at its usual place.
    pub fn new(socket: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(Inner {
                socket,
                client: Mutex::new(None),
            }),
        }
    }
}

impl Inner {
    async fn client(&self) -> Result<Arc<Client>, DaemonError> {
        let mut client = self.client.lock().await;
        if let Some(connected) = client.as_ref().filter(|client| client.is_connected()) {
            return Ok(connected.clone());
        }
        let path = self
            .socket
            .clone()
            .or_else(humata_wire::default_socket)
            .ok_or(DaemonError::NoSocket)?;
        let stream = UnixStream::connect(&path)
            .await
            .map_err(|source| DaemonError::Connect {
                path: path.clone(),
                source,
            })?;
        let (reader, writer) = stream.into_split();
        let connected = Client::connect(reader, writer, pb::Role::Client, VERSION)
            .await
            .map_err(|source| DaemonError::Handshake { path, source })?;
        let connected = Arc::new(connected);
        *client = Some(connected.clone());
        Ok(connected)
    }
}

impl Daemon for LiveDaemon {
    fn sessions(&self) -> BoxFuture<Result<Vec<Session>, DaemonError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let client = inner.client().await?;
            let request = pb::request::Kind::ListSessions(pb::ListSessions::default());
            match client.call(request).await {
                Ok(pb::response::Kind::Sessions(sessions)) => {
                    Ok(sessions.sessions.iter().map(Session::from).collect())
                }
                Ok(_) => Err(DaemonError::Unexpected),
                Err(source) => Err(DaemonError::List { source }),
            }
        })
    }

    fn watch(&self, session: String) -> ChatItems {
        let (items, received) = mpsc::unbounded_channel();
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let watching = async {
                let client = inner.client().await?;
                let request = pb::request::Kind::WatchSession(pb::WatchSession {
                    session_id: session,
                });
                let (_, mut stream) = client
                    .stream(request)
                    .await
                    .map_err(|source| DaemonError::Watch { source })?;
                while let Some(item) = stream.next().await {
                    let item = item.map_err(|error| DaemonError::WatchStopped {
                        message: error.message,
                    })?;
                    if let Some(item) = ChatItem::from_pb(item)
                        && items.send(Ok(item)).is_err()
                    {
                        break;
                    }
                }
                Ok(())
            };
            tokio::select! {
                // Dropping the stream tells the daemon to stop it.
                () = items.closed() => {}
                watched = watching => {
                    if let Err(error) = watched {
                        let _ = items.send(Err(error));
                    }
                }
            }
        });
        received
    }

    fn send_message(
        &self,
        session: String,
        text: String,
        client_id: String,
    ) -> BoxFuture<Result<(), DaemonError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let client = inner.client().await?;
            let request = pb::request::Kind::SendMessage(pb::SendMessage {
                session_id: session,
                text,
                mode: pb::SendMode::Enqueue.into(),
                client_id,
                images: Vec::new(),
            });
            client
                .call(request)
                .await
                .map(drop)
                .map_err(|source| DaemonError::Send { source })
        })
    }

    fn abort(&self, session: String) -> BoxFuture<Result<(), DaemonError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let client = inner.client().await?;
            let request = pb::request::Kind::AbortTurn(pb::AbortTurn {
                session_id: session,
                drop_queued: false,
            });
            client
                .call(request)
                .await
                .map(drop)
                .map_err(|source| DaemonError::Abort { source })
        })
    }
}

impl ChatItem {
    /// What the chat makes of a watch's item; None for what it doesn't
    /// follow, such as terminal output.
    fn from_pb(item: pb::StreamItem) -> Option<Self> {
        use pb::stream_item::Kind;
        match item.kind? {
            Kind::SessionEvent(event) => Some(Self::Event {
                kind: event.kind.map_or(EventKind::Other, EventKind::from),
                id: event.id,
            }),
            Kind::HistoryLoading(loading) => Some(Self::HistoryLoading {
                read: loading.read,
                total: loading.total,
            }),
            Kind::HistoryLoaded(_) => Some(Self::HistoryLoaded),
            Kind::ToolCall(_) | Kind::TerminalOutput(_) | Kind::PortData(_) => None,
        }
    }
}

impl From<pb::session_event::Kind> for EventKind {
    fn from(kind: pb::session_event::Kind) -> Self {
        use pb::session_event::Kind;
        match kind {
            Kind::UserMessage(message) => Self::Asked {
                text: message.text,
                images: u32::try_from(message.images.len()).unwrap_or(u32::MAX),
            },
            Kind::TurnStarted(_) => Self::TurnStarted,
            Kind::AssistantDelta(delta) => Self::Delta { text: delta.text },
            Kind::AssistantMessage(message) => Self::Replied { text: message.text },
            Kind::Reasoning(reasoning) => Self::Thought {
                text: reasoning.text,
            },
            Kind::ToolStarted(tool) => Self::ToolStarted {
                call_id: tool.call_id,
                tool: tool.tool,
                arguments: tool.arguments_json,
            },
            Kind::ToolCompleted(tool) => Self::ToolCompleted {
                call_id: tool.call_id,
                success: tool.success,
                output: tool.output,
            },
            Kind::PermissionRequested(asked) => Self::PermissionRequested {
                request_id: asked.request_id,
                summary: asked.summary,
            },
            Kind::PermissionCompleted(answered) => Self::PermissionCompleted {
                request_id: answered.request_id,
                approved: answered.approved,
                automatic: answered.automatic,
            },
            Kind::TurnEnded(ended) => Self::TurnEnded {
                aborted: ended.aborted,
            },
            Kind::Idle(_) => Self::Idle,
            Kind::Failed(failed) => Self::Failed {
                message: failed.message,
            },
            Kind::MessageQueued(queued) => Self::Queued {
                queue_id: queued.queue_id,
                text: queued.text,
            },
            Kind::Dequeued(dequeued) => Self::Dequeued {
                queue_id: dequeued.queue_id,
            },
            Kind::Waiting(waiting) => Self::Waiting {
                reason: waiting.reason,
            },
            Kind::Notice(notice) => Self::Notice { text: notice.text },
            Kind::Rewound(_) | Kind::RewindUndone(_) => Self::Rewritten,
            Kind::Usage(_) | Kind::Checkpoint(_) | Kind::Saved(_) | Kind::Compacted(_) => {
                Self::Other
            }
        }
    }
}
