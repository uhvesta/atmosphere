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
use humata2_mirror::{MirrorError, entry_count};
use humata2_revive::ReviveError;
use humata2_sessions::{AgentError, GitError, Starter, pick_branch, remote_url, restore_clone};
use humata2_store::{
    AgentLaunch, ChannelRecord, SessionDetail, SessionRecord, SessionRepo, StartStep, StoreError,
    TabKind, TabRecord,
};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Code, Request, Status};

use super::lineage::publish_family;
use super::{COLS, Gated, ROWS, SessionSvc, clean_title, now_ms, watch};
use crate::carry::Place;
use crate::channel_service::{blocking, error_of, failed};
use crate::host_relay::assigned;
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::status;

/// The host's scrubbed copy of a session's conversation file: `None` when
/// the mirror keeps none for it.
pub(crate) type RawLines = Arc<dyn Fn(&str) -> Result<Option<Vec<u8>>, MirrorError> + Send + Sync>;

/// Points the host's mirror of a session at its conversation's new file:
/// the session, its new machine (`""`: the host), and how many entries
/// that file repeats (`None`: as many as the old one held).
pub(crate) type Rebase = Arc<dyn Fn(&str, &str, Option<u64>) -> Result<(), MirrorError> + Send + Sync>;

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
    #[error("couldn't read the conversation the host sent to {path}")]
    ReadConversation {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{staged:?} is not where a host stages a conversation")]
    BadStaged { staged: String },
    #[error("the agent {agent} can't be started here")]
    Agent {
        agent: String,
        #[source]
        source: AgentError,
    },
    #[error("couldn't start the agent")]
    Launch(#[source] ReviveError),
    #[error("couldn't write {path}, which a restart reads to take a cut rebuild back")]
    Marker {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("couldn't remove {path}")]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the daemon stopped the rebuild's work early")]
    Task(#[source] tokio::task::JoinError),
}

/// The file a rebuild's conversation is staged as, in `.carry/<name>`.
const STAGED_FILE: &str = "conversation.jsonl";

/// Whether `staged` is `.carry/<name>/conversation.jsonl` with a name of
/// letters, digits and underscores only: the one shape a host sends.
fn staged_shape(staged: &str) -> bool {
    let mut parts = staged.split('/');
    let (carry, name, file, rest) = (parts.next(), parts.next(), parts.next(), parts.next());
    carry == Some(humata2_sessions::CARRY_DIR)
        && name.is_some_and(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        && file == Some(STAGED_FILE)
        && rest.is_none()
}

/// What the names of the folders a session's conversation is staged in
/// start with: unique to the session, so a machine can tell the ones an
/// earlier, cut ask left from those of other rebuilds.
fn stage_prefix(session: &str) -> String {
    let plain: String = session.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    format!("rb_{plain}_")
}

/// Removes the folders under `.carry` that an earlier ask for `session`
/// staged and nothing took back (a host cut after it sent the conversation
/// and before the machine began), except `keep`. Best effort.
fn sweep_stale_stages(channel_dir: &std::path::Path, session: &str, keep: &str) {
    let prefix = stage_prefix(session);
    let Ok(entries) = std::fs::read_dir(channel_dir.join(humata2_sessions::CARRY_DIR)) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let stale = name.strip_prefix(&prefix).is_some_and(|rest| rest.len() == 32 && rest.chars().all(|c| c.is_ascii_hexdigit()));
        if stale && name != keep {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The folder `staged` (`.carry/<name>/conversation.jsonl`) is in, under
/// the channel's folder `channel_dir`; none when `staged` isn't that shape.
fn staged_folder(channel_dir: &std::path::Path, staged: &str) -> Option<PathBuf> {
    staged_shape(staged).then(|| channel_dir.join(humata2_sessions::CARRY_DIR).join(staged_dir_name(staged)))
}

/// The name of the folder under `.carry` that `staged` is in, or empty.
fn staged_dir_name(staged: &str) -> String {
    staged.split('/').nth(1).unwrap_or_default().to_owned()
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
            parts.push(format!("{}/{} lost {}.", named.owner, named.name, lost.join(" and ")));
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
pub(super) struct Lines(Arc<OnceLock<RawLines>>, Arc<OnceLock<Rebase>>);

impl SessionSvc {
    /// Gives the host's SessionService the mirror's scrubbed lines. Once.
    pub(crate) fn use_raw_lines(&self, lines: RawLines) {
        let _ = self.raw_lines.0.set(lines);
    }

    /// Gives the host's SessionService the way to point its mirror at a
    /// moved or rebuilt conversation. Once.
    pub(crate) fn use_rebase(&self, rebase: Rebase) {
        let _ = self.raw_lines.1.set(rebase);
    }

    /// The scrubbed lines the host's mirror kept of `session`'s
    /// conversation; none when it keeps none or has no mirror.
    pub(super) fn mirrored_lines(&self, session: &str) -> Result<Option<Vec<u8>>, MirrorError> {
        match self.raw_lines.0.get() {
            Some(lines) => lines(session),
            None => Ok(None),
        }
    }

    /// Points the host's mirror of `session` at its file on `machine` (MV-13,
    /// D209), before the record says the session is there. Best effort: a
    /// mirror that can't be pointed may mirror the history twice, which loses
    /// nothing, so the move or rebuild goes on and the log says why.
    pub(super) async fn rebase_mirror(&self, session: &str, machine: &str, skip: Option<u64>) {
        let Some(rebase) = self.raw_lines.1.get().cloned() else {
            return;
        };
        let here = if machine == self.machine { String::new() } else { machine.to_owned() };
        let (id, shown) = (session.to_owned(), machine.to_owned());
        match blocking(move || rebase(&id, &here, skip)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => crate::log::say!(
                "humatad2: couldn't point the mirror of session {session} at its file on {shown}; its history may show twice: {}",
                status::chain(&error)
            ),
            Err(status) => crate::log::say!(
                "humatad2: couldn't point the mirror of session {session} at its file on {shown}: {}",
                status.message()
            ),
        }
    }

    /// Writes `lines`, a conversation the host's mirror kept, as the new
    /// conversation of `record`'s agent on this machine, with `old_folder`
    /// rewritten to the record's folder, and starts the agent resuming it.
    /// The records must already name this machine. A restore on this Mac
    /// whose machine is away uses it (D195).
    pub(super) async fn resume_here_from(
        &self,
        record: &SessionRecord,
        lines: Vec<u8>,
        old_folder: &str,
    ) -> Result<(), pb::Error> {
        let starter = &self.starter;
        let kind = starter.agents.get(&record.agent).map_err(|source| {
            error_of(
                &RebuildError::Agent { agent: record.agent.clone(), source },
                "install the session's agent on this Mac, then restore again",
                None,
            )
        })?;
        let kind = crate::config_sync::kind_for_session(kind, &record.id);
        let conversation_id = uuid::Uuid::new_v4().hyphenated().to_string();
        let plan = kind.launch(&starter.home, &record.folder, &conversation_id);
        let lines = replaced(&lines, old_folder.as_bytes(), record.folder.display().to_string().as_bytes());
        let file = plan.conversation_file.clone();
        blocking(move || write_lines(&file, &lines))
            .await
            .map_err(|status| failure_of(&status))?
            .map_err(|error| error_of(&error, "free space on this disk or fix the folder's permissions, then restore again", None))?;
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
            holder_generation: 0,
        };
        let env = |tab: &TabRecord| self.tokens.env(tab);
        let variable_names = env(&probe)
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        let launch = AgentLaunch {
            agent: record.agent.clone(),
            folder: record.folder.clone(),
            argv: plan.resume_argv.clone(),
            resume_argv: plan.resume_argv,
            variable_names,
            conversation_id,
            conversation_file: plan.conversation_file,
            holder_socket: socket,
            cols: COLS,
            rows: ROWS,
            cli_version: kind.cli_version().await,
        };
        humata2_revive::launch_agent(&starter.store, &starter.revive, record, &tab, launch, None, &env)
            .await
            .map(drop)
            .map_err(|source| {
                error_of(
                    &RebuildError::Launch(source),
                    "the session is on this Mac now; resume it from its menu",
                    None,
                )
            })
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
                fork_of: None,
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
            Ok(written) => {
                // A kill here leaves the session whole on the machine and
                // still archived as lost here: rebuilding it again finds the
                // machine's session and switches over to it.
                crate::fault::point("rebuild.machine_done");
                // The new file repeats exactly the entries of the lines sent.
                self.rebase_mirror(&id, &target, Some(written)).await;
                self.switch_to(&record, &summary, &target).await
            }
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

    /// Copies the mirror's `lines` into the channel's folder on `target`,
    /// as a file, so no message size bounds them, and says where. The
    /// host's own copy goes once it was sent, sent or not.
    async fn send_conversation(&self, channel: &ChannelRecord, session: &str, target: &str, lines: Vec<u8>) -> Result<String, pb::Error> {
        let Some(carry) = self.carry.get().cloned() else {
            return Err(failure_of(&only_host()));
        };
        let name = format!("{}{}", stage_prefix(session), uuid::Uuid::new_v4().simple());
        let folder = format!("{}/{name}", humata2_sessions::CARRY_DIR);
        let staged = format!("{folder}/{STAGED_FILE}");
        let here = self.starter.layout.channel(&channel.folder).join(&folder);
        let (made, file) = (here.clone(), here.join(STAGED_FILE));
        let written = blocking(move || write_lines(&file, &lines)).await.map_err(|status| failure_of(&status))?;
        if let Err(error) = written {
            let _ = blocking(move || std::fs::remove_dir_all(made)).await;
            return Err(error_of(&error, "check the host's disk has room, then rebuild again", None));
        }
        let host = Place { machine: String::new(), channel: channel.id.clone(), folder: channel.folder.clone() };
        let there = Place { machine: target.to_owned(), ..host.clone() };
        let copied = carry.copy(&format!("{name}_conversation"), (&host, &staged), (&there, &folder)).await;
        let _ = blocking(move || std::fs::remove_dir_all(made)).await;
        copied.map_err(|status| failure_of(&status))?;
        Ok(staged)
    }

    /// Hands `target` the session and waits for its operation to end,
    /// mirroring its steps into `progress`. Returns how many mirror entries
    /// the conversation it sent holds.
    async fn ask_machine(
        &self,
        progress: &Progress,
        record: &SessionRecord,
        summary: &pb::ArchiveSummary,
        channel: &ChannelRecord,
        target: &str,
    ) -> Result<u64, pb::Error> {
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
        let mut written = 0;
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
            conversation: Vec::new(),
            conversation_staged: match lines {
                Some(lines) if !lines.is_empty() => {
                    written = entry_count(&lines);
                    self.send_conversation(channel, &record.id, target, lines).await?
                }
                _ => String::new(),
            },
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
            review: None,
        };
        // Nothing is moved yet: a host killed here leaves the session archived.
        crate::fault::point("rebuild-clone-api");
        for repo in &summary.repos {
            if let Some(named) = &repo.repo {
                crate::fault::point(&format!("rebuild-clone-{}", named.name));
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
        // A session the machine already holds whole (an earlier rebuild
        // that the host was cut off from) answers with its operation done,
        // and that operation may be gone from the machine's memory: there
        // is nothing to watch then.
        let mut stream = None;
        if operation.state == pb::OperationState::Running as i32 {
            let mut operations = relayed.operation();
            stream = Some(
                operations
                    .watch_operation(Request::new(pb::WatchOperationRequest {
                        id: operation.id.clone(),
                        from_version: 0,
                    }))
                    .await
                    .map_err(|status| failure_of(&status))?
                    .into_inner(),
            );
        }
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
            let Some(open) = stream.as_mut() else {
                break;
            };
            match open.message().await {
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
            Ok(written)
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
        // A kill here leaves the session switched over: the lost machine's
        // own copy of it is all that is left to forget.
        crate::fault::point("rebuild.switched");
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
            // The machine already holds this session (an earlier ask that
            // the host was cut off from, or one the machine's own restart
            // kept): the conversation staged for this ask is not wanted, and
            // nothing else would ever take it.
            self.discard_staged(&request, &rebuild).await;
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
        if !rebuild.conversation_staged.is_empty() && !staged_shape(&rebuild.conversation_staged) {
            return Err(failed(
                Code::InvalidArgument,
                &RebuildError::BadStaged { staged: rebuild.conversation_staged.clone() },
                "the host stages the conversation itself; rebuild it from the host again",
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
                self.discard_staged(&request, &rebuild).await;
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
        // Written before anything else is, so a restart can tell a rebuild
        // cut short from a session started here: it takes the rebuild back,
        // where a start would be finished without the conversation.
        let staged_dir = std::path::Path::new(&rebuild.conversation_staged)
            .parent()
            .filter(|_| !rebuild.conversation_staged.is_empty())
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (channel_dir, marked_id, marked_folder) =
            (self.starter.layout.channel(&channel.folder), id.clone(), folder.clone());
        match blocking(move || write_marker(&channel_dir, &marked_id, &marked_folder, &staged_dir)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let fix = "check the channel's folder is writable and has room, then rebuild again";
                progress.fail(error_of(&error, fix, None));
                return Err(failed(Code::Internal, &error, fix, None));
            }
            Err(refused) => {
                progress.fail(failure_of(&refused));
                return Err(refused);
            }
        }
        // A host cut after it sent the conversation and before this ask
        // left its copy; the ask now made stages another, so that one goes.
        let (swept_dir, swept_id) = (self.starter.layout.channel(&channel.folder), id.clone());
        let keep = std::path::Path::new(&staged_dir_name(&rebuild.conversation_staged)).to_owned();
        let _ = blocking(move || sweep_stale_stages(&swept_dir, &swept_id, &keep.to_string_lossy())).await;
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
        // A kill here leaves the records saved and nothing cloned: the
        // restart takes the rebuild back.
        crate::fault::point("rebuild.recorded");
        let note = Some(request.first_message).filter(|note| !note.is_empty());
        tokio::spawn(self.clone().run_rebuild_here(
            progress,
            record,
            detail,
            channel,
            Conversation { lines: rebuild.conversation, staged: rebuild.conversation_staged },
            rebuild.old_folder,
            note,
            identity::actor(caller),
        ));
        Ok(Gated::Done(pb::StartSessionResponse {
            session: Some(pb::SessionId { value: id }),
            operation: Some(operation),
        }))
    }

    /// Removes the conversation `rebuild` staged here when this ask is not
    /// going to use it. Best effort: the staged name is unique to the ask, so
    /// nothing else reads it.
    async fn discard_staged(&self, request: &pb::StartSessionRequest, rebuild: &pb::RebuildFrom) {
        let Some(channel_id) = request.channel.as_ref().map(|channel| channel.value.clone()).filter(|id| !id.is_empty()) else {
            return;
        };
        let Ok(channel) = self.find_channel(&channel_id).await else {
            return;
        };
        let Some(folder) = staged_folder(&self.starter.layout.channel(&channel.folder), &rebuild.conversation_staged) else {
            return;
        };
        let _ = blocking(move || std::fs::remove_dir_all(folder)).await;
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
        conversation: Conversation,
        old_folder: String,
        note: Option<String>,
        by: pb::Actor,
    ) {
        let id = record.id.clone();
        let mut current = String::new();
        let built = self
            .rebuild_steps(&progress, &record, &mut detail, &channel, &conversation, &old_folder, note, &mut current)
            .await;
        // What the host staged here goes, built or not; best effort.
        if let Some(staged) = std::path::Path::new(&conversation.staged).parent().filter(|_| !conversation.staged.is_empty()) {
            let folder = self.starter.layout.channel(&channel.folder).join(staged);
            let _ = blocking(move || std::fs::remove_dir_all(folder)).await;
        }
        let marker = marker_file(&self.starter.layout.channel(&channel.folder), &id);
        let _ = blocking(move || std::fs::remove_file(marker)).await;
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
        conversation: &Conversation,
        old_folder: &str,
        note: Option<String>,
        current: &mut String,
    ) -> Result<(), (RebuildError, bool)> {
        let starter = &self.starter;
        for repo in detail.repos.clone() {
            *current = format!("clone:{}", repo.name);
            progress.set_step(current, pb::StepState::Running, None);
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
            let (starter_clone, ch_folder, owner, name, path, branch) = (
                starter.clone(),
                channel.folder.clone(),
                repo.owner.clone(),
                repo.name.clone(),
                repo.path.clone(),
                repo.branch.clone(),
            );
            blocking_git(move || {
                starter_clone.restore_repo_blocking(&ch_folder, &owner, &name, &url, &path, &branch)
            })
                .await
                .map_err(clone_error)?;
            progress.set_step(current, pb::StepState::Done, None);
        }
        self.set_start(detail, StartStep::Cloned).await.map_err(|error| (error, false))?;
        // A kill at each of these leaves a rebuild the restart takes back.
        crate::fault::point("rebuild.cloned");

        *current = "write_conversation".to_owned();
        progress.set_step(current, pb::StepState::Running, None);
        let kind = starter
            .agents
            .get(&record.agent)
            .map_err(|source| (RebuildError::Agent { agent: record.agent.clone(), source }, false))?;
        let kind = crate::config_sync::kind_for_session(kind, &record.id);
        let conversation_id = uuid::Uuid::new_v4().hyphenated().to_string();
        let plan = kind.launch(&starter.home, &record.folder, &conversation_id);
        let conversation = if conversation.staged.is_empty() {
            conversation.lines.clone()
        } else {
            let path = starter.layout.channel(&channel.folder).join(&conversation.staged);
            let read = path.clone();
            blocking(move || std::fs::read(read))
                .await
                .map_err(|status| (RebuildError::ReadConversation { path: path.clone(), source: io::Error::other(status.message().to_owned()) }, false))?
                .map_err(|source| (RebuildError::ReadConversation { path, source }, false))?
        };
        // With no lines the agent starts a conversation of its own.
        let resume = !conversation.is_empty();
        if resume {
            let lines = replaced(
                &conversation,
                old_folder.as_bytes(),
                record.folder.display().to_string().as_bytes(),
            );
            write_lines(&plan.conversation_file, &lines).map_err(|error| (error, false))?;
        }
        crate::fault::point("rebuild.conversation_written");
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
            holder_generation: 0,
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
            cli_version: kind.cli_version().await,
        };
        humata2_revive::launch_agent(&starter.store, &starter.revive, record, &tab, launch, None, &env)
            .await
            .map_err(|source| (RebuildError::Launch(source), false))?;
        // From here the records stay: revive owns the agent.
        self.set_start(detail, StartStep::Done).await.map_err(|error| (error, true))?;
        // A kill here leaves the agent launched: the restart keeps it.
        crate::fault::point("rebuild.launched");
        progress.set_step(current, pb::StepState::Done, None);
        *current = "verify".to_owned();
        if let Some(note) = note {
            // D762: the note waits for the agent's own questions (its
            // folder trust, in a folder it has never seen) to be answered.
            let session = record.id.clone();
            tokio::spawn(async move {
                if let Err(error) = type_note(&socket, &note).await {
                    crate::log::say!(
                        "humatad2: couldn't tell rebuilt session {session} what was lost: {}",
                        status::chain(&error)
                    );
                }
            });
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

/// The conversation a host sends with a rebuild: in the message (older
/// hosts) or staged as a file in the channel's folder.
struct Conversation {
    lines: Vec<u8>,
    staged: String,
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

/// How long a rebuilt session's note waits for the agent's input.
const NOTE_WAIT: std::time::Duration = std::time::Duration::from_secs(600);

/// Why a rebuilt session's note wasn't typed.
#[derive(Debug, thiserror::Error)]
enum NoteFailed {
    #[error("the agent's input wasn't ready within {} minutes, so the note wasn't typed", NOTE_WAIT.as_secs() / 60)]
    NotReady(#[source] crate::input_gate::InputRefused),
    #[error("the agent's terminal didn't take the note")]
    Holder(#[source] humata2_holder::HolderError),
}

/// Types `note` into the agent's terminal and presses return, once the
/// input gate lets text in (D762).
async fn type_note(socket: &std::path::Path, note: &str) -> Result<(), NoteFailed> {
    // The gate looks only once the agent is up: one slow to start hasn't
    // drawn its own question yet, and the note would answer it.
    humata2_holder::await_keys(socket, std::time::Duration::from_secs(5))
        .await
        .map_err(NoteFailed::Holder)?;
    let deadline = tokio::time::Instant::now() + NOTE_WAIT;
    while let Err(refused) = crate::input_gate::guard(socket).await {
        if tokio::time::Instant::now() >= deadline {
            return Err(NoteFailed::NotReady(refused));
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let pause = std::time::Duration::from_millis(400); // D671 (Provisional)
    // A note the agent doesn't show gets no return: the person sees it in
    // the agent's input (BUG-T1b).
    humata2_holder::type_then_return(socket, note.as_bytes(), pause)
        .await
        .map(drop)
        .map_err(NoteFailed::Holder)
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

/// Where a channel keeps the markers of the rebuilds under way on this
/// machine, one file per session.
const MARKERS: &str = ".rebuilding";

fn marker_file(channel_dir: &std::path::Path, session: &str) -> PathBuf {
    channel_dir.join(MARKERS).join(session)
}

/// Marks a rebuild begun on this machine: the session's folder and, if the
/// host staged the conversation, the folder it is staged in (relative to
/// the channel's), one per line. Whole or not at all.
fn write_marker(
    channel_dir: &std::path::Path,
    session: &str,
    folder: &std::path::Path,
    staged_dir: &str,
) -> Result<(), RebuildError> {
    let file = marker_file(channel_dir, session);
    let fail = |source| RebuildError::Marker { path: file.clone(), source };
    if let Some(markers) = file.parent() {
        make_dirs(markers).map_err(fail)?;
    }
    // Whole or not at all, and on disk: a power cut can't leave a rebuild
    // with no marker for the restart to take it back by.
    let text = format!("{}\n{staged_dir}\n", folder.display());
    humata2_store::write_atomic(&file, text.as_bytes())
        .map_err(std::io::Error::other)
        .map_err(fail)
}

/// Makes `folder` and its missing parents, and puts each new folder's entry
/// in its parent on disk, so a power cut can't take the folder back.
fn make_dirs(folder: &std::path::Path) -> std::io::Result<()> {
    let missing: Vec<&std::path::Path> = folder
        .ancestors()
        .take_while(|found| !found.as_os_str().is_empty() && std::fs::symlink_metadata(found).is_err())
        .collect();
    humata2_durable::create_dir_all(folder, 0o700)?;
    for found in missing {
        match found.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => humata2_durable::sync_dir(parent)?,
            _ => {}
        }
    }
    Ok(())
}

/// A rebuild the last run began on this machine and didn't end.
pub(crate) struct CutRebuild {
    marker: PathBuf,
    session: String,
    folder: PathBuf,
    staged: Option<PathBuf>,
}

impl CutRebuild {
    /// Says in the restart report that this rebuild is owed its take-back.
    /// Called when the rebuild is read, before the report is written (D241).
    pub(crate) fn note(&self, operations: &crate::restart::Operations) {
        operations.record(
            &format!("rebuild:{}", self.session),
            "rebuild",
            "cloning",
            crate::restart::OperationState::Pending { waits_for: "the start to take it back".to_owned() },
        );
    }
}

/// The rebuilds the last run left, read before anything is served so a
/// rebuild begun since is left to its own call. A marker that doesn't name
/// a folder directly in its channel's sessions folder (and a staged folder
/// of the shape a host stages in) is ignored: it isn't ours to act on.
pub(crate) fn cut_rebuilds(starter: &Starter) -> Vec<CutRebuild> {
    let Ok(channels) = starter.store.channels() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for channel in channels {
        let channel_dir = starter.layout.channel(&channel.folder);
        let sessions = starter.layout.sessions(&channel.folder);
        let Ok(entries) = std::fs::read_dir(channel_dir.join(MARKERS)) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(session) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if session.ends_with(".partial") {
                continue;
            }
            let marker = entry.path();
            let Ok(text) = std::fs::read_to_string(&marker) else {
                continue;
            };
            let mut lines = text.lines();
            let folder = PathBuf::from(lines.next().unwrap_or_default());
            if folder.parent() != Some(sessions.as_path()) {
                continue;
            }
            let staged = lines
                .next()
                .filter(|dir| staged_shape(&format!("{dir}/{STAGED_FILE}")))
                .map(|dir| channel_dir.join(dir));
            found.push(CutRebuild { marker, session, folder, staged });
        }
    }
    found
}

/// Takes back each cut rebuild whose agent was never launched (its folder
/// and records), clears what the host staged, and drops the marker. One
/// whose agent was launched is left to the restart's own recovery, which
/// keeps it. Safe to repeat; runs before the restart finishes the starts
/// it finds, which would start such a session with no conversation.
pub(crate) async fn undo_cut_rebuilds(
    starter: &Starter,
    cut: Vec<CutRebuild>,
    operations: &crate::restart::Operations,
) {
    use crate::restart::OperationState as State;
    for rebuild in cut {
        let (store, id) = (starter.store.clone(), rebuild.session.clone());
        let operation = format!("rebuild:{id}");
        let record = |state: State| operations.record(&operation, "rebuild", "cloning", state);
        let undone = tokio::task::spawn_blocking(move || undo_cut(&store, &rebuild))
            .await
            .map_err(RebuildError::Task)
            .and_then(|undone| undone);
        match undone {
            Ok(CutEnd::TakenBack) => record(State::Undone {
                reason: "its agent had not been launched, so its folder and records were removed; rebuild the session again".to_owned(),
            }),
            Ok(CutEnd::Kept) => record(State::Finished),
            Err(error) => {
                let reason = format!(
                    "{}; delete its folder by hand if it is still there, then restart humatad2 to try again",
                    status::chain(&error)
                );
                crate::log::say!("humatad2: couldn't take back the cut rebuild of session {id}: {reason}");
                record(State::Stuck { reason });
            }
        }
    }
}

/// How a cut rebuild ended.
enum CutEnd {
    /// Never launched its agent: its folder and records were removed.
    TakenBack,
    /// Its agent was launched: the session stays.
    Kept,
}

fn undo_cut(store: &humata2_store::Store, rebuild: &CutRebuild) -> Result<CutEnd, RebuildError> {
    let session = &rebuild.session;
    let store_error = |source| RebuildError::Store { session: session.clone(), source };
    let launched = store.agent(session).map_err(store_error)?.is_some()
        || store.session_detail(session).map_err(store_error)?.is_some_and(|detail| detail.start == StartStep::Done);
    if !launched {
        remove_dir(&rebuild.folder)?;
        store.remove_session(session).map_err(store_error)?;
    }
    if let Some(staged) = &rebuild.staged {
        remove_dir(staged)?;
    }
    match std::fs::remove_file(&rebuild.marker) {
        Err(source) if source.kind() != io::ErrorKind::NotFound => {
            Err(RebuildError::Remove { path: rebuild.marker.clone(), source })
        }
        _ => Ok(if launched { CutEnd::Kept } else { CutEnd::TakenBack }),
    }
}

fn remove_dir(path: &std::path::Path) -> Result<(), RebuildError> {
    match std::fs::remove_dir_all(path) {
        Err(source) if source.kind() != io::ErrorKind::NotFound => {
            Err(RebuildError::Remove { path: path.to_owned(), source })
        }
        _ => Ok(()),
    }
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

#[cfg(test)]
mod tests {
    use super::{CutRebuild, staged_folder, staged_shape};

    /// A rebuild the last run left is named pending the moment it is noted:
    /// the report that is written after names it, with nothing running yet.
    #[test]
    fn a_cut_rebuild_is_named_pending_when_noted() {
        let report = tempfile::tempdir().unwrap();
        let operations = crate::restart::Operations::default();
        operations.set_folder(report.path());
        let rebuild = CutRebuild {
            marker: "/nowhere/marker".into(),
            session: "se_1".to_owned(),
            folder: "/nowhere/folder".into(),
            staged: None,
        };
        rebuild.note(&operations);
        assert_eq!(crate::test_support::pending_in(report.path()), vec!["rebuild:se_1".to_owned()]);
    }

    #[test]
    fn a_staged_conversation_is_in_its_own_folder_under_the_channels_carry() {
        let channel = std::path::Path::new("/h/ch/payments");
        assert_eq!(
            staged_folder(channel, ".carry/rb_s1_0f3a/conversation.jsonl"),
            Some(channel.join(".carry").join("rb_s1_0f3a"))
        );
        assert_eq!(staged_folder(channel, ".carry/../conversation.jsonl"), None);
        assert_eq!(staged_folder(channel, ""), None);
    }

    #[test]
    fn only_a_carry_folder_of_plain_name_holding_the_conversation_is_staged() {
        assert!(staged_shape(".carry/rb_0f3a/conversation.jsonl"));
        for bad in [
            "",
            ".carry/conversation.jsonl",
            ".carry//conversation.jsonl",
            ".carry/../conversation.jsonl",
            ".carry/rb_0f3a/other.jsonl",
            ".carry/rb_0f3a/conversation.jsonl/more",
            "sessions/rb_0f3a/conversation.jsonl",
            "/etc/rb_0f3a/conversation.jsonl",
            ".carry/rb-0f3a/conversation.jsonl",
        ] {
            assert!(!staged_shape(bad), "{bad:?} was taken as staged");
        }
    }
}
