//! PreviewFork and ForkSession on this machine (FK-1..FK-13): the checks,
//! the access gate and the `ForkSession` operation over `Starter::fork`.
//!
//! This build forks a session's code and conversation on this machine:
//! from now, cut before one of its turns (D181), or with the conversation
//! only, working in its parent's clones (D185, D187). A fork that runs
//! another agent is handed the conversation as a file and a summary
//! (FK-6, D212). A fork elsewhere is in fork_remote.

use std::sync::Arc;

use humata2_access::{AgentId, Capability, ChannelId, MachineId, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_mirror::{MirrorError, turns_of};
use humata2_sessions::{ForkCopy, ForkRequest, Starter, handover_file};
use humata2_store::TabRecord;
use pb::invalid_field::Problem;
use tokio::sync::oneshot;
use tonic::{Code, Status};

use super::lineage::publish_family;
use super::start_fresh::{self, Handover, Source};
use super::{COLS, Loaded, ROWS, SessionSvc, Tell, advance, clean_title, start_error, turns, watch};
use crate::channel_service::{Gated, blocking, failed};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::restart::Tokens;
use crate::status;

const FORK_SESSION: &str = "humata.v1.SessionService.ForkSession";

#[derive(Debug, thiserror::Error)]
enum ForkError {
    #[error("session {session} is archived, so it can't be forked")]
    ParentArchived { session: String },
    #[error(
        "agent {agent} keeps its conversation in a form Humata can't read turns from, so a fork can't be cut before one of its turns"
    )]
    NoReadableTurns { agent: String },
}

/// The request, checked and the access rule passed.
struct Checked {
    parent: Loaded,
    copy: ForkCopy,
    title: String,
    /// The branch typed in the dialog; None: picked from the title.
    branch: Option<String>,
    before_turn: u32,
    /// The agent the fork runs: its parent's unless the request names another.
    agent: String,
    /// Whether the fork is handed the conversation as a file and a summary
    /// instead of resuming it (FK-6, D212).
    handover: bool,
}

/// One repository a fork would have: its owner, name and the branch it is on.
type PlannedRepo = (String, String, String);

/// `method` isn't built for `part`: the answer names the part.
pub(super) fn not_built_part(method: &str, part: &str) -> Status {
    status::not_built(&format!("{method} for {part}"))
}

