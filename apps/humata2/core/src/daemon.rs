//! What the features ask of `humatad`, and the live way to ask it.

use std::path::PathBuf;
use std::sync::Arc;

use humata_wire::{Client, ClientError, WireError, pb};
use tca::{BoxFuture, Identified};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

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
}

/// What the features ask of the daemon. Tests give the features a fake.
pub trait Daemon: Send + Sync + 'static {
    /// Every session, archived ones too.
    fn sessions(&self) -> BoxFuture<Result<Vec<Session>, DaemonError>>;
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
}
