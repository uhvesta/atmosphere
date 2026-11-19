//! The wrapper's link: Codex's app-server is spoken to over its websocket
//! JSON-RPC on a unix socket. Turns and the conversation go to the daemon,
//! asks go to the Inbox and their answers come back to Codex, across
//! dropped connections.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::SplitStream;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{WebSocketStream, client_async};

use crate::event::{AskKind, Event, RequestId, answer_frame, read_frame};

/// What the turn in Codex is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// A turn is running.
    Working,
    /// The turn finished.
    Idle,
    /// The person interrupted the turn.
    Stopped,
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

/// The Humata daemon, as the link needs it.
pub trait Humata: Send + Sync + 'static {
    /// Reports a changed turn.
    fn turn(&self, turn: Turn) -> impl Future<Output = ()> + Send;
    /// Names the thread the person is talking in.
    fn conversation(&self, thread: String) -> impl Future<Output = ()> + Send;
    /// Raises an ask and waits for the answer's body. An empty body is no
    /// decision. Dropping the future withdraws the ask.
    fn ask(
        &self,
        kind: AskKind,
        payload: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, HumataError>> + Send;
    /// The link to the thread it named dropped (D798): until it is named
    /// again, Humata reads the session's screen for asks.
    fn lost(&self) -> impl Future<Output = ()> + Send {
        async {}
    }
}

/// Why one connection to the app-server ended.
#[derive(Debug, thiserror::Error)]
enum LinkError {
    #[error(
        "Could not connect to Codex's app-server at {path}. It may have stopped; the link tries \
         again on its own."
    )]
    Connect {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "Codex's app-server did not complete the websocket handshake. The socket may not belong \
         to `codex app-server`; restart the session."
    )]
    Handshake {
        #[source]
        source: tokio_tungstenite::tungstenite::Error,
    },
    #[error(
        "Codex's app-server connection failed. It may have stopped; the link tries again on its \
         own."
    )]
    Socket {
        #[source]
        source: tokio_tungstenite::tungstenite::Error,
    },
    #[error("Codex's app-server closed the connection; the link tries again on its own.")]
    Closed,
    #[error(
        "Codex's app-server rejected `{method}`: {message}. Update Codex or Humata if their \
         versions differ."
    )]
    Rejected { method: &'static str, message: String },
    #[error(
        "Codex's reply to `{method}` has no {what}. This Codex may differ from 0.169.0: update \
         Humata if Codex is newer, or upgrade Codex if it is older."
    )]
    Unexpected { method: &'static str, what: &'static str },
}

/// Runs the link until it is aborted.
///
/// It reconnects after `retry` whenever the connection is lost.
pub async fn run_link<H: Humata>(socket: PathBuf, humata: Arc<H>, retry: Duration) {
    let mut named: Option<String> = None;
    loop {
        if let Err(error) = connection(&socket, &humata, &mut named).await {
            eprintln!("humata: Codex link: {error}");
        }
        // A link that had named its thread is told dropped, and names it
        // again once connected, so Humata hears the session once more.
        if named.take().is_some() {
            humata.lost().await;
        }
        tokio::time::sleep(retry).await;
    }
}

struct Conn<H: Humata> {
    humata: Arc<H>,
    out: mpsc::UnboundedSender<String>,
    writer: JoinHandle<()>,
    stream: SplitStream<WebSocketStream<UnixStream>>,
    backlog: VecDeque<String>,
    next_id: u64,
    asks: HashMap<RequestId, JoinHandle<()>>,
}

impl<H: Humata> Drop for Conn<H> {
    fn drop(&mut self) {
        self.writer.abort();
        for task in self.asks.values() {
            task.abort();
        }
    }
}

async fn connection<H: Humata>(
    socket: &Path,
    humata: &Arc<H>,
    named: &mut Option<String>,
) -> Result<(), LinkError> {
    let unix = UnixStream::connect(socket)
        .await
        .map_err(|source| LinkError::Connect { path: socket.to_owned(), source })?;
    let (ws, _) = client_async("ws://localhost/", unix)
        .await
        .map_err(|source| LinkError::Handshake { source })?;
    let (mut sink, stream) = ws.split();
    let (out, mut queue) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(text) = queue.recv().await {
            if sink.send(Message::text(text)).await.is_err() {
                break;
            }
        }
    });
    let mut conn = Conn {
        humata: Arc::clone(humata),
        out,
        writer,
        stream,
        backlog: VecDeque::new(),
        next_id: 1,
        asks: HashMap::new(),
    };

    conn.request(
        "initialize",
        json!({"clientInfo": {"name": "humata", "title": "Humata", "version": env!("CARGO_PKG_VERSION")}}),
    )
    .await?;
    conn.notify("initialized");
    let list = conn.request("thread/loaded/list", json!({})).await?;
    let loaded = list
        .get("data")
        .and_then(Value::as_array)
        .ok_or(LinkError::Unexpected { method: "thread/loaded/list", what: "`data` list" })?;
    for id in loaded.iter().filter_map(Value::as_str) {
        // A refused resume still leaves the thread worth naming.
        let _ = conn.request("thread/resume", resume(id)).await;
        conn.name(id, named).await;
    }

    loop {
        let text = match conn.backlog.pop_front() {
            Some(text) => text,
            None => conn.read_text().await?,
        };
        conn.handle(&text, named).await;
    }
}

