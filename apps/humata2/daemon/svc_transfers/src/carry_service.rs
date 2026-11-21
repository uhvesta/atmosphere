//! SessionCarryService (D195): how a session leaves one machine and
//! arrives on another. The host's driver calls it on the machine that has
//! the session and on the one that gets it (`session_service/move_session.rs`
//! over `carry.rs`); the work is `humata2_sessions`' carry core. Only the host
//! may call it: the app, a CLI login, an agent, a script and the peer link are
//! all refused, as they are for `HistoryService/ReadMirror` (D189).
//!
//! On the host the same code is called in-process, with no link.

use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{CHANNELS_ROOT, CarryError, DESK_CHANNEL, ForkCopy, ForkPack, ForkPackError, Starter};
use humata2_store::Take;
use tonic::{Code, Request, Response, Status};

use crate::ports::{self, Caller, TrashBinPort, UserVia, blocking, now_ms};

mod review_git;

/// Where a session's agent conversations live on this machine, when that is
/// not the agent's one global dir: a session with a composed config home
/// keeps them there. The carry reads and places conversations through the
/// [`Starter`] it is given, so each operation asks for the one that fits
/// its session.
pub trait SessionHomes: Send + Sync {
    /// `starter`, seeing `session`'s conversations where they are now.
    fn for_session(&self, starter: &Starter, session: &str) -> Starter;
    /// `starter` for a session arriving here, whose conversation is about
    /// to be placed: its home is made first so the later launch, which
    /// looks there, finds what lands.
    fn for_incoming(&self, starter: &Starter, session: &str) -> Starter;
}

/// No composed homes: every session's conversations are in the agent's own dir.
struct GlobalHomes;

impl SessionHomes for GlobalHomes {
    fn for_session(&self, starter: &Starter, _session: &str) -> Starter {
        starter.clone()
    }

    fn for_incoming(&self, starter: &Starter, _session: &str) -> Starter {
        starter.clone()
    }
}

pub struct SessionCarrySvc {
    starter: Starter,
    homes: Arc<dyn SessionHomes>,
    bin: Arc<dyn TrashBinPort>,
    /// What this machine takes in when a session arrives: the host already
    /// holds the session's record and place in the tree, a box doesn't.
    take: Take,
    /// Notified after a moved session is forgotten from this machine's store.
    on_released: std::sync::OnceLock<Arc<dyn Fn() + Send + Sync>>,
}

impl SessionCarrySvc {
    pub fn new(starter: Starter, bin: Arc<dyn TrashBinPort>, host: bool) -> Arc<Self> {
        Self::with_homes(starter, bin, host, Arc::new(GlobalHomes))
    }

    /// [`Self::new`] for a machine whose sessions may keep their
    /// conversations in composed config homes.
    pub fn with_homes(
        starter: Starter,
        bin: Arc<dyn TrashBinPort>,
        host: bool,
        homes: Arc<dyn SessionHomes>,
    ) -> Arc<Self> {
        Arc::new(Self {
            starter,
            homes,
            bin,
            take: if host { Take::AgentOnly } else { Take::Everything },
            on_released: std::sync::OnceLock::new(),
        })
    }

    /// Registers a callback invoked after [`Self::release`] forgets a moved
    /// session from this machine's store (so e.g. the FUSEFS view drops it).
    pub fn on_released(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let _ = self.on_released.set(callback);
    }

    /// Stages `session` for a move (see [`Starter::pack_for`]). `target` is
    /// the folder the session will have on the new machine, empty when the
    /// conversation is to travel as it is.
    pub async fn pack(
        &self,
        operation: &str,
        session: &str,
        target: &str,
    ) -> Result<humata2_sessions::Packed, Status> {
        let (starter, operation, session, target) =
            (self.homes.for_session(&self.starter, session), operation.to_owned(), session.to_owned(), target.to_owned());
        blocking(move || {
            let target = (!target.is_empty()).then(|| std::path::PathBuf::from(&target));
            starter.pack_for(&operation, &session, target.as_deref())
        })
        .await?
        .map_err(failed)
    }

