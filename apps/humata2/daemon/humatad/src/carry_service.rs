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

use crate::identity::{self, Caller, UserVia};
use crate::status;
use crate::trash_service::{Bin, blocking, now_ms};

mod review_git;

pub(crate) struct SessionCarrySvc {
    starter: Starter,
    bin: Arc<Bin>,
    /// What this machine takes in when a session arrives: the host already
    /// holds the session's record and place in the tree, a box doesn't.
    take: Take,
}

impl SessionCarrySvc {
    pub(crate) fn new(starter: Starter, bin: Arc<Bin>, host: bool) -> Arc<Self> {
        Arc::new(Self {
            starter,
            bin,
            take: if host { Take::AgentOnly } else { Take::Everything },
        })
    }

    /// Stages `session` for a move (see [`Starter::pack_for`]). `target` is
    /// the folder the session will have on the new machine, empty when the
    /// conversation is to travel as it is.
    pub(crate) async fn pack(
        &self,
        operation: &str,
        session: &str,
        target: &str,
    ) -> Result<humata2_sessions::Packed, Status> {
        let (starter, operation, session, target) =
            (self.starter.clone(), operation.to_owned(), session.to_owned(), target.to_owned());
        blocking(move || {
            let target = (!target.is_empty()).then(|| std::path::PathBuf::from(&target));
            starter.pack_for(&operation, &session, target.as_deref())
        })
        .await?
        .map_err(failed)
    }

    /// Where the session folder `folder` of `channel` will be here (see
    /// [`Starter::carry_target`]).
    pub(crate) async fn target(&self, channel: &str, folder: &str) -> Result<String, Status> {
        let (starter, channel, folder) = (self.starter.clone(), channel.to_owned(), folder.to_owned());
        blocking(move || starter.carry_target(&channel, &folder))
            .await?
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(failed)
    }

