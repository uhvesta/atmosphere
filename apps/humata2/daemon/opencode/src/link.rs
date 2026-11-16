//! The wrapper's link: turns go to the daemon, asks go to the Inbox and
//! their answers come back to OpenCode, across dropped streams.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::activity::{Activity, Turn};
use crate::event::{Asked, Event};

/// Why a call to OpenCode's server failed.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// The ask was already answered, say by the TUI.
    #[error("OpenCode no longer has that ask: it was already answered, so nothing is left to do.")]
    Gone,
    /// The server answered with an error status.
    #[error("{}", status_message(*.code, .body))]
    Status { code: u16, body: String },
    /// The URL is not `http://127.0.0.1:<port>`.
    #[error(
        "`{url}` is not a loopback URL of the form http://127.0.0.1:<port>. Humata only talks \
         to the OpenCode server it started itself; restart the session to start a fresh one."
    )]
    BadUrl { url: String },
    /// The connection could not be opened.
    #[error(
        "Could not connect to OpenCode's server at {addr}. It may have stopped; restart the \
         session to start a fresh one."
    )]
    Connect {
        addr: String,
        #[source]
        source: std::io::Error,
    },
    /// The request could not be built.
    #[error(
        "Could not build the request to OpenCode's server. This is a bug in Humata: report it."
    )]
    Build {
        #[source]
        source: hyper::http::Error,
    },
    /// The request failed on the connection.
    #[error(
        "The request to OpenCode's server failed on the connection. The server may have \
         stopped; restart the session to start a fresh one."
    )]
    Request {
        #[source]
        source: hyper::Error,
    },
    /// The response body could not be read.
    #[error(
        "Reading OpenCode's response failed part-way. Try again; if it keeps failing, restart the session."
    )]
    Body {
        #[source]
        source: hyper::Error,
    },
    /// The response is not the JSON Humata reads.
    #[error(
        "OpenCode's response is not valid JSON. The server may not be OpenCode 2.3.0 or later: \
         check with `opencode --version`."
    )]
    BadJson {
        #[source]
        source: serde_json::Error,
    },
    /// The response is JSON but lacks something Humata needs.
    #[error(
        "OpenCode's response has no {what}. This OpenCode may differ from 2.3.0: upgrade \
         OpenCode, or update Humata if OpenCode is newer."
    )]
    Unexpected { what: &'static str },
}

/// The message for an error status, with what to do about it.
fn status_message(code: u16, body: &str) -> String {
    if code == 401 {
        return "OpenCode's server refused the password. It is not the server this wrapper \
                started; restart the session to start a fresh one."
            .to_owned();
    }
    format!(
        "OpenCode's server answered with status {code}: {body}. Try again; if it keeps \
         happening, restart the session."
    )
}

/// Why the daemon could not take a call.
#[derive(Debug, thiserror::Error)]
pub enum HumataError {
    /// The daemon could not be reached or the stream dropped.
    #[error(
        "Lost the Humata daemon: {reason}. The link tries again on its own; if it doesn't \
         recover, start the daemon."
    )]
    Lost { reason: String },
}

/// OpenCode's server, as the link needs it.
pub trait Server: Send + Sync + 'static {
    /// Opens the event stream. It ends when the connection drops.
    fn events(&self) -> impl Future<Output = Result<mpsc::Receiver<Event>, ServerError>> + Send;
    /// The sessions running now.
    fn active(&self) -> impl Future<Output = Result<Vec<String>, ServerError>> + Send;
    /// The asks open now.
    fn pending(&self) -> impl Future<Output = Result<Vec<Asked>, ServerError>> + Send;
    /// Posts `body` as the answer to `request` in `session`.
    fn reply(
        &self,
        session: &str,
        request: &str,
        body: Vec<u8>,
    ) -> impl Future<Output = Result<(), ServerError>> + Send;
}

/// The Humata daemon, as the link needs it.
pub trait Humata: Send + Sync + 'static {
    /// Reports a changed turn.
    fn turn(&self, turn: Turn) -> impl Future<Output = ()> + Send;
    /// Raises an ask and waits for the answer's body. An empty body is no
    /// decision. Dropping the future withdraws the ask.
    fn ask(&self, payload: Vec<u8>) -> impl Future<Output = Result<Vec<u8>, HumataError>> + Send;
}

/// An ask task that finished, and whether it ended without a decision.
struct Done {
    id: String,
    undecided: bool,
}

/// The ask tasks in flight, aborted when this is dropped.
#[derive(Default)]
struct Asks {
    /// Ask id to its session and task.
    running: HashMap<String, (String, JoinHandle<()>)>,
    /// Ask id to session, for asks left undecided that are still open on the server.
    undecided: HashMap<String, String>,
}

