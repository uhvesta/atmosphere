//! `SessionService.Hook` (HOOKS, D191, D192): an agent's blocking permission
//! hook holds this stream while its Inbox card is open. The daemon reads the
//! agent's payload, raises the card, says it's still waiting every 15 s, and
//! ends with the bytes the hook prints. Empty bytes are no decision, so the
//! agent's own prompt asks: for a payload that isn't an ask, a card nobody
//! answered in time, one that was withdrawn, or every ask of an agent whose
//! Settings › Agents › "Ask in the Inbox" is off (D782, was D199's one
//! switch). The `question` event does the same for the agent's own ask-user
//! form (D761): its card is an Inbox question, and the session needs you
//! while it's held. The `conversation` event holds nothing: the agent's
//! hook names the conversation its CLI is in now, and when a Copilot
//! `/clear` or `/restart` has moved it to another one, the session follows.

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
use crate::inbox::permission::{Ask, Decision, Question};
use crate::status;

pub(crate) type HookStream = ReceiverStream<Result<pb::HookReply, Status>>;

/// How often the stream says the card is still open.
const KEEPALIVE: Duration = Duration::from_secs(15);

/// The events a hook holds the stream for: a permission ask, and the
/// agent's own ask-user form (D761).
const PERMISSION: &str = "permission";
const QUESTION: &str = "question";
/// The event of a hook that says which conversation its CLI is in.
const CONVERSATION: &str = "conversation";
/// The event `humata codex` sends when its link to Codex's thread dropped
/// (D798): the session's screen is read for asks again until it is heard.
const LOST: &str = "lost";
/// The event of Claude's Notification hook (D798): a prompt waits in its terminal.
const NOTIFICATION: &str = "notification";

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
        let kind = Agent::parse(&request.agent);
        if request.event == CONVERSATION {
            // D797: any conversation hook says the agent is up.
            crate::start_ready::heard(&agent.session.id.to_string());
            let named = kind.and_then(|kind| {
                agent_hooks::parse_conversation(kind, &request.payload).ok()
            });
            if let Some(id) = named {
                self.follow_conversation(&agent.session.id.to_string(), id)
                    .await?;
            }
            let _ = sender.try_send(Ok(final_reply(Vec::new())));
            return Ok(Response::new(ReceiverStream::new(receiver)));
        }
        if request.event == LOST {
            crate::start_ready::unheard(&agent.session.id.to_string());
            let _ = sender.try_send(Ok(final_reply(Vec::new())));
            return Ok(Response::new(ReceiverStream::new(receiver)));
        }
        if request.event == NOTIFICATION {
            let _ = sender.try_send(Ok(final_reply(Vec::new())));
            if kind == Some(Agent::Claude)
                && let Some(message) = waiting_message(&request.payload)
                && !self.inbox.asks_in_agent(&agent.agent)
                && !super::turns::asking(&agent.session.id.to_string())
            {
                self.raise_notice(caller, agent, message);
            }
            return Ok(Response::new(ReceiverStream::new(receiver)));
        }
        let hooked = match (request.event.as_str(), kind) {
            (PERMISSION, Some(kind)) => agent_hooks::parse_permission(kind, &request.payload)
                .ok()
                .map(|ask| Hooked::Permission(kind, ask)),
            (QUESTION, Some(kind)) => agent_hooks::parse_question(kind, &request.payload)
                .ok()
                .map(|asked| Hooked::Question(kind, asked, request.payload.clone())),
            _ => None,
        };
        // A call of Humata's own tool that only files an Inbox request is
        // let through, whoever keeps the agent's asks: the request it files
        // is the card the user answers, so a second one would ask twice.
        if let Some(Hooked::Permission(kind, ask)) = &hooked
            && only_files_an_inbox_request(&ask.tool)
        {
            let _ = sender.try_send(Ok(final_reply(permission_stdout(*kind, Decision::Allow))));
            return Ok(Response::new(ReceiverStream::new(receiver)));
        }
        // Not an ask this daemon answers, or the user keeps this agent's
        // asks in its own terminal (D782): no decision, no card.
        let hooked = match hooked {
            Some(hooked) if !self.inbox.asks_in_agent(&agent.agent) => Some(hooked),
            _ => None,
        };
        let Some(hooked) = hooked else {
            let _ = sender.try_send(Ok(final_reply(Vec::new())));
            return Ok(Response::new(ReceiverStream::new(receiver)));
        };
        match hooked {
            Hooked::Permission(kind, ask) => {
                self.inbox.withdraw_notice(&agent.session.id.to_string()).await;
                let ask = Ask {
                    tool: ask.tool,
                    command: ask.command,
                    description: ask.description,
                    edits_files: ask.edits_files,
                    key: ask.key,
                };
                let wait = self
                    .inbox
                    .raise_permission(&caller, &agent, kind.name(), ask)
                    .await?;
                tokio::spawn(async move {
                    let mut wait = wait;
                    let render = move |decision| permission_stdout(kind, decision);
                    hold(wait.decided(), render, sender, DAEMON_HOLD).await;
                    // Lets go of the card: the last hook to let go withdraws it.
                    drop(wait);
                });
            }
            Hooked::Question(kind, asked, payload) => {
                self.inbox.withdraw_notice(&agent.session.id.to_string()).await;
                let asked = Question {
                    question: asked.question,
                    choices: asked.choices,
                    key: asked.key,
                    only_choices: false,
                };
                let wait = self.inbox.raise_question(&caller, &agent, asked).await?;
                // The agent waits on the person while its form is held, so
                // the session needs you until the card is settled (D761).
                let asking = super::Asking::hold(&agent.session.id.to_string());
                tokio::spawn(async move {
                    let mut wait = wait;
                    let render = move |decision| question_stdout(kind, &payload, decision);
                    hold(wait.decided(), render, sender, DAEMON_HOLD).await;
                    drop(asking);
                    drop(wait);
                });
            }
        }
        Ok(Response::new(ReceiverStream::new(receiver)))
    }
}

