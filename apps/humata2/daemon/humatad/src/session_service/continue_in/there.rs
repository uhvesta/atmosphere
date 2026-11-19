//! Continue in another channel… of a session that runs on another machine
//! (CV-8). The conversation is on the session's machine, so the continue
//! runs there: the host relays ContinueInChannel to that machine, which
//! continues it as it continues any of its own sessions, into a channel the
//! machine is in. The host follows the machine's operation to its end, then
//! writes its own record of the new session, on that machine, so lists and
//! access see it. The session it came from is never touched. A channel the
//! machine isn't in is refused before anything is asked: its conversation
//! would have to move machines too.
//!
//! The machine is asked under a key made from the host's operation, and the
//! ask is kept on disk before it is made (`kept`, D554). A link lost before
//! the end, and a restart of the host while the link is down, are finished
//! once the machine's link is up again: the host asks again under the same
//! key, the machine answers with the continue it already made, and the host
//! writes its records once, naming its operation. Until then the host lists
//! nothing of it; a replay of the caller's key while the continue runs
//! answers with the session the machine named.
//!
//! Nothing here waits on a timer: the host follows the machine's operation
//! and the machine's link state.

mod kept;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata2_access::{AgentId, Capability, ChannelId, MachineId, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, MainClone, SessionDetail, SessionRecord, SessionRepo, StartStep};
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use super::super::archive_remote::{finished, follow_to_end, machine_failed};
use super::super::{SessionSvc, now_ms};
use crate::carry::Carry;
use crate::channel_service::{Gated, blocking, error_of, failed};
use crate::host_relay::{LinkState, Relayed, assigned};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::restart::OperationState;
use crate::status;
use kept::Kept;

/// How the restart report names a continue on another machine.
const KIND: &str = "remote_continue";
const STEP: &str = "asked";

/// What the restart report says of a journal that couldn't be read: what
/// failed, where the file went, and what to do. The continue is never
/// asked again from it: the machine may already have made the session.
fn unreadable_reason(bad: &kept::Unreadable) -> String {
    let file = match &bad.set_aside {
        Ok(path) => format!("the file was moved to {} so no start tries it again", path.display()),
        Err(error) => format!(
            "the file couldn't be moved aside ({}), so the next start meets it again; move it out of the folder by hand",
            status::chain(error)
        ),
    };
    format!(
        "{}; {file}. Nothing was asked of the other machine again, so nothing was sent twice, but the new session may already exist there: look in the channel on that machine, and continue the session again if it isn't there. Delete the moved file once you have looked",
        status::chain(&bad.problem)
    )
}

#[derive(Debug, thiserror::Error)]
enum ThereError {
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
enum Recovered {
    /// The host recorded the session the machine made.
    Listed,
    /// The machine made nothing, and why.
    Refused { reason: String },
    /// The machine was forgotten first; its sessions went with it.
    Forgotten,
}

/// How asking the machine, or following its operation, went wrong.
enum Asked {
    /// The link went before the machine answered: it may have the continue.
    Lost(Status),
    /// The machine answered with a refusal: it made nothing from this ask.
    Refused(Status),
}

/// `source`, a relayed call's failure, sorted into a lost link or the
/// machine's own answer.
fn asked_error(machine: &str, source: Status) -> Asked {
    let lost = status::error_of(&source).is_none()
        && matches!(source.code(), Code::Unavailable | Code::Unknown | Code::Cancelled);
    let status = machine_failed(machine, source);
    if lost { Asked::Lost(status) } else { Asked::Refused(status) }
}

/// The failure an operation records for `status`.
fn failure_of(status: &Status) -> pb::Error {
    status::error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: String::new(),
        retry: None,
        kind: None,
    })
}

fn no_answer(machine: &str) -> Status {
    failed(
        Code::Internal,
        &ThereError::NoAnswer {
            machine: machine.to_owned(),
        },
        "look at the machine's sessions: the new session may run there without this Mac listing it",
        None,
    )
}

