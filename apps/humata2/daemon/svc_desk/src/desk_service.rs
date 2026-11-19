//! DeskService (DK-1..DK-14, D182): sessions that belong to no channel. A
//! Desk session is an ordinary session filed under the channel "desk"; what
//! only the Desk knows (its folder, its two access switches, when it closed
//! or was last used) is its Desk row. Every call is the user's (the app, or
//! the CLI with presence), except Keep, which an agent may make on its own
//! session's folder (D182.6). This machine only.
//!
//! Starts, lists, closes, reopens, keeps and sets the access of a Desk
//! session; Clean up deletes closed ones, and a sweep deletes the scratch
//! folders whose 7 days are up (D182.4). A closed session's agent is saved
//! stopped and its shells' rows are removed, so a restart brings nothing back
//! and restart.rs needs no filter for Desk sessions.
//!
//! PlanDeskMove and MoveDeskSessionToChannel move a Desk session into a
//! channel (DK-15..DK-19, D305; move_to_channel.rs). The session's own agent
//! may call the move too, under the target channel's rule.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use humata2_access::Refusal;
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{DESK_CHANNEL, DeskError, Starter};
use humata2_store::{DeskRow, SessionRecord, Store, StoreError};
use tokio::sync::{Notify, OwnedMutexGuard};
use tonic::{Code, Request, Response, Status};

use crate::ports::{blocking, error_of};
use crate::ports::DeskGuardPort;
use humata2_daemon_ports::Caller;
use crate::ports::{self as identity, DeskIdentityPort};
use crate::ports::DeskInboxPort;
use crate::ports::DeskOperationsPort;
use crate::ports::DeskRemotePort;
use crate::ports::DeskTokensPort;
use crate::ports::start_error;
use crate::ports as status;
use crate::ports::DeskTerminalPort;
use crate::ports::DeskTrashPort as Bin;
use crate::ports::DeskMigrationsPort;

pub mod channel_desk;
pub mod channel_desk_close;
mod clean_up;
mod close;
mod copy;
mod doc_watch;
mod follow;
mod migrate;
mod move_to_channel;
mod remote;
mod remote_asked;
mod remote_close;
mod remote_view;
mod repos;
mod start;
mod view;
mod versions;

/// What the Desk service is built from. All of it is in serve.rs's scope
/// where the session service is made.
pub struct DeskDeps {
    pub guard: Arc<dyn DeskGuardPort>,
    pub operations: Arc<dyn DeskOperationsPort>,
    pub starter: Starter,
    pub identities: Arc<dyn DeskIdentityPort>,
    pub tokens: Arc<dyn DeskTokensPort>,
    pub inbox: Arc<dyn DeskInboxPort>,
    pub tell: Arc<dyn crate::ports::DeskTellPort>,
    pub machine: String,
    pub socket: PathBuf,
    /// The agent a start takes when none is named: `--default-agent`.
    pub default_agent: String,
    /// This machine's trash, where a deleted scratch folder goes (LD-7).
    pub bin: Arc<dyn Bin>,
    /// The terminals, whose in-memory holders a closed session forgets.
    pub terminals: Arc<dyn DeskTerminalPort>,
    /// Humata's state folder, which a picked folder may not be inside.
    pub state: PathBuf,
    /// How the trash and the channel delete reach this service (CD-4,
    /// CH-24); set here once the service is made.
    pub handle: channel_desk_close::DeskHandle,
    /// The host's way to other machines: a Desk session starts on any of
    /// them (MV-17). None on a machine's own daemon.
    pub remote: Option<Arc<dyn DeskRemotePort>>,
    /// Which machines are at another release than this host's (UP-1).
    pub migrations: Arc<dyn DeskMigrationsPort>,
    /// The channel service, which adds the repos a moved Desk session
    /// brings (DK-16).
    pub channels: std::sync::Arc<dyn crate::ports::DeskChannelPort>,
    /// This start, after which a Desk move cut short is settled (DK-17).
    pub last_start: Arc<dyn crate::ports::DeskRestartPort>,
    /// The session service's agent watch, turn, resume and Moving status,
    /// which a Desk session shares with a channel session.
    pub sessions: Arc<dyn crate::ports::DeskSessionPort>,
}