impl SessionSvc {
    /// D798: raises the "waiting in its terminal" card for a Claude prompt
    /// the agent shows, without holding the hook: the card lives in its own
    /// task, which lets it go when the session reports a turn, a held ask
    /// is raised, or the agent exits. The session needs you meanwhile.
    fn raise_notice(&self, caller: Caller, agent: crate::identity::AgentCaller, message: String) {
        let inbox = self.inbox.clone();
        let name = self.starter.agents.person_name(&agent.agent);
        let session = agent.session.id.to_string();
        tokio::spawn(async move {
            let mut wait = match inbox.raise_notice(&caller, &agent, &name, message).await {
                Ok(wait) => wait,
                Err(error) => {
                    crate::log::say!(
                        "humatad2: couldn't say in the Inbox that session {session}'s agent is waiting in its terminal ({}); answer it there",
                        status::chain(&error)
                    );
                    return;
                }
            };
            let _asking = super::Asking::hold(&session);
            let mut changes = super::turns::changes();
            let mut seen_up = super::turns::is_up(&session);
            loop {
                tokio::select! {
                    _ = wait.decided() => break,
                    changed = changes.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let up = super::turns::is_up(&session);
                        if seen_up && !up {
                            break;
                        }
                        seen_up |= up;
                    }
                }
            }
            drop(wait);
        });
    }

    /// Session `session`'s agent says its CLI is in conversation `id`: the
    /// session's agent record follows it there when it moved, and the feed
    /// is told, so a resume takes up the conversation the person was in.
    async fn follow_conversation(&self, session: &str, id: String) -> Result<(), Status> {
        let (store, agents, key) = (
            self.starter.store.clone(),
            self.starter.agents.clone(),
            session.to_owned(),
        );
        let followed = crate::channel_service::blocking(move || {
            agents.follow_conversation(&store, &key, &id)
        })
        .await?
        .map_err(|error| {
            status::store(error, &self.machine, "save the conversation the agent moved to")
        })?;
        if followed {
            self.tell.session(session, pb::Change::Updated, None).await;
        }
        Ok(())
    }
}

/// What Claude's Notification payload shows, when it is a prompt that waits
/// in its terminal; `None` for an idle prompt or a payload that isn't one.
fn waiting_message(payload: &[u8]) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let waiting = matches!(
        payload.get("notification_type").and_then(|kind| kind.as_str()),
        Some("permission_prompt" | "elicitation_dialog")
    );
    if !waiting {
        return None;
    }
    let message = payload.get("message").and_then(|message| message.as_str()).unwrap_or("");
    Some(message.to_owned())
}

/// What a hook holds a card for.
enum Hooked {
    Permission(Agent, agent_hooks::Ask),
    /// The question, and the payload it was parsed from, which its answer needs.
    Question(Agent, agent_hooks::Question, Vec<u8>),
}

/// Humata's own MCP tools whose call only files an Inbox request: the
/// daemon asks the user on that card, so the call itself needs no approval.
const ONLY_FILE_AN_INBOX_REQUEST: &[humata2_mcp::Operation] = &[humata2_mcp::Operation::ProviderImport];

/// Whether `tool`, as an agent's permission ask names it, is one of
/// [`ONLY_FILE_AN_INBOX_REQUEST`] on Humata's server: Claude's and Codex's
/// `mcp__humata__<tool>`, Copilot's `humata/<tool>` or `humata-<tool>`,
/// OpenCode's `humata_<tool>`.
fn only_files_an_inbox_request(tool: &str) -> bool {
    let server = agent_hooks::MCP_SERVER;
    let named = [
        format!("mcp__{server}__"),
        format!("{server}/"),
        format!("{server}-"),
        format!("{server}_"),
    ]
    .iter()
    .find_map(|prefix| tool.strip_prefix(prefix.as_str()))
    .map(str::to_owned);
    named.is_some_and(|name| {
        ONLY_FILE_AN_INBOX_REQUEST
            .iter()
            .any(|operation| operation.tool_name() == name)
    })
}

