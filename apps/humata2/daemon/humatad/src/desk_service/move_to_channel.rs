//! Moving a Desk session into a channel (DK-15..DK-19, D305, D315, MV-16).
//! PlanDeskMove says where the session would land and the token the move
//! must carry. MoveDeskSessionToChannel moves it by the carry, as a move to
//! another machine moves a session (MV-4): the session shows Moving and its
//! terminal is read-only while it goes, the folder and the conversation
//! follow it to `<channel folder>/sessions/<name>` on the machine it is on,
//! and a power cut at any point is finished or undone at the next start. A
//! folder the user picked is copied like a scratch folder and the original
//! stays where it was; `move_picked_folder` is ignored (D461).
//!
//! D801 E: a scratch folder that is on the same filesystem as the landing is
//! not copied but renamed, and nothing running stops. The agent and the
//! shells keep their holders and programs; the old folder stays as a link to
//! the new one while a holder that started under it runs (the alias of
//! `channel_service::rename_alias`); each agent gets a note beside its
//! holder's socket, which its next resume follows to carry its conversation
//! to the new folder's key. The rename is journaled first (`Renaming`); a
//! power cut before the switch renames the folder back, and one after it
//! only finishes. Across filesystems, and for a picked folder, the move is
//! the copy below, which stops the agent and the shells and resumes them.
//!
//! The move runs on the machine that has the session. The host relays a
//! move of a session on another machine there, follows its operation, and
//! files the session in the channel in its own view once the machine has.
//!
//! On the machine the order is: the repos the channel lacks are added; the
//! session is staged and its folder copied to the landing (journal
//! `copied`); the agent and the shells stop; the copy is taken up; each
//! landed clone is put on the session's branch and pointed at the channel;
//! the switch files it in the channel (journal `switched`); the old place
//! goes to the Bin, except a picked folder, which stays; and the agent resumes in its new folder. Anything that
//! fails before the switch takes the landing back and resumes the agent
//! where it was; after the switch the move is only finished.

use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata2_access::{Call, Capability, ChannelId, Decision, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{DESK_CHANNEL, DeskMoveRepo as FoundRepo, Landing, TurnEnd, desk_move_repos, repoint_clone};
use humata2_revive::{RekeyError, RekeyNote, Started as AgentStart, remove_note, write_note};
use humata2_store::{
    AgentState, ChannelRecord, DeskFolderKind, SessionDetail, SessionRecord, SessionRepo, StartStep, Store, StoreError,
    TabKind,
};
use humata2_transfers::NewItem;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::OwnedMutexGuard;
use tonic::transport::Channel;
use tonic::{Code, Request, Response, Status};

use super::{DeskSvc, failed_precondition, now_ms, session_id};
use crate::carry_service::failed as carry_failed;
use crate::channel_service::rename_folder::rename_alias as alias;
use crate::channel_service::{Gated, answer_gated, blocking, failed, gate};
use crate::host_relay::assigned;
use crate::identity::{self, Caller, UserVia};
use crate::operations::{Progress, Started};
use crate::remote_work::call_failed;
use crate::restart::OperationState;
use crate::session_service::{Loaded, Moving, run_resume, running_turn, start_error};
use crate::status;
use crate::trash_service::caller_data;

/// The folder under Humata's state folder where journals are kept.
const JOURNAL_DIR: &str = "desk_move";

/// The steps a move shows, on the machine and in the host's view of it.
const STEPS: [(&str, &str); 10] = [
    ("add_repos", "Adding the repositories to the channel"),
    ("pack", "Staging the session"),
    ("copy", "Copying the folder"),
    ("stop", "Stopping the agent and the shells"),
    ("unpack", "Taking up the copy"),
    ("branch", "Putting each clone on its branch"),
    ("switch", "Filing the session in the channel"),
    ("release", "Moving the old folder to the Bin"),
    ("launch", "Starting the agent again"),
    ("verify", "Checking it took up its conversation"),
];

fn move_steps() -> Vec<(String, String)> {
    STEPS.iter().map(|(key, title)| ((*key).to_owned(), (*title).to_owned())).collect()
}

/// What a move would do, as the plan shows it.
struct Planned {
    channel: ChannelRecord,
    /// The branch every repo is put on.
    branch: String,
    /// The session folder the Desk folder lands in.
    folder: PathBuf,
    /// `folder` under the channel's folder, `sessions/<name>`.
    rel: String,
    repos: Vec<FoundRepo>,
    /// The repos the channel lacks, as `(owner, name)`.
    adds: Vec<(String, String)>,
    token: String,
    /// D801 E: the folder is renamed into place and what runs in it keeps
    /// running; otherwise it is copied and the agent and shells restart.
    renames: bool,
}

impl Planned {
    fn plan(&self) -> pb::DeskMovePlan {
        let repo_ref = |owner: &str, name: &str| pb::RepoRef { owner: owner.to_owned(), name: name.to_owned() };
        pb::DeskMovePlan {
            channel: Some(pb::ChannelId { value: self.channel.id.clone() }),
            folder: self.folder.to_string_lossy().into_owned(),
            repos: self
                .repos
                .iter()
                .map(|repo| pb::DeskMoveRepo {
                    repo: Some(repo_ref(&repo.owner, &repo.name)),
                    branch: self.branch.clone(),
                    start_commit: repo.head.clone(),
                    start_branch: repo.head_branch.clone().unwrap_or_default(),
                    uncommitted_files: repo.uncommitted,
                })
                .collect(),
            adds: self.adds.iter().map(|(owner, name)| repo_ref(owner, name)).collect(),
            token: Some(pb::PlanToken { value: self.token.clone() }),
            branch: self.branch.clone(),
        }
    }

    /// What the plan says now, for a call whose plan no longer matches.
    fn changes(&self) -> Vec<String> {
        let mut changes = vec![format!("the session moves to {} on branch {}", self.folder.display(), self.branch)];
        for repo in &self.repos {
            changes.push(format!(
                "{}/{} is at {} with {} uncommitted file(s)",
                repo.owner, repo.name, repo.head, repo.uncommitted
            ));
        }
        for (owner, name) in &self.adds {
            changes.push(format!("the channel gets {owner}/{name}"));
        }
        if !self.renames {
            changes.push(
                "the folder is copied, not renamed, so the agent and the shells stop and start again in the new folder"
                    .to_owned(),
            );
        }
        changes
    }
}

/// How far a move got, as its journal says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    /// The session is being staged and copied; nothing of it changed yet.
    Packing,
    /// A rename move: its notes are written and the folder is renamed, or
    /// about to be. The records still say it is a Desk session.
    Renaming,
    /// The copy is whole at the landing; the session works where it was.
    Copied,
    /// The agent and its shells are stopping, and the landing is synced
    /// with what they wrote since the pack; no record changed yet.
    Stopping,
    /// The copy is being taken up: the records may name the landing.
    Unpacking,
    /// The session is filed in the channel: the move only finishes now.
    Switched,
    /// The old place is in the Bin; only the resume is left.
    Released,
}

/// One clone the move carries.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MovedRepo {
    owner: String,
    name: String,
    /// The clone's folder under the session's folder.
    dir: String,
}

/// Whether a move leaves its old Desk folder where it was: when its journal
/// says so, and, for a journal from before D461 that doesn't say, unless the
/// folder is one Humata made under `desk_root`. A folder the user picked is
/// never put in the Bin, whatever a power cut left to finish.
fn keeps_old_folder(said: Option<bool>, desk_root: &Path, old_folder: &Path) -> bool {
    said.unwrap_or_else(|| humata2_sessions::scratch_child(desk_root, old_folder).is_err())
}

/// What a move journals before it changes anything, kept until it ends.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Journal {
    session: String,
    title: String,
    /// The target channel.
    channel: String,
    branch: String,
    /// The session folder the Desk folder lands in.
    folder: PathBuf,
    /// The Desk folder.
    old_folder: PathBuf,
    /// D801 E: the folder was renamed into place (see the module doc), so a
    /// power cut renames it back or finishes, and nothing is resumed.
    #[serde(default)]
    rename: bool,
    /// The Desk folder is one the user picked, so the move copies it and
    /// leaves it where it was (D461). A journal from before doesn't say
    /// (`None`), and then only a folder Humata made may go to the Bin.
    #[serde(default)]
    keep_old_folder: Option<bool>,
    /// The machine the session's record names, which the copy's take-up
    /// leaves empty.
    machine: String,
    /// The move's operation, which names its staging.
    operation: String,
    phase: Phase,
    repos: Vec<MovedRepo>,
}