/// The Desk service, served beside the session service.
#[derive(Clone)]
pub struct DeskSvc {
    guard: Arc<dyn DeskGuardPort>,
    operations: Arc<dyn DeskOperationsPort>,
    starter: Starter,
    identities: Arc<dyn DeskIdentityPort>,
    tokens: Arc<dyn DeskTokensPort>,
    inbox: Arc<dyn DeskInboxPort>,
    tell: Arc<dyn crate::ports::DeskTellPort>,
    machine: String,
    default_agent: String,
    bin: Arc<dyn Bin>,
    terminals: Arc<dyn DeskTerminalPort>,
    state: PathBuf,
    locks: Arc<Locks>,
    /// Woken when a session closes, so the sweep runs at once (D182.4).
    wake: Arc<Notify>,
    /// Clean up's answers by idempotency key, so a retry answers what the
    /// first call did instead of a changed plan. Kept until restart.
    cleaned: Arc<Mutex<HashMap<String, pb::CleanUpDeskResponse>>>,
    /// CopyDeskDocument's answers by idempotency key, kept until restart.
    copied: Arc<Mutex<HashMap<String, pb::CopyDeskDocumentResponse>>>,
    remote: Option<Arc<dyn DeskRemotePort>>,
    migrations: Arc<dyn DeskMigrationsPort>,
    channels: std::sync::Arc<dyn crate::ports::DeskChannelPort>,
    /// This start, whose report says which starts it couldn't finish.
    restart: Arc<dyn crate::ports::DeskRestartPort>,
    /// The host's change feed, where a Desk session's changes are told.
    feed: Arc<dyn crate::ports::DeskFeedPort>,
    sessions: Arc<dyn crate::ports::DeskSessionPort>,
}

/// One lock per Desk session, held across any change to its row, its agent
/// or its folder: Close, Reopen, Keep, SetDeskAccess, the sweep and Clean up
/// all take it (D182.4).
#[derive(Default)]
pub struct Locks {
    sessions: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Locks {
    /// Waits for `session`'s lock and holds it until the guard is dropped.
    pub async fn hold(&self, session: &str) -> OwnedMutexGuard<()> {
        let lock = self
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(session.to_owned())
            .or_default()
            .clone();
        lock.lock_owned().await
    }
}

/// The gRPC service for `deps`.
pub fn server(deps: DeskDeps) -> pb::desk_service_server::DeskServiceServer<DeskSvc> {
    let svc = DeskSvc {
        guard: deps.guard,
        operations: deps.operations,
        starter: deps.starter,
        identities: deps.identities,
        tokens: deps.tokens,
        inbox: deps.inbox,
        tell: deps.tell,
        machine: deps.machine,
        default_agent: deps.default_agent,
        bin: deps.bin,
        terminals: deps.terminals,
        state: deps.state,
        locks: Arc::default(),
        wake: Arc::new(Notify::new()),
        cleaned: Arc::default(),
        copied: Arc::default(),
        remote: deps.remote,
        migrations: deps.migrations,
        channels: deps.channels,
        restart: deps.last_start.clone(),
        feed: deps.handle.feed(),
        sessions: deps.sessions,
    };
    // DK-17: a Desk move a restart cut short is finished or undone.
    move_to_channel::recover_after(svc.clone(), deps.last_start.clone());
    // D731: a Desk start the host asked a machine for and heard no answer
    // to is asked again under the same id once the machine links.
    remote_asked::recover_after(svc.clone(), deps.last_start.clone());
    // The sweep runs from here on: at start, after each Close and hourly.
    clean_up::spawn_sweep(svc.clone());
    // D458: Desk folders made before DOCV get their repository through a Migrate item.
    migrate::wire(&svc);
    // DV-12, DV-5: a new commit in a Desk folder records its comments and tells the app.
    doc_watch::spawn_watch(svc.clone(), deps.handle.feed());
    // DESKWATCH: every change to a Desk session is told on the feed as a Desk event.
    follow::spawn_follow(svc.clone(), deps.handle.feed());
    // CD-5: closes or stops a channel Desk session whose target or channel
    // is gone.
    deps.handle.set(svc.clone());
    channel_desk_close::spawn_watch(svc.clone(), deps.handle.feed());
    pb::desk_service_server::DeskServiceServer::new(svc)
}

#[tonic::async_trait]
impl pb::desk_service_server::DeskService for DeskSvc {
    async fn start_desk_session(
        &self,
        request: Request<pb::StartDeskSessionRequest>,
    ) -> Result<Response<pb::StartDeskSessionResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        let assigned = remote::assigned_of(&request, &caller)?;
        let request = request.into_inner();
        if request.channel.is_some() {
            return self
                .start_channel_desk(&caller, request, assigned)
                .await
                .map(Response::new);
        }
        self.start(&caller, request, assigned).await.map(Response::new)
    }

