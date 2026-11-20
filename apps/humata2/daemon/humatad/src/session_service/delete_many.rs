//! PlanDeleteSessions and DeleteSessions (MASSDELETE, LD-5 to LD-7, NV-27):
//! one plan for many sessions, one token, one delete. The one-session
//! PlanDelete and DeleteSession are this code with a one-id scope.
//!
//! The plan resolves the scope from the host's store, pulls in the
//! context-only forks that work in a chosen session's folder (D187), lists
//! the code forks that stay, and per session the unpushed work and the
//! terminals that end. The delete takes the token, lists again, and if
//! anything differs answers `changed` with a fresh plan and touches nothing.
//!
//! Each session's steps can run again, so a restart finishes a delete it
//! cut short: a note under `<root>/.deleting-sessions/<id>` is written
//! first and removed last. The session's records are removed after its
//! folder is in the trash.
//!
//! Sessions on other machines (T5): the host asks each online machine for
//! the plan of its own sessions over the peer link and keeps that machine's
//! token. A delete first sends every machine `check_only` (it re-lists and
//! touches nothing), and only when all pass does any machine delete. An
//! offline machine's sessions stay in `left_alone`, untouched. A machine
//! that fails after the checks leaves its sessions, and the error names
//! which sessions went and which stayed.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use humata2_access::{AccessRequest, AnswerAt, Call, Capability, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::DESK_CHANNEL;
use humata2_store::{AgentState, SIDE_FILE, Store, StoreError, TabKind};
use humata2_transfers::NewItem;
use pb::invalid_field::Problem;
use serde::{Deserialize, Serialize};
use tonic::{Code, Status};

use super::archive_remote::{follow_to_end, machine_failed};
use super::{Gated, Loaded, SessionSvc};
use crate::host_relay::{Relayed, assigned};
use crate::channel_service::delete::{
    DeleteError, ListError, RepoWorkListed, SessionListed, pid_alive, repos_listed, stale,
};
use crate::channel_service::{blocking, error_of};
use crate::guard;
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::status;
use crate::trash_service::{Bin, caller_data, now_ms, with_channel};

/// Where unfinished deletes are written, in the Humata folder.
const INTENTS: &str = ".deleting-sessions";

/// How long a plan's token stays good.
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// The plans not yet used: token to what it was made for.
static PLANS: LazyLock<Mutex<HashMap<String, (pb::DeleteScope, Listing, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn make_token(scope: &pb::DeleteScope, listing: &Listing) -> String {
    let token = format!("pl_{}", uuid::Uuid::new_v4().simple());
    let mut plans = PLANS.lock().unwrap_or_else(PoisonError::into_inner);
    plans.retain(|_, (_, _, made)| made.elapsed() < PLAN_LIFETIME);
    plans.insert(token.clone(), (scope.clone(), listing.clone(), Instant::now()));
    token
}

fn peek_token(token: &str) -> Option<(pb::DeleteScope, Listing)> {
    let plans = PLANS.lock().unwrap_or_else(PoisonError::into_inner);
    plans
        .get(token)
        .filter(|(_, _, made)| made.elapsed() < PLAN_LIFETIME)
        .map(|(scope, listing, _)| (scope.clone(), listing.clone()))
}

fn take_token(token: &str) {
    PLANS.lock().unwrap_or_else(PoisonError::into_inner).remove(token);
}

/// One session the delete covers.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    title: String,
    channel: String,
    machine: String,
    archived: bool,
    exited: bool,
    listed: SessionListed,
    /// Each terminal that ends: its name and the API's state. Only the
    /// names count as a change (a terminal's state moves on its own).
    terminals: Vec<(String, i32)>,
    goes_with: Option<String>,
}

impl Entry {
    fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.terminals.iter().map(|(name, _)| name.as_str()).collect();
        names.sort_unstable();
        names
    }
}

/// What a plan listed, and what the delete lists again to compare.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Listing {
    sessions: BTreeMap<String, Entry>,
    /// Sessions on another machine, by machine: id and title.
    left_alone: BTreeMap<String, Vec<(String, String)>>,
    /// Code forks of a deleted session: id and title.
    staying: BTreeMap<String, String>,
    desk_left_out: u32,
    all: bool,
    /// The machines whose sessions the host relays: machine to the token of
    /// the plan that machine made. Their sessions are in `sessions`, with
    /// `machine` set to that machine.
    remote: BTreeMap<String, String>,
}

/// What a machine's plan says about one of its sessions, as an entry.
fn entry_of(session: &pb::SessionToDelete, machine: &str) -> Option<(String, Entry)> {
    let id = session.session.as_ref()?.value.clone();
    let repos = session
        .unpushed
        .iter()
        .map(|repo| RepoWorkListed {
            owner: repo.repo.as_ref().map(|repo| repo.owner.clone()).unwrap_or_default(),
            name: repo.repo.as_ref().map(|repo| repo.name.clone()).unwrap_or_default(),
            branch: repo.branch.clone(),
            dirty_files: repo.dirty_files,
            unpushed_commits: repo.unpushed_commits,
            never_pushed: repo.never_pushed,
        })
        .collect();
    Some((
        id,
        Entry {
            title: session.title.clone(),
            channel: session.channel.as_ref().map(|channel| channel.value.clone()).unwrap_or_default(),
            machine: machine.to_owned(),
            archived: session.archived,
            exited: session.exited,
            listed: SessionListed { title: session.title.clone(), repos },
            terminals: session.terminals.iter().map(|ending| (ending.name.clone(), ending.state)).collect(),
            goes_with: session.goes_with.as_ref().map(|id| id.value.clone()),
        },
    ))
}