/// What a move runs on, once it is accepted.
struct Job {
    planned: Planned,
    journal: Journal,
}

/// A step that failed, and why.
struct Failure {
    step: &'static str,
    status: Status,
}

fn at(step: &'static str) -> impl FnOnce(Status) -> Failure {
    move |status| Failure { step, status }
}

/// How a journal could not be kept or read.
#[derive(Debug, thiserror::Error)]
enum JournalError {
    #[error(
        "couldn't save the record of the move of Desk session {session} at {path}: {source}; \
         check that Humata's state folder can be written to and has room, then move the session again"
    )]
    Write {
        session: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't encode the record of the move of Desk session {session}: {source}")]
    Encode {
        session: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "couldn't read {path}, a record of a move cut short: {source}; check the file's permissions. \
         The session was left as it is"
    )]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{path} isn't a record of a Desk move: {source}; delete the file if no move was cut short, \
         or restore it from a backup if one was"
    )]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

fn journal_path(state: &Path, session: &str) -> PathBuf {
    state.join(JOURNAL_DIR).join(format!("{session}.json"))
}

fn write_journal(state: &Path, journal: &Journal) -> Result<(), JournalError> {
    let dir = state.join(JOURNAL_DIR);
    let path = journal_path(state, &journal.session);
    let io = |source| JournalError::Write { session: journal.session.clone(), path: path.clone(), source };
    let bytes = serde_json::to_vec(journal)
        .map_err(|source| JournalError::Encode { session: journal.session.clone(), source })?;
    fs::create_dir_all(&dir).map_err(io)?;
    let temp = dir.join(format!(".{}.json.tmp", journal.session));
    let mut file = fs::File::create(&temp).map_err(io)?;
    file.write_all(&bytes).map_err(io)?;
    file.sync_all().map_err(io)?;
    fs::rename(&temp, &path).map_err(io)?;
    fs::File::open(&dir).and_then(|folder| folder.sync_all()).map_err(io)
}

/// Every journal left in `state`, each as it read.
fn read_journals(state: &Path) -> Vec<Result<Journal, JournalError>> {
    let dir = state.join(JOURNAL_DIR);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(source) => return vec![Err(JournalError::Read { path: dir, source })],
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let read = fs::read(&path)
            .map_err(|source| JournalError::Read { path: path.clone(), source })
            .and_then(|bytes| {
                serde_json::from_slice(&bytes).map_err(|source| JournalError::Parse { path: path.clone(), source })
            });
        found.push(read);
    }
    found
}

/// Takes the journal away; one already gone is as removed.
fn remove_journal(state: &Path, session: &str) {
    let path = journal_path(state, session);
    if let Err(error) = fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        crate::log::say!(
            "humatad2: couldn't remove the record of the move of Desk session {session} at {}: {error}; delete the file by hand",
            path.display()
        );
    }
}

/// The session a call moves: the user names it; an agent leaves it out, or
/// names its own. Another session is the user's to move (DK-15).
fn session_of_call(caller: &Caller, named: Option<&pb::SessionId>) -> Result<String, Status> {
    let named = named.map(|id| id.value.clone()).filter(|id| !id.is_empty());
    let own = match caller {
        Caller::Agent(agent) => Some(agent.session.id.as_str().to_owned()),
        _ => None,
    };
    match (named, own) {
        (None, Some(own)) => Ok(own),
        (Some(named), Some(own)) if named == own => Ok(own),
        (named, _) => {
            super::user_only(caller)?;
            session_id(named.map(|value| pb::SessionId { value }))
        }
    }
}

fn channel_named(channel: Option<&pb::ChannelId>) -> Result<String, Status> {
    channel
        .map(|channel| channel.value.clone())
        .filter(|channel| !channel.is_empty())
        .ok_or_else(|| status::missing("channel"))
}

fn no_channel(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Channel,
        id,
        format!("no channel {id} is on this machine"),
        "list the channels to see their ids; it may have been deleted or be on another machine",
    )
}

fn no_session(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Session,
        id,
        format!("no session {id} is on this machine"),
        "list the sessions to see their ids; it may have been removed or be on another machine",
    )
}

/// What a plan that no longer matches answers (DK-19).
fn stale(planned: &Planned) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            "what moving the Desk session would do has changed since it was planned, so nothing was moved",
            "plan the move again, check what it does now, then move with the new plan's token",
            pb::error::Kind::PlanStale(pb::PlanStale { changes: planned.changes() }),
        ),
    )
}

/// D314: a move would cut off the turn the agent is in; the user is told before it goes on.
fn busy(id: &str, title: &str) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!(
                "Desk session {title} is in the middle of a turn, and moving it stops its agent, so nothing was moved"
            ),
            "wait for the turn to end, or confirm the move to stop the agent now; it resumes in its conversation once the session has moved",
            pb::error::Kind::SessionBusy(pb::SessionBusy {
                session: Some(pb::SessionId { value: id.to_owned() }),
                running: None,
                running_title: title.to_owned(),
            }),
        ),
    )
}

/// Runs `work` off the async threads; its own failure is the answer.
async fn off<T: Send + 'static>(work: impl FnOnce() -> Result<T, Status> + Send + 'static) -> Result<T, Status> {
    blocking(work).await.and_then(|done| done)
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Fails `progress` at `step` with what `status` carries.
fn end_failed(progress: Progress, step: &str, status: &Status) {
    let error = status::error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        ..pb::Error::default()
    });
    progress.set_step(step, pb::StepState::Failed, Some(error.message.clone()));
    progress.fail(error);
}

fn session_ref(id: &str) -> pb::EntityRef {
    pb::EntityRef { entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.to_owned() })) }
}

