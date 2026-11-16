//! `SessionService.Hook` (HOOKS, D191, D192): an agent's blocking permission
//! hook holds this stream while its Inbox card is open. The daemon reads the
//! agent's payload, raises the card, says it's still waiting every 15 s, and
//! ends with the bytes the hook prints. Empty bytes are no decision, so the
//! agent's own prompt asks: for a payload that isn't an ask, a card nobody
//! answered in time, one that was withdrawn, or every ask while Settings ›
//! Agents › Ask for permission in the Inbox is off (D199).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_access::Refusal;
use humata2_agent_hooks::{self as agent_hooks, Agent, DAEMON_HOLD};
use humata2_grpc::humata::v1 as pb;
use humata2_store::StoreError;
use pb::invalid_field::Problem;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use super::{HooksError, SessionSvc};
use crate::identity::{self, Caller};
use crate::inbox::permission::{Ask, Decision};
use crate::status;

pub(crate) type HookStream = ReceiverStream<Result<pb::HookReply, Status>>;

/// How often the stream says the card is still open.
const KEEPALIVE: Duration = Duration::from_secs(15);

/// The one event a hook holds the stream for.
const PERMISSION: &str = "permission";

impl SessionSvc {
    pub(super) async fn hook_of(
        &self,
        request: Request<pb::HookRequest>,
    ) -> Result<Response<HookStream>, Status> {
        let caller = identity::caller(&request)?;
        let agent = match &caller {
            Caller::Agent(agent) => agent.clone(),
            Caller::Script(_) => return Err(status::refused(&Refusal::ScriptTokenHasNoSession)),
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::User(_) => {
                return Err(status::invalid_field(
                    "event",
                    Problem::NotAllowedHere,
                    "only a session's agent holds a permission hook; this connection is the user",
                    "the agent's hook runs `humata hook`, from the session's own terminal, where HUMATA_SESSION_TOKEN is set",
                ));
            }
        };
        self.inbox.hook_heard(&agent.agent).await;
        let request = request.into_inner();
        let (sender, receiver) = mpsc::channel(4);
        let asked = (request.event == PERMISSION)
            .then(|| Agent::parse(&request.agent))
            .flatten()
            .and_then(|kind| {
                let ask = agent_hooks::parse_permission(kind, &request.payload).ok()?;
                Some((kind, ask))
            });
        // Not an ask this daemon answers, or the user keeps asks in the
        // agent's own prompt (D199): no decision, no card.
        let asked = match asked {
            Some(asked) if !self.inbox.asks_in_agent().await? => Some(asked),
            _ => None,
        };
        let Some((kind, ask)) = asked else {
            let _ = sender.try_send(Ok(final_reply(Vec::new())));
            return Ok(Response::new(ReceiverStream::new(receiver)));
        };
        let ask = Ask {
            tool: ask.tool,
            command: ask.command,
            description: ask.description,
            key: ask.key,
        };
        let wait = self
            .inbox
            .raise_permission(&caller, &agent, kind.name(), ask)
            .await?;
        tokio::spawn(async move {
            let mut wait = wait;
            hold(wait.decided(), kind, sender, DAEMON_HOLD).await;
            // Lets go of the card: the last hook to let go withdraws it.
            drop(wait);
        });
        Ok(Response::new(ReceiverStream::new(receiver)))
    }
}

/// Waits for `decided` until the hook lets go or `limit` passes, sending
/// keepalives meanwhile; then sends what the hook prints.
async fn hold(
    decided: impl Future<Output = Decision>,
    agent: Agent,
    sender: mpsc::Sender<Result<pb::HookReply, Status>>,
    limit: Duration,
) {
    let deadline = tokio::time::sleep(limit);
    tokio::pin!(deadline);
    tokio::pin!(decided);
    let mut keepalive = tokio::time::interval(KEEPALIVE);
    let decision = loop {
        tokio::select! {
            biased;
            () = sender.closed() => return,
            decision = &mut decided => break decision,
            // Before the agent's own timeout, so the hook still exits 0
            // with no decision and the TUI asks (D192).
            () = &mut deadline => break Decision::Undecided,
            _ = keepalive.tick() => {
                let still = pb::HookReply { keepalive: true, stdout: Vec::new() };
                if sender.send(Ok(still)).await.is_err() {
                    return;
                }
            }
        }
    };
    let decision = match decision {
        Decision::Allow => agent_hooks::Decision::Allow,
        Decision::Deny { message } => agent_hooks::Decision::Deny { message },
        Decision::Undecided => agent_hooks::Decision::Undecided,
    };
    let stdout = agent_hooks::render_permission(agent, &decision);
    let _ = sender.send(Ok(final_reply(stdout))).await;
}

