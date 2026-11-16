//! RebuildSession (SL-4, LD-11, flow 26.23 steps 4 and 5; D195): a session
//! whose machine was forgotten comes back on another machine in its channel.
//! Each repo is cloned again at its branch, at the last pushed sha (main if
//! it was never pushed); the conversation resumes from the host's mirror; the
//! agent is told what was lost.
//!
//! Two halves. The host (`rebuild_session_of`) checks the session is archived
//! as lost, hands the target machine what it kept (the branches, the mirror's
//! scrubbed lines, the note) as a StartSession the host alone may make, follows
//! the machine's operation, and only when that succeeded moves the session
//! there and drops the archive. A host that dies before that leaves the session
//! archived and rebuildable. The machine (`begin_rebuild_here`) does the work
//! with what it has: its own main clones, its own agent files.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use humata2_access::{AgentId, Capability, ChannelId, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_mirror::MirrorError;
use humata2_revive::ReviveError;
use humata2_sessions::{AgentError, GitError, pick_branch, remote_url, restore_clone};
use humata2_store::{
    AgentLaunch, ChannelRecord, SessionDetail, SessionRecord, SessionRepo, StartStep, StoreError,
    TabKind, TabRecord,
};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Code, Request, Status};

use super::fork::stop_point;
use super::lineage::publish_family;
use super::{COLS, Gated, ROWS, SessionSvc, clean_title, now_ms, watch};
use crate::channel_service::{blocking, error_of, failed};
use crate::host_relay::assigned;
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::status;

/// The host's scrubbed copy of a session's conversation file: `None` when
/// the mirror keeps none for it.
pub(crate) type RawLines = Arc<dyn Fn(&str) -> Result<Option<Vec<u8>>, MirrorError> + Send + Sync>;

/// The longest list of names or commits the note to the agent spells out.
const NAMED: usize = 10;

