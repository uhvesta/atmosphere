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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use humata2_access::Refusal;
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{DESK_CHANNEL, DeskError, Starter};
use humata2_store::{DeskRow, SessionRecord, Store, StoreError};
use tokio::sync::{Notify, OwnedMutexGuard};
use tonic::{Code, Request, Response, Status};

use crate::channel_service::{blocking, error_of};
use crate::guard::Guard;
use crate::identity::{self, Caller, Identities};
use crate::inbox::Inbox;
use crate::operations::Operations;
use crate::restart::Tokens;
use crate::session_service::{Tell, start_error};
use crate::status;
use crate::terminal::Core;
use crate::trash_service::Bin;

mod clean_up;
mod close;
mod copy;
mod repos;
mod start;
mod view;

/// What the Desk service is built from. All of it is in serve.rs's scope
/// where the session service is made.
pub(crate) struct DeskDeps {
    pub(crate) guard: Arc<Guard>,
    pub(crate) operations: Arc<Operations>,
    pub(crate) starter: Starter,
    pub(crate) identities: Arc<Identities>,
    pub(crate) inbox: Arc<Inbox>,
    pub(crate) tell: Tell,
    pub(crate) machine: String,
    pub(crate) socket: PathBuf,
    /// The agent a start takes when none is named: `--default-agent`.
    pub(crate) default_agent: String,
    /// This machine's trash, where a deleted scratch folder goes (LD-7).
    pub(crate) bin: Arc<Bin>,
    /// The terminals, whose in-memory holders a closed session forgets.
    pub(crate) terminals: Arc<Core>,
    /// Humata's state folder, which a picked folder may not be inside.
    pub(crate) state: PathBuf,
}

/// The Desk service, served beside the session service.
#[derive(Clone)]
pub(crate) struct DeskSvc {
    guard: Arc<Guard>,
    operations: Arc<Operations>,
    starter: Starter,
    identities: Arc<Identities>,
    tokens: Arc<Tokens>,
    inbox: Arc<Inbox>,
    tell: Tell,
    machine: String,
    default_agent: String,
    bin: Arc<Bin>,
    terminals: Arc<Core>,
    state: PathBuf,
    locks: Arc<Locks>,
    /// Woken when a session closes, so the sweep runs at once (D182.4).
    wake: Arc<Notify>,
    /// Clean up's answers by idempotency key, so a retry answers what the
    /// first call did instead of a changed plan. Kept until restart.
    cleaned: Arc<Mutex<HashMap<String, pb::CleanUpDeskResponse>>>,
    /// CopyDeskDocument's answers by idempotency key, kept until restart.
    copied: Arc<Mutex<HashMap<String, pb::CopyDeskDocumentResponse>>>,
}