/// The steps of a continue into `channel`, as `(key, title)`: the
/// machine's own, which the host's operation shows as they go.
fn steps(channel: &ChannelRecord) -> Vec<(String, String)> {
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
fn result_of(operation: &pb::Operation) -> Option<String> {
    match &operation.result {
        Some(pb::operation::Result::Session(id)) if !id.value.is_empty() => Some(id.value.clone()),
        _ => None,
    }
}

/// The new session's id from the machine's ended operation, or why it has
/// none.
fn made_by(machine: &str, ended: pb::Operation) -> Result<String, Status> {
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
        return Err(status::failure(Code::FailedPrecondition, error));
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

/// Asks the machine for the continue.
async fn ask_machine(
    relayed: &Relayed,
    machine: &str,
    request: pb::ContinueInChannelRequest,
) -> Result<pb::StartSessionResponse, Asked> {
    let call = assigned(request, None).map_err(|error| Asked::Refused(Status::from(error)))?;
    relayed
        .session()
        .continue_in_channel(call)
        .await
        .map(tonic::Response::into_inner)
        .map_err(|source| asked_error(machine, source))
}

/// Follows the machine's operation `first` to its end, with no host
/// operation to show it on.
async fn watch_end(relayed: &Relayed, machine: &str, first: pb::Operation) -> Result<pb::Operation, Asked> {
    if finished(&first) {
        return Ok(first);
    }
    let id = first.id.map(|id| id.value).unwrap_or_default();
    let mut stream = relayed
        .operation()
        .watch_operation(pb::WatchOperationRequest {
            id: Some(pb::OperationId { value: id }),
            from_version: 0,
        })
        .await
        .map_err(|source| asked_error(machine, source))?
        .into_inner();
    loop {
        match stream.message().await {
            Ok(Some(operation)) if finished(&operation) => return Ok(operation),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => {
                return Err(Asked::Lost(Status::from(crate::host_relay::RelayError::Offline {
                    machine: machine.to_owned(),
                })));
            }
        }
    }
}

/// A continue the host asked a machine for.
struct There {
    carry: Carry,
    /// The source session's machine, where the continue runs.
    machine: String,
    /// The channel the new session is in.
    channel: String,
    /// The ask, under the host operation's key.
    request: pb::ContinueInChannelRequest,
    /// The host's operation.
    operation: String,
    /// The session the machine named when it answered; empty before.
    session: String,
}

impl There {
    /// What the journal keeps of it.
    fn kept(&self) -> Kept {
        Kept {
            operation: self.operation.clone(),
            machine: self.machine.clone(),
            channel: self.channel.clone(),
            request: self.request.clone(),
            session: self.session.clone(),
        }
    }

    /// The continue a journal the last run left names.
    fn from_kept(carry: Carry, kept: Kept) -> Self {
        Self {
            carry,
            machine: kept.machine,
            channel: kept.channel,
            request: kept.request,
            operation: kept.operation,
            session: kept.session,
        }
    }
}

impl SessionSvc {
    /// ContinueInChannel of `source`, a session on another machine, into
    /// `channel`: checked here, then made on the session's machine. A
    /// machine that refuses the continue answers the caller.
    pub(super) async fn continue_there(
        &self,
        caller: &Caller,
        source: &SessionRecord,
        channel: &ChannelRecord,
        request: pb::ContinueInChannelRequest,
        approved: bool,
    ) -> Result<Gated<pb::StartSessionResponse>, Status> {
        let machine = source.machine.clone();
        let Some(carry) = self.carry.get().cloned() else {
            return Err(failed(
                Code::FailedPrecondition,
                &ThereError::NoLinks {
                    session: source.id.clone(),
                    machine,
                },
                "continue the session from the Mac its machine is linked to",
                None,
            ));
        };
        // UP-1, D281 e: nothing runs on a machine at another release.
        crate::upgrade::gate::machine_ready(&self.migrations, &pb::MachineId { value: machine.clone() })?;
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
        for gated in [&source.channel, &channel.id] {
            if let Some(asked) = self.gate(
                caller,
                Capability::SessionStartOtherMachine,
                Target::Start {
                    channel: ChannelId::new(gated.as_str()),
                    machine: MachineId::new(machine.as_str()),
                    agent: AgentId::new(source.agent.as_str()),
                    fork_of: Some(humata2_access::SessionId::new(source.id.as_str())),
                },
                approved,
            )? {
                return Ok(Gated::Ask(asked));
            }
        }
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(self.answer_of(operation).await));
        }
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                value: channel.id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::ContinueInChannel,
            target,
            &steps(channel),
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(self.answer_of(operation).await)),
        };
        let operation = progress.operation();
        let host_operation = operation.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let relayed = match super::super::move_recover::relayed_provider(carry.links(), &machine) {
            Ok(relayed) => relayed,
            Err(error) => {
                let status = Status::from(error);
                progress.fail(failure_of(&status));
                return Err(status);
            }
        };
        let mut there = There {
            carry,
            machine,
            channel: channel.id.clone(),
            request: pb::ContinueInChannelRequest {
                idempotency_key: Some(pb::IdempotencyKey {
                    value: format!("host-continue-{host_operation}"),
                }),
                ..request
            },
            operation: host_operation,
            session: String::new(),
        };
        // Kept before it is asked: from here the machine may have it, and a
        // restart asks it again under the same key. A continue that can't
        // be kept isn't asked for.
        if let Err(status) = self.keep(&there).await {
            progress.fail(failure_of(&status));
            return Err(status);
        }
        // A cut here: kept, not asked. The next start asks once the machine
        // links, so the continue is made whole or not at all.
        crate::fault::point("continue-there-journaled");
        let first = ask_machine(&relayed, &there.machine, there.request.clone()).await;
        // A cut here: the machine may be continuing, and the host has no
        // record of it; the next start asks again under the same key.
        crate::fault::point("continue-there-asked");
        let answer = match first {
            Ok(answer) => answer,
            Err(Asked::Refused(status)) => {
                self.settle(&there).await;
                progress.fail(failure_of(&status));
                return Err(status);
            }
            Err(Asked::Lost(status)) => {
                progress.fail(failure_of(&status));
                self.continue_when_linked(there, None);
                return Err(status);
            }
        };
        let Some(first) = answer.operation else {
            self.settle(&there).await;
            let status = no_answer(&there.machine);
            progress.fail(failure_of(&status));
            return Err(status);
        };
        // From here a replay of the caller's key names the session.
        there.session = answer.session.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        if !there.session.is_empty()
            && let Err(status) = self.keep(&there).await
        {
            crate::log::say!(
                "humatad2: {}; until the continue ends, a replay of its key names no session",
                status.message()
            );
        }
        tokio::spawn(self.clone().finish_there(relayed, there, first, progress, identity::actor(caller)));
        Ok(Gated::Done(pb::StartSessionResponse {
            session: answer.session,
            operation: Some(operation),
        }))
    }

    /// Follows the machine's continue to its end, then writes the host's
    /// records of the new session and ends `progress`. A link lost before
    /// the end fails `progress`, and the continue is finished once the link
    /// is back.
    async fn finish_there(self, relayed: Relayed, there: There, first: pb::Operation, progress: Progress, by: pb::Actor) {
        // Held here in tests, so a link can be cut while the machine continues.
        crate::fault::hold_point("continue-there-following").await;
        let machine = there.machine.clone();
        let lost = move || {
            failed(
                Code::Unavailable,
                &ThereError::LinkLost { machine },
                "reconnect the machine: once its link is up this Mac lists the new session if the machine made it",
                None,
            )
        };
        let ended = match follow_to_end(&relayed, &there.machine, first, &progress, lost).await {
            Ok(ended) => ended,
            Err(status) => {
                progress.fail(failure_of(&status));
                self.continue_when_linked(there, None);
                return;
            }
        };
        match self.conclude_there(&relayed, &there, ended, Some(by)).await {
            Ok(session) => progress.succeed(Some(pb::operation::Result::Session(pb::SessionId { value: session }))),
            // The link went while the session was read: finish once it's back.
            Err(status) if status.code() == Code::Unavailable => {
                progress.fail(failure_of(&status));
                self.continue_when_linked(there, None);
            }
            Err(status) => progress.fail(failure_of(&status)),
        }
    }

    /// The machine's continue `ended`: a session it made gets the host's
    /// records, read from the machine. The journal goes once they're
    /// written, or when the machine made nothing; a record that can't be
    /// read or written keeps it, so the continue is finished later.
    async fn conclude_there(
        &self,
        relayed: &Relayed,
        there: &There,
        ended: pb::Operation,
        by: Option<pb::Actor>,
    ) -> Result<String, Status> {
        let session = match made_by(&there.machine, ended) {
            Ok(session) => session,
            Err(status) => {
                // The machine made nothing: there's nothing to ask again.
                self.settle(there).await;
                return Err(status);
            }
        };
        let asked = assigned(
            pb::GetSessionRequest {
                id: Some(pb::SessionId { value: session.clone() }),
            },
            None,
        )
        .map_err(Status::from)?;
        let shown = relayed
            .session()
            .get_session(asked)
            .await
            .map_err(|source| machine_failed(&there.machine, source))?
            .into_inner();
        self.record_there(there, &session, shown).await?;
        // Recorded: a restart from here has nothing to ask again.
        self.settle(there).await;
        self.tell.session(&session, pb::Change::Created, by).await;
        Ok(session)
    }

    /// The host's record of `session`, which the machine made and shows as
    /// `shown`: on that machine, in its folder there. Written once. Its
    /// start names the host's operation, as a local start's does, so a
    /// replay of the caller's key finds the session (D554).
    async fn record_there(&self, there: &There, session: &str, shown: pb::Session) -> Result<(), Status> {
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
        let store = self.starter.store.clone();
        blocking(move || -> Result<(), humata2_store::StoreError> {
            if store.session(&record.id)?.is_some() {
                return Ok(());
            }
            store.put_session_with_detail(&record, &detail)
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "save the continued session"))
    }

    /// The answer to a continue already asked for, from its operation:
    /// the session it made, or, while it runs, the one the machine named.
    async fn answer_of(&self, operation: pb::Operation) -> pb::StartSessionResponse {
        let session = match result_of(&operation) {
            Some(session) => Some(session),
            None => {
                let id = operation.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
                self.named_by(&id).await
            }
        };
        pb::StartSessionResponse {
            session: session.map(|value| pb::SessionId { value }),
            operation: Some(operation),
        }
    }

    /// The session continue `operation` makes: the host's record names its
    /// operation once written; before that its journal holds the session
    /// the machine named. None when neither knows one yet.
    async fn named_by(&self, operation: &str) -> Option<String> {
        let folder = self.carry.get().map(|carry| kept::folder_of(&carry.remote_archive_folder()));
        let (store, operation) = (self.starter.store.clone(), operation.to_owned());
        blocking(move || {
            let recorded = store.session_details().ok().and_then(|details| {
                details
                    .into_iter()
                    .find(|detail| detail.operation.as_deref() == Some(operation.as_str()))
                    .map(|detail| detail.session)
            });
            recorded.or_else(|| {
                folder
                    .and_then(|folder| kept::read(&folder, &operation))
                    .map(|kept| kept.session)
                    .filter(|session| !session.is_empty())
            })
        })
        .await
        .ok()
        .flatten()
    }

    /// Keeps `there` on disk, the folder's entry included, so a restart
    /// goes on with it.
    async fn keep(&self, there: &There) -> Result<(), Status> {
        let (folder, kept) = (kept::folder_of(&there.carry.remote_archive_folder()), there.kept());
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

    /// The continue `there` is recorded, or the machine made nothing: its
    /// journal goes. One that can't be removed is asked about again by the
    /// next start, which finds the record already written and lists
    /// nothing twice.
    async fn settle(&self, there: &There) {
        let folder = kept::folder_of(&there.carry.remote_archive_folder());
        let operation = there.operation.clone();
        if let Ok(Err(error)) = blocking(move || kept::remove(&folder, &operation)).await {
            crate::log::say!(
                "humatad2: {}; the next start asks {} about it again and lists nothing twice",
                status::chain(&error),
                there.machine
            );
        }
    }

    /// The continues on other machines the last run journaled and didn't
    /// record: each is asked again under its key once the start is done
    /// and its machine links, and recorded once (D554). Call it once the
    /// carry is set, before anything is served.
    pub(in crate::session_service) fn recover_remote_continues_after(&self, started: Arc<crate::restart::Started>) {
        let Some(carry) = self.carry.get().cloned() else {
            return;
        };
        let found = kept::read_all(&kept::folder_of(&carry.remote_archive_folder()));
        for bad in &found.unreadable {
            // Ends it in the report, once: nothing is asked of the machine
            // again, and the file is set aside so no start meets it again.
            let reason = unreadable_reason(bad);
            crate::log::say!("humatad2: the continue {} can't be asked again: {reason}", bad.operation);
            started
                .operations()
                .record(&bad.operation, KIND, STEP, OperationState::Stuck { reason });
        }
        for kept in found.kept {
            self.continue_when_linked(There::from_kept(carry.clone(), kept), Some(started.clone()));
        }
    }

    /// Finishes the continue `there` names once its machine's link is up
    /// (after `started`, when a restart found it): asks the machine again
    /// under the same key and writes the host's records when it ends. A
    /// machine that refuses, or is forgotten, ends it with nothing listed.
    /// Its journal keeps it until then, so a restart goes on with it.
    fn continue_when_linked(&self, there: There, started: Option<Arc<crate::restart::Started>>) {
        let svc = self.clone();
        let reports = started.as_ref().map(|started| started.operations().clone());
        if let Some(reports) = &reports {
            reports.record(
                &there.operation,
                KIND,
                STEP,
                OperationState::Pending {
                    waits_for: format!("the start to finish, then the link of {}", there.machine),
                },
            );
        }
        tokio::spawn(async move {
            if let Some(started) = started {
                started.wait().await;
            }
            let (operation, machine) = (there.operation.clone(), there.machine.clone());
            let outcome = svc.recover_there(there).await;
            let state = match &outcome {
                Ok(Recovered::Listed) => OperationState::Finished,
                Ok(Recovered::Refused { reason }) => OperationState::Undone {
                    reason: format!("{machine} made nothing: {reason}"),
                },
                Ok(Recovered::Forgotten) => OperationState::Undone {
                    reason: format!("{machine} was forgotten before the continue was listed"),
                },
                Err(error) => OperationState::Stuck {
                    reason: status::chain(error),
                },
            };
            if let Some(reports) = &reports {
                reports.record(&operation, KIND, STEP, state);
            }
            match outcome {
                Ok(Recovered::Listed) => {}
                Ok(Recovered::Refused { reason }) => crate::log::say!(
                    "humatad2: the continue {operation} on {machine} was cut short, and {machine} made nothing: {reason}. Nothing is listed; continue the session again"
                ),
                Ok(Recovered::Forgotten) => crate::log::say!(
                    "humatad2: the continue {operation} was cut short, and {machine} was forgotten before it was listed. Nothing was deleted"
                ),
                Err(error) => crate::log::say!(
                    "humatad2: the continue {operation} of a session on {machine} was cut short and isn't listed yet: {}. The next start tries again; meanwhile the new session may run on {machine} without this Mac listing it",
                    status::chain(&error)
                ),
            }
        });
    }

    async fn recover_there(&self, there: There) -> Result<Recovered, Status> {
        let mut state = super::super::move_recover::watch_provider(there.carry.links(), &there.machine);
        loop {
            let Ok(up) = state.wait_for(|now| matches!(now, LinkState::Up { .. })).await.map(|now| *now) else {
                self.settle(&there).await;
                return Ok(Recovered::Forgotten);
            };
            let asked = match super::super::move_recover::relayed_provider(there.carry.links(), &there.machine) {
                Ok(relayed) => match ask_machine(&relayed, &there.machine, there.request.clone()).await {
                    Ok(pb::StartSessionResponse {
                        operation: Some(first), ..
                    }) => watch_end(&relayed, &there.machine, first)
                        .await
                        .map(|ended| (relayed, ended)),
                    Ok(_) => Err(Asked::Refused(no_answer(&there.machine))),
                    Err(asked) => Err(asked),
                },
                Err(error) => Err(Asked::Lost(Status::from(error))),
            };
            match asked {
                Ok((relayed, ended)) => {
                    if let Err(status) = made_by(&there.machine, ended.clone()) {
                        self.settle(&there).await;
                        return Ok(Recovered::Refused {
                            reason: status.message().to_owned(),
                        });
                    }
                    match self.conclude_there(&relayed, &there, ended, None).await {
                        Ok(_) => return Ok(Recovered::Listed),
                        // The link went while the session was read.
                        Err(status) if status.code() == Code::Unavailable => {}
                        Err(status) => return Err(status),
                    }
                }
                Err(Asked::Refused(status)) => {
                    self.settle(&there).await;
                    return Ok(Recovered::Refused {
                        reason: status.message().to_owned(),
                    });
                }
                Err(Asked::Lost(_)) => {}
            }
            // The link went again: wait for the next one.
            if state.wait_for(|now| *now != up).await.is_err() {
                self.settle(&there).await;
                return Ok(Recovered::Forgotten);
            }
        }
    }
}