impl SessionSvc {
    /// What forking would do: the branch, what is copied and how much.
    pub(super) async fn preview_fork_of(
        &self,
        caller: &Caller,
        request: pb::ForkSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::ForkPreview>, Status> {
        if let Some(elsewhere) = self.preview_fork_elsewhere_of(caller, &request, approved).await? {
            return Ok(elsewhere);
        }
        let checked = match self.check_fork(caller, &request, approved).await? {
            Gated::Ask(asked) => return Ok(Gated::Ask(asked)),
            Gated::Done(checked) => checked,
        };
        let (planned, copy_bytes) = if checked.copy == ForkCopy::ContextOnly {
            // No copies and no new branch: the parent's clones and branches.
            let planned: Vec<PlannedRepo> = checked
                .parent
                .detail
                .as_ref()
                .map(|detail| {
                    detail
                        .repos
                        .iter()
                        .map(|r| (r.owner.clone(), r.name.clone(), r.branch.clone()))
                        .collect()
                })
                .unwrap_or_default();
            (planned, 0)
        } else {
            let (starter, parent, title, typed) = (
                self.starter.clone(),
                checked.parent.record.id.clone(),
                checked.title.clone(),
                checked.branch.clone(),
            );
            let plan = blocking(move || starter.preview_fork(&parent, &title, typed.as_deref()))
                .await?
                .map_err(|error| {
                    let (code, failure) = start_error(&error);
                    status::failure(code, failure)
                })?;
            let planned: Vec<PlannedRepo> = plan
                .repos
                .iter()
                .map(|(owner, name)| (owner.clone(), name.clone(), plan.branch.name.clone()))
                .collect();
            (planned, plan.copy_bytes)
        };
        let handed = if checked.handover {
            let (file, socket) = handover_from(&checked.parent);
            Some(blocking(move || start_fresh::handover_source(file.as_deref(), &socket)).await?)
        } else {
            None
        };
        let text = fork_text(&checked, &planned, copy_bytes, handed);
        Ok(Gated::Done(pb::ForkPreview {
            text,
            branches: planned
                .iter()
                .map(|(owner, name, branch)| pb::preview_branch_names_response::Name {
                    repo: Some(pb::RepoRef {
                        owner: owner.clone(),
                        name: name.clone(),
                    }),
                    branch: branch.clone(),
                    suffix_reason: String::new(),
                })
                .collect(),
            copy_bytes,
            seeded_with_summary: checked.handover,
        }))
    }

    /// Records the fork and answers its operation; the copies and the
    /// launch run on in it. The operation's result is the fork's id.
    pub(super) async fn fork_session_of(
        &self,
        caller: &Caller,
        request: pb::ForkSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        if let Some(elsewhere) = self.fork_elsewhere_of(caller, &request, approved).await? {
            return Ok(elsewhere);
        }
        let checked = match self.check_fork(caller, &request, approved).await? {
            Gated::Ask(asked) => return Ok(Gated::Ask(asked)),
            Gated::Done(checked) => checked,
        };
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
        turns::refresh_hooks().await;

        let parent = checked.parent.record.id.clone();
        let fork = format!("se_{}", uuid::Uuid::new_v4().simple());
        // A handover's first message is written and kept before anything
        // starts, so a restart types it if the fork is cut before it is.
        let seed = if checked.handover {
            let conversation = uuid::Uuid::new_v4().hyphenated().to_string();
            let (file, socket) = handover_from(&checked.parent);
            let (title, agent, before_turn, handed) = (
                checked.parent.record.title.clone(),
                checked.parent.record.agent.clone(),
                checked.before_turn,
                handover_file(&fork),
            );
            let text = blocking(move || {
                start_fresh::handover_seed(&Handover {
                    title: &title,
                    agent: &agent,
                    file: file.as_deref(),
                    before_turn,
                    socket: &socket,
                    handed: &handed,
                })
            })
            .await?;
            Some((conversation, text))
        } else {
            None
        };
        let mut keys = vec!["write_record".to_owned()];
        let mut steps = vec![("write_record".to_owned(), "Saving the fork".to_owned())];
        // A context-only fork copies no code.
        let repos = checked
            .parent
            .detail
            .iter()
            .flat_map(|detail| detail.repos.iter())
            .filter(|_| checked.copy != ForkCopy::ContextOnly);
        for repo in repos {
            keys.push(format!("copy_code:{}", repo.name));
            steps.push((
                format!("copy_code:{}", repo.name),
                format!("Copying {}'s code", repo.name),
            ));
        }
        if checked.copy != ForkCopy::CodeOnly {
            keys.push("copy_conversation".to_owned());
            steps.push((
                "copy_conversation".to_owned(),
                if checked.handover {
                    "Handing over the conversation"
                } else {
                    "Copying the conversation"
                }
                .to_owned(),
            ));
        }
        keys.push("launch".to_owned());
        keys.push("verify".to_owned());
        steps.push(("launch".to_owned(), "Starting the agent".to_owned()));
        steps.push(("verify".to_owned(), "Checking it started".to_owned()));
        if seed.is_some() {
            steps.push(("seed".to_owned(), "Telling the agent what it was handed".to_owned()));
        }
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: parent.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::ForkSession,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        if let Some((conversation, text)) = &seed
            && let Err(error) =
                start_fresh::keep_handover(&self.starter.store, &fork, conversation, text).await
        {
            let refused = status::failure(Code::Internal, error.clone());
            progress.fail(error);
            return Err(refused);
        }
        let fork_request = ForkRequest {
            id: Some(fork.clone()),
            parent: parent.clone(),
            title: checked.title,
            copy: checked.copy,
            kind: request.kind,
            before_turn: request.before_turn,
            branch: checked.branch,
            agent: Some(checked.agent),
            conversation: seed.as_ref().map(|(conversation, _)| conversation.clone()),
            operation: operation.id.as_ref().map(|id| id.value.clone()),
            cols: COLS,
            rows: ROWS,
        };
        let (ready, recorded) = oneshot::channel();
        let by = identity::actor(caller);
        tokio::spawn(run_fork(
            self.starter.clone(),
            self.tell.clone(),
            self.tokens.clone(),
            fork_request,
            progress,
            keys,
            seed,
            ready,
            by.clone(),
        ));
        match recorded.await {
            Ok(Ok(())) => {
                self.tell
                    .session(&fork, pb::Change::Created, Some(by.clone()))
                    .await;
                publish_family(&self.tell, &parent, Some(by)).await;
                Ok(Gated::Done(operation))
            }
            Ok(Err(refused)) => Err(refused),
            Err(_) => Err(status::internal(
                "the fork stopped before it was recorded",
                "",
            )),
        }
    }

    /// The checks a preview and a fork share, ending at the access rule.
    async fn check_fork(
        &self,
        caller: &Caller,
        request: &pb::ForkSessionRequest,
        approved: bool,
    ) -> Result<Gated<Checked>, Status> {
        Self::refuse_peer(caller)?;
        let parent_id = request
            .parent
            .as_ref()
            .map(|parent| parent.value.clone())
            .filter(|parent| !parent.is_empty())
            .ok_or_else(|| status::missing("parent"))?;
        let parent = self.load(&parent_id).await?;
        let copy = match pb::ForkKind::try_from(request.kind) {
            Ok(pb::ForkKind::CodeAndContext) => ForkCopy::CodeAndContext,
            Ok(pb::ForkKind::CodeOnly) => ForkCopy::CodeOnly,
            Ok(pb::ForkKind::ContextOnly) => ForkCopy::ContextOnly,
            _ => {
                return Err(status::invalid_field(
                    "kind",
                    Problem::Missing,
                    "the request doesn't say what the fork carries",
                    "set kind to CODE_AND_CONTEXT, CODE_ONLY or CONTEXT_ONLY, and send it again",
                ));
            }
        };
        if copy == ForkCopy::CodeOnly && request.before_turn != 0 {
            return Err(status::invalid_field(
                "before_turn",
                Problem::NotAllowedHere,
                "a fork before a turn carries the conversation up to that turn, and a code-only fork carries none",
                "fork with CODE_AND_CONTEXT or CONTEXT_ONLY, or set before_turn to 0 to fork from now",
            ));
        }
        let agent = request
            .agent
            .as_ref()
            .map(|agent| agent.value.clone())
            .filter(|agent| !agent.is_empty())
            .unwrap_or_else(|| parent.record.agent.clone());
        if agent != parent.record.agent && self.starter.agents.get(&agent).is_err() {
            return Err(status::invalid_field(
                "agent",
                Problem::UnknownValue,
                format!("this machine has no agent named {agent} to run the fork"),
                "pick one of the agents the new-session dialog lists, or leave agent empty to run the parent's",
            ));
        }
        let handover = self
            .starter
            .hands_over(&parent.record.agent, &agent, copy)
            .map_err(|error| {
                let (code, failure) = start_error(&error);
                status::failure(code, failure)
            })?;
        if request
            .machine
            .as_ref()
            .is_some_and(|machine| !machine.value.is_empty() && machine.value != self.machine)
            || self.lives_elsewhere(&parent.record).is_some()
        {
            return Err(not_built_part(FORK_SESSION, "a fork on another machine"));
        }
        let (store, id) = (self.starter.store.clone(), parent_id.clone());
        let archived = blocking(move || store.archive(&id))
            .await?
            .map_err(|error| status::store(error, &self.machine, "look up the session"))?;
        if archived.is_some() {
            return Err(failed(
                Code::FailedPrecondition,
                &ForkError::ParentArchived { session: parent_id },
                "restore the session first, then fork it",
                None,
            ));
        }
        let channel = self.find_channel(&parent.record.channel).await?;
        if let Some(asked) = self.gate(
            caller,
            Capability::SessionStartSameMachine,
            Target::Start {
                channel: ChannelId::new(channel.id.as_str()),
                machine: MachineId::new(self.machine.as_str()),
                agent: AgentId::new(agent.as_str()),
            },
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        if request.before_turn != 0 {
            self.check_turn(&parent, request.before_turn).await?;
        }
        let title = clean_title(&request.title)?;
        let title = if title.is_empty() {
            clean_title(&format!("Fork of {}", parent.record.title))?
        } else {
            title
        };
        let branch = Some(request.branch.clone()).filter(|branch| !branch.is_empty());
        if let Some(typed) = &branch {
            if copy == ForkCopy::ContextOnly {
                return Err(status::invalid_field(
                    "branch",
                    Problem::NotAllowedHere,
                    "a fork that carries only the conversation works in its parent's clones, so it has no branch to name",
                    "clear the branch, or fork with CODE_AND_CONTEXT or CODE_ONLY to get a branch of its own",
                ));
            }
            // A bad or taken name is refused here, before anything is copied.
            let (starter, parent_id, title, typed) = (
                self.starter.clone(),
                parent.record.id.clone(),
                title.clone(),
                typed.clone(),
            );
            blocking(move || starter.check_fork_branch(&parent_id, &title, &typed))
                .await?
                .map_err(|error| {
                    let (code, failure) = start_error(&error);
                    status::failure(code, failure)
                })?;
        }
        Ok(Gated::Done(Checked {
            parent,
            copy,
            title,
            branch,
            before_turn: request.before_turn,
            agent,
            handover,
        }))
    }

    /// What a fork before turn `turn` (above 0) needs: an agent whose
    /// conversation turns can be read, and a turn the session has. Turn 1 is
    /// the first prompt, so nothing comes before it to carry (D181).
    pub(super) async fn check_turn(&self, parent: &Loaded, turn: u32) -> Result<(), Status> {
        let agent = &parent.record.agent;
        let reads = self
            .starter
            .agents
            .get(agent)
            .is_ok_and(|kind| kind.conversation_format.reads_turns());
        if !reads {
            return Err(failed(
                Code::FailedPrecondition,
                &ForkError::NoReadableTurns {
                    agent: agent.clone(),
                },
                "fork from now, with before_turn 0, or use an agent whose conversation is Claude's JSONL",
                None,
            ));
        }
        if turn == 1 {
            return Err(status::invalid_field(
                "before_turn",
                Problem::NotAllowedHere,
                "turn 1 is the first prompt, so there is no conversation before it to carry",
                "use CODE_ONLY for a fresh agent, or fork from now with before_turn 0",
            ));
        }
        let path = parent
            .agent
            .as_ref()
            .map(|agent| agent.launch.conversation_file.clone());
        let turns = match path {
            Some(path) => blocking(move || turns_of(&path))
                .await?
                .map_err(|error| turns_failed(&error))?,
            None => Vec::new(),
        };
        if turns.iter().any(|mark| mark.index == turn) {
            return Ok(());
        }
        Err(status::invalid_field(
            "before_turn",
            Problem::UnknownValue,
            format!(
                "the session has {} turns, so there is no turn {turn} to fork before",
                turns.len()
            ),
            "pick a turn the session has, or fork from now with before_turn 0",
        ))
    }
}

/// A conversation whose turns couldn't be read, as the API answers it.
fn turns_failed(error: &MirrorError) -> Status {
    failed(
        Code::Unavailable,
        error,
        "check that the session's conversation file is still there and readable by this user, then fork again",
        None,
    )
}

/// Where a handover's message is made from: the parent's conversation
/// file, when its agent keeps one, and its terminal.
fn handover_from(parent: &Loaded) -> (Option<std::path::PathBuf>, std::path::PathBuf) {
    match &parent.agent {
        Some(agent) => (
            Some(agent.launch.conversation_file.clone()).filter(|file| !file.as_os_str().is_empty()),
            agent.launch.holder_socket.clone(),
        ),
        None => (None, std::path::PathBuf::new()),
    }
}

/// What a fork's new agent is told of its parent's conversation, as the
/// preview says it (FK-6, D212).
fn handover_text(checked: &Checked, source: Source) -> String {
    let (from, to) = (&checked.parent.record.agent, &checked.agent);
    match source {
        Source::Transcript => format!(
            " {to} isn't {from}, so it can't resume the conversation: it is handed the transcript and a short summary, and knows what was said and done but not {from}'s exact internal state."
        ),
        Source::Scrollback => format!(
            " {to} can't resume {from}'s conversation, and it can't be read, so {to} is handed the end of the terminal's output instead, not the conversation."
        ),
        Source::Nothing => format!(
            " {to} can't resume {from}'s conversation, and nothing of it can be read, so {to} is told only the session's title."
        ),
    }
}

/// What a fork does, in the words a preview shows the user. `planned` is
/// each repository, its owner, name and branch; `handed`, what a handover
/// is made from.
fn fork_text(checked: &Checked, planned: &[PlannedRepo], copy_bytes: u64, handed: Option<Source>) -> String {
    let handover = handed.map(|source| handover_text(checked, source)).unwrap_or_default();
    let title = &checked.parent.record.title;
    if checked.copy == ForkCopy::ContextOnly {
        let on = planned
            .iter()
            .map(|(_, name, branch)| format!("{name} on {branch}"))
            .collect::<Vec<_>>()
            .join(", ");
        return format!(
            "Forks \u{201c}{title}\u{201d} into a new session that works in the parent's files: its clones are the parent's, on their branches ({on}), so what it changes lands in them. It copies no code and {}.{handover}",
            if handed.is_some() { "starts a fresh conversation that is handed the old one" } else { "carries on the conversation from a copy of it" }
        );
    }
    let repos: Vec<&str> = planned.iter().map(|(_, name, _)| name.as_str()).collect();
    let branch = planned.first().map_or("", |(_, _, branch)| branch.as_str());
    let context = match (checked.copy, checked.before_turn) {
        (ForkCopy::CodeOnly, _) => "starts a fresh conversation".to_owned(),
        _ if handed.is_some() => "starts a fresh conversation that is handed the old one".to_owned(),
        (_, 0) => "carries on the conversation from a copy of it".to_owned(),
        (_, turn) => format!("carries on the conversation up to turn {turn} from a copy of it"),
    };
    format!(
        "Forks \u{201c}{title}\u{201d} into a new session on branch {branch} in {}. It copies the code as it is now, with committed, uncommitted and untracked files ({}), and {context}.{handover}",
        repos.join(", "),
        human_bytes(copy_bytes),
    )
}

/// Runs the fork for `progress`. `ready` hears once the fork is recorded
/// (the call answers then), or the failure when it didn't get that far.
async fn run_fork(
    starter: Starter,
    tell: Tell,
    tokens: Arc<Tokens>,
    request: ForkRequest,
    progress: Progress,
    keys: Vec<String>,
    seed: Option<(String, String)>,
    ready: oneshot::Sender<Result<(), Status>>,
    by: pb::Actor,
) {
    let mut ready = Some(ready);
    let operation = request.operation.clone();
    let parent = request.parent.clone();
    let fork_id = request.id.clone();
    let mut current = "write_record".to_owned();
    progress.set_step("write_record", pb::StepState::Running, None);
    let env = |tab: &TabRecord| tokens.env(tab);
    let forked = {
        let mut note = |event: &str| {
            current = advance(&progress, &keys, event);
            if event == "write_record"
                && let Some(ready) = ready.take()
            {
                let _ = ready.send(Ok(()));
            }
            crate::fault::point(&format!("fork.{event}"));
        };
        starter.fork(request, &env, &mut note).await
    };
    match forked {
        Ok(started) => {
            progress.set_step("verify", pb::StepState::Done, None);
            let session = started.session.id;
            if let Some((conversation, text)) = &seed {
                progress.set_step("seed", pb::StepState::Running, None);
                let socket = &started.agent.launch.holder_socket;
                match start_fresh::hand_over(&starter.store, socket, &session, conversation, text).await {
                    Ok(()) => progress.set_step("seed", pb::StepState::Done, None),
                    // The fork runs; its message is typed at the next
                    // restart, or the user tells the agent themselves.
                    Err(chain) => {
                        crate::log::say!(
                            "humatad2: the fork {session} started but its handover message wasn't typed: {chain}"
                        );
                        progress.set_step("seed", pb::StepState::Failed, Some(chain));
                    }
                }
            }
            progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: session.clone(),
            })));
            tell.session(&session, pb::Change::Updated, Some(by.clone()))
                .await;
            publish_family(&tell, &parent, Some(by)).await;
            watch::agent(tell, session);
        }
        Err(error) => {
            if let Some((conversation, _)) = &seed
                && let Some(fork) = &fork_id
            {
                start_fresh::drop_handover(&starter.store, fork, conversation).await;
            }
            let (code, failure) = start_error(&error);
            progress.set_step(&current, pb::StepState::Failed, Some(status::chain(&error)));
            if let Some(ready) = ready.take() {
                let _ = ready.send(Err(status::failure(code, failure.clone())));
            }
            progress.fail(failure);
            if let Some(operation) = operation
                && let Some(session) = tell.session_of(&operation).await
            {
                tell.session(&session, pb::Change::Updated, Some(by.clone()))
                    .await;
            }
            publish_family(&tell, &parent, Some(by)).await;
        }
    }
}

/// The size as a person reads it.
pub(super) fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_the_unit_a_person_expects() {
        assert_eq!(human_bytes(12), "12 bytes");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