    /// Where the session folder `folder` of `channel` will be here (see
    /// [`Starter::carry_target`]).
    pub async fn target(&self, channel: &str, folder: &str) -> Result<String, Status> {
        let (starter, channel, folder) = (self.starter.clone(), channel.to_owned(), folder.to_owned());
        blocking(move || starter.carry_target(&channel, &folder))
            .await?
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(failed)
    }

    /// Stages a fork of `parent` for another machine (see [`Starter::pack_fork`]).
    pub async fn pack_fork(
        &self,
        operation: &str,
        parent: &str,
        fork: &pb::CarryFork,
    ) -> Result<pb::CarryPackResponse, Status> {
        let pack = fork_pack_of(operation, parent, fork)?;
        let starter = self.homes.for_session(&self.starter, parent);
        let staged = blocking(move || starter.pack_fork(&pack))
            .await?
            .map_err(fork_failed)?;
        Ok(pb::CarryPackResponse {
            folder: staged.packed.folder,
            stage: staged.packed.stage,
            digest: staged.packed.digest,
            files: staged.packed.files,
            bytes: staged.packed.bytes,
            branch: staged.branch,
            conversation_id: staged.conversation_id,
            agent_running: false,
        })
    }

    /// What a fork of `parent` onto another machine would get (see
    /// [`Starter::preview_fork`]): its branch and bytes, staging nothing.
    pub async fn preview_fork(
        &self,
        parent: &str,
        title: &str,
        typed: &str,
    ) -> Result<pb::CarryPreviewForkResponse, Status> {
        let (starter, parent, title) =
            (self.homes.for_session(&self.starter, parent), parent.to_owned(), title.to_owned());
        let typed = Some(typed.to_owned()).filter(|typed| !typed.is_empty());
        let plan = blocking(move || starter.preview_fork(&parent, &title, typed.as_deref()))
            .await?
            .map_err(|error| {
                failure(
                    Code::FailedPrecondition,
                    ports::chain(&error),
                    "fix what the message names, then fork again",
                )
            })?;
        Ok(pb::CarryPreviewForkResponse {
            branch: plan.branch.name,
            repos: plan
                .repos
                .into_iter()
                .map(|(owner, name)| pb::RepoRef { owner, name })
                .collect(),
            copy_bytes: plan.copy_bytes,
        })
    }

    /// Takes in a new session (a fork) whole, as a machine with no record of
    /// it does: on the host too (see [`Starter::unpack`]).
    pub async fn unpack_new(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
    ) -> Result<humata2_sessions::Unpacked, Status> {
        let (starter, operation, session, channel) = (
            self.homes.for_incoming(&self.starter, session),
            operation.to_owned(),
            session.to_owned(),
            channel.to_owned(),
        );
        blocking(move || starter.unpack(&operation, &session, &channel, Take::Everything))
            .await?
            .map_err(failed)
    }

    /// Takes back a new session a carry put on this machine, records included.
    pub async fn abort_new(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        folder: &str,
    ) -> Result<(), Status> {
        let (starter, operation, session, channel, folder) = (
            self.homes.for_session(&self.starter, session),
            operation.to_owned(),
            session.to_owned(),
            channel.to_owned(),
            folder.to_owned(),
        );
        blocking(move || starter.abort_incoming(&operation, &session, &channel, &folder, Take::Everything))
            .await?
            .map_err(failed)
    }

    /// Checks and takes in the copy of `session` (see [`Starter::unpack`]).
    pub async fn unpack(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
    ) -> Result<humata2_sessions::Unpacked, Status> {
        let (starter, operation, session, channel, take) = (
            self.homes.for_incoming(&self.starter, session),
            operation.to_owned(),
            session.to_owned(),
            channel.to_owned(),
            self.take,
        );
        blocking(move || starter.unpack(&operation, &session, &channel, take))
            .await?
            .map_err(failed)
    }