/// The name of a clone's folder under a Desk folder.
fn clone_dir(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

impl DeskSvc {
    /// What moving `request.session` into `request.channel` would do.
    pub(super) async fn plan_move(&self, caller: &Caller, request: pb::PlanDeskMoveRequest) -> Result<pb::DeskMovePlan, Status> {
        let id = session_id(request.session.clone())?;
        let channel = channel_named(request.channel.as_ref())?;
        let (record, _) = self.load(&id).await?;
        if let Some(machine) = self.elsewhere(&record) {
            super::user_only(caller)?;
            crate::upgrade::gate::machine_ready(&self.migrations, &pb::MachineId { value: machine.clone() })?;
            let mut desk = self.desk_on(&machine)?;
            return desk
                .plan_desk_move(assigned(request, None).map_err(Status::from)?)
                .await
                .map(Response::into_inner)
                .map_err(|source| Status::from(call_failed(&machine, "plan the Desk session's move", source)));
        }
        super::refuse_if_closed(&self.starter.store, &self.machine, &id).await?;
        self.check_landing(&id).await?;
        let planned = self.plan_of(&record, &channel, &request.branch).await?;
        Ok(planned.plan())
    }

    /// Moves a Desk session into a channel, the user's or its own agent's.
    pub(super) async fn move_session(
        &self,
        caller: &Caller,
        mut request: pb::MoveDeskSessionToChannelRequest,
    ) -> Result<Response<pb::Operation>, Status> {
        let id = session_of_call(caller, request.session.as_ref())?;
        let (record, _) = self.load(&id).await?;
        if let Some(machine) = self.elsewhere(&record) {
            // The machine judges the move; an agent there asks its own daemon.
            super::user_only(caller)?;
            return self.move_there(caller, &machine, &id, request).await.map(Response::new);
        }
        if request.plan.is_none() && matches!(caller, Caller::Agent(_)) {
            // The agent's call is planned now, so the approved run answers
            // PlanStale when what it would do has changed meanwhile.
            request.plan = Some(self.preflight(caller, &request).await?);
        }
        let key = request.idempotency_key.as_ref().map(|key| key.value.clone());
        let first = self.start_move(caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(&self.inbox, caller, key, "humata.v1.Operation", first, move || async move {
            me.start_move(&who, request, true).await
        })
        .await
    }

    /// Refuses a session whose agent's conversation no adapter carries
    /// (CV-3, CV-7), before anything changes.
    async fn check_landing(&self, id: &str) -> Result<(), Status> {
        let (starter, key) = (self.starter.clone(), id.to_owned());
        off(move || starter.check_landing(&key).map_err(carry_failed)).await
    }

    /// An agent's call before it asks: its session moves, it is no plain
    /// terminal, and the target would take the repos it adds. Answers the
    /// plan's token.
    async fn preflight(&self, caller: &Caller, request: &pb::MoveDeskSessionToChannelRequest) -> Result<pb::PlanToken, Status> {
        let id = session_of_call(caller, request.session.as_ref())?;
        let channel = channel_named(request.channel.as_ref())?;
        let (record, _) = self.load(&id).await?;
        super::refuse_if_closed(&self.starter.store, &self.machine, &id).await?;
        self.check_mover(caller, &record)?;
        self.check_landing(&id).await?;
        let planned = self.plan_of(&record, &channel, &request.branch).await?;
        let never = self.guard.with_policy(|policy, _| {
            policy
                .channel_rules(&ChannelId::new(channel.as_str()))
                .and_then(|rules| rules.decision(Capability::ChannelRepoAdd))
        }) == Some(Decision::Never);
        if never && !planned.adds.is_empty() {
            return Err(status::refused(&Refusal::NeverRule {
                channel: ChannelId::new(channel.as_str()),
                capability: Capability::ChannelRepoAdd,
            }));
        }
        Ok(pb::PlanToken { value: planned.token })
    }

    /// An agent has no plain terminal to move: only the user moves one
    /// (DK-15).
    fn check_mover(&self, caller: &Caller, record: &SessionRecord) -> Result<(), Status> {
        if !matches!(caller, Caller::Agent(_)) {
            return Ok(());
        }
        let kind = self.starter.agents.get(&record.agent).map_err(|error| {
            failed(Code::FailedPrecondition, &error, "check the agent in Settings > Agents, then ask again", None)
        })?;
        if kind.turn_end == TurnEnd::None {
            return Err(failed_precondition(
                format!(
                    "Desk session {} runs a plain terminal, which has no agent to propose a move, so nothing was asked",
                    record.id
                ),
                "ask the user to move it with Move into a channel on the session's menu",
            ));
        }
        Ok(())
    }

    /// What moving `record` into `channel` would do.
    async fn plan_of(&self, record: &SessionRecord, channel: &str, typed: &str) -> Result<Planned, Status> {
        let (store, key) = (self.starter.store.clone(), channel.to_owned());
        let found = blocking(move || store.channel(&key)).await?;
        let channel_record = match found {
            Ok(Some(channel)) => channel,
            Ok(None) | Err(StoreError::InvalidId { .. }) => return Err(no_channel(channel)),
            Err(error) => return Err(status::store(error, &self.machine, "read the channel")),
        };
        if !channel_record.machines.contains(&self.machine) {
            return Err(failed_precondition(
                format!(
                    "channel {channel} isn't on this machine, and a Desk session moves into a channel on the machine it works on, so nothing was planned"
                ),
                "add this machine to the channel, or pick a channel that has it, then plan the move again",
            ));
        }
        let folder = record.folder.clone();
        let repos = off(move || {
            desk_move_repos(&folder).map_err(|error| {
                failed(
                    Code::FailedPrecondition,
                    &error,
                    "fix or remove the repository the message names, then plan the move again",
                    None,
                )
            })
        })
        .await?;
        let names: Vec<(String, String)> = repos.iter().map(|repo| (repo.owner.clone(), repo.name.clone())).collect();
        let (starter, id, title) = (self.starter.clone(), channel_record.id.clone(), record.title.clone());
        let typed = Some(typed.to_owned()).filter(|typed| !typed.is_empty());
        let branch = off(move || {
            starter.desk_move_branch(&id, &names, &title, typed.as_deref()).map_err(|error| {
                let (code, error) = start_error(&error);
                status::failure(code, error)
            })
        })
        .await?;
        let folder = self.starter.layout.session(&channel_record.folder, &branch.folder);
        let rel = folder
            .strip_prefix(self.starter.layout.channel(&channel_record.folder))
            .map(|rel| rel.to_string_lossy().into_owned())
            .map_err(|_| {
                status::internal(
                    format!("the session folder {} isn't under its channel's folder", folder.display()),
                    "",
                )
            })?;
        let adds: Vec<(String, String)> = repos
            .iter()
            .map(|repo| (repo.owner.clone(), repo.name.clone()))
            .filter(|(owner, name)| !channel_record.repos.iter().any(|repo| &repo.owner == owner && &repo.name == name))
            .collect();
        let mut digest = Sha256::new();
        let mut feed = |text: &str| {
            digest.update(text.as_bytes());
            digest.update([0u8]);
        };
        feed(&channel_record.id);
        feed(&channel_record.revision.to_string());
        for repo in &channel_record.repos {
            feed(&repo.owner);
            feed(&repo.name);
        }
        feed(&branch.name);
        feed(&folder.to_string_lossy());
        for repo in &repos {
            feed(&repo.owner);
            feed(&repo.name);
            feed(&repo.head);
            feed(&repo.uncommitted.to_string());
            feed(repo.head_branch.as_deref().unwrap_or(""));
        }
        let token = hex(&digest.finalize());
        let (store, id, old, landing, machine) =
            (self.starter.store.clone(), record.id.clone(), record.folder.clone(), folder.clone(), self.machine.clone());
        let renames = off(move || {
            let row = store.desk(&id).map_err(|error| status::store(error, &machine, "look up the Desk session"))?;
            Ok(row.is_some_and(|row| row.kind != DeskFolderKind::Picked) && same_device(&old, &landing))
        })
        .await?;
        Ok(Planned { channel: channel_record, branch: branch.name, folder, rel, repos, adds, token, renames })
    }

    /// The body of a move; `approved` once the user said yes to an agent's
    /// call. Every refusal comes before anything changes. Answers the
    /// Operation at once, and the move goes on in the background.
    async fn start_move(
        &self,
        caller: &Caller,
        request: pb::MoveDeskSessionToChannelRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        let id = session_of_call(caller, request.session.as_ref())?;
        let channel = channel_named(request.channel.as_ref())?;
        if let Some(operation) = self.operations.existing(caller, request.idempotency_key.as_ref())? {
            return Ok(Gated::Done(operation));
        }
        let held = self.locks.hold(&id).await;
        let (record, row) = self.load(&id).await?;
        super::refuse_if_closed(&self.starter.store, &self.machine, &id).await?;
        self.check_mover(caller, &record)?;
        self.check_landing(&id).await?;
        let planned = self.plan_of(&record, &channel, &request.branch).await?;
        match (&request.plan, caller) {
            (Some(token), _) => {
                if token.value != planned.token {
                    return Err(stale(&planned));
                }
            }
            (None, Caller::Agent(_)) => {}
            (None, _) => return Err(status::missing("plan")),
        }
        let call = Call::Use {
            capability: Capability::DeskMoveToChannel,
            target: Target::Channel(ChannelId::new(channel.as_str())),
        };
        if let Some(asked) = gate(&self.guard, caller, &call, approved)? {
            return Ok(Gated::Ask(asked));
        }
        // A folder the user picked is copied like a scratch folder (D461).
        let (store, key) = (self.starter.store.clone(), id.clone());
        let agent = blocking(move || store.agent(&key))
            .await?
            .map_err(|error| status::store(error, &self.machine, "read the Desk session's agent"))?;
        match agent.as_ref().map(|agent| &agent.state) {
            Some(AgentState::Launching | AgentState::Resuming) => {
                return Err(failed_precondition(
                    format!("Desk session {id}'s agent is still starting, so the session can't be moved yet"),
                    "wait until the agent is running, then move the session again",
                ));
            }
            // An agent's own call is made in its turn: the move is its pick.
            Some(AgentState::Running { holder_pid })
                if !planned.renames
                    && !request.confirm_running
                    && !approved
                    && !matches!(caller, Caller::Agent(_))
                    && running_turn(&id, *holder_pid) != pb::SessionStatus::Idle =>
            {
                return Err(busy(&id, &record.title));
            }
            _ => {}
        }
        if fs::symlink_metadata(&planned.folder).is_ok() {
            return Err(failed_precondition(
                format!("{} is already there, so Desk session {id} can't be moved into it", planned.folder.display()),
                "pick another branch name, or move what is in that folder away if it is not needed, then move the session again",
            ));
        }
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::MoveSession,
            session_ref(&id),
            &move_steps(),
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        let journal = Journal {
            session: id,
            title: record.title.clone(),
            channel: planned.channel.id.clone(),
            branch: planned.branch.clone(),
            folder: planned.folder.clone(),
            old_folder: record.folder.clone(),
            rename: planned.renames,
            keep_old_folder: Some(row.kind == DeskFolderKind::Picked),
            machine: record.machine.clone(),
            operation: operation.id.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
            phase: Phase::Packing,
            repos: planned
                .repos
                .iter()
                .map(|repo| MovedRepo { owner: repo.owner.clone(), name: repo.name.clone(), dir: clone_dir(&repo.path) })
                .collect(),
        };
        let (svc, who) = (self.clone(), caller.clone());
        tokio::spawn(async move {
            let job = Job { planned, journal };
            if job.planned.renames {
                svc.run_rename(who, held, job, progress).await;
            } else {
                svc.run_move(who, held, job, progress).await;
            }
        });
        Ok(Gated::Done(operation))
    }

    async fn save_journal(&self, journal: &Journal) -> Result<(), Status> {
        let (state, journal) = (self.state.clone(), journal.clone());
        off(move || {
            write_journal(&state, &journal).map_err(|error| {
                failed(Code::Internal, &error, "check Humata's state folder, then move the session again", None)
            })
        })
        .await
    }

    /// Runs an accepted move, holding the session's lock and showing it as
    /// Moving until it ends.
    async fn run_move(&self, caller: Caller, _held: OwnedMutexGuard<()>, mut job: Job, progress: Progress) {
        let _moving = Moving::begin(&job.journal.session, &self.machine, &job.journal.operation);
        if let Err(failure) = self.add_repos(&caller, &job, &progress).await {
            end_failed(progress, failure.step, &failure.status);
            return;
        }
        if let Err(error) = self.save_journal(&job.journal).await {
            end_failed(progress, "pack", &error);
            return;
        }
        let mut stopped = false;
        if let Err(failure) = self.before_switch(&mut job, &progress, &mut stopped).await {
            if let Err(error) = self.take_back(&caller, &job.journal, stopped).await {
                crate::log::say!(
                    "humatad2: couldn't take back the move of Desk session {}: {}; restart humatad2 to try again",
                    job.journal.session,
                    error.message()
                );
            }
            end_failed(progress, failure.step, &failure.status);
            return;
        }
        progress.set_step("switch", pb::StepState::Done, None);
        crate::fault::point("move-switched");
        if let Err(error) = self.finish_switch(&job.journal, Some(&caller)).await {
            end_failed(progress, "release", &error);
            return;
        }
        progress.set_step("release", pb::StepState::Running, None);
        if let Err(error) = self.release(&mut job.journal, Some(&caller)).await {
            end_failed(progress, "release", &error);
            return;
        }
        progress.set_step("release", pb::StepState::Done, None);
        let id = job.journal.session.clone();
        let (store, key) = (self.starter.store.clone(), id.clone());
        let loaded = match blocking(move || Loaded::read(&store, &key)).await {
            Ok(Ok(Some(loaded))) => loaded,
            Ok(Ok(None)) => {
                end_failed(progress, "launch", &no_session(&id));
                return;
            }
            Ok(Err(error)) => {
                end_failed(progress, "launch", &status::store(error, &self.machine, "read the moved session"));
                return;
            }
            Err(error) => {
                end_failed(progress, "launch", &error);
                return;
            }
        };
        // The agent and the shells come back in the new folder, and the
        // operation ends with the resume.
        run_resume(self.starter.clone(), self.tell.clone(), identity::actor(&caller), self.tokens.clone(), loaded, progress)
            .await;
        remove_journal(&self.state, &id);
        self.tell.session(&id, pb::Change::Updated, Some(identity::actor(&caller))).await;
    }

    /// Adds the repos the channel lacks, before anything of the session
    /// changes, so a failure here leaves it as it was.
    async fn add_repos(&self, caller: &Caller, job: &Job, progress: &Progress) -> Result<(), Failure> {
        progress.set_step("add_repos", pb::StepState::Running, None);
        for (owner, name) in &job.planned.adds {
            self.add_channel_repo(caller, &job.planned.channel.id, owner, name).await.map_err(at("add_repos"))?;
        }
        progress.set_step("add_repos", pb::StepState::Done, None);
        Ok(())
    }

    /// The steps up to the switch. `stopped` says whether the agent was
    /// stopped, so that a failure resumes it.
    async fn before_switch(&self, job: &mut Job, progress: &Progress, stopped: &mut bool) -> Result<(), Failure> {
        let id = job.journal.session.clone();
        let (op, channel) = (job.journal.operation.clone(), job.planned.channel.id.clone());
        let landing = Landing { channel: channel.clone(), rel: job.planned.rel.clone() };
        let landing_again = landing.clone();

        progress.set_step("pack", pb::StepState::Running, None);
        let (starter, key, at_op) = (self.starter.clone(), id.clone(), op.clone());
        let packed =
            off(move || starter.pack_landing(&at_op, &key, &landing).map_err(carry_failed)).await.map_err(at("pack"))?;
        progress.set_step("pack", pb::StepState::Done, None);

        progress.set_step("copy", pb::StepState::Running, None);
        let (starter, key, at_op, into) = (self.starter.clone(), id.clone(), op.clone(), channel.clone());
        off(move || starter.land_copy(&at_op, &key, &into, &packed).map(drop).map_err(carry_failed))
            .await
            .map_err(at("copy"))?;
        job.journal.phase = Phase::Copied;
        self.save_journal(&job.journal).await.map_err(at("copy"))?;
        progress.set_step("copy", pb::StepState::Done, None);
        crate::fault::point("move-copied");
        crate::fault::hold_point("move-copied").await;

        progress.set_step("stop", pb::StepState::Running, None);
        job.journal.phase = Phase::Stopping;
        self.save_journal(&job.journal).await.map_err(at("stop"))?;
        *stopped = true;
        self.stop_for_move(&id).await.map_err(at("stop"))?;
        progress.set_step("stop", pb::StepState::Done, None);

        progress.set_step("unpack", pb::StepState::Running, None);
        // What the agent and its shells wrote since the pack, up to the stop,
        // lands too: one more pass, now that nothing writes.
        let (starter, key, at_op, again) = (self.starter.clone(), id.clone(), op.clone(), landing_again);
        off(move || starter.resync_landing(&at_op, &key, &again).map_err(carry_failed)).await.map_err(at("unpack"))?;
        job.journal.phase = Phase::Unpacking;
        self.save_journal(&job.journal).await.map_err(at("unpack"))?;
        let (starter, key, at_op, into) = (self.starter.clone(), id.clone(), op.clone(), channel.clone());
        off(move || starter.unpack_landed(&at_op, &key, &into).map(drop).map_err(carry_failed))
            .await
            .map_err(at("unpack"))?;
        progress.set_step("unpack", pb::StepState::Done, None);

        progress.set_step("branch", pb::StepState::Running, None);
        self.repoint(&job.journal, &job.planned.channel.folder).await.map_err(at("branch"))?;
        progress.set_step("branch", pb::StepState::Done, None);

        progress.set_step("switch", pb::StepState::Running, None);
        self.switch(&job.journal).await.map_err(at("switch"))?;
        job.journal.phase = Phase::Switched;
        // The session is in the channel now, so a journal that couldn't be
        // saved still finishes: the store says the switch was made.
        if let Err(error) = self.save_journal(&job.journal).await {
            crate::log::say!("humatad2: {}", error.message());
        }
        Ok(())
    }

    /// Ends the session's shells and stops its agent for the move. The
    /// shells' tabs stay: the resume starts them again in the new folder.
    async fn stop_for_move(&self, id: &str) -> Result<(), Status> {
        let (starter, key) = (self.starter.clone(), id.to_owned());
        let shells = off(move || starter.shell_holders(&key).map_err(carry_failed)).await?;
        for (socket, pid) in shells {
            // A holder already gone is as ended as one that was asked to.
            if let Err(error) =
                crate::holder_end::shut_or_end(&socket, pid, &format!("Desk session {id}'s shell")).await
            {
                crate::log::say!("humatad2: {error}");
            }
        }
        self.stop_agent(id).await
    }

    /// Puts each landed clone on the session's branch and points it at the
    /// channel. The clone is the one copied, never one cloned again.
    async fn repoint(&self, journal: &Journal, channel_folder: &str) -> Result<(), Status> {
        let (starter, journal, channel_folder) = (self.starter.clone(), journal.clone(), channel_folder.to_owned());
        off(move || {
            for repo in &journal.repos {
                let remote = starter.desk_move_remote(&channel_folder, &repo.owner, &repo.name).map_err(|error| {
                    failed(
                        Code::FailedPrecondition,
                        &error,
                        "check the channel's remote template in Settings, then move the session again",
                        None,
                    )
                })?;
                repoint_clone(&journal.folder.join(&repo.dir), &journal.branch, &remote).map_err(|error| {
                    failed(Code::FailedPrecondition, &error, "fix what git says, then move the session again", None)
                })?;
            }
            Ok(())
        })
        .await
    }

    /// Files the session in the channel: its record names the channel and
    /// the new folder, its Desk row goes, and its clones are the session's.
    async fn switch(&self, journal: &Journal) -> Result<(), Status> {
        let (store, journal, machine) = (self.starter.store.clone(), journal.clone(), self.machine.clone());
        off(move || {
            let id = journal.session.as_str();
            let mut record = store
                .session(id)
                .map_err(|error| status::store(error, &machine, "read the session"))?
                .ok_or_else(|| no_session(id))?;
            // The take-up leaves the machine empty, as a move to this
            // machine does; it stays the machine the session was on.
            if record.machine != journal.machine {
                record.machine = journal.machine.clone();
                store.put_session(&record).map_err(|error| status::store(error, &machine, "save the session"))?;
            }
            let old = store
                .session_detail(id)
                .map_err(|error| status::store(error, &machine, "read the session's start"))?;
            let detail = SessionDetail {
                session: id.to_owned(),
                repos: journal
                    .repos
                    .iter()
                    .map(|repo| SessionRepo {
                        owner: repo.owner.clone(),
                        name: repo.name.clone(),
                        branch: journal.branch.clone(),
                        path: journal.folder.join(&repo.dir),
                    })
                    .collect(),
                model: old.as_ref().and_then(|old| old.model.clone()),
                first_message: old.as_ref().and_then(|old| old.first_message.clone()),
                start: StartStep::Done,
                operation: None,
                revision: old.as_ref().map_or(1, |old| old.revision + 1),
            };
            record.channel = journal.channel.clone();
            record.folder = journal.folder.clone();
            store
                .desk_to_channel(&record, &detail)
                .map_err(|error| status::store(error, &machine, "file the session in the channel"))
        })
        .await
    }

    /// Adds `owner/name` to `channel` as the channel's own add does, and
    /// waits for it to end.
    async fn add_channel_repo(&self, caller: &Caller, channel: &str, owner: &str, name: &str) -> Result<(), Status> {
        let request = pb::AddRepoRequest {
            idempotency_key: None,
            channel: Some(pb::ChannelId { value: channel.to_owned() }),
            repo: Some(pb::RepoRef { owner: owner.to_owned(), name: name.to_owned() }),
        };
        let started = match self.channels.add_repo_to_channel(caller, request, true).await? {
            Gated::Done(operation) => operation,
            Gated::Ask(_) => {
                return Err(status::internal("a repo add the move made itself asked for approval", ""));
            }
        };
        let id = started.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let mut watch = self.operations.progress_of(caller, &id)?.watch();
        let ended = loop {
            let now = watch.borrow_and_update().clone();
            if now.state != pb::OperationState::Running as i32 {
                break now;
            }
            if watch.changed().await.is_err() {
                return Err(status::internal(format!("the operation adding {owner}/{name} to the channel was lost"), ""));
            }
        };
        if ended.state == pb::OperationState::Succeeded as i32 {
            return Ok(());
        }
        Err(match ended.error {
            Some(error) => status::failure(Code::FailedPrecondition, error),
            None => failed_precondition(
                format!("adding {owner}/{name} to channel {channel} didn't succeed"),
                "check the repository's name and that this machine can reach it, then move the session again",
            ),
        })
    }

    /// Takes back a move that never switched: the landing goes, the
    /// session's records are as they were, and an agent the move stopped
    /// resumes in the Desk folder.
    async fn take_back(&self, caller: &Caller, journal: &Journal, stopped: bool) -> Result<(), Status> {
        let (starter, at_journal) = (self.starter.clone(), journal.clone());
        off(move || {
            let (op, id, channel) = (&at_journal.operation, &at_journal.session, &at_journal.channel);
            starter
                .take_landing_back(op, id, channel, at_journal.phase >= Phase::Unpacking)
                .and_then(|()| starter.unstage(op, channel))
                .map_err(carry_failed)
        })
        .await?;
        if stopped && self.agent_state_is_stopped(&journal.session).await {
            self.resume_again(caller, &journal.session).await?;
        }
        remove_journal(&self.state, &journal.session);
        self.tell.session(&journal.session, pb::Change::Updated, Some(identity::actor(caller))).await;
        Ok(())
    }

    /// What the switch leaves to do: its requests withdrawn as moved, its
    /// grants ended (DK-18) and the change told. Safe to repeat.
    async fn finish_switch(&self, journal: &Journal, by: Option<&Caller>) -> Result<(), Status> {
        let id = journal.session.as_str();
        // The requests are withdrawn first, as moved, so none is left for
        // the end of the grants to withdraw as stopped.
        self.inbox.withdraw_pending_of(id, pb::WithdrawReason::SessionMoved).await?;
        self.end_grants(id).await?;
        self.tell.session(id, pb::Change::Updated, by.map(identity::actor)).await;
        Ok(())
    }

    /// Puts what the old place still holds of the session (its Desk folder,
    /// unless the user picked it, and its old conversation's files) in this
    /// machine's Bin, then drops
    /// the move's staging. Safe to repeat until the journal says released.
    async fn release(&self, journal: &mut Journal, by: Option<&Caller>) -> Result<(), Status> {
        if journal.phase < Phase::Released {
            self.bin_left_behind(journal, by).await?;
            journal.phase = Phase::Released;
            self.save_journal(journal).await?;
        }
        let (starter, op, channel) = (self.starter.clone(), journal.operation.clone(), journal.channel.clone());
        off(move || starter.unstage(&op, &channel).map_err(carry_failed)).await
    }

    /// Puts each thing the move left behind in the Bin; one already gone is
    /// skipped, so this is safe to repeat.
    async fn bin_left_behind(&self, journal: &Journal, by: Option<&Caller>) -> Result<(), Status> {
        let (starter, op, channel) = (self.starter.clone(), journal.operation.clone(), journal.channel.clone());
        let data = by.map_or_else(String::new, |caller| caller_data(&identity::actor(caller), &session_ref(&journal.session)));
        let bin = self.bin.clone();
        let (said, old_folder) = (journal.keep_old_folder, journal.old_folder.clone());
        off(move || {
            let desk = starter.layout.root().join(DESK_CHANNEL);
            let kept = keeps_old_folder(said, &desk, &old_folder).then_some(old_folder);
            let discards = starter.left_behind(&op, &channel).map_err(carry_failed)?;
            if discards.is_empty() {
                return Ok(());
            }
            let trash = bin.get()?;
            for discard in discards {
                // A folder the user picked is never put in the Bin (D461).
                if kept.as_ref().is_some_and(|folder| discard.path.starts_with(folder)) {
                    continue;
                }
                if fs::symlink_metadata(&discard.path).is_err() {
                    continue;
                }
                let item = NewItem { description: discard.what.clone(), caller_data: data.clone(), now_unix_ms: now_ms() };
                trash.put(&discard.path, item).map(drop).map_err(|error| {
                    status::internal(
                        format!(
                            "couldn't move {} to the Bin: {}; check that the Bin's folder can be written to, then restart humatad2 to try again",
                            discard.path.display(),
                            status::chain(&error)
                        ),
                        "",
                    )
                })?;
            }
            Ok(())
        })
        .await
    }

    /// Starts the session's agent again with its own operation, and waits
    /// for the resume to end: a failed move's Desk session runs as before,
    /// and a recovered one comes up.
    async fn resume_again(&self, caller: &Caller, id: &str) -> Result<(), Status> {
        let steps: Vec<(String, String)> = vec![
            ("launch".to_owned(), "Starting the agent again".to_owned()),
            ("verify".to_owned(), "Checking it took up its conversation".to_owned()),
        ];
        let Started::New(progress) =
            self.operations.start_steps(caller, pb::OperationKind::StartSession, session_ref(id), &steps, false, None)?
        else {
            return Err(status::internal("a resume without a key was taken for a repeat", ""));
        };
        let (store, key) = (self.starter.store.clone(), id.to_owned());
        let loaded = match blocking(move || Loaded::read(&store, &key)).await {
            Ok(Ok(Some(loaded))) => loaded,
            Ok(Ok(None)) => {
                let error = no_session(id);
                end_failed(progress, "launch", &error);
                return Err(error);
            }
            Ok(Err(error)) => {
                let error = status::store(error, &self.machine, "read the session to resume");
                end_failed(progress, "launch", &error);
                return Err(error);
            }
            Err(error) => {
                end_failed(progress, "launch", &error);
                return Err(error);
            }
        };
        run_resume(self.starter.clone(), self.tell.clone(), identity::actor(caller), self.tokens.clone(), loaded, progress)
            .await;
        Ok(())
    }

    /// Finishes or undoes the move `journal` names, with nothing else
    /// running.
    async fn recover_one(&self, mut journal: Journal) -> Result<Settled, Status> {
        let host = Caller::User(UserVia::Host);
        let id = journal.session.clone();
        let (store, key) = (self.starter.store.clone(), id.clone());
        let record = blocking(move || store.session(&key))
            .await?
            .map_err(|error| status::store(error, &self.machine, "read the session"))?;
        let Some(record) = record else {
            remove_journal(&self.state, &id);
            return Ok(Settled::Undone("the session was deleted".to_owned()));
        };
        if journal.rename {
            return self.recover_rename(&journal, &record).await;
        }
        if record.channel == DESK_CHANNEL {
            // Cut before the switch: it is a Desk session still.
            let stopped = self.desk_open(&id).await?;
            self.take_back(&host, &journal, stopped).await?;
            return Ok(Settled::Undone(format!(
                "it was cut short before the switch, so it is a Desk session still, in {}",
                journal.old_folder.display()
            )));
        }
        if record.channel != journal.channel {
            remove_journal(&self.state, &id);
            return Ok(Settled::Undone("the session is in another channel now".to_owned()));
        }
        self.finish_switch(&journal, None).await?;
        self.release(&mut journal, None).await?;
        if self.agent_state_is_stopped(&id).await {
            self.resume_again(&host, &id).await?;
        }
        remove_journal(&self.state, &id);
        Ok(Settled::Finished)
    }

    /// Whether the Desk session is open: it has a row that isn't closed or
    /// deleted.
    async fn desk_open(&self, session: &str) -> Result<bool, Status> {
        let (store, key) = (self.starter.store.clone(), session.to_owned());
        let row = blocking(move || store.desk(&key))
            .await?
            .map_err(|error| status::store(error, &self.machine, "look up the Desk session"))?;
        Ok(row.is_some_and(|row| row.closed_unix_ms.is_none() && row.deleted_unix_ms.is_none()))
    }

    async fn agent_state_is_stopped(&self, session: &str) -> bool {
        let (store, key) = (self.starter.store.clone(), session.to_owned());
        blocking(move || store.agent(&key))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
            .is_some_and(|agent| matches!(agent.state, AgentState::Stopped))
    }

    // -----------------------------------------------------------------------
    // A session on another machine (MV-17).

    /// Relays the move of `id` to `machine`, which has the session, and
    /// answers the host's operation, which follows the machine's.
    async fn move_there(
        &self,
        caller: &Caller,
        machine: &str,
        id: &str,
        request: pb::MoveDeskSessionToChannelRequest,
    ) -> Result<pb::Operation, Status> {
        crate::upgrade::gate::machine_ready(&self.migrations, &pb::MachineId { value: machine.to_owned() })?;
        if let Some(operation) = self.operations.existing(caller, request.idempotency_key.as_ref())? {
            return Ok(operation);
        }
        let held = self.locks.hold(id).await;
        let mut desk = self.desk_on(machine)?;
        let planning = pb::PlanDeskMoveRequest {
            session: request.session.clone(),
            channel: request.channel.clone(),
            branch: request.branch.clone(),
        };
        // The plan says where the session lands; the machine refuses a move
        // whose token doesn't match it.
        let plan = desk
            .plan_desk_move(assigned(planning, None).map_err(Status::from)?)
            .await
            .map(Response::into_inner)
            .map_err(|source| Status::from(call_failed(machine, "plan the Desk session's move", source)))?;
        let key = request.idempotency_key.clone();
        let started = desk
            .move_desk_session_to_channel(assigned(request, None).map_err(Status::from)?)
            .await
            .map(Response::into_inner)
            .map_err(|source| Status::from(call_failed(machine, "move the Desk session", source)))?;
        let there = started.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let operations = self
            .remote
            .as_ref()
            .ok_or_else(|| {
                failed_precondition(
                    format!("this machine has no way to reach {machine}, so the move there can't be followed"),
                    "move the Desk session from the host",
                )
            })?
            .relayed(machine)
            .map_err(Status::from)?
            .operation();
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::MoveSession,
            session_ref(id),
            &move_steps(),
            false,
            key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(operation),
        };
        let operation = progress.operation();
        let shown = operation.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let (svc, who, machine, id) = (self.clone(), caller.clone(), machine.to_owned(), id.to_owned());
        tokio::spawn(async move {
            let _held = held;
            let _moving = Moving::begin(&id, &machine, &shown);
            svc.follow_there(&who, Following { machine, id, plan, there }, operations, progress).await;
        });
        Ok(operation)
    }

    /// Follows the machine's move `there` into `progress`, then files the
    /// session in the channel in the host's view once the machine has.
    async fn follow_there(
        &self,
        by: &Caller,
        following: Following,
        mut operations: pb::operation_service_client::OperationServiceClient<Channel>,
        progress: Progress,
    ) {
        let Following { machine, id, plan, there } = following;
        let ended = match watch_there(&mut operations, &there, &progress).await {
            Some(ended) => ended,
            None => {
                progress.fail(pb::Error {
                    message: format!(
                        "the link to {machine} broke while Desk session {id} was moving there, so how the move ended isn't known here yet"
                    ),
                    fix: "wait for the machine to come back: the session shows where it is once it reports, and the move finishes or is undone there on its own".to_owned(),
                    ..pb::Error::default()
                });
                return;
            }
        };
        if ended.state != pb::OperationState::Succeeded as i32 {
            progress.fail(ended.error.unwrap_or_else(|| pb::Error {
                message: format!("moving Desk session {id} on {machine} didn't succeed"),
                fix: "look at the session on that machine, then move it again".to_owned(),
                ..pb::Error::default()
            }));
            return;
        }
        match self.switch_here(&id, &plan, by).await {
            Ok(()) => progress.succeed(Some(pb::operation::Result::Session(pb::SessionId { value: id.to_owned() }))),
            Err(error) => end_failed(progress, "switch", &error),
        }
    }

    /// The host's own record of a session `plan` moved on its machine.
    async fn switch_here(&self, id: &str, plan: &pb::DeskMovePlan, by: &Caller) -> Result<(), Status> {
        let (store, key, plan, machine) = (self.starter.store.clone(), id.to_owned(), plan.clone(), self.machine.clone());
        off(move || {
            let mut record = store
                .session(&key)
                .map_err(|error| status::store(error, &machine, "read the session"))?
                .ok_or_else(|| no_session(&key))?;
            let old = store
                .session_detail(&key)
                .map_err(|error| status::store(error, &machine, "read the session's start"))?;
            let folder = PathBuf::from(&plan.folder);
            let detail = SessionDetail {
                session: key.clone(),
                repos: plan
                    .repos
                    .iter()
                    .filter_map(|repo| repo.repo.as_ref())
                    .map(|repo| SessionRepo {
                        owner: repo.owner.clone(),
                        name: repo.name.clone(),
                        branch: plan.branch.clone(),
                        path: folder.join(&repo.name),
                    })
                    .collect(),
                model: old.as_ref().and_then(|old| old.model.clone()),
                first_message: old.as_ref().and_then(|old| old.first_message.clone()),
                start: StartStep::Done,
                operation: None,
                revision: old.as_ref().map_or(1, |old| old.revision + 1),
            };
            record.channel = plan.channel.as_ref().map(|channel| channel.value.clone()).unwrap_or_default();
            record.folder = folder;
            store
                .desk_to_channel(&record, &detail)
                .map_err(|error| status::store(error, &machine, "file the session in the channel"))
        })
        .await?;
        self.inbox.withdraw_pending_of(id, pb::WithdrawReason::SessionMoved).await?;
        self.end_grants(id).await?;
        self.tell.session(id, pb::Change::Updated, Some(identity::actor(by))).await;
        Ok(())
    }
}

/// Mirrors the steps of the machine's operation `id` into `progress` until
/// it ends. `None` when the link broke first.
async fn watch_there(
    operations: &mut pb::operation_service_client::OperationServiceClient<Channel>,
    id: &str,
    progress: &Progress,
) -> Option<pb::Operation> {
    let watching = operations
        .watch_operation(Request::new(pb::WatchOperationRequest {
            id: Some(pb::OperationId { value: id.to_owned() }),
            from_version: 0,
        }))
        .await;
    let mut stream = watching.ok()?.into_inner();
    let mut shown: HashMap<String, i32> = HashMap::new();
    while let Ok(Some(operation)) = stream.message().await {
        for step in &operation.steps {
            if shown.get(&step.key) == Some(&step.state) {
                continue;
            }
            shown.insert(step.key.clone(), step.state);
            let tail = Some(step.output_tail.clone()).filter(|tail| !tail.is_empty());
            progress.set_step(&step.key, step.state(), tail);
        }
        if operation.state != pb::OperationState::Running as i32 {
            return Some(operation);
        }
    }
    None
}

/// A move relayed to the machine that has the session.
struct Following {
    machine: String,
    id: String,
    /// The machine's plan, which says where the session lands.
    plan: pb::DeskMovePlan,
    /// The machine's operation.
    there: String,
}

/// How a recovered move ended.
enum Settled {
    Finished,
    Undone(String),
}

/// Finishes or undoes the moves a restart cut short, once `started` has
/// brought the sessions back (DK-17). Each shows as Moving from now until
/// it is settled.
pub(super) fn recover_after(svc: DeskSvc, started: Arc<crate::restart::Started>) {
    // Read now, before anything is served, so a move a call starts while
    // this waits is left to that call.
    let found = read_journals(&svc.state);
    if found.is_empty() {
        return;
    }
    let mut journals = Vec::new();
    for read in found {
        match read {
            Ok(journal) => {
                let moving = Moving::begin(&journal.session, &svc.machine, &journal.operation);
                journals.push((journal, moving));
            }
            Err(error) => crate::log::say!("humatad2: {}", status::chain(&error)),
        }
    }
    tokio::spawn(async move {
        started.wait().await;
        for (journal, moving) in journals {
            let id = journal.session.clone();
            let operations = started.operations();
            operations.record(
                &id,
                "desk_move",
                "cut short",
                OperationState::Pending { waits_for: "the start to finish".to_owned() },
            );
            let held = svc.locks.hold(&id).await;
            let state = match svc.recover_one(journal).await {
                Ok(Settled::Finished) => OperationState::Finished,
                Ok(Settled::Undone(reason)) => OperationState::Undone { reason },
                Err(error) => OperationState::Stuck {
                    reason: format!(
                        "{}; the record of the move was kept, so the next start tries again",
                        error.message()
                    ),
                },
            };
            drop(held);
            drop(moving);
            svc.tell.session(&id, pb::Change::Updated, None).await;
            operations.record(&id, "desk_move", "recovered", state);
        }
    });
}

// ---------------------------------------------------------------------------
// D801 E: the rename.

/// Whether `folder` is a folder on the filesystem the landing is made on
/// (the nearest folder that exists above it), so `rename(2)` works under
/// what runs in it. Any doubt answers no: the copy is always safe.
fn same_device(folder: &Path, landing: &Path) -> bool {
    let Some(parent) = landing
        .parent()
        .and_then(|parent| parent.ancestors().find(|above| fs::metadata(above).is_ok_and(|meta| meta.is_dir())))
    else {
        return false;
    };
    match (fs::symlink_metadata(folder), fs::metadata(parent)) {
        (Ok(folder), Ok(parent)) => folder.is_dir() && folder.dev() == parent.dev(),
        _ => false,
    }
}

/// The name the old folder's link is recorded under.
fn alias_key(session: &str) -> String {
    format!("desk-{session}")
}

/// Why a rename move couldn't be prepared, done or undone.
#[derive(Debug, thiserror::Error)]
enum RenameError {
    #[error(
        "couldn't make {} to move Desk session {session} into: {source}; check that the channel's folder can be written to and has room, then move the session again",
        path.display()
    )]
    Landing {
        session: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "couldn't read Desk session {session}'s agent and shells to move it: {source}; check Humata's state folder, then move the session again"
    )]
    Records {
        session: String,
        #[source]
        source: StoreError,
    },
    #[error(
        "couldn't keep the note that carries Desk session {session}'s conversation to its new folder: {source}; free the disk space or fix the permissions of Humata's holders folder, then move the session again"
    )]
    Rekey {
        session: String,
        #[source]
        source: RekeyError,
    },
    #[error("couldn't keep the old folder's name as a link to the new one: {source}")]
    Alias {
        #[source]
        source: alias::AliasError,
    },
    #[error(
        "couldn't rename {} to {}: {source}; check that both folders can be written to, then move the session again. Nothing running was stopped",
        from.display(),
        to.display()
    )]
    Move {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn rename_status(error: &RenameError) -> Status {
    failed(Code::Internal, error, "fix what the message names, then move the session again", None)
}

/// Writes the note each of the session's agents follows at its next resume,
/// and answers the holders that started under the old folder and still run.
fn prepare_rename(starter: &humata2_sessions::Starter, journal: &Journal) -> Result<Vec<alias::Holder>, RenameError> {
    let store = starter.store.as_ref();
    let records = |source| RenameError::Records { session: journal.session.clone(), source };
    let mut holders = Vec::new();
    if let Some(agent) = store.agent(&journal.session).map_err(records)? {
        let launch = &agent.launch;
        let note = RekeyNote {
            path: journal.old_folder.clone(),
            target: journal.folder.clone(),
            started: AgentStart {
                folder: launch.folder.clone(),
                argv: launch.argv.clone(),
                resume_argv: launch.resume_argv.clone(),
                conversation_file: launch.conversation_file.clone(),
            },
            known: journal.repos.iter().map(|repo| journal.old_folder.join(&repo.dir)).collect(),
            // Codex's and Copilot's conversations name their folder inside them.
            follows: crate::channel_service::rename_folder::session_follows(
                store,
                &starter.agents,
                &starter.home,
                &starter.layout.root(),
                &journal.session,
                &journal.old_folder,
                &journal.folder,
            )
            .map_err(records)?,
        };
        write_note(&launch.holder_socket, &note)
            .map_err(|source| RenameError::Rekey { session: journal.session.clone(), source })?;
        if let AgentState::Running { holder_pid } = agent.state {
            holders.push(alias::Holder { socket: launch.holder_socket.clone(), pid: holder_pid });
        }
    }
    for tab in store.tabs().map_err(records)? {
        if tab.kind == TabKind::Shell && tab.holder_pid != 0 && tab.session.as_deref() == Some(journal.session.as_str()) {
            holders.push(alias::Holder { socket: tab.holder_socket, pid: tab.holder_pid });
        }
    }
    holders.retain(alias::alive);
    holders.dedup();
    Ok(holders)
}

/// Renames the Desk folder to the landing, leaving a link at the old name
/// while `holders` run.
fn rename_in_place(root: &Path, journal: &Journal, holders: &[alias::Holder]) -> Result<(), RenameError> {
    let (old, new) = (&journal.old_folder, &journal.folder);
    if let Some(parent) = new.parent() {
        humata2_durable::create_dir_all(parent, 0o700).map_err(|source| RenameError::Landing {
            session: journal.session.clone(),
            path: parent.to_owned(),
            source,
        })?;
    }
    let key = alias_key(&journal.session);
    let before = alias::begin(root, &key, old, new, holders).map_err(|source| RenameError::Alias { source })?;
    let moved = alias::rename_folder(old, new, !holders.is_empty())
        .and_then(|()| {
            for folder in [old.parent(), new.parent()].into_iter().flatten() {
                humata2_durable::sync_dir(folder)?;
            }
            Ok(())
        })
        .map_err(|source| RenameError::Move { from: old.clone(), to: new.clone(), source });
    if moved.is_err() {
        alias::undo(root, &key, &before);
    }
    moved
}

/// Puts the folder back where the Desk had it and drops the notes: what a
/// failed or cut rename move that never switched leaves. Safe to repeat.
fn undo_rename(store: &Store, root: &Path, journal: &Journal) -> Result<(), RenameError> {
    let (old, new) = (&journal.old_folder, &journal.folder);
    let key = alias_key(&journal.session);
    // The link goes first, so the old name is free for the folder.
    alias::release(root, &key, old).map_err(|source| RenameError::Alias { source })?;
    let gone = fs::symlink_metadata(old).is_err();
    if gone && fs::symlink_metadata(new).is_ok_and(|meta| meta.is_dir()) {
        alias::rename_folder(new, old, false).map_err(|source| RenameError::Move {
            from: new.clone(),
            to: old.clone(),
            source,
        })?;
    }
    if let Some(agent) = store
        .agent(&journal.session)
        .map_err(|source| RenameError::Records { session: journal.session.clone(), source })?
    {
        remove_note(&agent.launch.holder_socket);
    }
    Ok(())
}

impl DeskSvc {
    /// Runs an accepted rename move, holding the session's lock and showing
    /// it as Moving until it ends. Nothing it does stops a terminal.
    async fn run_rename(&self, caller: Caller, _held: OwnedMutexGuard<()>, mut job: Job, progress: Progress) {
        let _moving = Moving::begin(&job.journal.session, &self.machine, &job.journal.operation);
        if let Err(failure) = self.add_repos(&caller, &job, &progress).await {
            end_failed(progress, failure.step, &failure.status);
            return;
        }
        for key in ["copy", "stop", "release", "launch", "verify"] {
            progress.set_step(key, pb::StepState::Skipped, None);
        }
        // The notes come first, so a restart that finds the folder moved
        // finds them too; then the journal says the rename is next.
        progress.set_step("pack", pb::StepState::Running, None);
        let (starter, at_journal) = (self.starter.clone(), job.journal.clone());
        let prepared = off(move || prepare_rename(&starter, &at_journal).map_err(|error| rename_status(&error))).await;
        let holders = match prepared {
            Ok(holders) => holders,
            Err(error) => {
                self.undo_and_fail(&caller, &job.journal, progress, "pack", &error).await;
                return;
            }
        };
        job.journal.phase = Phase::Renaming;
        if let Err(error) = self.save_journal(&job.journal).await {
            self.undo_and_fail(&caller, &job.journal, progress, "pack", &error).await;
            return;
        }
        progress.set_step("pack", pb::StepState::Done, None);

        progress.set_step("unpack", pb::StepState::Running, None);
        let (root, at_journal) = (self.starter.layout.root(), job.journal.clone());
        let renamed =
            off(move || rename_in_place(&root, &at_journal, &holders).map_err(|error| rename_status(&error))).await;
        if let Err(error) = renamed {
            self.undo_and_fail(&caller, &job.journal, progress, "unpack", &error).await;
            return;
        }
        // A kill here leaves the folder moved and the records not: the
        // restart renames it back.
        crate::fault::point("move-renamed");
        progress.set_step("unpack", pb::StepState::Done, None);

        progress.set_step("branch", pb::StepState::Running, None);
        if let Err(error) = self.repoint(&job.journal, &job.planned.channel.folder).await {
            self.undo_and_fail(&caller, &job.journal, progress, "branch", &error).await;
            return;
        }
        progress.set_step("branch", pb::StepState::Done, None);

        progress.set_step("switch", pb::StepState::Running, None);
        if let Err(error) = self.switch(&job.journal).await {
            self.undo_and_fail(&caller, &job.journal, progress, "switch", &error).await;
            return;
        }
        job.journal.phase = Phase::Switched;
        if let Err(error) = self.save_journal(&job.journal).await {
            crate::log::say!("humatad2: {}", error.message());
        }
        progress.set_step("switch", pb::StepState::Done, None);
        crate::fault::point("move-switched");
        if let Err(error) = self.finish_switch(&job.journal, Some(&caller)).await {
            end_failed(progress, "switch", &error);
            return;
        }
        let id = job.journal.session.clone();
        remove_journal(&self.state, &id);
        self.channels.watch_aliases();
        self.tell.session(&id, pb::Change::Updated, Some(identity::actor(&caller))).await;
        progress.succeed(Some(pb::operation::Result::Session(pb::SessionId { value: id })));
    }

    /// Ends a rename move that failed before the switch: the folder goes
    /// back, the notes are dropped, and the session is a Desk session as it
    /// was, still running.
    async fn undo_and_fail(&self, caller: &Caller, journal: &Journal, progress: Progress, step: &'static str, status: &Status) {
        if let Err(error) = self.undo_rename(journal).await {
            crate::log::say!(
                "humatad2: couldn't take back the move of Desk session {}: {}; restart humatad2 to try again",
                journal.session,
                error.message()
            );
        } else {
            remove_journal(&self.state, &journal.session);
        }
        self.tell.session(&journal.session, pb::Change::Updated, Some(identity::actor(caller))).await;
        end_failed(progress, step, status);
    }

    async fn undo_rename(&self, journal: &Journal) -> Result<(), Status> {
        let (store, root, journal) = (self.starter.store.clone(), self.starter.layout.root(), journal.clone());
        off(move || undo_rename(&store, &root, &journal).map_err(|error| rename_status(&error))).await
    }

    /// Finishes or undoes a rename move a restart cut short. Before the
    /// switch the folder is renamed back; after it, only what the switch
    /// leaves to do is done. A holder that died meanwhile is resumed by the
    /// revive, whose note carries its conversation.
    async fn recover_rename(&self, journal: &Journal, record: &SessionRecord) -> Result<Settled, Status> {
        let id = journal.session.as_str();
        if record.channel == DESK_CHANNEL {
            self.undo_rename(journal).await?;
            remove_journal(&self.state, id);
            return Ok(Settled::Undone(format!(
                "it was cut short before the switch, so the folder is back in {} and it is a Desk session still",
                journal.old_folder.display()
            )));
        }
        if record.channel != journal.channel {
            remove_journal(&self.state, id);
            return Ok(Settled::Undone("the session is in another channel now".to_owned()));
        }
        self.finish_switch(journal, None).await?;
        remove_journal(&self.state, id);
        self.channels.watch_aliases();
        Ok(Settled::Finished)
    }
}

#[cfg(test)]
mod journal_tests {
    use super::keeps_old_folder;
    use std::fs;

    #[test]
    fn a_journal_from_before_picked_folders_were_kept_never_bins_a_picked_folder() {
        let temp = tempfile::tempdir().unwrap();
        let desk = temp.path().join("channels").join("desk");
        let scratch = desk.join("notes");
        let picked = temp.path().join("Documents").join("plans");
        fs::create_dir_all(&scratch).unwrap();
        fs::create_dir_all(&picked).unwrap();
        // An old journal doesn't say: the picked folder stays, a scratch one goes.
        assert!(keeps_old_folder(None, &desk, &picked));
        assert!(!keeps_old_folder(None, &desk, &scratch));
        // A folder that is gone isn't Humata's to bin either way.
        assert!(keeps_old_folder(None, &desk, &desk.join("gone")));
        // A journal that says wins.
        assert!(keeps_old_folder(Some(true), &desk, &scratch));
        assert!(!keeps_old_folder(Some(false), &desk, &scratch));
    }

    #[test]
    fn an_old_journal_reads_as_not_saying() {
        #[derive(serde::Deserialize)]
        struct Shape {
            #[serde(default)]
            keep_old_folder: Option<bool>,
        }
        let old: Shape = serde_json::from_str("{}").unwrap();
        assert_eq!(old.keep_old_folder, None);
    }
}