#[derive(Debug, thiserror::Error)]
enum RebuildError {
    #[error("session {session} was not archived with its machine, so there is nothing to rebuild")]
    NotLost { session: String },
    #[error("{machine} is not one of the machines in channel {channel}")]
    NotInChannel { machine: String, channel: String },
    #[error("session {session} was lost with {machine}; it can't be rebuilt there")]
    LostMachine { session: String, machine: String },
    #[error("only the host places a rebuilt session on a machine")]
    NotTheHost,
    #[error("{folder:?} is not a plain folder name")]
    BadFolder { folder: String },
    #[error("couldn't read or save the records of session {session}")]
    Store {
        session: String,
        #[source]
        source: StoreError,
    },
    #[error("couldn't clone {owner}/{name} at branch {branch}")]
    Clone {
        owner: String,
        name: String,
        branch: String,
        #[source]
        source: GitError,
    },
    #[error("couldn't write the conversation to {path}")]
    Conversation {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the agent {agent} can't be started here")]
    Agent {
        agent: String,
        #[source]
        source: AgentError,
    },
    #[error("couldn't start the agent")]
    Launch(#[source] ReviveError),
    #[error("couldn't remove {path}")]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the daemon stopped the rebuild's work early")]
    Task(#[source] tokio::task::JoinError),
}

/// A plain name for one folder: no path, no dots that climb.
fn plain(folder: &str) -> bool {
    !folder.is_empty()
        && folder != "."
        && folder != ".."
        && !folder.contains(['/', '\\', '\0'])
}

/// `text` as a list of at most `NAMED`, with how many more there were.
fn listed(names: &[String]) -> String {
    let shown = names.iter().take(NAMED).cloned().collect::<Vec<_>>().join(", ");
    match names.len().saturating_sub(NAMED) {
        0 => shown,
        more => format!("{shown} and {more} more"),
    }
}

/// What to tell the agent: where it was rebuilt and what the machine took
/// with it. One line, because it is typed at the agent's prompt.
fn note_of(session: &pb::ArchiveSummary, lost_machine: &str, on: &str) -> String {
    let mut parts = vec![format!(
        "Humata rebuilt this session on {on} because {lost_machine} is gone. Each repo is back at the last commit that was pushed to its branch."
    )];
    for repo in &session.repos {
        let Some(named) = &repo.repo else { continue };
        let mut lost = Vec::new();
        if !repo.files.is_empty() {
            let paths: Vec<String> = repo.files.iter().map(|file| file.path.clone()).collect();
            lost.push(format!("uncommitted changes to {}", listed(&paths)));
        }
        if !repo.unpushed_subjects.is_empty() {
            let subjects: Vec<String> = repo.unpushed_subjects.iter().map(|subject| format!("\"{subject}\"")).collect();
            lost.push(format!("commits that were never pushed: {}", listed(&subjects)));
        }
        if !lost.is_empty() {
            parts.push(format!("{}/{}, branch {}, lost {}.", named.owner, named.name, repo.branch, lost.join(" and ")));
        }
    }
    if parts.len() == 1 {
        parts.push("Nothing was lost.".to_owned());
    }
    parts.push("A name in ‹›, like ‹NAME›, stands for a host variable whose value was removed.".to_owned());
    parts.join(" ")
}

/// `data` with every `from` replaced by `to`.
fn replaced(data: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    if from.is_empty() || from == to {
        return data.to_vec();
    }
    let mut out = Vec::with_capacity(data.len());
    let mut at = 0;
    while at < data.len() {
        if data[at..].starts_with(from) {
            out.extend_from_slice(to);
            at += from.len();
        } else {
            out.push(data[at]);
            at += 1;
        }
    }
    out
}

fn only_host() -> Status {
    failed(
        Code::PermissionDenied,
        &RebuildError::NotTheHost,
        "rebuild the session from its menu, or with `humata session rebuild`; the host does the rest",
        None,
    )
}

/// The host's way to what its mirror kept: set once by `serve`.
#[derive(Clone, Default)]
pub(super) struct Lines(Arc<OnceLock<RawLines>>);

impl SessionSvc {
    /// Gives the host's SessionService the mirror's scrubbed lines. Once.
    pub(crate) fn use_raw_lines(&self, lines: RawLines) {
        let _ = self.raw_lines.0.set(lines);
    }

    // -- the host's half --------------------------------------------------

    /// Rebuilds the lost session on the machine the request names.
    pub(super) async fn rebuild_session_of(
        &self,
        caller: &Caller,
        request: pb::RebuildSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        Self::refuse_peer(caller)?;
        let id = super::archive::session_id(request.id.clone())?;
        let loaded = self.load(&id).await?;
        let target = request
            .machine
            .as_ref()
            .map(|machine| machine.value.clone())
            .filter(|machine| !machine.is_empty())
            .ok_or_else(|| status::missing("machine"))?;
        let Some(remote) = self.remote.clone() else {
            return Err(only_host());
        };
        let channel = self.find_channel(&loaded.record.channel).await?;
        let agent = loaded.record.agent.clone();
        if let Some(asked) = self.gate(
            caller,
            Capability::SessionStartOtherMachine,
            Target::Start {
                channel: ChannelId::new(channel.id.as_str()),
                machine: humata2_access::MachineId::new(target.as_str()),
                agent: AgentId::new(agent.as_str()),
            },
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self.operations.existing(caller, request.idempotency_key.as_ref())? {
            return Ok(Gated::Done(operation));
        }
        let (store, key) = (self.starter.store.clone(), id.clone());
        let row = blocking(move || store.archive(&key))
            .await?
            .map_err(|error| status::store(error, &self.machine, "read the archive"))?;
        let summary = row
            .as_ref()
            .and_then(|row| pb::ArchiveSummary::decode(row.summary.as_slice()).ok())
            .filter(|summary| summary.machine_lost)
            .ok_or_else(|| {
                failed(
                    Code::FailedPrecondition,
                    &RebuildError::NotLost { session: id.clone() },
                    "restore the session instead, or forget its machine first",
                    None,
                )
            })?;
        if target == loaded.record.machine {
            return Err(failed(
                Code::FailedPrecondition,
                &RebuildError::LostMachine { session: id.clone(), machine: target },
                "pick another machine in the channel",
                None,
            ));
        }
        if !channel.machines.contains(&target) {
            return Err(failed(
                Code::InvalidArgument,
                &RebuildError::NotInChannel { machine: target, channel: channel.name.clone() },
                "add the machine to the channel first, or pick one that is in it",
                Some(pb::error::Kind::InvalidField(pb::InvalidField {
                    field: "machine".to_owned(),
                    problem: Problem::UnknownValue as i32,
                })),
            ));
        }
        if target == self.machine {
            return Err(status::invalid_field(
                "machine",
                Problem::NotAllowedHere,
                "a rebuild on the host's own machine isn't built yet",
                "pick another machine in the channel",
            ));
        }
        remote.require_link(&target).map_err(Status::from)?;

        let repos: Vec<_> = summary.repos.iter().filter_map(|repo| repo.repo.clone().map(|named| (named, repo.branch.clone()))).collect();
        let mut steps = vec![("check".to_owned(), format!("Checking {target}"))];
        for (named, _) in &repos {
            steps.push((format!("clone:{}", named.name), format!("Cloning {}", named.name)));
        }
        steps.push(("write_conversation".to_owned(), "Putting the conversation back".to_owned()));
        steps.push(("launch".to_owned(), "Starting the agent".to_owned()));
        steps.push(("verify".to_owned(), "Checking it started".to_owned()));
        steps.push(("switch".to_owned(), format!("Moving the session to {target}")));
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::RebuildSession,
            pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.clone() })),
            },
            &steps,
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        tokio::spawn(self.clone().run_rebuild(
            progress,
            loaded.record,
            summary,
            channel,
            target,
            identity::actor(caller),
        ));
        Ok(Gated::Done(operation))
    }