    /// Puts what the session left on this machine in the trash and drops its
    /// records. What went to the trash is named in the answer.
    pub async fn release(&self, operation: &str, session: &str, channel: &str) -> Result<Vec<String>, Status> {
        // D801 I: the session's shells run on its new machine now (MV-13), but
        // a shell here is a running terminal, and Humata ends one only when
        // the owner does. So release leaves each running: `forget_moved`
        // drops the session's records and tabs, and the running-holders
        // listing then shows every shell still here under "Not in a channel"
        // (D784), where the owner attaches, adopts or ends it. The move's
        // confirm names them (a UI seam).
        let (starter, bin, operation, session, channel, take) = (
            self.homes.for_session(&self.starter, session),
            self.bin.clone(),
            operation.to_owned(),
            session.to_owned(),
            channel.to_owned(),
            self.take,
        );
        let trashed = blocking(move || -> Result<Vec<String>, Status> {
            let trash = bin.get()?;
            let mut trashed = Vec::new();
            for discard in starter.discards(&session).map_err(failed)? {
                let item = humata2_transfers::NewItem {
                    description: discard.what.clone(),
                    caller_data: String::new(),
                    now_unix_ms: now_ms(),
                };
                trash.put(&discard.path, item).map_err(|error| {
                    failure(
                        Code::Internal,
                        format!(
                            "couldn't move {} to this machine's trash: {}",
                            discard.path.display(),
                            ports::chain(&error)
                        ),
                        "the session already runs on its new machine; free space on this disk or fix the folder's permissions, then delete the old copy from the trash page",
                    )
                })?;
                trashed.push(discard.what);
            }
            // A cut here: the copy is in the trash, its records are still
            // here, and the host asks again.
            humata2_store::fault::point("carry.trashed");
            starter.forget_moved(&operation, &session, &channel, take).map_err(failed)?;
            humata2_store::fault::point("carry.forgotten");
            Ok(trashed)
        })
        .await??;
        if let Some(callback) = self.on_released.get() {
            callback();
        }
        Ok(trashed)
    }

    /// Takes back what a move that didn't switch put on this machine
    /// (`incoming`), or removes what it staged here (the machine it left).
    pub async fn abort(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        folder: &str,
        incoming: bool,
    ) -> Result<(), Status> {
        let (starter, operation, session, channel, folder, take) = (
            self.homes.for_session(&self.starter, session),
            operation.to_owned(),
            session.to_owned(),
            channel.to_owned(),
            folder.to_owned(),
            self.take,
        );
        blocking(move || {
            if incoming {
                starter.abort_incoming(&operation, &session, &channel, &folder, take)
            } else {
                starter.unstage(&operation, &channel)
            }
        })
        .await?
        .map_err(failed)
    }
}