/// Where Codex and Copilot find their hooks (D193): the value of Codex's
/// `-c hooks=…`, and the folder of the plugin `--plugin-dir` loads.
pub(crate) struct AgentHooks {
    pub(crate) codex: String,
    pub(crate) copilot_plugin: PathBuf,
}

/// Writes Copilot's hooks plugin into `state` for `humata` talking to
/// `socket`, and says where both agents find their hooks. At the daemon's
/// start, before it serves. Only this daemon's own folder is written: the
/// agents' own settings are left alone.
pub(crate) fn install_agent_hooks(
    state: &Path,
    humata: &Path,
    socket: &Path,
) -> Result<AgentHooks, HooksError> {
    let folder = state.join("copilot-plugin");
    std::fs::create_dir_all(&folder).map_err(|source| HooksError::Write {
        path: folder.clone(),
        source: StoreError::Write {
            path: folder.clone(),
            source,
        },
    })?;
    for file in agent_hooks::copilot_plugin(humata, socket) {
        let path = folder.join(&file.path);
        if std::fs::read(&path).is_ok_and(|have| have == file.contents.as_bytes()) {
            continue;
        }
        humata2_store::write_atomic(&path, file.contents.as_bytes())
            .map_err(|source| HooksError::Write { path, source })?;
    }
    let codex = agent_hooks::codex_hooks(humata, socket);
    crate::inbox::permission::codex_trust::remember_hooks(&codex);
    Ok(AgentHooks {
        codex,
        copilot_plugin: folder,
    })
}

fn final_reply(stdout: Vec<u8>) -> pb::HookReply {
    pb::HookReply {
        keepalive: false,
        stdout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilots_plugin_is_written_into_the_state_folder_and_rewritten_alike() {
        let state = tempfile::tempdir().expect("a temporary folder");
        let humata = Path::new("/opt/humata/bin/humata");
        let socket = state.path().join("humatad.sock");
        let hooks = install_agent_hooks(state.path(), humata, &socket).expect("installed");
        assert!(hooks.copilot_plugin.starts_with(state.path()));
        for file in agent_hooks::copilot_plugin(humata, &socket) {
            let written = std::fs::read_to_string(hooks.copilot_plugin.join(&file.path)).expect("written");
            assert_eq!(written, file.contents);
        }
        assert_eq!(hooks.codex, agent_hooks::codex_hooks(humata, &socket));
        // Again at the next start: same files, no error.
        install_agent_hooks(state.path(), humata, &socket).expect("installed");
    }

    /// The replies `hold` sends, keepalives first, until it ends.
    async fn replies(decided: impl Future<Output = Decision>, limit: Duration) -> Vec<pb::HookReply> {
        let (sender, mut receiver) = mpsc::channel::<Result<pb::HookReply, Status>>(8);
        let held = tokio::spawn(async move {
            let mut sent = Vec::new();
            while let Some(reply) = receiver.recv().await {
                match reply {
                    Ok(reply) => sent.push(reply),
                    Err(status) => panic!("the hold sent an error: {status}"),
                }
            }
            sent
        });
        hold(decided, Agent::Claude, sender, limit).await;
        held.await.expect("the reader ended")
    }

    #[tokio::test(start_paused = true)]
    async fn nobody_answering_ends_the_hold_with_no_decision_before_the_agent_gives_up() {
        assert!(DAEMON_HOLD.as_secs() < agent_hooks::HOLD_SECONDS);
        let sent = replies(std::future::pending(), DAEMON_HOLD).await;
        let (last, before) = sent.split_last().expect("a last reply");
        assert!(!last.keepalive);
        assert!(last.stdout.is_empty());
        // About one keepalive every 15 s for the whole hold.
        let expected = DAEMON_HOLD.as_secs() / KEEPALIVE.as_secs();
        assert!(before.len() as u64 >= expected, "{} keepalives", before.len());
        assert!(before.iter().all(|reply| reply.keepalive));
    }

    #[tokio::test(start_paused = true)]
    async fn an_answer_ends_the_hold_with_what_the_agent_reads() {
        let sent = replies(std::future::ready(Decision::Allow), DAEMON_HOLD).await;
        let last = sent.last().expect("a last reply");
        assert!(!last.keepalive);
        assert_eq!(
            last.stdout,
            agent_hooks::render_permission(Agent::Claude, &agent_hooks::Decision::Allow)
        );
    }
}