/// One lock per Desk session, held across any change to its row, its agent
/// or its folder: Close, Reopen, Keep, SetDeskAccess, the sweep and Clean up
/// all take it (D182.4).
#[derive(Default)]
pub(crate) struct Locks {
    sessions: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Locks {
    /// Waits for `session`'s lock and holds it until the guard is dropped.
    pub(crate) async fn hold(&self, session: &str) -> OwnedMutexGuard<()> {
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
pub(crate) fn server(deps: DeskDeps) -> pb::desk_service_server::DeskServiceServer<DeskSvc> {
    let tokens = Arc::new(Tokens::new(
        deps.starter.store.clone(),
        deps.identities.clone(),
        deps.socket,
    ));
    let svc = DeskSvc {
        guard: deps.guard,
        operations: deps.operations,
        starter: deps.starter,
        identities: deps.identities,
        tokens,
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
    };
    // The sweep runs from here on: at start, after each Close and hourly.
    clean_up::spawn_sweep(svc.clone());
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
        self.start(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn list_desk_sessions(
        &self,
        request: Request<pb::ListDeskSessionsRequest>,
    ) -> Result<Response<pb::ListDeskSessionsResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        let sessions = self.list().await?;
        Ok(Response::new(pb::ListDeskSessionsResponse { sessions }))
    }

    async fn close_desk_session(
        &self,
        request: Request<pb::CloseDeskSessionRequest>,
    ) -> Result<Response<pb::DeskSession>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.close(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn reopen_desk_session(
        &self,
        request: Request<pb::ReopenDeskSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.reopen(&caller, request.into_inner())
            .await
            .map(Response::new)
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

    async fn set_desk_access(
        &self,
        request: Request<pb::SetDeskAccessRequest>,
    ) -> Result<Response<pb::DeskSession>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        self.set_access(&caller, request.into_inner())
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
}

impl DeskSvc {
    /// Every Desk session on this machine, open and closed, deleted ones
    /// left out (DK-5). Runs no git.
    async fn list(&self) -> Result<Vec<pb::DeskSession>, Status> {
        let store = self.starter.store.clone();
        let (sessions, rows) = blocking(move || -> Result<_, StoreError> {
            Ok((store.sessions()?, store.desks()?))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "list the Desk's sessions"))?;
        let rows: HashMap<String, DeskRow> = rows
            .into_iter()
            .map(|row| (row.session_id.clone(), row))
            .collect();
        let mut sessions: Vec<SessionRecord> = sessions
            .into_iter()
            .filter(|session| session.channel == DESK_CHANNEL)
            .collect();
        sessions.sort_by(|a, b| (a.created_unix_ms, &a.id).cmp(&(b.created_unix_ms, &b.id)));
        Ok(sessions
            .iter()
            .filter_map(|session| {
                rows.get(&session.id)
                    .filter(|row| row.deleted_unix_ms.is_none())
                    .map(|row| view::desk_pb(session, row))
            })
            .collect())
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
            .map_err(|error| status::store(error, &self.machine, "save the Desk session"))
    }
}

/// The session's record and Desk row, or the status that says why there
/// isn't a Desk session by that id. A deleted one is not there.
pub(crate) async fn load(
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
        Err(error) => Err(status::store(error, machine, "look up the Desk session")),
    }
}

/// Refuses every caller but the user: the app, or the CLI with presence.
/// An agent or a script is told the call is the user's, and a peer is
/// refused outright.
pub(crate) fn user_only(caller: &Caller) -> Result<(), Status> {
    match caller {
        Caller::User(_) => Ok(()),
        Caller::Peer => Err(status::refused(&Refusal::PeerCannotCall)),
        Caller::Agent(_) | Caller::Script(_) => {
            Err(status::refused(&Refusal::UserOnly { capability: None }))
        }
    }
}

/// Refuses a closed Desk session for an action that needs it open, such as
/// resuming it or opening a tab in it. Only Reopen brings it back (DK-11).
pub(crate) async fn refuse_if_closed(
    store: &Arc<Store>,
    machine: &str,
    session: &str,
) -> Result<(), Status> {
    let (store, key) = (store.clone(), session.to_owned());
    let row = blocking(move || store.desk(&key))
        .await?
        .map_err(|error| status::store(error, machine, "look up the Desk session"))?;
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
pub(crate) async fn refuse_channel_action(
    store: &Arc<Store>,
    machine: &str,
    session: &str,
    action: &str,
) -> Result<(), Status> {
    let (store, key) = (store.clone(), session.to_owned());
    let row = blocking(move || store.desk(&key))
        .await?
        .map_err(|error| status::store(error, machine, "look up the Desk session"))?;
    match row {
        Some(_) => Err(failed_precondition(
            format!("{action} isn't for session {session}: it is a Desk session, which belongs to no channel"),
            format!("use the Desk's own actions for it (Close, Keep, Clean up); {action} works on channel sessions only"),
        )),
        None => Ok(()),
    }
}

/// The session id a request names.
pub(crate) fn session_id(id: Option<pb::SessionId>) -> Result<String, Status> {
    id.map(|id| id.value)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| status::missing("session"))
}

/// Now, in Unix milliseconds: the time a Desk change is stamped with.
pub(crate) fn now_ms() -> i64 {
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
pub(crate) fn desk_status(error: &DeskError) -> Status {
    let (code, error) = start_failure(error);
    status::failure(code, error)
}

/// The status for a Desk start that failed, with the error the app shows.
pub(crate) fn start_failure(error: &DeskError) -> (Code, pb::Error) {
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
        DeskError::NotAbsolute { .. } => picked_failure(error, pb::invalid_field::Problem::BadFormat),
        DeskError::NotAFolder { .. } => picked_failure(error, pb::invalid_field::Problem::BadFormat),
        DeskError::Missing { .. } => picked_failure(error, pb::invalid_field::Problem::UnknownValue),
        DeskError::InsideHumata { .. } => picked_failure(error, pb::invalid_field::Problem::NotAllowedHere),
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

fn picked_failure(error: &DeskError, problem: pb::invalid_field::Problem) -> (Code, pb::Error) {
    (
        Code::InvalidArgument,
        error_of(
            error,
            "pick a folder that exists, given by its full path, outside Humata's channels folder",
            Some(pb::error::Kind::InvalidField(pb::InvalidField {
                field: "picked_path".to_owned(),
                problem: problem as i32,
            })),
        ),
    )
}
