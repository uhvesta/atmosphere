//! `humata codex` (D795): the program of the built-in Codex agent. It runs
//! Codex's TUI on an app-server of its own and, as the session's agent,
//! names the thread the person talks in, reports the turns and passes each
//! approval or question to the Inbox, the same way the other agents' hooks
//! do. It exits as the TUI does.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_codex::{AskKind, Humata, HumataError, Turn, Wrapper, run_wrapper};
use humata2_proto::humata::v1 as pb;
use prost::Message;
use tokio::sync::{mpsc, oneshot};

use crate::call::{Ctx, Done, Outcome};
use crate::console::Console;
use crate::daemon::{Daemon, Replies, rpc};
use crate::error::CliError;

/// The longest a turn report waits for the daemon.
const TURN_WAIT: Duration = Duration::from_secs(2);
/// The longest opening a hook stream waits for the daemon.
const OPEN_WAIT: Duration = Duration::from_secs(10);
/// How long the link waits before trying Codex or the daemon again.
const RETRY: Duration = Duration::from_secs(1);
/// How long the app-server has to make its socket.
const SOCKET_WAIT: Duration = Duration::from_secs(10);
/// The longest a socket path can be (macOS limits them to about 104 bytes).
const SOCKET_PATH_MAX: usize = 100;

/// What the link asks of the daemon, carried to the command's own task,
/// which holds the connection.
enum Call {
    Turn(Turn),
    Conversation(String),
    /// The link to the named thread dropped (D798).
    Lost,
    Ask {
        kind: AskKind,
        payload: Vec<u8>,
        answer: oneshot::Sender<Result<Vec<u8>, HumataError>>,
    },
}

/// The link's side of the daemon: each call goes to the command's task.
struct Bridge {
    calls: mpsc::UnboundedSender<Call>,
}

impl Humata for Bridge {
    async fn turn(&self, turn: Turn) {
        let _ = self.calls.send(Call::Turn(turn));
    }

    async fn conversation(&self, thread: String) {
        let _ = self.calls.send(Call::Conversation(thread));
    }

    async fn lost(&self) {
        let _ = self.calls.send(Call::Lost);
    }

    async fn ask(&self, kind: AskKind, payload: Vec<u8>) -> Result<Vec<u8>, HumataError> {
        let (answer, answered) = oneshot::channel();
        let lost = || HumataError::Lost {
            reason: "the `humata codex` command stopped".to_owned(),
        };
        self.calls
            .send(Call::Ask {
                kind,
                payload,
                answer,
            })
            .map_err(|_| lost())?;
        answered.await.unwrap_or_else(|_| Err(lost()))
    }
}

/// Where the app-server's socket folder goes: the temp folder, unless that
/// makes the socket's path too long for a unix socket.
fn socket_root() -> PathBuf {
    let temp = std::env::temp_dir();
    // The wrapper's folder is `h2cx-` and 16 characters, with `s` in it.
    let longest = temp.join("h2cx-0123456789abcdef").join("s");
    if longest.as_os_str().len() <= SOCKET_PATH_MAX {
        temp
    } else {
        PathBuf::from("/tmp")
    }
}

pub(super) async fn codex<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    program: PathBuf,
    hooks: Option<String>,
    args: Vec<OsString>,
) -> Result<Done, CliError> {
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let wrapper = Wrapper {
        program,
        hooks,
        args,
        folder,
        socket_root: socket_root(),
        retry: RETRY,
        wait_for_socket: SOCKET_WAIT,
    };
    let (calls, mut asked) = mpsc::unbounded_channel();
    let ran = run_wrapper(wrapper, Arc::new(Bridge { calls }));
    tokio::pin!(ran);
    let ran = loop {
        tokio::select! {
            ran = &mut ran => break ran,
            Some(call) = asked.recv() => match call {
                Call::Turn(turn) => report(ctx, turn).await,
                Call::Conversation(thread) => {
                    let payload = serde_json::json!({ "session_id": thread });
                    tell(ctx, "conversation", payload).await;
                }
                Call::Lost => tell(ctx, "lost", serde_json::json!({})).await,
                Call::Ask { kind, payload, answer } => open(ctx, kind, payload, answer).await,
            },
        }
    };
    let code = ran?;
    Ok(Done {
        result: serde_json::Value::Null,
        outcome: Outcome::Exited(code),
    })
}

