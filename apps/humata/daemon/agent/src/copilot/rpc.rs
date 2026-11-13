//! A client for the Copilot CLI's server mode: JSON-RPC 2.0 over its stdin
//! and stdout, each message framed by a `Content-Length` header.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};

use super::CopilotError;

/// The protocol this client speaks, as `connect` reports it.
pub(super) const PROTOCOL_VERSION: i64 = 3;

/// A message bigger than this is taken as a broken stream.
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

/// How a call failed on the other side.
type Answer = Result<Value, (i64, String)>;

/// One running Copilot CLI.
pub(super) struct Cli {
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    next_id: AtomicU64,
    shared: Arc<Mutex<Shared>>,
    /// Killed when dropped.
    _child: Child,
    _noted: Option<crate::children::Noted>,
}

#[derive(Default)]
struct Shared {
    closed: bool,
    pending: HashMap<u64, oneshot::Sender<Answer>>,
    /// Each Copilot session's events, to whoever is playing its turn.
    subscribers: HashMap<String, mpsc::UnboundedSender<Value>>,
    /// Copilot sessions this process has created or resumed, with what
    /// they were opened with.
    live: HashMap<String, String>,
}

impl Cli {
    /// Starts `program` in server mode and checks that it speaks our protocol.
    pub(super) async fn start(
        program: &Path,
        experimental: &str,
        env: &[(String, String)],
        running: &Path,
    ) -> Result<Arc<Self>, CopilotError> {
        let mut child = Command::new(program)
            .args(["--server", "--stdio", "--no-auto-update", experimental])
            .env("COPILOT_AUTO_UPDATE", "false")
            .envs(env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| CopilotError::Start {
                program: program.to_owned(),
                source,
            })?;
        let noted = crate::children::Noted::note(running, child.id());
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(CopilotError::Exited);
        };
        let stdin = Arc::new(tokio::sync::Mutex::new(stdin));
        let shared = Arc::new(Mutex::new(Shared::default()));
        tokio::spawn(read(stdout, stdin.clone(), shared.clone()));
        let cli = Arc::new(Self {
            stdin,
            next_id: AtomicU64::new(1),
            shared,
            _child: child,
            _noted: noted,
        });
        let hello = cli.call("connect", json!({})).await?;
        let found = hello.get("protocolVersion").and_then(Value::as_i64);
        if found != Some(PROTOCOL_VERSION) {
            return Err(CopilotError::Protocol {
                found: found.map_or_else(|| "none".to_owned(), |version| version.to_string()),
            });
        }
        Ok(cli)
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// False once the process has stopped or its output broke.
    pub(super) fn is_running(&self) -> bool {
        !self.lock().closed
    }

    /// Whether `session` is open here, opened with `with`.
    pub(super) fn is_live(&self, session: &str, with: &str) -> bool {
        self.lock()
            .live
            .get(session)
            .is_some_and(|opened| opened == with)
    }

    pub(super) fn set_live(&self, session: &str, with: &str) {
        self.lock().live.insert(session.to_owned(), with.to_owned());
    }

    /// The events of `session` from now on, replacing any earlier subscriber.
    /// The receiver ends if the process stops.
    pub(super) fn subscribe(&self, session: &str) -> mpsc::UnboundedReceiver<Value> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut shared = self.lock();
        if !shared.closed {
            shared.subscribers.insert(session.to_owned(), sender);
        }
        receiver
    }

    pub(super) async fn call(&self, method: &str, params: Value) -> Result<Value, CopilotError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        {
            let mut shared = self.lock();
            if shared.closed {
                return Err(CopilotError::Exited);
            }
            shared.pending.insert(id, answer);
        }
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if let Err(source) = write(&self.stdin, &message).await {
            self.lock().pending.remove(&id);
            return Err(CopilotError::Pipe { source });
        }
        match answered.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err((code, message))) => Err(CopilotError::Rpc {
                method: method.to_owned(),
                code,
                message,
            }),
            Err(_) => Err(CopilotError::Exited),
        }
    }
}

async fn write(
    stdin: &tokio::sync::Mutex<ChildStdin>,
    message: &Value,
) -> Result<(), std::io::Error> {
    let body = message.to_string();
    let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    framed.extend_from_slice(body.as_bytes());
    let mut stdin = stdin.lock().await;
    stdin.write_all(&framed).await?;
    stdin.flush().await
}

/// Reads messages until the output ends or breaks, then fails every call
/// still waiting and ends every subscription.
async fn read(
    stdout: ChildStdout,
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    shared: Arc<Mutex<Shared>>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        match next_message(&mut reader).await {
            Ok(Some(message)) => dispatch(message, &stdin, &shared).await,
            Ok(None) => break,
            Err(error) => {
                eprintln!("humata: the Copilot CLI's output broke: {error}");
                break;
            }
        }
    }
    let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
    shared.closed = true;
    shared.pending.clear();
    shared.subscribers.clear();
    shared.live.clear();
}

#[derive(Debug, thiserror::Error)]
enum FrameError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("a message had no usable Content-Length")]
    Length,
    #[error("a message wasn't JSON: {0}")]
    Json(#[from] serde_json::Error),
}

async fn next_message(reader: &mut BufReader<ChildStdout>) -> Result<Option<Value>, FrameError> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| FrameError::Length)?,
            );
        }
    }
    let length = length
        .filter(|length| *length <= MAX_MESSAGE)
        .ok_or(FrameError::Length)?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    Ok(Some(serde_json::from_slice(&body)?))
}

async fn dispatch(message: Value, stdin: &tokio::sync::Mutex<ChildStdin>, shared: &Mutex<Shared>) {
    let lock = || shared.lock().unwrap_or_else(PoisonError::into_inner);
    let id = message.get("id").cloned();
    match (message.get("method").and_then(Value::as_str), id) {
        // A response.
        (None, Some(id)) => {
            let Some(id) = id.as_u64() else { return };
            let Some(answer) = lock().pending.remove(&id) else {
                return;
            };
            let result = match message.get("error") {
                Some(error) => Err((
                    error.get("code").and_then(Value::as_i64).unwrap_or(0),
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                )),
                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = answer.send(result);
        }
        // A request to us: none is needed, so each is refused.
        (Some(method), Some(id)) => {
            let refusal = json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("Humata doesn't handle {method}")},
            });
            let _ = write(stdin, &refusal).await;
        }
        (Some("session.event"), None) => {
            let Some(params) = message.get("params") else {
                return;
            };
            let (Some(session), Some(event)) = (
                params.get("sessionId").and_then(Value::as_str),
                params.get("event"),
            ) else {
                return;
            };
            let mut shared = lock();
            if let Some(subscriber) = shared.subscribers.get(session)
                && subscriber.send(event.clone()).is_err()
            {
                shared.subscribers.remove(session);
            }
        }
        _ => {}
    }
}
