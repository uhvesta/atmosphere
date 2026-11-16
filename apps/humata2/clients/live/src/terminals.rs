//! Terminal tabs: which tab is which index, and one attach per open tab.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use humata2_clients::model::{Agent, Session, SessionId};
use humata2_clients::{DaemonError, Event, Reply, Terminals};
use humata2_grpc::humata::v1 as pb;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Streaming;

use crate::connection::{LiveDaemon, Shared, fresh_key};
use crate::map;

/// How many times a tab not listed yet is looked for, and how long apart:
/// about two seconds in all.
const TAB_TRIES: u32 = 20;
const TAB_RETRY: std::time::Duration = std::time::Duration::from_millis(100);

/// Bytes the daemon may send before we acknowledge them.
const WINDOW_BYTES: u32 = 4 << 20;

/// An open attach: where input goes.
pub(crate) struct Attach {
    to: mpsc::Sender<pb::AttachClientMessage>,
    input_seq: AtomicU64,
    generation: u64,
}

impl Attach {
    async fn send(&self, message: pb::attach_client_message::Message) -> Result<(), DaemonError> {
        self.to
            .send(pb::AttachClientMessage {
                message: Some(message),
            })
            .await
            .map_err(|_| DaemonError::Refused {
                what: "The terminal".to_owned(),
                reason: "its connection closed. Open the tab again".to_owned(),
            })
    }
}

fn pb_session(id: &SessionId) -> pb::SessionId {
    pb::SessionId {
        value: id.as_str().to_owned(),
    }
}

impl Shared {
    /// The session's tabs, read from the daemon and remembered.
    pub(crate) async fn refresh_tabs(
        &self,
        session: &pb::SessionId,
    ) -> Result<Vec<pb::Tab>, DaemonError> {
        let tabs = self
            .terminal()
            .list_tabs(pb::ListTabsRequest {
                session: Some(session.clone()),
            })
            .await
            .map_err(|status| self.error(&status, "The session's terminals"))?
            .into_inner()
            .tabs;
        let tabs = map::order_tabs(tabs);
        self.state().tabs.insert(session.value.clone(), tabs.clone());
        Ok(tabs)
    }

    /// The session as the app shows it.
    pub(crate) async fn fetch_session(&self, id: &pb::SessionId) -> Result<Session, DaemonError> {
        let session = self
            .sessions()
            .get_session(pb::GetSessionRequest {
                id: Some(id.clone()),
            })
            .await
            .map_err(|status| self.error(&status, "The session"))?
            .into_inner();
        let tabs = self.refresh_tabs(id).await.unwrap_or_default();
        Ok(map::session(&session, &tabs, &self.agents))
    }

    /// Tab `index` of the session: 0 is the agent's.
    pub(crate) async fn tab(&self, session: &SessionId, index: usize) -> Result<pb::Tab, DaemonError> {
        let cached = self
            .state()
            .tabs
            .get(session.as_str())
            .and_then(|tabs| tabs.get(index).cloned());
        if let Some(tab) = cached {
            return Ok(tab);
        }
        // A session just started lists its tabs a moment after the app
        // hears of it, so a tab not listed yet gets a few short tries
        // before it is refused.
        for _ in 0..TAB_TRIES {
            if let Some(tab) = self
                .refresh_tabs(&pb_session(session))
                .await?
                .into_iter()
                .nth(index)
            {
                return Ok(tab);
            }
            // On the client's runtime: the caller may be polled elsewhere,
            // and a tokio sleep needs the runtime from when it is made, so
            // it is made inside the spawned task.
            let _ = self.spawn(async { tokio::time::sleep(TAB_RETRY).await }).await;
        }
        Err(DaemonError::Refused {
            what: "The terminal".to_owned(),
            reason: format!(
                "the session has no terminal tab {index}. Open the tab again from the session"
            ),
        })
    }

    /// The attach on tab `index`, starting it if there is none.
    async fn attach(self: &Arc<Self>, session: &SessionId, index: usize) -> Result<Arc<Attach>, DaemonError> {
        let size = *self
            .state()
            .wanted
            .entry((session.as_str().to_owned(), index))
            .or_default();
        let tab = self.tab(session, index).await?;
        let tab_id = tab.id.clone().unwrap_or_default();
        let key = (session.as_str().to_owned(), tab_id.value.clone());
        if let Some(open) = self.state().attaches.get(&key) {
            return Ok(open.clone());
        }
        let (columns, rows) = size.unwrap_or((80, 24));
        let (to, outgoing) = mpsc::channel(64);
        let open = pb::AttachOpen {
            session: Some(pb_session(session)),
            tab: Some(tab_id),
            from_offset: 0,
            read_only: false,
            cols: u32::from(columns),
            rows: u32::from(rows),
            window_bytes: WINDOW_BYTES,
            accept_codecs: Vec::new(),
            checkpoint_engine: String::new(),
        };
        to.send(pb::AttachClientMessage {
            message: Some(pb::attach_client_message::Message::Open(open)),
        })
        .await
        .map_err(|_| DaemonError::Refused {
            what: "The terminal".to_owned(),
            reason: "its connection closed before it opened. Open the tab again".to_owned(),
        })?;
        let stream = self
            .terminal()
            .attach(ReceiverStream::new(outgoing))
            .await
            .map_err(|status| self.error(&status, "The terminal"))?
            .into_inner();
        let attach = Arc::new(Attach {
            to,
            input_seq: AtomicU64::new(0),
            generation: self.generation.fetch_add(1, Ordering::Relaxed),
        });
        {
            let mut state = self.state();
            if let Some(existing) = state.attaches.get(&key) {
                return Ok(existing.clone());
            }
            state.attaches.insert(key.clone(), attach.clone());
        }
        self.spawn(read_attach(
            self.clone(),
            stream,
            attach.clone(),
            key,
            session.clone(),
            index,
        ));
        Ok(attach)
    }
}