    /// The rebuild's work for `progress`: ask the machine, follow it, then
    /// switch the session over.
    async fn run_rebuild(
        self,
        progress: Progress,
        record: SessionRecord,
        summary: pb::ArchiveSummary,
        channel: ChannelRecord,
        target: String,
        by: pb::Actor,
    ) {
        let id = record.id.clone();
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        let sent = self.ask_machine(&progress, &record, &summary, &channel, &target).await;
        let outcome = match sent {
            Ok(()) => self.switch_to(&record, &summary, &target).await,
            Err(error) => Err(error),
        };
        drop(guard);
        match outcome {
            Ok(()) => {
                progress.set_step("switch", pb::StepState::Done, None);
                self.tell.session(&id, pb::Change::Updated, Some(by.clone())).await;
                publish_family(&self.tell, &id, Some(by)).await;
                progress.succeed(Some(pb::operation::Result::Session(pb::SessionId { value: id })));
            }
            Err(failure) => progress.fail(failure),
        }
    }

    /// Hands `target` the session and waits for its operation to end,
    /// mirroring its steps into `progress`.
    async fn ask_machine(
        &self,
        progress: &Progress,
        record: &SessionRecord,
        summary: &pb::ArchiveSummary,
        channel: &ChannelRecord,
        target: &str,
    ) -> Result<(), pb::Error> {
        let Some(remote) = &self.remote else {
            return Err(failure_of(&only_host()));
        };
        let relayed = remote.relayed(target).map_err(|error| failure_of(&Status::from(error)))?;
        progress.set_step("check", pb::StepState::Done, None);

        let (store, key) = (self.starter.store.clone(), record.id.clone());
        let detail = blocking(move || store.session_detail(&key))
            .await
            .map_err(|status| failure_of(&status))?
            .map_err(|source| error_of(&RebuildError::Store { session: record.id.clone(), source }, "check the host's state folder, then rebuild again", None))?;
        let model = detail.as_ref().and_then(|detail| detail.model.clone()).unwrap_or_default();
        let lines = match &self.raw_lines.0.get() {
            Some(lines) => lines(&record.id)
                .map_err(|error| error_of(&error, "the host's copy of the conversation couldn't be read; rebuild again, or start a fresh session", None))?,
            None => None,
        };
        let old_folder = summary
            .repos
            .first()
            .and(detail.as_ref())
            .and_then(|detail| detail.repos.first())
            .and_then(|repo| repo.path.parent())
            .map(|folder| folder.display().to_string())
            .unwrap_or_default();
        let rebuild = pb::RebuildFrom {
            repos: summary
                .repos
                .iter()
                .map(|repo| pb::rebuild_from::Repo {
                    repo: repo.repo.clone(),
                    branch: repo.branch.clone(),
                })
                .collect(),
            folder: record
                .folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            old_folder,
            conversation: lines.unwrap_or_default(),
        };
        let asked = pb::StartSessionRequest {
            idempotency_key: None,
            channel: Some(pb::ChannelId { value: channel.id.clone() }),
            title: record.title.clone(),
            agent: Some(pb::AgentId { value: record.agent.clone() }),
            model,
            machine: None,
            repos: Vec::new(),
            first_message: note_of(summary, &record.machine, target),
            board_item: None,
            branch: String::new(),
            rebuild: Some(rebuild),
        };
        // Nothing is moved yet: a host killed here leaves the session archived.
        stop_point("rebuild-clone-api");
        for repo in &summary.repos {
            if let Some(named) = &repo.repo {
                stop_point(&format!("rebuild-clone-{}", named.name));
            }
        }
        let mut sessions = relayed.session();
        let call = assigned(asked, Some(&record.id)).map_err(|error| failure_of(&Status::from(error)))?;
        let started = sessions
            .start_session(call)
            .await
            .map_err(|status| failure_of(&status))?
            .into_inner();
        let operation = started.operation.unwrap_or_default();
        let mut operations = relayed.operation();
        let mut stream = operations
            .watch_operation(Request::new(pb::WatchOperationRequest {
                id: operation.id.clone(),
                from_version: 0,
            }))
            .await
            .map_err(|status| failure_of(&status))?
            .into_inner();
        let mut latest = operation;
        loop {
            for step in &latest.steps {
                let state = pb::StepState::try_from(step.state).unwrap_or(pb::StepState::Waiting);
                let tail = Some(step.output_tail.clone()).filter(|tail| !tail.is_empty());
                progress.set_step(&step.key, state, tail);
            }
            if latest.state != pb::OperationState::Running as i32 {
                break;
            }
            match stream.message().await {
                Ok(Some(next)) => latest = next,
                Ok(None) | Err(_) => {
                    return Err(error_of(
                        &RebuildOnLinkLost { machine: target.to_owned() },
                        "wait for the machine to come back, then rebuild the session again",
                        None,
                    ));
                }
            }
        }
        if latest.state == pb::OperationState::Succeeded as i32 {
            Ok(())
        } else {
            Err(latest.error.unwrap_or_else(|| {
                error_of(
                    &RebuildOnLinkLost { machine: target.to_owned() },
                    "look at the machine's session, then rebuild again",
                    None,
                )
            }))
        }
    }