    /// Stages a fork of `parent` for another machine (see [`Starter::pack_fork`]).
    pub(crate) async fn pack_fork(
        &self,
        operation: &str,
        parent: &str,
        fork: &pb::CarryFork,
    ) -> Result<pb::CarryPackResponse, Status> {
        let pack = fork_pack_of(operation, parent, fork)?;
        let starter = self.starter.clone();
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
    pub(crate) async fn preview_fork(
        &self,
        parent: &str,
        title: &str,
        typed: &str,
    ) -> Result<pb::CarryPreviewForkResponse, Status> {
        let (starter, parent, title) = (self.starter.clone(), parent.to_owned(), title.to_owned());
        let typed = Some(typed.to_owned()).filter(|typed| !typed.is_empty());
        let plan = blocking(move || starter.preview_fork(&parent, &title, typed.as_deref()))
            .await?
            .map_err(|error| {
                let (code, failure) = crate::session_service::start_error(&error);
                status::failure(code, failure)
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
    pub(crate) async fn unpack_new(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
    ) -> Result<humata2_sessions::Unpacked, Status> {
        let (starter, operation, session, channel) = (
            self.starter.clone(),
            operation.to_owned(),
            session.to_owned(),
            channel.to_owned(),
        );
        blocking(move || starter.unpack(&operation, &session, &channel, Take::Everything))
            .await?
            .map_err(failed)
    }

    /// Takes back a new session a carry put on this machine, records included.
    pub(crate) async fn abort_new(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        folder: &str,
    ) -> Result<(), Status> {
        let (starter, operation, session, channel, folder) = (
            self.starter.clone(),
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
    pub(crate) async fn unpack(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
    ) -> Result<humata2_sessions::Unpacked, Status> {
        let (starter, operation, session, channel, take) = (
            self.starter.clone(),
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
    pub(crate) async fn release(&self, operation: &str, session: &str, channel: &str) -> Result<Vec<String>, Status> {
        // The session's shells run on its new machine now (MV-13); here they stop before their tabs are forgotten.
        let holders = {
            let (starter, session) = (self.starter.clone(), session.to_owned());
            blocking(move || starter.shell_holders(&session)).await?.map_err(failed)?
        };
        for (socket, pid) in holders {
            let owner = format!("a shell of session {session} it left");
            if let Err(error) = crate::holder_end::shut_or_end(&socket, pid, &owner).await {
                crate::log::say!(
                    "humatad2: stopping a shell of session {session} it left: {}; that shell may still be running here, and nothing in Humata shows it any more",
                    status::chain(&error)
                );
            }
        }
        let (starter, bin, operation, session, channel, take) = (
            self.starter.clone(),
            self.bin.clone(),
            operation.to_owned(),
            session.to_owned(),
            channel.to_owned(),
            self.take,
        );
        blocking(move || -> Result<Vec<String>, Status> {
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
                            status::chain(&error)
                        ),
                        "the session already runs on its new machine; free space on this disk or fix the folder's permissions, then delete the old copy from the trash page",
                    )
                })?;
                trashed.push(discard.what);
            }
            // A cut here: the copy is in the trash, its records are still
            // here, and the host asks again.
            crate::fault::point("carry.trashed");
            starter.forget_moved(&operation, &session, &channel, take).map_err(failed)?;
            crate::fault::point("carry.forgotten");
            Ok(trashed)
        })
        .await?
    }

    /// Takes back what a move that didn't switch put on this machine
    /// (`incoming`), or removes what it staged here (the machine it left).
    pub(crate) async fn abort(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        folder: &str,
        incoming: bool,
    ) -> Result<(), Status> {
        let (starter, operation, session, channel, folder, take) = (
            self.starter.clone(),
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
    status::failure(
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
pub(crate) fn failed(error: CarryError) -> Status {
    // CV-3, CV-7: the agent's conversation can't be carried: no adapter
    // reads it, or its adapter doesn't know its records or version.
    if let CarryError::Conversation { agent, .. } = &error {
        return status::failure(
            Code::FailedPrecondition,
            status::error(
                status::chain(&error),
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
        status::chain(&error),
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
        status::chain(&error),
        "the parent is unchanged and still running; fix what the message names, then fork again",
    )
}

/// Only the host calls this service.
fn require_host(caller: &Caller) -> Result<(), Status> {
    if matches!(caller, Caller::User(UserVia::Host)) {
        return Ok(());
    }
    Err(status::failure(
        Code::PermissionDenied,
        status::error(
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
        .ok_or_else(|| status::missing("session"))
}

fn channel_of(id: Option<pb::ChannelId>) -> Result<String, Status> {
    id.map(|id| id.value)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| status::missing("channel"))
}

fn operation_of(operation: String) -> Result<String, Status> {
    if operation.is_empty() || !operation.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
        return Err(status::missing("operation"));
    }
    Ok(operation)
}

#[tonic::async_trait]
impl pb::session_carry_service_server::SessionCarryService for SessionCarrySvc {
    async fn preview_fork(
        &self,
        request: Request<pb::CarryPreviewForkRequest>,
    ) -> Result<Response<pb::CarryPreviewForkResponse>, Status> {
        require_host(&identity::caller(&request)?)?;
        let request = request.into_inner();
        let session = session_of(request.session)?;
        Ok(Response::new(self.preview_fork(&session, &request.title, &request.branch).await?))
    }

    async fn pack(&self, request: Request<pb::CarryPackRequest>) -> Result<Response<pb::CarryPackResponse>, Status> {
        require_host(&identity::caller(&request)?)?;
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
        require_host(&identity::caller(&request)?)?;
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
        require_host(&identity::caller(&request)?)?;
        let request = request.into_inner();
        let folder = self.target(&channel_of(request.channel)?, &request.folder).await?;
        Ok(Response::new(pb::CarryTargetResponse { folder }))
    }

    async fn unpack(
        &self,
        request: Request<pb::CarryUnpackRequest>,
    ) -> Result<Response<pb::CarryUnpackResponse>, Status> {
        require_host(&identity::caller(&request)?)?;
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
        require_host(&identity::caller(&request)?)?;
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
        require_host(&identity::caller(&request)?)?;
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
    use crate::identity::UserVia;

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