/// Tells the daemon the agent's turn, as `humata turn` does. A lost report
/// is set right by the next one.
async fn report<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>, turn: Turn) {
    let state = match turn {
        Turn::Working => pb::TurnState::Working,
        // The person stopped it: the agent is waiting for them.
        Turn::Idle | Turn::Stopped => pb::TurnState::Idle,
    };
    let reported_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_nanos()).ok())
        .unwrap_or(0);
    let request = pb::ReportTurnRequest {
        state: state as i32,
        reported_at_unix_ns: reported_at,
    };
    let _ = tokio::time::timeout(TURN_WAIT, ctx.raw(rpc::session::REPORT_TURN, &request)).await;
}

/// Tells the daemon `event` with `payload` on the Hook stream and reads
/// it until it ends: `conversation` names the thread, as the agents' own
/// session-start hooks do, and `lost` says the link to it dropped. A call
/// that fails is set right when the thread is named again.
async fn tell<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>, event: &str, payload: serde_json::Value) {
    if ctx.dry_run_guard(rpc::session::HOOK).is_err() {
        return;
    }
    let request = pb::HookRequest {
        agent: "codex".to_owned(),
        event: event.to_owned(),
        payload: payload.to_string().into_bytes(),
    };
    let _ = tokio::time::timeout(OPEN_WAIT, async {
        let Ok(mut replies) = ctx
            .daemon
            .server_stream(rpc::session::HOOK, request.encode_to_vec())
            .await
        else {
            return;
        };
        while let Some(Ok(_)) = replies.recv().await {}
    })
    .await;
}

/// Opens the ask's hold on the daemon, as `humata hook` does, and leaves a
/// task reading it: the card stays up while the stream is held, and the
/// link letting go of the answer drops the stream, which withdraws it.
async fn open<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    kind: AskKind,
    payload: Vec<u8>,
    answer: oneshot::Sender<Result<Vec<u8>, HumataError>>,
) {
    // The hold raises an Inbox card; under `--dry-run` the ask is refused
    // (no answer), and nothing is sent.
    if let Err(error) = ctx.dry_run_guard(rpc::session::HOOK) {
        let _ = answer.send(Err(HumataError::Lost {
            reason: error.to_string(),
        }));
        return;
    }
    let event = match kind {
        AskKind::Question => "question",
        AskKind::Command | AskKind::FileChange => "permission",
    };
    let request = pb::HookRequest {
        agent: "codex".to_owned(),
        event: event.to_owned(),
        payload,
    };
    let opened = tokio::time::timeout(
        OPEN_WAIT,
        ctx.daemon
            .server_stream(rpc::session::HOOK, request.encode_to_vec()),
    )
    .await;
    match opened {
        Ok(Ok(replies)) => {
            tokio::spawn(hold(replies, answer));
        }
        Ok(Err(error)) => {
            let _ = answer.send(Err(HumataError::Lost {
                reason: error.to_string(),
            }));
        }
        Err(_) => {
            let _ = answer.send(Err(HumataError::Lost {
                reason: "the daemon didn't answer in time".to_owned(),
            }));
        }
    }
}

/// Reads the hold until its last reply, or until the link lets go.
async fn hold(mut replies: Replies, mut answer: oneshot::Sender<Result<Vec<u8>, HumataError>>) {
    let lost = |reason: String| Err(HumataError::Lost { reason });
    let result = loop {
        tokio::select! {
            () = answer.closed() => return,
            reply = replies.recv() => match reply {
                None => break lost("the daemon ended the hold without an answer".to_owned()),
                Some(Err(error)) => break lost(error.to_string()),
                Some(Ok(bytes)) => match pb::HookReply::decode(bytes.as_slice()) {
                    Ok(reply) if reply.keepalive => continue,
                    Ok(reply) => break Ok(reply.stdout),
                    Err(error) => break lost(error.to_string()),
                },
            },
        }
    };
    let _ = answer.send(result);
}