    /// The machine runs the session: the host's record says so, the branches
    /// the machine used are kept, and the archive is dropped.
    async fn switch_to(
        &self,
        record: &SessionRecord,
        summary: &pb::ArchiveSummary,
        target: &str,
    ) -> Result<(), pb::Error> {
        let (store, moved, branches, lost) = (
            self.starter.store.clone(),
            SessionRecord { machine: target.to_owned(), ..record.clone() },
            summary.repos.clone(),
            record.machine.clone(),
        );
        let id = record.id.clone();
        let saving = id.clone();
        blocking(move || -> Result<(), StoreError> {
            if let Some(mut detail) = store.session_detail(&saving)? {
                for repo in &mut detail.repos {
                    if let Some(used) = branches.iter().find(|used| {
                        used.repo.as_ref().is_some_and(|named| named.owner == repo.owner && named.name == repo.name)
                    }) && !used.branch.is_empty()
                    {
                        repo.branch.clone_from(&used.branch);
                    }
                }
                detail.revision += 1;
                store.put_session_detail(&detail)?;
            }
            store.put_session(&moved)?;
            store.remove_archive(&saving)
        })
        .await
        .map_err(|error| failure_of(&error))?
        .map_err(|source| error_of(&RebuildError::Store { session: id.clone(), source }, "check the host's state folder, then rebuild again", None))?;
        if let Some(remote) = &self.remote {
            remote.forget_session(&lost, &id).await;
        }
        Ok(())
    }

    // -- the machine's half -----------------------------------------------

