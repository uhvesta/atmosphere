//! Continue in another channel… of a session that runs on another machine
//! (CV-8). The conversation is on the session's machine, so the continue
//! runs there: the host relays ContinueInChannel to that machine, which
//! continues it as it continues any of its own sessions, into a channel the
//! machine is in. The host follows the machine's operation to its end, then
//! writes its own record of the new session, on that machine, so lists and
//! access see it. The session it came from is never touched. A channel the
//! machine isn't in is refused before anything is asked: its conversation
//! would have to move machines too.

pub mod kept;

use std::path::{Path, PathBuf};

use humata2_access::{AgentId, Call, Capability, ChannelId, MachineId, Target};
use humata2_daemon_ports::Caller;
use humata2_rpc_kit::{Gated, blocking, chain, error_of, failed, failure, now_ms};
use humata2_store::{ChannelRecord, MainClone, SessionDetail, SessionRecord, SessionRepo, StartStep};
use humata2_svc_sessions_common::pb;
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use crate::SessionTransferSvc;
use crate::ports::Checked;
use kept::Kept;

/// How the restart report names a continue on another machine.
pub const KIND: &str = "remote_continue";
pub const STEP: &str = "asked";

/// What the restart report says of a journal that couldn't be read: what
/// failed, where the file went, and what to do. The continue is never
/// asked again from it: the machine may already have made the session.
pub fn unreadable_reason(bad: &kept::Unreadable) -> String {
    let file = match &bad.set_aside {
        Ok(path) => format!("the file was moved to {} so no start tries it again", path.display()),
        Err(error) => format!(
            "the file couldn't be moved aside ({}), so the next start meets it again; move it out of the folder by hand",
            chain(error)
        ),
    };
    format!(
        "{}; {file}. Nothing was asked of the other machine again, so nothing was sent twice, but the new session may already exist there: look in the channel on that machine, and continue the session again if it isn't there. Delete the moved file once you have looked",
        chain(&bad.problem)
    )
}

#[derive(Debug, thiserror::Error)]
pub enum ThereError {
    #[error("session {session} runs on {machine}, which channel {channel} isn't on, so its conversation can't be continued there")]
    NotInChannel {
        session: String,
        machine: String,
        channel: String,
    },
    #[error("session {session} runs on {machine}, and this daemon has no links to other machines")]
    NoLinks { session: String, machine: String },
    #[error("the link to {machine} was lost while it continued the session, so it isn't known yet whether it finished")]
    LinkLost { machine: String },
    #[error("{machine} ended the continue without saying how it went")]
    NoAnswer { machine: String },
    #[error("{machine} said the continue succeeded but didn't name the session it made")]
    NoSession { machine: String },
    #[error("couldn't keep the continue on {machine} on disk before asking {machine} for it")]
    Keep {
        machine: String,
        #[source]
        source: kept::KeptError,
    },
}

/// How a continue cut short ended once its machine was back.
#[derive(Debug)]
pub enum Recovered {
    /// The host recorded the session the machine made.
    Listed,
    /// The machine made nothing, and why.
    Refused { reason: String },
    /// The machine was forgotten first; its sessions went with it.
    Forgotten,
}

/// The steps of a continue into `channel`, as `(key, title)`: the
/// machine's own, which the host's operation shows as they go.
pub fn steps(channel: &ChannelRecord) -> Vec<(String, String)> {
    let mut steps = vec![("write_record".to_owned(), "Saving the session".to_owned())];
    for repo in channel.repos.iter().filter(|repo| repo.main == MainClone::Ready) {
        steps.push((format!("clone:{}", repo.name), format!("Cloning {}", repo.name)));
    }
    for (key, title) in [
        ("copy_conversation", "Copying the conversation"),
        ("launch", "Starting the agent"),
        ("verify", "Checking it started"),
    ] {
        steps.push((key.to_owned(), title.to_owned()));
    }
    steps
}

/// The session an operation's result names, once it ended with one.
pub fn result_of(operation: &pb::Operation) -> Option<String> {
    match &operation.result {
        Some(pb::operation::Result::Session(id)) if !id.value.is_empty() => Some(id.value.clone()),
        _ => None,
    }
}

/// The new session's id from the machine's ended operation, or why it has
/// none.
pub fn made_by(machine: &str, ended: pb::Operation) -> Result<String, Status> {
    if ended.state != pb::OperationState::Succeeded as i32 {
        let error = ended.error.unwrap_or_else(|| {
            error_of(
                &ThereError::NoAnswer {
                    machine: machine.to_owned(),
                },
                "look at the session on its machine, then continue it again",
                None,
            )
        });
        return Err(failure(Code::FailedPrecondition, error));
    }
    match ended.result {
        Some(pb::operation::Result::Session(id)) if !id.value.is_empty() => Ok(id.value),
        _ => Err(failed(
            Code::Internal,
            &ThereError::NoSession {
                machine: machine.to_owned(),
            },
            "look at the machine's sessions: the new session may run there without this Mac listing it",
            None,
        )),
    }
}