impl Asks {
    fn known(&self, id: &str) -> bool {
        self.running.contains_key(id) || self.undecided.contains_key(id)
    }

    fn forget(&mut self, id: &str) {
        if let Some((_, task)) = self.running.remove(id) {
            task.abort();
        }
        self.undecided.remove(id);
    }

    fn forget_session(&mut self, session: &str) {
        let ids: Vec<String> = self
            .running
            .iter()
            .map(|(id, (asker, _))| (id, asker))
            .chain(self.undecided.iter())
            .filter(|(_, asker)| asker.as_str() == session)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.forget(&id);
        }
    }

    fn finished(&mut self, done: Done) {
        if let Some((session, _)) = self.running.remove(&done.id)
            && done.undecided
        {
            self.undecided.insert(done.id, session);
        }
    }
}

impl Drop for Asks {
    fn drop(&mut self) {
        for (_, task) in self.running.values() {
            task.abort();
        }
    }
}

/// Runs the link until it is aborted.
///
/// It reconnects after `retry` whenever the stream or the server is lost.
pub async fn run_link<S: Server, H: Humata>(server: Arc<S>, humata: Arc<H>, retry: Duration) {
    let mut activity = Activity::default();
    let mut asks = Asks::default();
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<Done>();
    loop {
        let Some(mut events) = connect(&*server, &mut activity, &humata, retry).await else {
            continue;
        };
        for asked in events.pending {
            start_ask(&mut asks, asked, &server, &humata, retry, &done_tx);
        }
        loop {
            tokio::select! {
                event = events.stream.recv() => {
                    let Some(event) = event else { break };
                    match &event {
                        Event::Asked(asked) => {
                            start_ask(&mut asks, asked.clone(), &server, &humata, retry, &done_tx);
                        }
                        Event::Replied { request, .. } => asks.forget(request),
                        Event::Ended { session } => asks.forget_session(session),
                        _ => {}
                    }
                    if let Some(turn) = activity.apply(&event) {
                        humata.turn(turn).await;
                    }
                }
                Some(done) = done_rx.recv() => asks.finished(done),
            }
        }
        tokio::time::sleep(retry).await;
    }
}

/// A stream opened and what the server said is open on it.
struct Opened {
    stream: mpsc::Receiver<Event>,
    pending: Vec<Asked>,
}

/// Opens the stream and reconciles with the server, or waits `retry` and gives `None`.
async fn connect<S: Server, H: Humata>(
    server: &S,
    activity: &mut Activity,
    humata: &Arc<H>,
    retry: Duration,
) -> Option<Opened> {
    let opened = async {
        let stream = server.events().await?;
        let active = server.active().await?;
        let pending = server.pending().await?;
        Ok::<_, ServerError>((stream, active, pending))
    }
    .await;
    let Ok((stream, active, pending)) = opened else {
        tokio::time::sleep(retry).await;
        return None;
    };
    let open = pending.iter().map(|a| (a.id.clone(), a.session.clone()));
    if let Some(turn) = activity.reset(active, open) {
        humata.turn(turn).await;
    }
    Some(Opened { stream, pending })
}

/// Starts the task that raises `asked`, unless one already has.
fn start_ask<S: Server, H: Humata>(
    asks: &mut Asks,
    asked: Asked,
    server: &Arc<S>,
    humata: &Arc<H>,
    retry: Duration,
    done: &mpsc::UnboundedSender<Done>,
) {
    if asks.known(&asked.id) {
        return;
    }
    let payload = serde_json::json!({
        "id": asked.id,
        "sessionID": asked.session,
        "action": asked.action,
        "resources": asked.resources,
    })
    .to_string()
    .into_bytes();
    let (server, humata, done) = (Arc::clone(server), Arc::clone(humata), done.clone());
    let (id, session) = (asked.id, asked.session);
    let task = tokio::spawn({
        let (id, session) = (id.clone(), session.clone());
        async move {
            let undecided = answer(&*server, &*humata, &session, &id, payload, retry).await;
            let _ = done.send(Done { id, undecided });
        }
    });
    asks.running.insert(id, (session, task));
}

/// Asks the daemon until it answers, then posts the answer. True if no decision was made.
async fn answer<S: Server, H: Humata>(
    server: &S,
    humata: &H,
    session: &str,
    id: &str,
    payload: Vec<u8>,
    retry: Duration,
) -> bool {
    loop {
        match humata.ask(payload.clone()).await {
            Err(_) => tokio::time::sleep(retry).await,
            Ok(body) if body.is_empty() => return true,
            Ok(body) => {
                match server.reply(session, id, body).await {
                    Ok(()) | Err(ServerError::Gone) => {}
                    Err(error) => {
                        eprintln!("humata: could not answer OpenCode's ask {id}: {error}")
                    }
                }
                return false;
            }
        }
    }
}