fn failure(code: Code, message: String, fix: &str) -> Status {
    ports::failure(
        code,
        pb::Error {
            message,
            fix: fix.to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// A carry core failure as the status the host's driver reads. Every message
/// already says what to do; none of them means anything was deleted.
pub fn failed(error: CarryError) -> Status {
    // CV-3, CV-7: the agent's conversation can't be carried: no adapter
    // reads it, or its adapter doesn't know its records or version.
    if let CarryError::Conversation { agent, .. } = &error {
        return ports::failure(
            Code::FailedPrecondition,
            ports::error(
                ports::chain(&error),
                "move a session whose agent keeps its conversation in files Humata can carry; nothing was changed",
                pb::error::Kind::AgentLacksFeature(pb::AgentLacksFeature {
                    agent: Some(pb::AgentId { value: agent.clone() }),
                    feature: pb::agent_lacks_feature::Feature::ConversationMove as i32,
                }),
            ),
        );
    }
    let code = match error {
        CarryError::Io { .. } | CarryError::Store { .. } => Code::Internal,
        // The copy is this machine's live session now: a fork's driver
        // takes that as the fork made, not as a failure.
        CarryError::Running { .. } => Code::AlreadyExists,
        _ => Code::FailedPrecondition,
    };
    failure(
        code,
        ports::chain(&error),
        "fix what the message names, then move the session again",
    )
}

/// The fork `fork` of `parent` as the sessions crate stages it.
fn fork_pack_of(operation: &str, parent: &str, fork: &pb::CarryFork) -> Result<ForkPack, Status> {
    let copy = match pb::ForkKind::try_from(fork.kind) {
        Ok(pb::ForkKind::CodeAndContext) => ForkCopy::CodeAndContext,
        Ok(pb::ForkKind::CodeOnly) => ForkCopy::CodeOnly,
        _ => {
            return Err(failure(
                Code::InvalidArgument,
                "only a fork that carries code can be staged for another machine".to_owned(),
                "fork with CODE_AND_CONTEXT or CODE_ONLY",
            ));
        }
    };
    Ok(ForkPack {
        operation: operation.to_owned(),
        parent: parent.to_owned(),
        session: session_of(fork.session.clone())?,
        title: fork.title.clone(),
        copy,
        before_turn: fork.before_turn,
        branch: Some(fork.branch.clone()).filter(|branch| !branch.is_empty()),
    })
}

/// A fork that couldn't be staged, as the status the host's driver reads.
fn fork_failed(error: ForkPackError) -> Status {
    failure(
        Code::FailedPrecondition,
        ports::chain(&error),
        "the parent is unchanged and still running; fix what the message names, then fork again",
    )
}

/// Only the host calls this service.
fn require_host(caller: &Caller) -> Result<(), Status> {
    if matches!(caller, Caller::User(UserVia::Host)) {
        return Ok(());
    }
    Err(ports::failure(
        Code::PermissionDenied,
        ports::error(
            "only the host moves sessions between machines",
            "move the session from its menu, or with `humata sessions move`; the host does the rest",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    ))
}

fn session_of(id: Option<pb::SessionId>) -> Result<String, Status> {
    id.map(|id| id.value)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ports::missing("session"))
}

fn channel_of(id: Option<pb::ChannelId>) -> Result<String, Status> {
    id.map(|id| id.value)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ports::missing("channel"))
}

fn operation_of(operation: String) -> Result<String, Status> {
    if operation.is_empty() || !operation.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
        return Err(ports::missing("operation"));
    }
    Ok(operation)
}

#[tonic::async_trait]
impl pb::session_carry_service_server::SessionCarryService for SessionCarrySvc {
    async fn preview_fork(
        &self,
        request: Request<pb::CarryPreviewForkRequest>,
    ) -> Result<Response<pb::CarryPreviewForkResponse>, Status> {
        require_host(&ports::caller(&request)?)?;
        let request = request.into_inner();
        let session = session_of(request.session)?;
        Ok(Response::new(self.preview_fork(&session, &request.title, &request.branch).await?))
    }

    async fn pack(&self, request: Request<pb::CarryPackRequest>) -> Result<Response<pb::CarryPackResponse>, Status> {
        require_host(&ports::caller(&request)?)?;
        let request = request.into_inner();
        let (operation, session) = (operation_of(request.operation)?, session_of(request.session)?);
        if let Some(fork) = request.fork {
            return Ok(Response::new(self.pack_fork(&operation, &session, &fork).await?));
        }
        let packed = self.pack(&operation, &session, &request.target_folder).await?;
        Ok(Response::new(pb::CarryPackResponse {
            folder: packed.folder,
            stage: packed.stage,
            digest: packed.digest,
            files: packed.files,
            bytes: packed.bytes,
            agent_running: packed.agent_running,
            ..pb::CarryPackResponse::default()
        }))
    }

    async fn run_review_git(
        &self,
        request: Request<pb::RunReviewGitRequest>,
    ) -> Result<Response<pb::RunReviewGitResponse>, Status> {
        require_host(&ports::caller(&request)?)?;
        let request = request.into_inner();
        let session = session_of(request.session)?;
        Ok(Response::new(
            self.run_review_git(session, request.repo, request.args, request.stdin).await?,
        ))
    }

    async fn target(
        &self,
        request: Request<pb::CarryTargetRequest>,
    ) -> Result<Response<pb::CarryTargetResponse>, Status> {
        require_host(&ports::caller(&request)?)?;
        let request = request.into_inner();
        let folder = self.target(&channel_of(request.channel)?, &request.folder).await?;
        Ok(Response::new(pb::CarryTargetResponse { folder }))
    }

    async fn unpack(
        &self,
        request: Request<pb::CarryUnpackRequest>,
    ) -> Result<Response<pb::CarryUnpackResponse>, Status> {
        require_host(&ports::caller(&request)?)?;
        let request = request.into_inner();
        let unpacked = self
            .unpack(
                &operation_of(request.operation)?,
                &session_of(request.session)?,
                &channel_of(request.channel)?,
            )
            .await?;
        Ok(Response::new(pb::CarryUnpackResponse {
            files: unpacked.files,
            bytes: unpacked.bytes,
        }))
    }

    async fn release(
        &self,
        request: Request<pb::CarryReleaseRequest>,
    ) -> Result<Response<pb::CarryReleaseResponse>, Status> {
        require_host(&ports::caller(&request)?)?;
        let request = request.into_inner();
        let session = session_of(request.session)?;
        // No record: an earlier release of this move already trashed the
        // copy and forgot the session, and the host asks again after a
        // restart. Nothing is left to release.
        let Some(channel) = self.channel_if_any(&session).await? else {
            return Ok(Response::new(pb::CarryReleaseResponse { trashed: Vec::new() }));
        };
        let trashed = self
            .release(&operation_of(request.operation)?, &session, &channel)
            .await?;
        Ok(Response::new(pb::CarryReleaseResponse { trashed }))
    }

    async fn abort(&self, request: Request<pb::CarryAbortRequest>) -> Result<Response<pb::CarryAbortResponse>, Status> {
        require_host(&ports::caller(&request)?)?;
        let request = request.into_inner();
        self.abort(
            &operation_of(request.operation)?,
            &session_of(request.session)?,
            &channel_of(request.channel)?,
            &request.folder,
            request.incoming,
        )
        .await?;
        Ok(Response::new(pb::CarryAbortResponse {}))
    }
}

impl SessionCarrySvc {
    /// The channel of `session`'s record here; None when there is none.
    async fn channel_if_any(&self, session: &str) -> Result<Option<String>, Status> {
        let (store, id) = (self.starter.store.clone(), session.to_owned());
        match blocking(move || store.session(&id)).await? {
            // A Desk session, a channel's own too, is carried through the
            // channels folder itself (MV-17).
            Ok(record) => Ok(record.map(|record| {
                if record.channel == DESK_CHANNEL {
                    CHANNELS_ROOT.to_owned()
                } else {
                    record.channel
                }
            })),
            Err(source) => Err(failed(CarryError::Store {
                doing: "read the session",
                source,
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_host_may_call_the_carry_service() {
        let refused = [
            ("the app", Caller::User(UserVia::App)),
            (
                "a cli login",
                Caller::User(UserVia::CliLogin {
                    login: "login_1".to_owned(),
                    method: humata2_access::PresenceMethod::CliLogin,
                    on_host: true,
                }),
            ),
            ("the peer link", Caller::Peer),
        ];
        for (who, caller) in refused {
            let denied = require_host(&caller).expect_err(who);
            assert_eq!(denied.code(), Code::PermissionDenied, "{who}");
        }
        assert!(require_host(&Caller::User(UserVia::Host)).is_ok());
    }

    #[test]
    fn a_copy_whose_agent_runs_here_reads_as_already_there() {
        let refused = failed(CarryError::Running {
            session: "se_1".to_owned(),
        });
        assert_eq!(refused.code(), Code::AlreadyExists);
        let other = failed(CarryError::NoSession {
            session: "se_1".to_owned(),
        });
        assert_eq!(other.code(), Code::FailedPrecondition);
    }
}