/// A continue the host asked a machine for.
#[derive(Clone, Debug)]
pub struct There {
    /// The source session's machine, where the continue runs.
    pub machine: String,
    /// The channel the new session is in.
    pub channel: String,
    /// The ask, under the host operation's key.
    pub request: pb::ContinueInChannelRequest,
    /// The host's operation.
    pub operation: String,
    /// The session the machine named when it answered; empty before.
    pub session: String,
}

impl There {
    /// What the journal keeps of it.
    pub fn kept(&self) -> Kept {
        Kept {
            operation: self.operation.clone(),
            machine: self.machine.clone(),
            channel: self.channel.clone(),
            request: self.request.clone(),
            session: self.session.clone(),
        }
    }

    /// The continue a journal the last run left names.
    pub fn from_kept(kept: Kept) -> Self {
        Self {
            machine: kept.machine,
            channel: kept.channel,
            request: kept.request,
            operation: kept.operation,
            session: kept.session,
        }
    }
}

impl SessionTransferSvc {
    /// ContinueInChannel of `source`, a session on another machine, into
    /// `channel`: checked here, then made on the session's machine.
    pub async fn handle_continue_there(
        &self,
        caller: &Caller,
        source: &SessionRecord,
        channel: &ChannelRecord,
        _request: pb::ContinueInChannelRequest,
        approved: bool,
    ) -> Result<Gated<pb::StartSessionResponse>, Status> {
        let machine = source.machine.clone();
        if !channel.machines.iter().any(|listed| listed == &machine) {
            return Err(failed(
                Code::FailedPrecondition,
                &ThereError::NotInChannel {
                    session: source.id.clone(),
                    machine,
                    channel: channel.name.clone(),
                },
                "add the session's machine to that channel first, or pick a channel the machine is in; nothing was changed",
                Some(pb::error::Kind::InvalidField(pb::InvalidField {
                    field: "channel".to_owned(),
                    problem: Problem::NotAllowedHere as i32,
                })),
            ));
        }
        if !approved {
            for gated in [&source.channel, &channel.id] {
                if let Checked::Ask(asked) = self.env.check_capability(
                    caller,
                    Capability::SessionStartOtherMachine,
                    Target::Start {
                        channel: ChannelId::new(gated.as_str()),
                        machine: MachineId::new(machine.as_str()),
                        agent: AgentId::new(source.agent.as_str()),
                        fork_of: Some(humata2_access::SessionId::new(source.id.as_str())),
                    },
                )? {
                    return Ok(Gated::Ask(Box::new(asked)));
                }
            }
        }
        Err(self.env.not_wired("SessionService.ContinueInChannel (remote)"))
    }

    /// The host's record of `session`, which the machine made and shows as
    /// `shown`: on that machine, in its folder there. Written once.
    pub async fn record_there(&self, there: &There, session: &str, shown: pb::Session) -> Result<(), Status> {
        let repos: Vec<SessionRepo> = shown
            .repos
            .iter()
            .map(|repo| SessionRepo {
                owner: repo.repo.as_ref().map(|named| named.owner.clone()).unwrap_or_default(),
                name: repo.repo.as_ref().map(|named| named.name.clone()).unwrap_or_default(),
                branch: repo.branch.clone(),
                path: PathBuf::from(&repo.path),
            })
            .collect();
        // A session's folder holds its clones, one per repository.
        let folder = repos
            .first()
            .and_then(|repo| repo.path.parent())
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let record = SessionRecord {
            id: session.to_owned(),
            channel: shown
                .channel
                .map(|channel| channel.value)
                .filter(|channel| !channel.is_empty())
                .unwrap_or_else(|| there.channel.clone()),
            machine: there.machine.clone(),
            title: shown.title,
            agent: shown.agent.map(|agent| agent.value).unwrap_or_default(),
            folder,
            created_unix_ms: now_ms(),
        };
        let detail = SessionDetail {
            session: session.to_owned(),
            repos,
            model: Some(shown.model).filter(|model| !model.is_empty()),
            first_message: None,
            start: StartStep::Done,
            operation: Some(there.operation.clone()),
            revision: 1,
        };
        let store = self.core.store.clone();
        blocking(move || -> Result<(), humata2_store::StoreError> {
            if store.session(&record.id)?.is_some() {
                return Ok(());
            }
            store.put_session_with_detail(&record, &detail)
        })
        .await?
        .map_err(|error| {
            failed(
                Code::Internal,
                &error,
                "save the continued session",
                None,
            )
        })
    }

    /// Keeps `there` on disk, the folder's entry included, so a restart
    /// goes on with it.
    pub async fn keep_continue(&self, there: &There) -> Result<(), Status> {
        let folder = kept::folder_of(&self.env.state_dir().join("remote_archive"));
        let kept = there.kept();
        blocking(move || kept::write(&folder, &kept)).await?.map_err(|source| {
            failed(
                Code::Internal,
                &ThereError::Keep {
                    machine: there.machine.clone(),
                    source,
                },
                "check that the data folder's disk isn't full and that the folder belongs to you, then continue the session again; nothing was asked of the machine",
                None,
            )
        })
    }

    /// Removes the journal for `there`.
    pub async fn settle_continue(&self, there: &There) {
        let folder = kept::folder_of(&self.env.state_dir().join("remote_archive"));
        let operation = there.operation.clone();
        let _ = blocking(move || kept::remove(&folder, &operation)).await;
    }
}