/// What `agent`'s permission hook prints for `decision`.
fn permission_stdout(agent: Agent, decision: Decision) -> Vec<u8> {
    let decision = match decision {
        Decision::Allow => agent_hooks::Decision::Allow,
        Decision::Deny { message } => agent_hooks::Decision::Deny { message },
        // A permission card takes no reply.
        Decision::Reply { .. } | Decision::Undecided => agent_hooks::Decision::Undecided,
    };
    agent_hooks::render_permission(agent, &decision)
}

/// What `agent`'s ask-user hook prints for `decision` (D761).
fn question_stdout(agent: Agent, payload: &[u8], decision: Decision) -> Vec<u8> {
    let reply = match decision {
        Decision::Reply { text } => agent_hooks::Reply::Answer(text),
        Decision::Deny { message } => agent_hooks::Reply::Declined { message },
        // A question card takes no approval.
        Decision::Allow | Decision::Undecided => agent_hooks::Reply::Undecided,
    };
    agent_hooks::render_question(agent, payload, &reply)
}

/// Waits for `decided` until the hook lets go or `limit` passes, sending
/// keepalives meanwhile; then sends what `render` says the hook prints.
async fn hold(
    decided: impl Future<Output = Decision>,
    render: impl FnOnce(Decision) -> Vec<u8>,
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
    let _ = sender.send(Ok(final_reply(render(decision)))).await;
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

/// Writes the MCP config files Claude Code and Copilot load Humata's own
/// server from (D748) into `state`, for `humatad` (this daemon's binary,
/// run as `mcp`) talking to `socket`, and says where each agent finds it:
/// the files, and the value of Codex's `-c mcp_servers.humata=…`. At the
/// daemon's start, before it serves. The files hold the command and the
/// socket only; the session's token stays in each agent's environment.
pub(crate) fn install_mcp_configs(
    state: &Path,
    humatad: &Path,
    socket: &Path,
) -> Result<humata2_sessions::HumataTools, HooksError> {
    let claude_mcp = state.join("humata-mcp-claude.json");
    let copilot_mcp = state.join("humata-mcp-copilot.json");
    for (path, contents) in [
        (&claude_mcp, agent_hooks::claude_mcp_config(humatad, socket)),
        (&copilot_mcp, agent_hooks::copilot_mcp_config(humatad, socket)),
    ] {
        if std::fs::read(path).is_ok_and(|have| have == contents.as_bytes()) {
            continue;
        }
        humata2_store::write_atomic(path, contents.as_bytes())
            .map_err(|source| HooksError::Write { path: path.clone(), source })?;
    }
    Ok(humata2_sessions::HumataTools {
        claude_mcp,
        copilot_mcp,
        codex_mcp: agent_hooks::codex_mcp(humatad, socket),
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

    #[test]
    fn the_mcp_configs_are_written_into_the_state_folder_and_rewritten_alike() {
        let state = tempfile::tempdir().expect("a temporary folder");
        let humatad = Path::new("/opt/humata/bin/humatad2");
        let socket = state.path().join("humatad.sock");
        let tools = install_mcp_configs(state.path(), humatad, &socket).expect("installed");
        let read = |path: &Path| std::fs::read_to_string(path).expect("written");
        assert!(tools.claude_mcp.starts_with(state.path()));
        assert_eq!(read(&tools.claude_mcp), agent_hooks::claude_mcp_config(humatad, &socket));
        assert_eq!(read(&tools.copilot_mcp), agent_hooks::copilot_mcp_config(humatad, &socket));
        assert_eq!(tools.codex_mcp, agent_hooks::codex_mcp(humatad, &socket));
        // Again at the next start: same files, no error.
        install_mcp_configs(state.path(), humatad, &socket).expect("installed");
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
        hold(decided, |decision| permission_stdout(Agent::Claude, decision), sender, limit).await;
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

    #[test]
    fn only_humatas_own_request_filing_tools_skip_the_card() {
        for tool in [
            "mcp__humata__provider_import",
            "humata/provider_import",
            "humata-provider_import",
            "humata_provider_import",
        ] {
            assert!(only_files_an_inbox_request(tool), "{tool}");
        }
        for tool in [
            "Bash",
            "provider_import",
            "mcp__other__provider_import",
            "other/provider_import",
            "mcp__humata__machine_create",
            "mcp__humata__provider_import_all",
        ] {
            assert!(!only_files_an_inbox_request(tool), "{tool}");
        }
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