    /// StartSession with `rebuild` set, on the machine that gets the
    /// session. The host alone may ask.
    pub(super) async fn begin_rebuild_here(
        &self,
        caller: &Caller,
        request: pb::StartSessionRequest,
        rebuild: pb::RebuildFrom,
        assigned: Option<String>,
    ) -> Result<Gated<pb::StartSessionResponse>, Status> {
        let Some(id) = assigned else {
            return Err(only_host());
        };
        if let Some(known) = self.known_start(&id).await? {
            return Ok(Gated::Done(known));
        }
        let channel_id = request
            .channel
            .as_ref()
            .map(|channel| channel.value.clone())
            .filter(|channel| !channel.is_empty())
            .ok_or_else(|| status::missing("channel"))?;
        let channel = self.find_channel(&channel_id).await?;
        let agent = self.agent_for(request.agent.as_ref(), &channel)?;
        let title = clean_title(&request.title)?;
        if !plain(&rebuild.folder) {
            return Err(failed(
                Code::InvalidArgument,
                &RebuildError::BadFolder { folder: rebuild.folder.clone() },
                "the host sends the session's folder name; rebuild it from the host again",
                None,
            ));
        }
        let mut repos = Vec::new();
        for (n, repo) in rebuild.repos.iter().enumerate() {
            let named = repo.repo.clone().unwrap_or_default();
            if !channel.repos.iter().any(|known| known.owner == named.owner && known.name == named.name) {
                return Err(status::invalid_field(
                    &format!("rebuild.repos[{n}].repo"),
                    Problem::UnknownValue,
                    format!("the channel has no repository {}/{}", named.owner, named.name),
                    "pick one of the channel's repositories",
                ));
            }
            repos.push((named, repo.branch.clone()));
        }
        if repos.is_empty() {
            return Err(status::missing("rebuild.repos"));
        }
        let mut steps = Vec::new();
        for (named, _) in &repos {
            steps.push((format!("clone:{}", named.name), format!("Cloning {}", named.name)));
        }
        steps.push(("write_conversation".to_owned(), "Putting the conversation back".to_owned()));
        steps.push(("launch".to_owned(), "Starting the agent".to_owned()));
        steps.push(("verify".to_owned(), "Checking it started".to_owned()));
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId { value: channel.id.clone() })),
            },
            &steps,
            false,
            None,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => {
                return Ok(Gated::Done(pb::StartSessionResponse {
                    session: Some(pb::SessionId { value: id }),
                    operation: Some(operation),
                }));
            }
        };
        let operation = progress.operation();

        let (starter, session_channel, folder_name) = (self.starter.clone(), channel.clone(), rebuild.folder.clone());
        let folder = blocking(move || {
            let name = pick_branch("", &folder_name, |name| {
                starter.layout.session(&session_channel.folder, name).exists()
            })
            .name;
            starter.layout.session(&session_channel.folder, &name)
        })
        .await?;
        let record = SessionRecord {
            id: id.clone(),
            channel: channel.id.clone(),
            machine: self.machine.clone(),
            title: if title.is_empty() { "Session".to_owned() } else { title },
            agent,
            folder: folder.clone(),
            created_unix_ms: now_ms(),
        };
        let detail = SessionDetail {
            session: id.clone(),
            repos: repos
                .iter()
                .map(|(named, branch)| SessionRepo {
                    owner: named.owner.clone(),
                    name: named.name.clone(),
                    branch: branch.clone(),
                    path: folder.join(&named.name),
                })
                .collect(),
            model: Some(request.model).filter(|model| !model.is_empty()),
            first_message: None,
            start: StartStep::Recorded,
            operation: operation.id.as_ref().map(|id| id.value.clone()),
            revision: 1,
        };
        let (store, saved, saved_detail) = (self.starter.store.clone(), record.clone(), detail.clone());
        let saved = blocking(move || -> Result<(), StoreError> {
            store.put_session(&saved)?;
            store.put_session_detail(&saved_detail)
        })
        .await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(source)) => {
                let error = RebuildError::Store { session: id.clone(), source };
                let fix = "check the state folder has room, then rebuild again";
                progress.fail(error_of(&error, fix, None));
                return Err(failed(Code::Internal, &error, fix, None));
            }
            Err(refused) => {
                progress.fail(failure_of(&refused));
                return Err(refused);
            }
        }
        self.tell
            .session(&id, pb::Change::Created, Some(identity::actor(caller)))
            .await;
        let note = Some(request.first_message).filter(|note| !note.is_empty());
        tokio::spawn(self.clone().run_rebuild_here(
            progress,
            record,
            detail,
            channel,
            rebuild.conversation,
            rebuild.old_folder,
            note,
            identity::actor(caller),
        ));
        Ok(Gated::Done(pb::StartSessionResponse {
            session: Some(pb::SessionId { value: id }),
            operation: Some(operation),
        }))
    }

    /// The machine's work: clone each repo at its branch, write the
    /// conversation, resume the agent on it, tell it what was lost. A failure
    /// before the agent starts takes the records and the folder back.
    #[allow(clippy::too_many_arguments)]
    async fn run_rebuild_here(
        self,
        progress: Progress,
        record: SessionRecord,
        mut detail: SessionDetail,
        channel: ChannelRecord,
        conversation: Vec<u8>,
        old_folder: String,
        note: Option<String>,
        by: pb::Actor,
    ) {
        let id = record.id.clone();
        let mut current = String::new();
        let built = self
            .rebuild_steps(&progress, &record, &mut detail, &channel, &conversation, &old_folder, note, &mut current)
            .await;
        match built {
            Ok(()) => {
                progress.set_step("verify", pb::StepState::Done, None);
                progress.succeed(Some(pb::operation::Result::Session(pb::SessionId { value: id.clone() })));
                self.tell.session(&id, pb::Change::Updated, Some(by)).await;
                watch::agent(self.tell.clone(), id);
            }
            Err((error, launched)) => {
                progress.set_step(&current, pb::StepState::Failed, Some(status::chain(&error)));
                if !launched {
                    let _ = undo(self.starter.store.clone(), &record).await;
                }
                progress.fail(error_of(
                    &error,
                    "fix the cause the message names, then rebuild the session again",
                    None,
                ));
                self.tell.session(&id, pb::Change::Updated, Some(by)).await;
            }
        }
    }

    /// The steps. On failure, whether the agent had been launched.
    #[allow(clippy::too_many_arguments)]
    async fn rebuild_steps(
        &self,
        progress: &Progress,
        record: &SessionRecord,
        detail: &mut SessionDetail,
        channel: &ChannelRecord,
        conversation: &[u8],
        old_folder: &str,
        note: Option<String>,
        current: &mut String,
    ) -> Result<(), (RebuildError, bool)> {
        let starter = &self.starter;
        for repo in detail.repos.clone() {
            *current = format!("clone:{}", repo.name);
            progress.set_step(current, pb::StepState::Running, None);
            let main = starter.layout.main_clone(&channel.folder, &repo.name);
            let clone_error = |source| (
                RebuildError::Clone {
                    owner: repo.owner.clone(),
                    name: repo.name.clone(),
                    branch: repo.branch.clone(),
                    source,
                },
                false,
            );
            let url = remote_url(&starter.remote_template, &repo.owner, &repo.name).map_err(clone_error)?;
            let (path, branch) = (repo.path.clone(), repo.branch.clone());
            blocking_git(move || restore_clone(&main, &url, &path, &branch))
                .await
                .map_err(clone_error)?;
            progress.set_step(current, pb::StepState::Done, None);
        }
        self.set_start(detail, StartStep::Cloned).await.map_err(|error| (error, false))?;

        *current = "write_conversation".to_owned();
        progress.set_step(current, pb::StepState::Running, None);
        let kind = starter
            .agents
            .get(&record.agent)
            .map_err(|source| (RebuildError::Agent { agent: record.agent.clone(), source }, false))?;
        let conversation_id = uuid::Uuid::new_v4().hyphenated().to_string();
        let plan = kind.launch(&starter.home, &record.folder, &conversation_id);
        // With no lines the agent starts a conversation of its own.
        let resume = !conversation.is_empty();
        if resume {
            let lines = replaced(
                conversation,
                old_folder.as_bytes(),
                record.folder.display().to_string().as_bytes(),
            );
            write_lines(&plan.conversation_file, &lines).map_err(|error| (error, false))?;
        }
        progress.set_step(current, pb::StepState::Done, None);

        *current = "launch".to_owned();
        progress.set_step(current, pb::StepState::Running, None);
        self.set_start(detail, StartStep::Launching).await.map_err(|error| (error, false))?;
        let tab = format!("tb_{}", uuid::Uuid::new_v4().simple());
        let socket = starter.holders.join(format!("{tab}.sock"));
        let probe = TabRecord {
            id: tab.clone(),
            session: Some(record.id.clone()),
            kind: TabKind::Agent,
            title: record.agent.clone(),
            holder_socket: socket.clone(),
            holder_pid: 0,
            created_unix_ms: now_ms(),
        };
        let env = |tab: &TabRecord| self.tokens.env(tab);
        let variable_names = env(&probe)
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        let launch = AgentLaunch {
            agent: record.agent.clone(),
            folder: record.folder.clone(),
            argv: if resume { plan.resume_argv.clone() } else { plan.argv },
            resume_argv: plan.resume_argv,
            variable_names,
            conversation_id,
            conversation_file: plan.conversation_file,
            holder_socket: socket.clone(),
            cols: COLS,
            rows: ROWS,
        };
        humata2_revive::launch_agent(&starter.store, &starter.revive, record, &tab, launch, None, &env)
            .await
            .map_err(|source| (RebuildError::Launch(source), false))?;
        // From here the records stay: revive owns the agent.
        self.set_start(detail, StartStep::Done).await.map_err(|error| (error, true))?;
        progress.set_step(current, pb::StepState::Done, None);
        *current = "verify".to_owned();
        if let Some(note) = note
            && let Err(error) = type_note(&socket, &note).await
        {
            crate::log::say!(
                "humatad2: couldn't tell rebuilt session {} what was lost: {}",
                record.id,
                status::chain(&error)
            );
        }
        Ok(())
    }

    async fn set_start(&self, detail: &mut SessionDetail, step: StartStep) -> Result<(), RebuildError> {
        detail.start = step;
        detail.revision += 1;
        let (store, saving) = (self.starter.store.clone(), detail.clone());
        let session = saving.session.clone();
        tokio::task::spawn_blocking(move || store.put_session_detail(&saving))
            .await
            .map_err(RebuildError::Task)?
            .map_err(|source| RebuildError::Store { session, source })
    }
}

