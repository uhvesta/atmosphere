//! The MCP server: one per agent process, on stdio (MCP.md §1).
//!
//! Lines are read in order; `initialize`, `ping` and `tools/list` are
//! answered at once, and each `tools/call` runs as its own task so a call
//! that waits for the user doesn't hold up the others. One writer sends
//! every reply and notification.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};
use uuid::Uuid;

use crate::daemon::{Call, Daemon, ErrorKind, IdempotencyKey, Reply, RequestId, ToolError, WaitEvent};
use crate::error::{ServeError, TokenError};
use crate::jsonrpc::{self, INVALID_PARAMS, Incoming, METHOD_NOT_FOUND, PARSE_ERROR, RpcError};
use crate::outcome;
use crate::session::SessionToken;
use crate::tools::{self, Operation};
use crate::user_only;

/// The MCP versions this server speaks, newest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];

const SERVER_NAME: &str = "humata";

/// The server's version in `initialize`; the daemon's own version is in
/// `whoami`.
const SERVER_VERSION: &str = "2.0.0";

/// TP-11: how long a call waits for the user when it doesn't say.
const DEFAULT_WAIT_SECONDS: u64 = 45;

/// Progress is sent only while fewer lines than this wait to be written:
/// each one only says "still waiting", so when the agent stops reading
/// they're dropped rather than queued without end (TR-8). Replies are
/// never dropped.
const PROGRESS_BACKLOG: usize = 64;

/// The MCP server for one agent process.
pub struct Server<D> {
    daemon: Arc<D>,
    token: Result<SessionToken, TokenError>,
    nonce: String,
}

impl<D: Daemon> Server<D> {
    /// A server for the session whose token is `token`, usually
    /// [`SessionToken::from_env`]. Without a token it still serves, and
    /// every call fails with `identity_mismatch` saying why.
    pub fn new(daemon: D, token: Result<SessionToken, TokenError>) -> Self {
        Self {
            daemon: Arc::new(daemon),
            token,
            // D29: each MCP process adds its own nonce to its keys, so a
            // restarted server's request ids are new intents.
            nonce: Uuid::new_v4().simple().to_string(),
        }
    }

    /// Serves on this process's stdin and stdout until stdin closes.
    pub async fn serve_stdio(self) -> Result<(), ServeError> {
        self.serve(tokio::io::stdin(), tokio::io::stdout()).await
    }

    /// Serves newline-delimited JSON-RPC on `input` and `output` until
    /// `input` ends. Then every call still open is stopped, and each
    /// request one was waiting on that never returned `pending` is
    /// withdrawn (TP-15).
    pub async fn serve<R, W>(self, input: R, output: W) -> Result<(), ServeError>
    where
        R: AsyncRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        // Unbounded but not unlimited: replies are one per request the
        // agent made, and progress is skipped when the writer is behind.
        let (out, outgoing) = mpsc::unbounded_channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let writer = tokio::spawn(write_all(output, outgoing, Arc::clone(&queued)));
        let shared = Arc::new(Shared {
            daemon: self.daemon,
            token: self.token,
            nonce: self.nonce,
            binding: tokio::sync::Mutex::new(Binding::Unbound),
            out,
            queued,
            calls: Mutex::new(HashMap::new()),
            withdrawals: Mutex::new(JoinSet::new()),
        });

        // MCP.md §1: present the token on start. A call that comes first
        // waits for it.
        let binder = {
            let shared = Arc::clone(&shared);
            tokio::spawn(async move {
                // A failure is kept and returned to every call.
                let _ = shared.ensure_bound().await;
            })
        };

        let read = shared.read_all(input).await;

        let stopped: Vec<InFlight> = lock(&shared.calls).drain().map(|(_, call)| call).collect();
        for call in stopped {
            call.abort.abort();
            if let Some(request) = call.open {
                shared.withdraw_later(request);
            }
        }
        let mut withdrawals = std::mem::take(&mut *lock(&shared.withdrawals));
        while withdrawals.join_next().await.is_some() {}
        binder.abort();
        shared.send_close();

        let written = match writer.await {
            Ok(written) => written.map_err(|source| ServeError::Write { source }),
            Err(failed) => Err(ServeError::Write {
                source: io::Error::other(failed),
            }),
        };
        read?;
        written
    }
}