fn resume(thread: &str) -> Value {
    json!({"threadId": thread, "excludeTurns": true})
}

impl<H: Humata> Conn<H> {
    fn write(&self, frame: Value) {
        let _ = self.out.send(frame.to_string());
    }

    fn notify(&self, method: &str) {
        self.write(json!({"jsonrpc": "2.0", "method": method}));
    }

    fn send_request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    /// The next text message, or why the connection is over.
    async fn read_text(&mut self) -> Result<String, LinkError> {
        loop {
            let message = tokio::select! {
                _ = &mut self.writer => return Err(LinkError::Closed),
                message = self.stream.next() => message,
            };
            match message {
                None | Some(Ok(Message::Close(_))) => return Err(LinkError::Closed),
                Some(Err(source)) => return Err(LinkError::Socket { source }),
                Some(Ok(Message::Text(text))) => return Ok(text.as_str().to_owned()),
                Some(Ok(_)) => {}
            }
        }
    }

    /// Sends a request and waits for its reply; other messages wait in the backlog.
    async fn request(&mut self, method: &'static str, params: Value) -> Result<Value, LinkError> {
        let id = self.send_request(method, params);
        loop {
            let text = self.read_text().await?;
            let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if value.get("method").is_some() || value.get("id").is_none() {
                self.backlog.push_back(text);
                continue;
            }
            if value["id"] != id {
                continue;
            }
            if let Some(error) = value.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("no reason given")
                    .to_owned();
                return Err(LinkError::Rejected { method, message });
            }
            return Ok(value["result"].take());
        }
    }

    /// Names `thread` as the conversation unless it already is.
    async fn name(&self, thread: &str, named: &mut Option<String>) {
        if named.as_deref() != Some(thread) {
            *named = Some(thread.to_owned());
            self.humata.conversation(thread.to_owned()).await;
        }
    }

    async fn handle(&mut self, text: &str, named: &mut Option<String>) {
        let Ok(Some(event)) = read_frame(text) else {
            return;
        };
        match event {
            Event::ThreadStarted { thread, .. } => {
                self.name(&thread, named).await;
                self.send_request("thread/resume", resume(&thread));
            }
            Event::TurnStarted { .. } => self.humata.turn(Turn::Working).await,
            Event::TurnCompleted { interrupted, .. } => {
                self.humata.turn(if interrupted { Turn::Stopped } else { Turn::Idle }).await;
            }
            Event::Ask { request, thread, kind, params, .. } => {
                self.asks.retain(|_, task| !task.is_finished());
                let payload = ask_payload(kind, &thread, &params).to_string().into_bytes();
                let (humata, out, id) = (Arc::clone(&self.humata), self.out.clone(), request.clone());
                let task = tokio::spawn(async move {
                    if let Ok(body) = humata.ask(kind, payload).await
                        && let Some(result) = decision(kind, &body)
                    {
                        let _ = out.send(answer_frame(&id, result));
                    }
                });
                if let Some(old) = self.asks.insert(request, task) {
                    old.abort();
                }
            }
            Event::Resolved { request, .. } => {
                if let Some(task) = self.asks.remove(&request) {
                    task.abort();
                }
            }
        }
    }
}

/// An ask as the daemon reads it: Codex's own permission hook payload for
/// approvals, the question's params as they are.
fn ask_payload(kind: AskKind, thread: &str, params: &Value) -> Value {
    let reason = params.get("reason").and_then(Value::as_str);
    let (tool, command) = match kind {
        AskKind::Question => return params.clone(),
        AskKind::Command => ("Bash", params.get("command").and_then(Value::as_str).unwrap_or("")),
        AskKind::FileChange => ("apply_patch", reason.unwrap_or("a file change")),
    };
    json!({
        "session_id": thread,
        "turn_id": params.get("turnId"),
        "cwd": params.get("cwd").and_then(Value::as_str),
        "hook_event_name": "PermissionRequest",
        "tool_name": tool,
        "tool_input": {"command": command, "description": reason},
    })
}

/// What Codex is told for the daemon's answer; `None` leaves it to Codex's popup.
fn decision(kind: AskKind, body: &[u8]) -> Option<Value> {
    if body.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_slice(body).ok()?;
    match kind {
        AskKind::Question => Some(value),
        AskKind::Command | AskKind::FileChange => {
            let behavior = value.pointer("/hookSpecificOutput/decision/behavior")?.as_str()?;
            match behavior {
                "allow" => Some(json!({"decision": "accept"})),
                "deny" => Some(json!({"decision": "decline"})),
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approvals_become_a_decision_and_anything_else_none() {
        let allow = br#"{"hookSpecificOutput":{"decision":{"behavior":"allow"}}}"#;
        let deny = br#"{"hookSpecificOutput":{"decision":{"behavior":"deny"}}}"#;
        assert_eq!(decision(AskKind::Command, allow), Some(json!({"decision": "accept"})));
        assert_eq!(decision(AskKind::FileChange, deny), Some(json!({"decision": "decline"})));
        assert_eq!(decision(AskKind::Command, b""), None);
        assert_eq!(decision(AskKind::Command, b"{}"), None);
    }
}