impl Shared {
    /// Pushes `bytes` a holder numbered from `at` as the app's next bytes
    /// of the tab (`Placed`).
    fn place(&self, session: &SessionId, tab: usize, at: u64, mut bytes: Vec<u8>) {
        let (offset, skip) = self
            .state()
            .placed
            .entry((session.as_str().to_owned(), tab))
            .or_default()
            .place(at, bytes.len() as u64);
        bytes.drain(..usize::try_from(skip).unwrap_or(bytes.len()));
        if bytes.is_empty() {
            return;
        }
        self.publish(Event::TerminalData {
            session: session.clone(),
            tab,
            offset: usize::try_from(offset).unwrap_or(usize::MAX),
            bytes,
        });
    }

    /// Attaches again each tab of `session` the app opened whose attach
    /// ended, as when its holder died in a reboot and the daemon resumed
    /// the agent in a new one. A tab still attached is left as it is; one
    /// that can't be attached yet is tried again at the session's next
    /// change.
    pub(crate) fn reattach(self: &Arc<Self>, session: &SessionId) {
        let indexes: Vec<usize> = self
            .state()
            .wanted
            .keys()
            .filter(|(id, _)| id == session.as_str())
            .map(|(_, index)| *index)
            .collect();
        for index in indexes {
            let shared = self.clone();
            let session = session.clone();
            self.spawn(async move {
                // Not attached now is not an error to show: the next
                // change of the session tries again, and typing into the
                // tab reports why it can't attach.
                let _ = shared.attach(&session, index).await;
            });
        }
    }
}

/// Pushes what an attach delivers to every subscriber, until it ends.
async fn read_attach(
    shared: Arc<Shared>,
    mut stream: Streaming<pb::AttachServerMessage>,
    attach: Arc<Attach>,
    key: (String, String),
    session: SessionId,
    tab: usize,
) {
    use pb::attach_server_message::Message;
    while let Ok(Some(message)) = stream.message().await {
        match message.message {
            Some(Message::Snapshot(snapshot)) => {
                // The snapshot is the screen up to its offset: place it so
                // the output that follows continues from there.
                let offset = snapshot.offset.saturating_sub(snapshot.vt.len() as u64);
                shared.place(&session, tab, offset, snapshot.vt);
            }
            Some(Message::Output(output)) => {
                let end = output.offset + output.data.len() as u64;
                shared.place(&session, tab, output.offset, output.data);
                let _ = attach
                    .send(pb::attach_client_message::Message::Ack(pb::OutputAck {
                        offset: end,
                    }))
                    .await;
            }
            // Status changes come from the daemon's session events (D136).
            _ => {}
        }
    }
    let mut state = shared.state();
    if state
        .attaches
        .get(&key)
        .is_some_and(|current| current.generation == attach.generation)
    {
        state.attaches.remove(&key);
    }
}

impl Terminals for LiveDaemon {
    fn open(&self, session: &SessionId, agent: Agent) -> Reply<(usize, Session)> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            let id = pb_session(&session);
            let index = if agent == Agent::Shell {
                let tab = shared
                    .terminal()
                    .open_shell_tab(pb::OpenShellTabRequest {
                        idempotency_key: fresh_key(),
                        session: Some(id.clone()),
                        repo: None,
                    })
                    .await
                    .map_err(|status| shared.error(&status, "The new shell"))?
                    .into_inner();
                let tabs = shared.refresh_tabs(&id).await?;
                tabs.iter()
                    .position(|candidate| candidate.id == tab.id)
                    .ok_or_else(|| DaemonError::Refused {
                        what: "The new shell".to_owned(),
                        reason: "the daemon opened it but doesn't list it. Reload and try again"
                            .to_owned(),
                    })?
            } else {
                let current = shared.fetch_session(&id).await?;
                if current.agent != agent {
                    return Err(DaemonError::Refused {
                        what: format!("Starting {} in this session", agent.name()),
                        reason: format!(
                            "the session runs {}, and a second agent comes in a later release of Humata. Start a new session for {} instead",
                            current.agent.name(),
                            agent.name()
                        ),
                    });
                }
                shared.refresh_tabs(&id).await?;
                0
            };
            shared.attach(&session, index).await?;
            let current = shared.fetch_session(&id).await?;
            Ok((index, current))
        })
    }

    fn write(&self, session: &SessionId, tab: usize, text: &str) -> Reply<()> {
        self.input(session, tab, format!("{text}\r").into_bytes())
    }

    fn input(&self, session: &SessionId, tab: usize, bytes: Vec<u8>) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            let attach = shared.attach(&session, tab).await?;
            let input_seq = attach.input_seq.fetch_add(1, Ordering::Relaxed) + 1;
            attach
                .send(pb::attach_client_message::Message::Input(pb::TerminalInput {
                    data: bytes,
                    input_seq,
                }))
                .await
        })
    }

    fn resize(&self, session: &SessionId, tab: usize, columns: u16, rows: u16) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            shared
                .state()
                .wanted
                .insert((session.as_str().to_owned(), tab), Some((columns, rows)));
            let attach = shared.attach(&session, tab).await?;
            attach
                .send(pb::attach_client_message::Message::Resize(pb::TerminalResize {
                    cols: u32::from(columns),
                    rows: u32::from(rows),
                    ..Default::default()
                }))
                .await
        })
    }
}