/// What the writer sends.
enum Outgoing {
    Message(Value),
    /// Everything before this has been sent; stop.
    Close,
}

async fn write_all<W>(
    mut output: W,
    mut outgoing: mpsc::UnboundedReceiver<Outgoing>,
    queued: Arc<AtomicUsize>,
) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    while let Some(next) = outgoing.recv().await {
        match next {
            Outgoing::Message(message) => {
                queued.fetch_sub(1, Ordering::Relaxed);
                let mut line = message.to_string();
                line.push('\n');
                output.write_all(line.as_bytes()).await?;
                output.flush().await?;
            }
            Outgoing::Close => break,
        }
    }
    Ok(())
}

/// Whether the daemon has bound this connection to the session.
enum Binding {
    Unbound,
    Bound,
    Refused(ToolError),
}

/// A `tools/call` that hasn't answered yet.
struct InFlight {
    id: Value,
    abort: AbortHandle,
    /// The Inbox request it waits on before telling the agent `pending`.
    open: Option<RequestId>,
}

struct Shared<D> {
    daemon: Arc<D>,
    token: Result<SessionToken, TokenError>,
    nonce: String,
    binding: tokio::sync::Mutex<Binding>,
    out: mpsc::UnboundedSender<Outgoing>,
    /// Messages sent to the writer and not yet taken by it.
    queued: Arc<AtomicUsize>,
    calls: Mutex<HashMap<u64, InFlight>>,
    withdrawals: Mutex<JoinSet<()>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<D: Daemon> Shared<D> {
    fn send(&self, message: Value) {
        // A closed writer means the agent is gone; nothing reads replies.
        self.queued.fetch_add(1, Ordering::Relaxed);
        if self.out.send(Outgoing::Message(message)).is_err() {
            self.queued.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Sends a notification that only says the call is still going, unless
    /// the writer is behind (PROGRESS_BACKLOG).
    fn send_progress(&self, message: Value) {
        if self.queued.load(Ordering::Relaxed) < PROGRESS_BACKLOG {
            self.send(message);
        }
    }

    fn send_close(&self) {
        let _ = self.out.send(Outgoing::Close);
    }

    async fn read_all<R>(self: &Arc<Self>, input: R) -> Result<(), ServeError>
    where
        R: AsyncRead + Unpin + Send,
    {
        let mut reader = BufReader::new(input);
        let mut buffer = Vec::new();
        let mut next_call: u64 = 0;
        loop {
            buffer.clear();
            let read = reader
                .read_until(b'\n', &mut buffer)
                .await
                .map_err(|source| ServeError::Read { source })?;
            if read == 0 {
                return Ok(());
            }
            let Ok(line) = std::str::from_utf8(&buffer) else {
                self.send(jsonrpc::error(
                    &Value::Null,
                    &RpcError::new(PARSE_ERROR, "the line isn't UTF-8"),
                ));
                continue;
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match jsonrpc::parse(line) {
                Incoming::Request { id, method, params } if method == "tools/call" => {
                    let seq = next_call;
                    next_call += 1;
                    let mut calls = lock(&self.calls);
                    let task = tokio::spawn(Arc::clone(self).run_call(seq, id.clone(), params));
                    calls.insert(
                        seq,
                        InFlight {
                            id,
                            abort: task.abort_handle(),
                            open: None,
                        },
                    );
                }
                Incoming::Request { id, method, params } => {
                    let reply = match answer(&method, &params) {
                        Ok(result) => jsonrpc::result(&id, result),
                        Err(error) => jsonrpc::error(&id, &error),
                    };
                    self.send(reply);
                }
                Incoming::Notification { method, params } => {
                    if method == "notifications/cancelled" {
                        self.cancel(&params);
                    }
                    // notifications/initialized and the rest need nothing.
                }
                Incoming::Response => {}
                Incoming::Invalid { id, error } => self.send(jsonrpc::error(&id, &error)),
            }
        }
    }

    /// The agent's client gave up on a call: stop it, send nothing for it,
    /// and withdraw the request it was waiting on (TP-15).
    fn cancel(&self, params: &Value) {
        let Some(target) = params.get("requestId") else {
            return;
        };
        let cancelled: Vec<InFlight> = {
            let mut calls = lock(&self.calls);
            let seqs: Vec<u64> = calls
                .iter()
                .filter(|(_, call)| &call.id == target)
                .map(|(seq, _)| *seq)
                .collect();
            seqs.iter().filter_map(|seq| calls.remove(seq)).collect()
        };
        for call in cancelled {
            call.abort.abort();
            if let Some(request) = call.open {
                self.withdraw_later(request);
            }
        }
    }

    /// Withdraws `request` in a task of its own, which `serve` waits for
    /// before it returns.
    fn withdraw_later(&self, request: RequestId) {
        let daemon = Arc::clone(&self.daemon);
        lock(&self.withdrawals).spawn(async move {
            // Nobody is left to tell if this fails; the request then stays
            // in the Inbox until the session stops.
            let _ = daemon.withdraw(&request).await;
        });
    }

    async fn run_call(self: Arc<Self>, seq: u64, id: Value, params: Value) {
        let reply = match self.tools_call(seq, &id, params).await {
            Ok(result) => jsonrpc::result(&id, result),
            Err(error) => jsonrpc::error(&id, &error),
        };
        // A cancelled call has left the table, and its client wants no
        // answer. Leaving it here also drops its open request: the agent is
        // told the outcome, `pending` included, so nothing is withdrawn.
        let finished = lock(&self.calls).remove(&seq).is_some();
        if finished {
            self.send(reply);
        }
    }

    async fn tools_call(&self, seq: u64, id: &Value, params: Value) -> Result<Value, RpcError> {
        let Value::Object(mut params) = params else {
            return Err(RpcError::new(
                INVALID_PARAMS,
                "tools/call takes an object with the tool's name and arguments",
            ));
        };
        let name = match params.remove("name") {
            Some(Value::String(name)) => name,
            _ => {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    "tools/call needs the tool's name; tools/list gives them",
                ));
            }
        };
        let arguments = match params.remove("arguments") {
            None | Some(Value::Null) => Value::Object(Map::new()),
            Some(arguments @ Value::Object(_)) => arguments,
            Some(_) => {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    format!("{name}: arguments must be an object"),
                )
                .with_data(json!({ "tool": name, "field": "/" })));
            }
        };
        let progress = params
            .get("_meta")
            .and_then(|meta| meta.get("progressToken"))
            .filter(|token| token.is_string() || token.is_number())
            .cloned();

        let Some(operation) = Operation::from_tool_name(&name) else {
            // MCP.md §7: refused before anything else, and nothing reaches
            // the daemon or the Inbox.
            if let Some(refusal) = user_only::refusal(&name) {
                return Ok(outcome::failure(&refusal));
            }
            return Err(RpcError::new(
                INVALID_PARAMS,
                format!("there is no tool {name}; tools/list gives the tools"),
            )
            .with_data(json!({ "tool": name })));
        };
        if let Err(violation) = tools::input_schema(operation).validate(&arguments) {
            return Err(RpcError::new(INVALID_PARAMS, format!("{name}: {violation}")).with_data(
                json!({
                    "tool": name,
                    "field": violation.path,
                    "problem": violation.problem.name(),
                }),
            ));
        }
        let mut arguments = match arguments {
            Value::Object(arguments) => arguments,
            _ => Map::new(),
        };

        if let Err(refused) = self.ensure_bound().await {
            return Ok(outcome::failure(&refused));
        }

        let wait = Duration::from_secs(
            arguments
                .remove("wait_seconds")
                .and_then(|seconds| seconds.as_u64())
                .unwrap_or(DEFAULT_WAIT_SECONDS),
        );

        if operation == Operation::InboxWait {
            let request = RequestId::new(
                arguments
                    .get("request_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
            return Ok(self.wait_for(request, wait, progress.as_ref()).await);
        }

        let call = Call {
            operation,
            arguments,
            idempotency_key: self.key(id),
        };
        match self.daemon.call(call).await {
            Ok(Reply::Pending { request }) if !wait.is_zero() => {
                if !self.mark_open(seq, &request) {
                    // Cancelled while the daemon was making the request.
                    self.withdraw_later(request.clone());
                    return Ok(outcome::success(Reply::Pending { request }, None));
                }
                Ok(self.wait_for(request, wait, progress.as_ref()).await)
            }
            Ok(reply) => Ok(outcome::success(reply, None)),
            Err(error) => Ok(outcome::failure(&error)),
        }
    }

    /// Records that call `seq` waits on `request`; false when the call was
    /// cancelled already.
    fn mark_open(&self, seq: u64, request: &RequestId) -> bool {
        match lock(&self.calls).get_mut(&seq) {
            Some(call) => {
                call.open = Some(request.clone());
                true
            }
            None => false,
        }
    }

    /// Waits for the user's answer to `request` (TP-11, TP-12), sending
    /// progress on each keepalive when the client asked for it.
    async fn wait_for(&self, request: RequestId, wait: Duration, progress: Option<&Value>) -> Value {
        let mut events = match self.daemon.wait(&request, wait).await {
            Ok(events) => events,
            Err(error) => return outcome::failure(&error),
        };
        let mut sent: u64 = 0;
        while let Some(event) = events.recv().await {
            match event {
                WaitEvent::Keepalive => {
                    if let Some(token) = progress {
                        sent += 1;
                        self.send_progress(jsonrpc::notification(
                            "notifications/progress",
                            json!({
                                "progressToken": token,
                                "progress": sent,
                                "message": "Waiting for the user to answer in the Inbox",
                            }),
                        ));
                    }
                }
                WaitEvent::Done(Ok(reply)) => return outcome::success(reply, Some(&request)),
                WaitEvent::Done(Err(error)) => return outcome::failure(&error),
            }
        }
        // The daemon ended the wait without an answer. The request is still
        // in the Inbox; inbox_wait picks it up again.
        outcome::success(Reply::Pending { request }, None)
    }

    /// TP-14, D29: the JSON-RPC id plus this process's nonce. A string id
    /// and a number id never share a key.
    fn key(&self, id: &Value) -> IdempotencyKey {
        let id = match id {
            Value::String(text) => format!("s:{text}"),
            other => format!("n:{other}"),
        };
        IdempotencyKey::new(format!("mcp-{}-{id}", self.nonce))
    }

    async fn ensure_bound(&self) -> Result<(), ToolError> {
        let mut binding = self.binding.lock().await;
        match &*binding {
            Binding::Bound => return Ok(()),
            Binding::Refused(refused) if !retryable(refused) => return Err(refused.clone()),
            Binding::Refused(_) | Binding::Unbound => {}
        }
        let token = match &self.token {
            Ok(token) => token,
            Err(missing) => {
                let refused = no_token(missing);
                *binding = Binding::Refused(refused.clone());
                return Err(refused);
            }
        };
        match self.daemon.bind(token).await {
            Ok(()) => {
                *binding = Binding::Bound;
                Ok(())
            }
            Err(refused) => {
                *binding = Binding::Refused(refused.clone());
                Err(refused)
            }
        }
    }
}

fn retryable(error: &ToolError) -> bool {
    error.retry.as_ref().is_some_and(|retry| retry.retryable)
}

fn no_token(missing: &TokenError) -> ToolError {
    ToolError {
        kind: ErrorKind::IdentityMismatch,
        message: missing.to_string(),
        fix: "Start or restart this session from Humata, which launches the agent with its token. \
              Setting the variable by hand doesn't help: the daemon accepts a token only from \
              inside its own session's process tree."
            .to_owned(),
        retry: None,
        details: Map::new(),
    }
}

/// Requests answered without the daemon.
fn answer(method: &str, params: &Value) -> Result<Value, RpcError> {
    match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools::list()),
        _ => Err(RpcError::new(
            METHOD_NOT_FOUND,
            format!("{method} isn't offered; this server has tools only"),
        )
        .with_data(json!({ "method": method }))),
    }
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = PROTOCOL_VERSIONS
        .iter()
        .copied()
        .find(|known| asked == Some(*known))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
        "instructions": "Humata's tools for this session. You are always this session: no tool takes a caller. A call that needs the user's approval returns pending with a request_id; then call inbox_wait with it, and never repeat the call itself. Branch on error.kind, never on the text.",
    })
}