/// Writes the conversation file's lines, whole or not at all.
fn write_lines(path: &std::path::Path, lines: &[u8]) -> Result<(), RebuildError> {
    let fail = |source| RebuildError::Conversation { path: path.to_owned(), source };
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(fail)?;
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, lines).map_err(fail)?;
    std::fs::rename(&partial, path).map_err(fail)
}

/// Types `note` into the agent's terminal and presses return.
async fn type_note(socket: &std::path::Path, note: &str) -> Result<(), humata2_holder::HolderError> {
    let mut attached = humata2_holder::attach(socket, u64::MAX).await?;
    attached.control.input(1, note.as_bytes()).await?;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    attached.control.input(2, b"\r").await
}

async fn blocking_git<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, GitError> + Send + 'static,
) -> Result<T, GitError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| GitError::Io {
            path: PathBuf::new(),
            source: io::Error::other(error),
        })?
}

/// A failure's error as the operation records it.
fn failure_of(status: &Status) -> pb::Error {
    super::archive::failure_of(status)
}

#[derive(Debug, thiserror::Error)]
#[error("the link to {machine} broke before the rebuild finished")]
struct RebuildOnLinkLost {
    machine: String,
}

/// Takes a rebuild back: the folder with whatever was cloned in it, and the
/// session's records.
async fn undo(store: Arc<humata2_store::Store>, record: &SessionRecord) -> Result<(), RebuildError> {
    let (id, folder) = (record.id.clone(), record.folder.clone());
    tokio::task::spawn_blocking(move || -> Result<(), RebuildError> {
        match std::fs::remove_dir_all(&folder) {
            Err(source) if source.kind() != io::ErrorKind::NotFound => {
                return Err(RebuildError::Remove { path: folder, source });
            }
            _ => {}
        }
        store
            .remove_session(&id)
            .map_err(|source| RebuildError::Store { session: id.clone(), source })
    })
    .await
    .map_err(RebuildError::Task)?
}
