//! `humata opencode` (D201, D201a): the program of the built-in OpenCode
//! agent. It runs OpenCode's TUI attached to a server of its own and, as the
//! session's agent, reports the turns that server sees and passes each of
//! its permission asks to the Inbox, the same way the other agents' hooks
//! do. It exits as the TUI does.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_opencode::{Humata, HumataError, Turn, Wrapper, run_wrapper};
use humata2_proto::humata::v1 as pb;
use prost::Message;
use tokio::sync::{mpsc, oneshot};

use crate::call::{Ctx, Done, Outcome};
use crate::console::Console;
use crate::daemon::{Daemon, Replies, rpc};
use crate::error::CliError;

/// The longest a turn report waits for the daemon.
const TURN_WAIT: Duration = Duration::from_secs(2);
/// The longest opening an ask's hold waits for the daemon.
const OPEN_WAIT: Duration = Duration::from_secs(10);
/// How long the link waits before trying OpenCode or the daemon again.
const RETRY: Duration = Duration::from_secs(1);
/// How long `opencode serve` has to say where it listens.
const URL_WAIT: Duration = Duration::from_secs(20);

/// What the link asks of the daemon, carried to the command's own task,
/// which holds the connection.
enum Call {
    Turn(Turn),
    Ask {
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

    async fn ask(&self, payload: Vec<u8>) -> Result<Vec<u8>, HumataError> {
        let (answer, answered) = oneshot::channel();
        let lost = || HumataError::Lost {
            reason: "the `humata opencode` command stopped".to_owned(),
        };
        self.calls
            .send(Call::Ask { payload, answer })
            .map_err(|_| lost())?;
        answered.await.unwrap_or_else(|_| Err(lost()))
    }
}

pub(super) async fn opencode<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    conversation: String,
    state: PathBuf,
    program: PathBuf,
) -> Result<Done, CliError> {
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let wrapper = Wrapper {
        program,
        conversation,
        state,
        folder,
        retry: RETRY,
        wait_for_url: URL_WAIT,
    };
    let (calls, mut asked) = mpsc::unbounded_channel();
    let ran = run_wrapper(wrapper, Arc::new(Bridge { calls }));
    tokio::pin!(ran);
    let ran = loop {
        tokio::select! {
            ran = &mut ran => break ran,
            Some(call) = asked.recv() => match call {
                Call::Turn(turn) => report(ctx, turn).await,
                Call::Ask { payload, answer } => open(ctx, payload, answer).await,
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
        Turn::Idle => pb::TurnState::Idle,
        Turn::NeedsInput => pb::TurnState::NeedsInput,
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

/// Opens the ask's hold on the daemon, as `humata hook` does, and leaves a
/// task reading it: the card stays up while the stream is held, and the
/// link letting go of the answer drops the stream, which withdraws it.
async fn open<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    payload: Vec<u8>,
    answer: oneshot::Sender<Result<Vec<u8>, HumataError>>,
) {
    let request = pb::HookRequest {
        agent: "opencode".to_owned(),
        event: "permission".to_owned(),
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