impl Listing {
    /// Takes the machines `before` relayed, and their sessions, from it: a
    /// fresh listing of this machine knows them only as left alone.
    fn adopt_remote(&mut self, before: &Self) {
        for (machine, token) in &before.remote {
            self.left_alone.remove(machine);
            self.remote.insert(machine.clone(), token.clone());
        }
        for (id, entry) in &before.sessions {
            if before.remote.contains_key(&entry.machine) {
                self.sessions.insert(id.clone(), entry.clone());
            }
        }
    }

    /// Puts what `machine`'s plan lists in place of what this listing had
    /// of it.
    fn take_machine_plan(&mut self, machine: &str, plan: &pb::DeleteSessionsPlan) {
        self.sessions.retain(|_, entry| entry.machine != machine);
        for session in &plan.sessions {
            if let Some((id, entry)) = entry_of(session, machine) {
                self.sessions.insert(id, entry);
            }
        }
        for work in &plan.forks_staying {
            if let Some(id) = &work.session {
                self.staying.insert(id.value.clone(), work.title.clone());
            }
        }
        if let Some(token) = &plan.token {
            self.remote.insert(machine.to_owned(), token.value.clone());
        }
    }
}

impl Listing {
    /// What differs from `before`, in words for the person who planned.
    fn changes_since(&self, before: &Self) -> Vec<String> {
        let mut changes = Vec::new();
        for (id, now) in &self.sessions {
            let Some(then) = before.sessions.get(id) else {
                changes.push(format!("\"{}\" joined the delete (it started or came back)", now.title));
                continue;
            };
            if then.listed != now.listed {
                changes.push(format!(
                    "\"{}\" now has {} uncommitted files and {} unpushed commits (it had {} and {})",
                    now.title,
                    now.listed.dirty(),
                    now.listed.unpushed(),
                    then.listed.dirty(),
                    then.listed.unpushed()
                ));
            }
            for name in now.names() {
                if !then.names().contains(&name) {
                    changes.push(format!("{}: a new terminal started ({name}).", now.title));
                }
            }
            for name in then.names() {
                if !now.names().contains(&name) {
                    changes.push(format!("{}: a terminal ended ({name}).", now.title));
                }
            }
            if then.archived != now.archived {
                changes.push(format!(
                    "\"{}\" was {}",
                    now.title,
                    if now.archived { "archived" } else { "restored" }
                ));
            }
        }
        for (id, then) in &before.sessions {
            if !self.sessions.contains_key(id) {
                changes.push(format!("\"{}\" is no longer there", then.title));
            }
        }
        for (id, title) in &self.staying {
            if !before.staying.contains_key(id) {
                changes.push(format!("\"{title}\" was forked from a session in the delete and stays"));
            }
        }
        changes
    }
}