    async fn list_desk_sessions(
        &self,
        request: Request<pb::ListDeskSessionsRequest>,
    ) -> Result<Response<pb::ListDeskSessionsResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        let request = request.into_inner();
        let sessions = if request.channel.is_some() || request.target.is_some() {
            self.list_channel_desk(request.channel, request.target).await?
        } else {
            self.list().await?
        };
        Ok(Response::new(pb::ListDeskSessionsResponse { sessions }))
    }

    async fn close_desk_session(
        &self,
        request: Request<pb::CloseDeskSessionRequest>,
    ) -> Result<Response<pb::DeskSession>, Status> {
        let caller = identity::caller(&request)?;
        self.user_or_destroy_switch(&caller)?;
        let request = request.into_inner();
        if let Some(closed) = self.close_channel_desk(&caller, &request).await? {
            return Ok(Response::new(closed));
        }
        self.close(&caller, request).await.map(Response::new)
    }

    async fn reopen_desk_session(
        &self,
        request: Request<pb::ReopenDeskSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        let request = request.into_inner();
        if let Some(operation) = self.reopen_channel_desk(&caller, &request).await? {
            return Ok(Response::new(operation));
        }
        self.reopen(&caller, request).await.map(Response::new)
    }

    async fn keep_desk_folder(
        &self,
        request: Request<pb::KeepDeskFolderRequest>,
    ) -> Result<Response<pb::DeskSession>, Status> {
        let caller = identity::caller(&request)?;
        self.keep(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn list_doc_versions(
        &self,
        request: Request<pb::ListDocVersionsRequest>,
    ) -> Result<Response<pb::ListDocVersionsResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.list_versions(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn read_doc_version(
        &self,
        request: Request<pb::ReadDocVersionRequest>,
    ) -> Result<Response<pb::DocVersionText>, Status> {
        let caller = identity::caller(&request)?;
        self.read_version(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn save_doc_version(
        &self,
        request: Request<pb::SaveDocVersionRequest>,
    ) -> Result<Response<pb::DocVersion>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.save_version(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn restore_doc_version(
        &self,
        request: Request<pb::RestoreDocVersionRequest>,
    ) -> Result<Response<pb::RestoreDocVersionResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.restore_version(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn diff_doc_version(
        &self,
        request: Request<pb::DiffDocVersionRequest>,
    ) -> Result<Response<pb::DocVersionDiff>, Status> {
        let caller = identity::caller(&request)?;
        self.diff_version(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn set_desk_access(
        &self,
        request: Request<pb::SetDeskAccessRequest>,
    ) -> Result<Response<pb::DeskSession>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        let request = request.into_inner();
        if request.switches.is_some() {
            return self
                .set_channel_desk_switches(&caller, request)
                .await
                .map(Response::new);
        }
        self.set_access(&caller, request).await.map(Response::new)
    }

    async fn set_desk_target(
        &self,
        request: Request<pb::SetDeskTargetRequest>,
    ) -> Result<Response<pb::DeskSession>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.set_channel_desk_target(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn add_desk_repo(
        &self,
        request: Request<pb::AddDeskRepoRequest>,
    ) -> Result<Response<pb::DeskSession>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.add_repo(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn remove_desk_repo(
        &self,
        request: Request<pb::RemoveDeskRepoRequest>,
    ) -> Result<Response<pb::DeskSession>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.remove_repo(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn copy_desk_document(
        &self,
        request: Request<pb::CopyDeskDocumentRequest>,
    ) -> Result<Response<pb::CopyDeskDocumentResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.copy_document(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn plan_desk_clean_up(
        &self,
        request: Request<pb::PlanDeskCleanUpRequest>,
    ) -> Result<Response<pb::DeskCleanUpPlan>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.plan().await.map(Response::new)
    }

    async fn clean_up_desk(
        &self,
        request: Request<pb::CleanUpDeskRequest>,
    ) -> Result<Response<pb::CleanUpDeskResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.clean_up(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn plan_desk_move(
        &self,
        request: Request<pb::PlanDeskMoveRequest>,
    ) -> Result<Response<pb::DeskMovePlan>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.plan_move(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn move_desk_session_to_channel(
        &self,
        request: Request<pb::MoveDeskSessionToChannelRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        self.move_session(&caller, request.into_inner()).await
    }

    async fn check_desk_folders(
        &self,
        request: Request<pb::CheckDeskFoldersRequest>,
    ) -> Result<Response<pb::CheckDeskFoldersResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.count_stale().await.map(Response::new)
    }

    async fn migrate_desk_folders(
        &self,
        request: Request<pb::MigrateDeskFoldersRequest>,
    ) -> Result<Response<pb::MigrateDeskFoldersResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.migrate_asked().await.map(Response::new)
    }
}

impl DeskSvc {
    /// D801 H: tells the clients a session changed (a delete filed it on
    /// the Desk).
    pub async fn tell_session_updated(&self, id: &str) {
        self.tell.session(id, pb::Change::Updated, None).await;
    }

    /// Every Desk session on this machine, open and closed, deleted ones
    /// left out (DK-5). Runs no git.
    async fn list(&self) -> Result<Vec<pb::DeskSession>, Status> {
        let store = self.starter.store.clone();
        let (sessions, rows) = blocking(move || -> Result<_, StoreError> {
            Ok((store.sessions()?, store.desks()?))
        })
        .await?
        .map_err(|error| crate::ports::store_error(error, &self.machine, "list the Desk's sessions"))?;
        let rows: HashMap<String, DeskRow> = rows
            .into_iter()
            .map(|row| (row.session_id.clone(), row))
            .collect();
        let mut sessions: Vec<SessionRecord> = sessions
            .into_iter()
            .filter(|session| session.channel == DESK_CHANNEL)
            .collect();
        sessions.sort_by(|a, b| (a.created_unix_ms, &a.id).cmp(&(b.created_unix_ms, &b.id)));
        let shown: Vec<_> = sessions
            .iter()
            .filter_map(|session| {
                rows.get(&session.id)
                    .filter(|row| row.deleted_unix_ms.is_none())
                    .map(|row| (self.elsewhere(session), view::desk_pb(session, row)))
            })
            .collect();
        // A session on another machine shows what that machine holds (MV-17).
        Ok(self.fill_remote(shown, pb::ListDeskSessionsRequest::default()).await)
    }

    /// Keep (DK-2, D182.6): the user may keep any Desk folder; an agent may
    /// keep only its own session's scratch folder, and a picked folder is
    /// never changed.
    async fn keep(
        &self,
        caller: &Caller,
        request: pb::KeepDeskFolderRequest,
    ) -> Result<pb::DeskSession, Status> {
        // With no session named, an agent keeps its own (D182.6).
        let id = match (request.session, caller) {
            (None, Caller::Agent(agent)) => agent.session.id.as_str().to_owned(),
            (session, _) => session_id(session)?,
        };
        match caller {
            Caller::User(_) => {}
            Caller::Agent(agent) if agent.session.id.as_str() == id => {}
            _ => user_only(caller)?,
        }
        let _held = self.locks.hold(&id).await;
        let (session, mut row) = self.load(&id).await?;
        if let humata2_store::DeskFolderKind::Scratch { kept: false } = row.kind {
            row.kind = humata2_store::DeskFolderKind::Scratch { kept: true };
        }
        row.last_used_unix_ms = now_ms();
        self.save_row(&row).await?;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(view::desk_pb(&session, &row))
    }

    /// Changes the two switches (DK-4, D182.3). The agent's next call is
    /// judged by them: the policy reads a Desk session's switches per call.
    async fn set_access(
        &self,
        caller: &Caller,
        request: pb::SetDeskAccessRequest,
    ) -> Result<pb::DeskSession, Status> {
        let id = session_id(request.session)?;
        let access = request
            .access
            .ok_or_else(|| status::missing("access"))?;
        let _held = self.locks.hold(&id).await;
        let (session, mut row) = self.load(&id).await?;
        row.boards = access.boards;
        row.channels_and_machines = access.channels_and_machines;
        row.last_used_unix_ms = now_ms();
        self.save_row(&row).await?;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(view::desk_pb(&session, &row))
    }

    /// The session's record and Desk row, or the status that says why there
    /// isn't a Desk session by that id.
    async fn load(&self, id: &str) -> Result<(SessionRecord, DeskRow), Status> {
        load(&self.starter.store, &self.machine, id).await
    }

    /// Saves a Desk row, in place of the one it had.
    async fn save_row(&self, row: &DeskRow) -> Result<(), Status> {
        let (store, row) = (self.starter.store.clone(), row.clone());
        blocking(move || store.put_desk(&row))
            .await?
            .map_err(|error| crate::ports::store_error(error, &self.machine, "save the Desk session"))
    }
}

/// The session's record and Desk row, or the status that says why there
/// isn't a Desk session by that id. A deleted one is not there.
pub async fn load(
    store: &Arc<Store>,
    machine: &str,
    id: &str,
) -> Result<(SessionRecord, DeskRow), Status> {
    let (store, key) = (store.clone(), id.to_owned());
    let found = blocking(move || -> Result<_, StoreError> {
        Ok((store.session(&key)?, store.desk(&key)?))
    })
    .await?;
    match found {
        Ok((Some(session), Some(row))) if row.deleted_unix_ms.is_none() => Ok((session, row)),
        Ok(_) | Err(StoreError::InvalidId { .. }) => Err(not_desk(id)),
        Err(error) => Err(crate::ports::store_error(error, machine, "look up the Desk session")),
    }
}

/// Refuses every caller but the user: the app, or the CLI with presence.
/// An agent or a script is told the call is the user's, and a peer is
/// refused outright.
pub fn user_only(caller: &Caller) -> Result<(), Status> {
    match caller {
        Caller::User(_) => Ok(()),
        Caller::Peer => Err(status::refused(&Refusal::PeerCannotCall)),
        Caller::Agent(_) | Caller::Script(_) => {
            Err(status::refused(&Refusal::UserOnly { capability: None }))
        }
    }
}

impl DeskSvc {
    /// The user closes a Desk session, which ends its terminals; an agent
    /// may only with the destroy switch on (D801 G, PER-32), which is off
    /// by default. A peer or a script is refused.
    fn user_or_destroy_switch(&self, caller: &Caller) -> Result<(), Status> {
        match caller {
            Caller::Agent(_) => self.guard.require(
                caller,
                &humata2_access::Call::Use {
                    capability: humata2_access::Capability::MachineDestroy,
                    target: humata2_access::Target::Machine(humata2_access::MachineAction {
                        machine: humata2_access::MachineId::new(self.machine.as_str()),
                        action: humata2_access::MachineActionKind::Delete,
                    }),
                },
            ),
            _ => user_only(caller),
        }
    }
}

/// Refuses a closed Desk session for an action that needs it open, such as
/// resuming it or opening a tab in it. Only Reopen brings it back (DK-11).
pub async fn refuse_if_closed(
    store: &Arc<Store>,
    machine: &str,
    session: &str,
) -> Result<(), Status> {
    let (store, key) = (store.clone(), session.to_owned());
    let row = blocking(move || store.desk(&key))
        .await?
        .map_err(|error| crate::ports::store_error(error, machine, "look up the Desk session"))?;
    match row {
        Some(row) if row.closed_unix_ms.is_some() && row.deleted_unix_ms.is_none() => {
            Err(failed_precondition(
                format!("session {session} is a closed Desk session, so it can't be resumed or given a new tab"),
                "reopen it on the Desk first; that resumes its agent in the same folder",
            ))
        }
        _ => Ok(()),
    }
}

/// Refuses a channel action (archive, restore, fork, give own copy, push,
/// move) on a Desk session, which belongs to no channel (D182.2).
pub async fn refuse_channel_action(
    store: &Arc<Store>,
    machine: &str,
    session: &str,
    action: &str,
) -> Result<(), Status> {
    let (store, key) = (store.clone(), session.to_owned());
    let rows = blocking(move || -> Result<_, StoreError> { Ok((store.desk(&key)?, store.channel_desk(&key)?)) })
        .await?
        .map_err(|error| crate::ports::store_error(error, machine, "look up the Desk session"))?;
    match rows {
        // CD-1: a channel's Desk session has no fork, archive or clones of
        // its own.
        (_, Some(_)) => Err(failed_precondition(
            format!("{action} isn't for session {session}: it is a channel's Desk session, which has no branch, clones or fork of its own"),
            format!("close it from the channel's Desk instead; copy any files you want to keep to the channel's shared folder first, since {action} works on channel sessions only"),
        )),
        (Some(_), None) => Err(failed_precondition(
            format!("{action} isn't for session {session}: it is a Desk session, which belongs to no channel"),
            format!("use the Desk's own actions for it (Close, Keep, Clean up); {action} works on channel sessions only"),
        )),
        (None, None) => Ok(()),
    }
}

/// The session id a request names.
pub fn session_id(id: Option<pb::SessionId>) -> Result<String, Status> {
    id.map(|id| id.value)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| status::missing("session"))
}

/// Now, in Unix milliseconds: the time a Desk change is stamped with.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

fn not_desk(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Session,
        id,
        format!("no Desk session {id} is on this machine"),
        "list the Desk's sessions to see their ids; it may have been removed or be on another machine",
    )
}

fn failed_precondition(message: String, fix: impl Into<String>) -> Status {
    status::failure(
        Code::FailedPrecondition,
        pb::Error {
            message,
            fix: fix.into(),
            retry: None,
            kind: None,
        },
    )
}

/// The status for a Desk error, with the error the app shows.
pub fn desk_status(error: &DeskError) -> Status {
    let (code, error) = start_failure(error);
    status::failure(code, error)
}

/// The status for a Desk start that failed, with the error the app shows.
pub fn start_failure(error: &DeskError) -> (Code, pb::Error) {
    match error {
        DeskError::Start(source) => start_error(source),
        DeskError::BadRepoName { .. } => invalid_field_failure(error, "repo", pb::invalid_field::Problem::BadFormat),
        DeskError::NotInFolder { .. } | DeskError::OutsideFolder { .. } => {
            invalid_field_failure(error, "path", pb::invalid_field::Problem::BadFormat)
        }
        DeskError::NoSuchFile { .. } | DeskError::NotARegularFile { .. } => {
            invalid_field_failure(error, "path", pb::invalid_field::Problem::UnknownValue)
        }
        DeskError::RepoExists { .. } | DeskError::ChannelHasFile { .. } => (
            Code::AlreadyExists,
            error_of(error, "rename or remove the file or folder that is there, then try again", None),
        ),
        DeskError::NoSuchRepo { .. } => (
            Code::NotFound,
            error_of(error, "list the session's repositories to see the names it has", None),
        ),
        DeskError::CloneFailed { .. } => (
            Code::FailedPrecondition,
            error_of(error, "check the repository's name and that this Mac can reach its remote, then try again", None),
        ),
        _ => (
            Code::Internal,
            error_of(
                error,
                "check humatad2's log for what failed, then start the session again",
                None,
            ),
        ),
    }
}

/// A request field the error names, as an invalid argument.
fn invalid_field_failure(
    error: &DeskError,
    field: &str,
    problem: pb::invalid_field::Problem,
) -> (Code, pb::Error) {
    (
        Code::InvalidArgument,
        error_of(
            error,
            "give the file or repository as the message says, then try again",
            Some(pb::error::Kind::InvalidField(pb::InvalidField {
                field: field.to_owned(),
                problem: problem as i32,
            })),
        ),
    )
}