/// What resolving a scope can fail on.
#[derive(Debug, thiserror::Error)]
enum ResolveError {
    #[error("the store could not be read")]
    Store(#[from] StoreError),
    #[error("git could not be read")]
    Git(#[from] ListError),
    #[error("no session {session} is on this machine")]
    NoSession { session: String },
    #[error("no channel {channel}")]
    NoChannel { channel: String },
    #[error("session {session} is the one asking, and a session can't delete itself")]
    OwnSession { session: String },
}

fn resolve_status(error: ResolveError, machine: &str) -> Status {
    match error {
        ResolveError::Store(error) => status::store(error, machine, "list the sessions to delete"),
        ResolveError::Git(error) => status::internal(
            format!("couldn't read the work in a session's clone: {}", status::chain(&error)),
            "fix the cause the message names, then plan the delete again",
        ),
        ResolveError::NoSession { session } => status::not_found(
            pb::EntityKind::Session,
            &session,
            format!("no session {session} is on this machine"),
            "list the sessions to see their ids; it may have been removed or be on another machine",
        ),
        ResolveError::NoChannel { channel } => status::not_found(
            pb::EntityKind::Channel,
            &channel,
            format!("no channel {channel}"),
            "list the channels to see their ids",
        ),
        ResolveError::OwnSession { session } => status::invalid_field(
            "scope",
            Problem::NotAllowedHere,
            format!("session {session} is the one asking, and a session can't delete itself"),
            "ask the user to delete it, or leave it out of the scope",
        ),
    }
}

fn is_desk(store: &Store, session: &humata2_store::SessionRecord) -> Result<bool, StoreError> {
    Ok(session.channel == DESK_CHANNEL || store.channel_desk(&session.id)?.is_some())
}

/// The state a terminal shows as in a plan.
fn terminal_state(status: pb::SessionStatus) -> i32 {
    match status {
        pb::SessionStatus::Working => pb::terminal_ending::State::Working as i32,
        pb::SessionStatus::NeedsInput => pb::terminal_ending::State::WaitingForYou as i32,
        _ => pb::terminal_ending::State::Idle as i32,
    }
}

/// The running terminals of a session: its agent's holder and its shells.
fn terminals_of(store: &Store, session: &str, here: &str) -> Result<(Vec<(String, i32)>, bool), StoreError> {
    let mut terminals = Vec::new();
    let mut exited = false;
    if let Some(agent) = store.agent(session)? {
        match agent.state {
            AgentState::Running { holder_pid } if pid_alive(holder_pid) => {
                let state = match Loaded::read(store, session)? {
                    Some(loaded) => terminal_state(
                        super::session_pb(
                            &loaded.record,
                            loaded.detail.as_ref(),
                            loaded.agent.as_ref(),
                            loaded.summary_source,
                            here,
                        )
                        .status(),
                    ),
                    None => pb::terminal_ending::State::Idle as i32,
                };
                terminals.push((agent.launch.agent.clone(), state));
            }
            AgentState::Exited { .. } => exited = true,
            _ => {}
        }
    }
    for tab in store.tabs()? {
        if tab.kind == TabKind::Shell && tab.session.as_deref() == Some(session) && pid_alive(tab.holder_pid) {
            let name = if tab.title.is_empty() { "shell".to_owned() } else { tab.title };
            terminals.push((name, pb::terminal_ending::State::Idle as i32));
        }
    }
    let exited = exited && terminals.is_empty();
    Ok((terminals, exited))
}

/// Resolves `scope` on this machine from the store. `own` is the asking
/// agent's session: it never takes part. Reads only.
fn resolve(
    store: &Store,
    here: &str,
    scope: &pb::DeleteScope,
    own: Option<&str>,
) -> Result<Listing, ResolveError> {
    use pb::delete_scope::Scope;
    let all = store.sessions()?;
    let mut listing = Listing::default();
    let mut chosen: BTreeSet<String> = BTreeSet::new();
    let mut take = |listing: &mut Listing, session: &humata2_store::SessionRecord| -> Result<(), StoreError> {
        if is_desk(store, session)? {
            listing.desk_left_out += 1;
        } else if !session.machine.is_empty() && session.machine != here {
            listing
                .left_alone
                .entry(session.machine.clone())
                .or_default()
                .push((session.id.clone(), session.title.clone()));
        } else if own != Some(session.id.as_str()) {
            chosen.insert(session.id.clone());
        }
        Ok(())
    };
    match &scope.scope {
        Some(Scope::Sessions(picked)) => {
            for id in picked.ids.iter().map(|id| id.value.as_str()) {
                if own == Some(id) {
                    return Err(ResolveError::OwnSession { session: id.to_owned() });
                }
                let record = all
                    .iter()
                    .find(|session| session.id == id)
                    .ok_or_else(|| ResolveError::NoSession { session: id.to_owned() })?;
                take(&mut listing, record)?;
            }
        }
        Some(Scope::Channel(channel)) => {
            if store.channel(&channel.value)?.is_none() {
                return Err(ResolveError::NoChannel { channel: channel.value.clone() });
            }
            for session in all.iter().filter(|session| session.channel == channel.value) {
                take(&mut listing, session)?;
            }
        }
        Some(Scope::Machine(machine)) => {
            for session in all.iter().filter(|session| {
                session.machine == machine.value || (session.machine.is_empty() && machine.value == here)
            }) {
                take(&mut listing, session)?;
            }
        }
        Some(Scope::All(true)) => {
            listing.all = true;
            for session in &all {
                take(&mut listing, session)?;
            }
        }
        Some(Scope::All(false)) | None => {}
    }
    // D187: a context-only fork works in its parent's folder, so it goes
    // with the parent. Other forks of a deleted session stay.
    let origins = store.fork_origins()?;
    let mut goes_with: BTreeMap<String, String> = BTreeMap::new();
    loop {
        let mut added = false;
        for origin in &origins {
            if chosen.contains(&origin.session_id) || !chosen.contains(&origin.parent) {
                continue;
            }
            if origin.kind != pb::ForkKind::ContextOnly as i32 {
                continue;
            }
            let (Some(fork), Some(parent)) = (
                all.iter().find(|session| session.id == origin.session_id),
                all.iter().find(|session| session.id == origin.parent),
            ) else {
                continue;
            };
            if fork.folder == parent.folder && !is_desk(store, fork)? && own != Some(fork.id.as_str()) {
                chosen.insert(fork.id.clone());
                goes_with.insert(fork.id.clone(), parent.id.clone());
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    for origin in &origins {
        if chosen.contains(&origin.parent)
            && !chosen.contains(&origin.session_id)
            && let Some(fork) = all.iter().find(|session| session.id == origin.session_id)
        {
            listing.staying.insert(fork.id.clone(), fork.title.clone());
        }
    }
    for id in &chosen {
        let Some(record) = all.iter().find(|session| &session.id == id) else {
            continue;
        };
        let archived = store.archive(id)?.is_some();
        let (terminals, exited) = terminals_of(store, id, here)?;
        listing.sessions.insert(
            id.clone(),
            Entry {
                title: record.title.clone(),
                channel: record.channel.clone(),
                machine: if record.machine.is_empty() { here.to_owned() } else { record.machine.clone() },
                archived,
                exited: exited && !archived,
                listed: SessionListed {
                    title: record.title.clone(),
                    repos: repos_listed(store, id)?,
                },
                terminals,
                goes_with: goes_with.get(id).cloned(),
            },
        );
    }
    Ok(listing)
}

fn unpushed_pb(repos: &[RepoWorkListed]) -> Vec<pb::UnpushedRepo> {
    repos
        .iter()
        .filter(|repo| repo.dirty_files > 0 || repo.unpushed_commits > 0)
        .map(|repo| pb::UnpushedRepo {
            repo: Some(pb::RepoRef { owner: repo.owner.clone(), name: repo.name.clone() }),
            branch: repo.branch.clone(),
            dirty_files: repo.dirty_files,
            unpushed_commits: repo.unpushed_commits,
            never_pushed: repo.never_pushed,
        })
        .collect()
}

fn plan_pb(listing: &Listing, token: &str) -> pb::DeleteSessionsPlan {
    let work = |id: &str, title: &str| pb::SessionWork {
        session: Some(pb::SessionId { value: id.to_owned() }),
        title: title.to_owned(),
        dirty_files: 0,
        unpushed_commits: 0,
    };
    pb::DeleteSessionsPlan {
        sessions: listing
            .sessions
            .iter()
            .map(|(id, entry)| pb::SessionToDelete {
                session: Some(pb::SessionId { value: id.clone() }),
                title: entry.title.clone(),
                channel: Some(pb::ChannelId { value: entry.channel.clone() }),
                machine: Some(pb::MachineId { value: entry.machine.clone() }),
                archived: entry.archived,
                unpushed: unpushed_pb(&entry.listed.repos),
                running_terminals: u32::try_from(entry.terminals.len()).unwrap_or(u32::MAX),
                goes_with: entry.goes_with.as_ref().map(|id| pb::SessionId { value: id.clone() }),
                terminals: entry
                    .terminals
                    .iter()
                    .map(|(name, state)| pb::TerminalEnding { name: name.clone(), state: *state })
                    .collect(),
                exited: entry.exited,
            })
            .collect(),
        left_alone: listing
            .left_alone
            .iter()
            .map(|(machine, sessions)| pb::LeftAlone {
                machine: Some(pb::MachineId { value: machine.clone() }),
                sessions: sessions.iter().map(|(id, title)| work(id, title)).collect(),
            })
            .collect(),
        forks_staying: listing.staying.iter().map(|(id, title)| work(id, title)).collect(),
        desk_sessions_left_out: listing.desk_left_out,
        token: Some(pb::PlanToken { value: token.to_owned() }),
        needs_count: listing.all,
    }
}

/// Work that deleting would lose (LD-5).
fn unpushed_work(session: &str, entry: &Entry) -> Status {
    let repos = unpushed_pb(&entry.listed.repos)
        .into_iter()
        .map(|repo| pb::unpushed_work::Repo {
            repo: repo.repo,
            branch: repo.branch,
            dirty_files: repo.dirty_files,
            unpushed_commits: repo.unpushed_commits,
            never_pushed: repo.never_pushed,
        })
        .collect();
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!(
                "session \"{}\" has {} uncommitted files and {} unpushed commits that exist only on this machine; nothing was deleted",
                entry.title,
                entry.listed.dirty(),
                entry.listed.unpushed()
            ),
            "commit, push and open a pull request from the session first, then delete it. Deleting with \"Delete that work\" ticked loses that work unless it is pushed",
            pb::error::Kind::UnpushedWork(pb::UnpushedWork {
                session: Some(pb::SessionId { value: session.to_owned() }),
                repos,
            }),
        ),
    )
}

/// What a session's delete note says.
#[derive(Debug, Serialize, Deserialize)]
struct Note {
    session: String,
    /// Who deleted it and what, for the trash item.
    data: String,
    /// The session whose folder this one shares (a context-only fork).
    #[serde(default)]
    shares: Option<String>,
}

/// What stops a session's delete part way.
#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("the session's records could not be read or written")]
    Store(#[source] StoreError),
    #[error("a deleting worker stopped")]
    Worker(#[source] Status),
    #[error("this humatad2 was started without its channel service or trash, so it can't delete sessions; restart Humata on this machine")]
    NotReady,
    #[error("machine {machine} could not delete its sessions")]
    Remote {
        machine: String,
        #[source]
        source: Status,
    },
    #[error("deleted: {deleted}. Not deleted: {left}")]
    Partial {
        deleted: String,
        left: String,
        #[source]
        source: Box<RunError>,
    },
    #[error("session {session}'s terminals could not be ended")]
    Stop {
        session: String,
        #[source]
        source: DeleteError,
    },
    #[error("session {session}'s requests or grants could not be withdrawn")]
    Withdraw {
        session: String,
        #[source]
        source: Status,
    },
    #[error("the delete's note at {} could not be saved", path.display())]
    Intent {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("session {session}'s folder {} could not be moved to the trash", folder.display())]
    Trash {
        session: String,
        folder: PathBuf,
        #[source]
        source: Status,
    },
}

fn note_path(root: &Path, session: &str) -> PathBuf {
    root.join(INTENTS).join(session)
}

fn marker_path(root: &Path, session: &str) -> PathBuf {
    root.join(INTENTS).join(format!("{session}.trashed"))
}

/// The folder that goes to the trash for `session`: its own folder when it
/// is a plain `sessions/<name>` folder, otherwise a fresh folder holding
/// only the side file.
fn trash_folder(root: &Path, record: &humata2_store::SessionRecord, shares: bool) -> Option<PathBuf> {
    if shares {
        return None;
    }
    let plain = record.folder.is_dir()
        && record
            .folder
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "sessions");
    Some(if plain {
        record.folder.clone()
    } else {
        root.join(INTENTS).join(format!("{}.folder", record.id))
    })
}

impl SessionSvc {
    /// Who may plan or delete many (D306 style): a peer never; an agent
    /// only with the capability, never for every session. Gates agents.
    fn many_gate(
        &self,
        caller: &Caller,
        scope: Option<&pb::DeleteScope>,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        Self::refuse_peer(caller)?;
        if matches!(caller, Caller::Agent(_)) {
            if matches!(scope.and_then(|scope| scope.scope.as_ref()), Some(pb::delete_scope::Scope::All(true))) {
                return Err(status::refused(&Refusal::UserOnly {
                    capability: Some(Capability::SessionsDeleteMany),
                }));
            }
            return self.gate(caller, Capability::SessionsDeleteMany, Target::Global, approved);
        }
        Ok(None)
    }

    fn own_session(caller: &Caller) -> Option<String> {
        match caller {
            Caller::Agent(agent) => Some(agent.session.id.as_str().to_owned()),
            _ => None,
        }
    }

    async fn resolve_now(&self, scope: &pb::DeleteScope, caller: &Caller, exempt_own: bool) -> Result<Listing, Status> {
        let (store, here, scope, own) = (
            self.starter.store.clone(),
            self.machine.clone(),
            scope.clone(),
            Self::own_session(caller).filter(|_| !exempt_own),
        );
        let machine = self.machine.clone();
        blocking(move || resolve(&store, &here, &scope, own.as_deref()))
            .await?
            .map_err(|error| resolve_status(error, &machine))
    }

    /// The peer link to `machine`, or why there is none: this daemon has no
    /// links, or the machine is offline.
    fn relay_for(&self, machine: &str) -> Result<Relayed, Status> {
        let links = self
            .carry
            .get()
            .map(|carry| carry.links().clone())
            .ok_or_else(|| {
                status::failure(
                    Code::FailedPrecondition,
                    status::error(
                        format!("this daemon has no way to reach {machine}, so its sessions can't be deleted from here"),
                        "delete them from the host, or on that machine",
                        pb::error::Kind::Internal(pb::Internal::default()),
                    ),
                )
            })?;
        super::move_recover::relayed_provider(&links, machine).map_err(Status::from)
    }

    /// Asks each online machine that holds some of the listed sessions for
    /// the plan of those sessions, and keeps its token. Offline machines
    /// stay in `left_alone`.
    async fn relay_plans(&self, listing: &mut Listing) -> Result<(), Status> {
        for (machine, sessions) in listing.left_alone.clone() {
            let Ok(relayed) = self.relay_for(&machine) else {
                continue;
            };
            let ids = sessions.iter().map(|(id, _)| pb::SessionId { value: id.clone() }).collect();
            let request = pb::PlanDeleteSessionsRequest {
                scope: Some(pb::DeleteScope {
                    scope: Some(pb::delete_scope::Scope::Sessions(pb::delete_scope::Picked { ids })),
                }),
            };
            let plan = relayed
                .session()
                .plan_delete_sessions(assigned(request, None).map_err(Status::from)?)
                .await
                .map_err(|source| machine_failed(&machine, source))?
                .into_inner();
            listing.left_alone.remove(&machine);
            listing.take_machine_plan(&machine, &plan);
        }
        Ok(())
    }

    /// Sends every relayed machine `check_only`. A machine whose sessions
    /// changed has its part of `now` replaced by its new plan; the changes
    /// are returned, in words. Touches nothing on any machine.
    async fn check_remote(&self, now: &mut Listing, delete_work: bool) -> Result<Vec<String>, Status> {
        let mut changes = Vec::new();
        for (machine, token) in now.remote.clone() {
            let relayed = self.relay_for(&machine).map_err(|_| {
                stale(
                    &format!("{machine} went offline since the plan was made; nothing was deleted"),
                    vec![format!("{machine} is offline")],
                )
            })?;
            let asked = pb::DeleteSessionsRequest {
                idempotency_key: None,
                plan: Some(pb::PlanToken { value: token }),
                delete_unpushed_work: delete_work,
                typed_count: 0,
                check_only: true,
            };
            let answered = relayed
                .session()
                .delete_sessions(assigned(asked, None).map_err(Status::from)?)
                .await
                .map_err(|source| machine_failed(&machine, source))?
                .into_inner();
            if let Some(pb::delete_sessions_response::Result::Changed(changed)) = answered.result {
                changes.extend(changed.changes);
                if let Some(plan) = &changed.plan {
                    now.take_machine_plan(&machine, plan);
                }
            }
        }
        Ok(changes)
    }

    /// Deletes what `machine` was planned to delete and waits for its
    /// operation to end. Returns how many sessions went and their trash
    /// items.
    async fn delete_remote(
        &self,
        machine: &str,
        token: &str,
        delete_work: bool,
        progress: &Progress,
    ) -> Result<(u32, Vec<String>), Status> {
        let relayed = self.relay_for(machine)?;
        let asked = pb::DeleteSessionsRequest {
            idempotency_key: None,
            plan: Some(pb::PlanToken { value: token.to_owned() }),
            delete_unpushed_work: delete_work,
            typed_count: 0,
            check_only: false,
        };
        let answered = relayed
            .session()
            .delete_sessions(assigned(asked, None).map_err(Status::from)?)
            .await
            .map_err(|source| machine_failed(machine, source))?
            .into_inner();
        let done = match answered.result {
            Some(pb::delete_sessions_response::Result::Deleted(done)) => done,
            Some(pb::delete_sessions_response::Result::Changed(changed)) => {
                return Err(stale(
                    &format!("{machine}'s sessions changed after the check; nothing was deleted there"),
                    changed.changes,
                ));
            }
            None => {
                return Err(status::internal(format!("{machine} answered the delete with nothing"), ""));
            }
        };
        if let Some(operation) = done.operation {
            let lost = || {
                status::failure(
                    Code::Unavailable,
                    status::error(
                        format!("the link to {machine} was lost while it deleted its sessions, so it isn't known how far it got"),
                        format!("reconnect {machine} and list its sessions; it finishes the delete on its own"),
                        pb::error::Kind::Internal(pb::Internal::default()),
                    ),
                )
            };
            let ended = follow_to_end(&relayed, machine, operation, progress, lost).await?;
            if ended.state != pb::OperationState::Succeeded as i32 {
                let error = ended.error.unwrap_or_else(|| {
                    status::error(
                        format!("{machine} ended the delete without saying how it went"),
                        format!("look at its sessions on {machine}, then delete the rest again"),
                        pb::error::Kind::Internal(pb::Internal::default()),
                    )
                });
                return Err(status::failure(Code::Unavailable, error));
            }
        }
        Ok((done.sessions, done.trash_items.into_iter().map(|item| item.value).collect()))
    }

    /// The host's own rows of sessions a machine deleted go too.
    async fn forget_remote(&self, ids: &[String]) {
        for id in ids {
            self.tell.session(id, pb::Change::Deleted, None).await;
            let (store, key) = (self.starter.store.clone(), id.clone());
            let _ = blocking(move || store.remove_session(&key)).await;
        }
    }

    /// The body of PlanDeleteSessions; changes nothing.
    pub(super) async fn plan_delete_sessions_of(
        &self,
        caller: &Caller,
        scope: Option<pb::DeleteScope>,
        approved: bool,
    ) -> Result<Gated<pb::DeleteSessionsPlan>, Status> {
        if let Some(asked) = self.many_gate(caller, scope.as_ref(), approved)? {
            return Ok(Gated::Ask(asked));
        }
        let scope = scope.ok_or_else(|| status::missing("scope"))?;
        let mut listing = self.resolve_now(&scope, caller, false).await?;
        self.relay_plans(&mut listing).await?;
        let token = make_token(&scope, &listing);
        Ok(Gated::Done(plan_pb(&listing, &token)))
    }

    /// The body of DeleteSessions.
    pub(super) async fn delete_sessions_of(
        &self,
        caller: &Caller,
        request: pb::DeleteSessionsRequest,
        approved: bool,
        exempt_own: bool,
    ) -> Result<Gated<pb::DeleteSessionsResponse>, Status> {
        Self::refuse_peer(caller)?;
        let token = request
            .plan
            .as_ref()
            .map(|plan| plan.value.clone())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("plan"))?;
        let (scope, before) = peek_token(&token).ok_or_else(|| {
            stale(
                "the plan to delete the sessions is unknown, used or older than ten minutes",
                Vec::new(),
            )
        })?;
        if let Some(asked) = self.many_gate(caller, Some(&scope), approved)? {
            return Ok(Gated::Ask(asked));
        }
        if before.all {
            if !matches!(caller, Caller::Agent(_)) {
                let principal = guard::principal(caller);
                if self.guard.with_policy(|_, context| principal.present_user(context)).is_none() {
                    return Err(status::refused(&Refusal::UserOnly {
                        capability: Some(Capability::SessionsDeleteMany),
                    }));
                }
            }
            let planned = u32::try_from(before.sessions.len()).unwrap_or(u32::MAX);
            if request.typed_count != planned {
                return Err(status::invalid_field(
                    "typed_count",
                    Problem::OutOfRange,
                    format!(
                        "deleting every session needs the number of sessions typed ({planned}); {} was given",
                        request.typed_count
                    ),
                    format!("type {planned}, the number of sessions the plan lists, and delete again"),
                ));
            }
        }
        let mut now = self.resolve_now(&scope, caller, exempt_own).await?;
        now.adopt_remote(&before);
        let mut changes = now.changes_since(&before);
        if changes.is_empty() {
            // Every machine passes its check before any machine deletes.
            changes = self.check_remote(&mut now, request.delete_unpushed_work).await?;
        }
        if !changes.is_empty() {
            take_token(&token);
            let fresh = make_token(&scope, &now);
            return Ok(Gated::Done(pb::DeleteSessionsResponse {
                result: Some(pb::delete_sessions_response::Result::Changed(
                    pb::delete_sessions_response::Changed {
                        plan: Some(plan_pb(&now, &fresh)),
                        changes,
                    },
                )),
            }));
        }
        if !request.delete_unpushed_work
            && let Some((id, entry)) = now.sessions.iter().find(|(_, entry)| entry.listed.has_work())
        {
            return Err(unpushed_work(id, entry));
        }
        // PER-32: only the owner ends terminals. An agent's delete that ends
        // any goes to the Agent inbox as an Ask, whatever its rule says.
        if !approved
            && !request.check_only
            && let Caller::Agent(agent) = caller
            && now.sessions.values().any(|entry| !entry.terminals.is_empty())
        {
            return Ok(Gated::Ask(Box::new(AccessRequest {
                capability: Capability::SessionsDeleteMany,
                channel: agent.session.channel.clone(),
                requester: agent.session.id.clone(),
                call: Call::Use { capability: Capability::SessionsDeleteMany, target: Target::Global },
                always_let: None,
                ahead: false,
                answer_at: AnswerAt::Host,
            })));
        }
        if request.check_only {
            return Ok(Gated::Done(pb::DeleteSessionsResponse {
                result: Some(pb::delete_sessions_response::Result::Deleted(
                    pb::delete_sessions_response::Deleted::default(),
                )),
            }));
        }
        take_token(&token);
        let channels = self.channels.get().ok_or_else(|| status::internal(RunError::NotReady.to_string(), ""))?;
        let bin = channels.trash_bin().ok_or_else(|| status::internal(RunError::NotReady.to_string(), ""))?;

        let target = match (&scope.scope, now.sessions.keys().next()) {
            (Some(pb::delete_scope::Scope::Channel(channel)), _) => pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Channel(channel.clone())),
            },
            (Some(pb::delete_scope::Scope::Machine(machine)), _) => pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Machine(machine.clone())),
            },
            (_, Some(first)) => pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: first.clone() })),
            },
            _ => pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Machine(pb::MachineId { value: self.machine.clone() })),
            },
        };
        let steps = [
            ("stop".to_owned(), "Ending terminals".to_owned()),
            ("withdraw".to_owned(), "Withdrawing requests".to_owned()),
            ("trash".to_owned(), "Moving to the trash".to_owned()),
        ];
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::DeleteSessions,
            target.clone(),
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => {
                return Ok(Gated::Done(deleted(0, Vec::new(), Some(operation))));
            }
        };
        let by = identity::actor(caller);
        let operation_id = progress.operation().id.map(|id| id.value).unwrap_or_default();

        // Forks that share a folder go first, so the parent's folder takes
        // their side files to the trash with it.
        let mut order: Vec<(String, Option<String>)> = now
            .sessions
            .iter()
            .map(|(id, entry)| (id.clone(), entry.goes_with.clone()))
            .collect();
        order.sort_by_key(|(_, shares)| shares.is_none());
        order.retain(|(id, _)| now.sessions.get(id).is_some_and(|entry| !now.remote.contains_key(&entry.machine)));
        let total = now.sessions.len();
        let mut items = Vec::new();
        let mut failure = None;
        let mut gone: Vec<String> = Vec::new();
        for (machine, token) in &now.remote {
            match self.delete_remote(machine, token, request.delete_unpushed_work, &progress).await {
                Ok((_, trash)) => {
                    items.extend(trash);
                    let ids: Vec<String> = now
                        .sessions
                        .iter()
                        .filter(|(_, entry)| &entry.machine == machine)
                        .map(|(id, _)| id.clone())
                        .collect();
                    self.forget_remote(&ids).await;
                    gone.extend(ids);
                }
                Err(source) => {
                    failure = Some(RunError::Remote { machine: machine.clone(), source });
                    break;
                }
            }
        }
        for (at, (id, shares)) in order.iter().enumerate() {
            if failure.is_some() {
                break;
            }
            // Each session's trash item names that session, and its channel
            // for the restore's check.
            let was = pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.clone() })),
            };
            let channel = now.sessions.get(id).map(|entry| entry.channel.as_str()).unwrap_or_default();
            let data = with_channel(&caller_data(&by, &was), channel);
            let note = Note { session: id.clone(), data, shares: shares.clone() };
            match self.run_one(&note, &bin, Some((&progress, at, order.len()))).await {
                Ok(item) => {
                    items.extend(item);
                    gone.push(id.clone());
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        match failure {
            None => {
                progress.succeed(None);
                let operation = self.operations.progress_of(caller, &operation_id).ok().map(|p| p.operation());
                Ok(Gated::Done(deleted(
                    u32::try_from(total).unwrap_or(u32::MAX),
                    items.into_iter().map(|value| pb::TrashItemId { value }).collect(),
                    operation,
                )))
            }
            Some(error) => {
                // With sessions on other machines, the error says what went.
                let error = if now.remote.is_empty() {
                    error
                } else {
                    let named = |went: bool| {
                        let titles: Vec<String> = now
                            .sessions
                            .iter()
                            .filter(|(id, _)| gone.contains(id) == went)
                            .map(|(_, entry)| format!("\"{}\" on {}", entry.title, entry.machine))
                            .collect();
                        if titles.is_empty() { "none".to_owned() } else { titles.join(", ") }
                    };
                    RunError::Partial { deleted: named(true), left: named(false), source: Box::new(error) }
                };
                let failed = error_of(
                    &error,
                    "fix the cause the message names and delete the sessions again; restarting Humata on this machine also finishes the delete. A session's folder is untouched until its last steps",
                    None,
                );
                progress.fail(failed.clone());
                Err(status::failure(Code::Unavailable, failed))
            }
        }
    }

    /// The steps of one session's delete, each safe to run again. Returns
    /// the trash item's id when this run made one.
    async fn run_one(
        &self,
        note: &Note,
        bin: &Arc<Bin>,
        progress: Option<(&Progress, usize, usize)>,
    ) -> Result<Option<String>, RunError> {
        let step = |key: &str, state: pb::StepState, first: bool, last: bool| {
            if let Some((progress, at, total)) = progress {
                if state == pb::StepState::Running && at == 0 && first {
                    progress.set_step(key, state, None);
                } else if state == pb::StepState::Done && at + 1 == total && last {
                    progress.set_step(key, state, None);
                }
            }
        };
        let id = note.session.clone();
        let root = self.starter.layout.root();
        let channels = self.channels.get().ok_or(RunError::NotReady)?;
        let (store, key) = (self.starter.store.clone(), id.clone());
        let Some(record) = blocking(move || store.session(&key))
            .await
            .map_err(RunError::Worker)?
            .map_err(RunError::Store)?
        else {
            // Its records went last: nothing is left but the note.
            self.finish_note(&root, &id);
            return Ok(None);
        };

        // 1. The intent, outside the folder.
        let path = note_path(&root, &id);
        let text = serde_json::to_string(note)
            .map_err(|error| RunError::Intent { path: path.clone(), source: std::io::Error::other(error) })?;
        let at = path.clone();
        blocking(move || -> Result<(), std::io::Error> {
            if let Some(parent) = at.parent() {
                humata2_durable::create_dir_all(parent, 0o700)?;
            }
            humata2_store::write_atomic(&at, text.as_bytes()).map_err(std::io::Error::other)
        })
        .await
        .map_err(RunError::Worker)?
        .map_err(|source| RunError::Intent { path: path.clone(), source })?;
        crate::fault::point("delete_sessions.intent");

        // 2. Its agent and shells.
        step("stop", pb::StepState::Running, true, false);
        channels
            .stop_all(std::slice::from_ref(&id))
            .await
            .map_err(|source| RunError::Stop { session: id.clone(), source })?;
        crate::fault::point("delete_sessions.stopped");
        step("stop", pb::StepState::Done, false, true);

        // 3. Its pending requests, 4. its grants.
        step("withdraw", pb::StepState::Running, true, false);
        self.inbox
            .withdraw_pending_of(&id, pb::WithdrawReason::SessionDeleted)
            .await
            .map_err(|source| RunError::Withdraw { session: id.clone(), source })?;
        channels
            .end_grants(&id)
            .await
            .map_err(|source| RunError::Withdraw { session: id.clone(), source })?;
        step("withdraw", pb::StepState::Done, false, true);

        // 5. and 6. The records to a side file, the folder to the trash.
        step("trash", pb::StepState::Running, true, false);
        let marker = marker_path(&root, &id);
        let mut item = None;
        if !marker.exists()
            && let Some(folder) = trash_folder(&root, &record, note.shares.is_some())
        {
            let side = folder.join(SIDE_FILE);
            let (store, key, made) = (self.starter.store.clone(), id.clone(), folder.clone());
            blocking(move || -> Result<(), StoreError> {
                humata2_durable::create_dir_all(&made, 0o700).map_err(|source| StoreError::CreateDir {
                    path: made.clone(),
                    source,
                })?;
                store.export_session(&key, &side)
            })
            .await
            .map_err(RunError::Worker)?
            .map_err(RunError::Store)?;
            crate::fault::point("delete_sessions.snapshotted");
            let (moving, text, title, bin) = (folder.clone(), note.data.clone(), record.title.clone(), bin.clone());
            item = blocking(move || -> Result<String, Status> {
                let put = bin
                    .get()?
                    .put(
                        &moving,
                        NewItem {
                            description: format!("Session {title}"),
                            caller_data: text,
                            now_unix_ms: now_ms(),
                        },
                    )
                    .map_err(|error| status::internal(status::chain(&error), ""))?;
                Ok(put.id)
            })
            .await
            .map_err(RunError::Worker)?
            .map(Some)
            .map_err(|source| RunError::Trash { session: id.clone(), folder: folder.clone(), source })?;
            let _ = fs::write(&marker, b"");
            crate::fault::point("delete_sessions.trashed");
        } else if !marker.exists() && note.shares.is_some() {
            // A fork in its parent's folder: its records ride in that
            // folder as a second side file, which the parent's trash item
            // takes along. The parent's folder is still here: forks go first.
            let side = record.folder.join(format!("{SIDE_FILE}.fork-{id}"));
            let (store, key) = (self.starter.store.clone(), id.clone());
            blocking(move || store.export_session(&key, &side))
                .await
                .map_err(RunError::Worker)?
                .map_err(RunError::Store)?;
            let _ = fs::write(&marker, b"");
        }
        step("trash", pb::StepState::Done, false, true);

        // 7. The records go last; 8. the change is told while they can
        // still be read.
        self.tell.session(&id, pb::Change::Deleted, None).await;
        let (store, key) = (self.starter.store.clone(), id.clone());
        blocking(move || store.remove_session(&key))
            .await
            .map_err(RunError::Worker)?
            .map_err(RunError::Store)?;
        crate::fault::point("delete_sessions.forgotten");
        self.finish_note(&root, &id);
        Ok(item)
    }

    fn finish_note(&self, root: &Path, id: &str) {
        let _ = fs::remove_file(note_path(root, id));
        let _ = fs::remove_file(marker_path(root, id));
    }

    /// Finishes the deletes a restart cut short, forks first so their
    /// parent's folder is still there to carry their side files. Never
    /// blocks the start.
    pub(crate) async fn recover_session_deletes(&self, bin: Arc<Bin>) {
        let root = self.starter.layout.root();
        let Ok(entries) = fs::read_dir(root.join(INTENTS)) else {
            return;
        };
        let mut notes: Vec<Note> = entries
            .flatten()
            .filter(|entry| !entry.file_name().to_string_lossy().contains('.'))
            .filter_map(|entry| fs::read_to_string(entry.path()).ok())
            .filter_map(|text| serde_json::from_str(&text).ok())
            .collect();
        notes.sort_by_key(|note| note.shares.is_none());
        for note in notes {
            if let Err(error) = self.run_one(&note, &bin, None).await {
                crate::log::say!(
                    "humatad2: couldn't finish deleting session {}: {}; delete it again",
                    note.session,
                    status::chain(&error)
                );
            }
        }
    }

    /// The one-session PlanDelete and DeleteSession: this code with a
    /// one-id scope, behind the caller's own gate (done by the caller).
    pub(super) async fn plan_one(&self, caller: &Caller, id: &str) -> Result<pb::DeletePlan, Status> {
        let scope = one(id);
        let listing = self.resolve_now(&scope, caller, true).await?;
        let token = make_token(&scope, &listing);
        let unpushed = listing
            .sessions
            .get(id)
            .map(|entry| unpushed_pb(&entry.listed.repos))
            .unwrap_or_default();
        Ok(pb::DeletePlan {
            id: Some(pb::SessionId { value: id.to_owned() }),
            unpushed,
            token: Some(pb::PlanToken { value: token }),
        })
    }

    /// Deletes `id` for DeleteSession: plans it when no token came, then
    /// deletes as the bulk call does. Work in it is lost only when the
    /// caller gave a plan (the token), as a channel's delete is.
    pub(super) async fn delete_one(
        &self,
        caller: &Caller,
        id: &str,
        plan: Option<pb::PlanToken>,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<pb::DeleteSessionResponse, Status> {
        let planned = plan.is_some();
        let plan = match plan {
            Some(plan) => plan,
            None => self.plan_one(caller, id).await?.token.unwrap_or_default(),
        };
        let answered = self
            .delete_many_ungated(caller, pb::DeleteSessionsRequest {
                idempotency_key: key,
                plan: Some(plan),
                delete_unpushed_work: planned,
                typed_count: 0,
                check_only: false,
            })
            .await?;
        match answered.result {
            Some(pb::delete_sessions_response::Result::Deleted(done)) => Ok(pb::DeleteSessionResponse {
                trash_item: done.trash_items.into_iter().next(),
            }),
            Some(pb::delete_sessions_response::Result::Changed(changed)) => Err(stale(
                "the session changed since the plan was made; nothing was deleted",
                changed.changes,
            )),
            None => Err(status::internal("the delete answered nothing", "")),
        }
    }

    /// The bulk body behind a one-id call's own gate: the capability gate
    /// of the bulk call is not asked again.
    async fn delete_many_ungated(
        &self,
        caller: &Caller,
        request: pb::DeleteSessionsRequest,
    ) -> Result<pb::DeleteSessionsResponse, Status> {
        match self.delete_sessions_of(caller, request, true, true).await? {
            Gated::Done(done) => Ok(done),
            Gated::Ask(_) => Err(status::internal("an approved delete was sent to Ask again", "delete it again")),
        }
    }
}

fn one(id: &str) -> pb::DeleteScope {
    pb::DeleteScope {
        scope: Some(pb::delete_scope::Scope::Sessions(pb::delete_scope::Picked {
            ids: vec![pb::SessionId { value: id.to_owned() }],
        })),
    }
}

fn deleted(sessions: u32, items: Vec<pb::TrashItemId>, operation: Option<pb::Operation>) -> pb::DeleteSessionsResponse {
    pb::DeleteSessionsResponse {
        result: Some(pb::delete_sessions_response::Result::Deleted(pb::delete_sessions_response::Deleted {
            sessions,
            trash_items: items,
            operation,
        })),
    }
}
