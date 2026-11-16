//! ForkSession onto another machine (FK-8, FK-12, flow 26.5; D195). The
//! parent keeps running where it is. The machine it is on stages a copy
//! of it as a new session on a new branch (`Starter::pack_fork`: the code as
//! it is now, with committed, uncommitted and untracked files, and, for code
//! and context, the conversation under an id of its own), and the fork
//! travels as a moved session does: the folder and the staged files are
//! copied through the carry driver, the target checks every hash and takes
//! the fork in, the host records it, and its agent starts there.
//!
//! The steps, in the operation: **pack**, **copy**, **unpack**, **launch**
//! and **verify**. A fork that fails at any step takes back what it put on
//! the target and what it staged, and says which step failed; the parent
//! is never touched. A target that is offline is refused before anything is
//! made (FK-12).
//!
//! A code-only fork has no conversation to resume: its agent is started
//! fresh by the target's ResumeSession (`unlaunched`, `launch_unlaunched`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use humata2_access::{AgentId, Capability, ChannelId, MachineId, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Starter;
use humata2_store::{ChannelRecord, ForkOrigin, SessionDetail, SessionRecord, SessionRepo, StartStep, TabRecord};
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use super::fork::not_built_part;
use super::lineage::publish_family;
use super::{COLS, Gated, Loaded, ROWS, SessionSvc, Tell, advance, clean_title, now_ms, start_error, watch};
use crate::carry::{Carry, Place};
use crate::channel_service::{blocking, failed};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::restart::Tokens;
use crate::status;

const FORK_SESSION: &str = "humata.v1.SessionService.ForkSession";

/// The steps, as `(key, title)`.
const STEPS: [(&str, &str); 5] = [
    ("pack", "Copying the code and conversation"),
    ("copy", "Sending them to the other machine"),
    ("unpack", "Checking every file arrived"),
    ("launch", "Starting the agent"),
    ("verify", "Checking it started"),
];

#[derive(Debug, thiserror::Error)]
enum ForkElsewhereError {
    #[error("session {session} is archived, so it can't be forked")]
    ParentArchived { session: String },
    #[error("machine {machine} isn't one of channel {channel}'s machines, so session {session} can't be forked onto it")]
    NotInChannel {
        session: String,
        machine: String,
        channel: String,
    },
    #[error("{folder} already exists on {machine}, so the fork's folder can't be copied there")]
    FolderTaken { folder: String, machine: String },
    #[error("the parent's start wasn't recorded on this machine, so it can't be forked onto another")]
    NoDetail { session: String },
    #[error("the fork's agent didn't start on {machine}")]
    NotStarted { machine: String },
}

/// The machine `record` lives on, from this host's point of view.
fn machine_of(record: &SessionRecord, here: &str) -> String {
    if record.machine.is_empty() {
        here.to_owned()
    } else {
        record.machine.clone()
    }
}

/// An offline answer as a failed precondition: the fork is refused as it
/// stands, and nothing changes (FK-12).
fn offline(status: Status) -> Status {
    match status::error_of(&status) {
        Some(error) => status::failure(Code::FailedPrecondition, error),
        None => status,
    }
}

/// Whether `loaded` is a carried session that never had an agent: its first
/// ResumeSession starts one fresh.
pub(super) fn unlaunched(loaded: &Loaded) -> bool {
    loaded.agent.is_none() && loaded.detail.as_ref().is_some_and(|detail| detail.start == StartStep::Cloned)
}

impl SessionSvc {
    /// A typed branch is refused when the fork is asked for, not when the
    /// parent's machine packs it: a name git doesn't accept always, and one
    /// that is taken when the parent's clones are on this machine. A parent
    /// on another machine has its taken names checked there, at pack.
    async fn check_typed_branch(&self, parent: &Loaded, from: &str, carry: &Carry, typed: &str) -> Result<(), Status> {
        if typed.is_empty() {
            return Ok(());
        }
        let refused = |error: &humata2_sessions::StartError| {
            let (code, failure) = start_error(error);
            status::failure(code, failure)
        };
        if !humata2_sessions::valid_branch_name(typed) {
            return Err(refused(&humata2_sessions::StartError::BadBranch { branch: typed.to_owned() }));
        }
        if !carry.is_host(from) {
            return Ok(());
        }
        let (starter, id, title, typed) =
            (self.starter.clone(), parent.record.id.clone(), parent.record.title.clone(), typed.to_owned());
        blocking(move || starter.check_fork_branch(&id, &title, &typed))
            .await?
            .map_err(|error| refused(&error))
    }

    /// ForkSession when the fork or its parent is on another machine than
    /// this host: the operation that makes it. None when the fork is a local
    /// one, which the caller goes on with.
    pub(super) async fn fork_elsewhere_of(
        &self,
        caller: &Caller,
        request: &pb::ForkSessionRequest,
        approved: bool,
    ) -> Result<Option<Gated<pb::Operation>>, Status> {
        let (Some(carry), Some(remote)) = (self.carry.get().cloned(), self.remote.clone()) else {
            return Ok(None);
        };
        let Some(parent_id) = request
            .parent
            .as_ref()
            .map(|parent| parent.value.clone())
            .filter(|parent| !parent.is_empty())
        else {
            return Ok(None);
        };
        // A parent this machine doesn't have is the local path's to refuse.
        let Ok(parent) = self.load(&parent_id).await else {
            return Ok(None);
        };
        let from = machine_of(&parent.record, &self.machine);
        let to = request
            .machine
            .as_ref()
            .map(|machine| machine.value.clone())
            .filter(|machine| !machine.is_empty())
            .unwrap_or_else(|| from.clone());
        if carry.is_host(&from) && carry.is_host(&to) {
            return Ok(None);
        }
        Self::refuse_peer(caller)?;

        let copy = match pb::ForkKind::try_from(request.kind) {
            Ok(pb::ForkKind::CodeAndContext) => pb::ForkKind::CodeAndContext,
            Ok(pb::ForkKind::CodeOnly) => pb::ForkKind::CodeOnly,
            Ok(pb::ForkKind::ContextOnly) => {
                return Err(not_built_part(FORK_SESSION, "a context-only fork on another machine"));
            }
            _ => {
                return Err(status::invalid_field(
                    "kind",
                    Problem::Missing,
                    "the request doesn't say what the fork carries",
                    "set kind to CODE_AND_CONTEXT, CODE_ONLY or CONTEXT_ONLY, and send it again",
                ));
            }
        };
        if copy == pb::ForkKind::CodeOnly && request.before_turn != 0 {
            return Err(status::invalid_field(
                "before_turn",
                Problem::NotAllowedHere,
                "a fork before a turn carries the conversation up to that turn, and a code-only fork carries none",
                "fork with CODE_AND_CONTEXT or CONTEXT_ONLY, or set before_turn to 0 to fork from now",
            ));
        }
        self.check_typed_branch(&parent, &from, &carry, &request.branch).await?;
        if request
            .agent
            .as_ref()
            .is_some_and(|agent| !agent.value.is_empty() && agent.value != parent.record.agent)
        {
            return Err(not_built_part(
                FORK_SESSION,
                "a fork that runs another agent than its parent",
            ));
        }
        let (store, id) = (self.starter.store.clone(), parent_id.clone());
        let archived = blocking(move || store.archive(&id))
            .await?
            .map_err(|error| status::store(error, &self.machine, "look up the session"))?;
        if archived.is_some() {
            return Err(failed(
                Code::FailedPrecondition,
                &ForkElsewhereError::ParentArchived { session: parent_id },
                "restore the session first, then fork it",
                None,
            ));
        }
        let channel = self.find_channel(&parent.record.channel).await?;
        if !channel.machines.iter().any(|machine| machine == &to) && !carry.is_host(&to) {
            return Err(failed(
                Code::FailedPrecondition,
                &ForkElsewhereError::NotInChannel {
                    session: parent.record.id.clone(),
                    machine: to,
                    channel: channel.name.clone(),
                },
                "add the machine to the channel first, or pick one of the channel's machines",
                None,
            ));
        }
        let capability = if carry.is_host(&to) {
            Capability::SessionStartSameMachine
        } else {
            Capability::SessionStartOtherMachine
        };
        if let Some(asked) = self.gate(
            caller,
            capability,
            Target::Start {
                channel: ChannelId::new(channel.id.as_str()),
                machine: MachineId::new(to.as_str()),
                agent: AgentId::new(parent.record.agent.as_str()),
            },
            approved,
        )? {
            return Ok(Some(Gated::Ask(asked)));
        }
        // An offline machine answers at once and leaves nothing (FK-12).
        for machine in [&to, &from] {
            if !carry.is_host(machine) {
                remote.require_link(machine).map_err(|error| offline(Status::from(error)))?;
            }
        }
        if request.before_turn != 0 && carry.is_host(&from) {
            self.check_turn(&parent, request.before_turn).await?;
        }
        let title = clean_title(&request.title)?;
        let title = if title.is_empty() {
            clean_title(&format!("Fork of {}", parent.record.title))?
        } else {
            title
        };
        if let Some(operation) = self.operations.existing(caller, request.idempotency_key.as_ref())? {
            return Ok(Some(Gated::Done(operation)));
        }
        let Some(parent_detail) = parent.detail.clone() else {
            return Err(failed(
                Code::FailedPrecondition,
                &ForkElsewhereError::NoDetail {
                    session: parent.record.id.clone(),
                },
                "wait for the session to finish starting, then fork it",
                None,
            ));
        };

        let steps: Vec<(String, String)> = STEPS
            .iter()
            .map(|(key, title)| ((*key).to_owned(), (*title).to_owned()))
            .collect();
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: parent.record.id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::ForkSession,
            target,
            &steps,
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Some(Gated::Done(operation))),
        };
        let operation = progress.operation();
        let place = |machine: &str| Place {
            machine: machine.to_owned(),
            channel: channel.id.clone(),
            folder: channel.folder.clone(),
        };
        let forker = Forker {
            svc: self.clone(),
            carry,
            caller: caller.clone(),
            operation: operation.id.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
            session: format!("se_{}", uuid::Uuid::new_v4().simple()),
            fork: pb::CarryFork {
                session: None,
                title,
                kind: copy as i32,
                before_turn: request.before_turn,
                branch: request.branch.clone(),
            },
            parent: parent.record,
            parent_detail,
            at_from: place(&from),
            at_to: place(&to),
            channel,
            from,
            to,
            progress: Mutex::new(Some(progress)),
            placing: AtomicBool::new(false),
            recorded: AtomicBool::new(false),
            staged: Mutex::new(String::new()),
        };
        tokio::spawn(async move { forker.run().await });
        Ok(Some(Gated::Done(operation)))
    }

    /// ResumeSession for a carried session that never had an agent: starts
    /// one fresh, as a launch does.
    pub(super) async fn launch_unlaunched(
        &self,
        caller: &Caller,
        key: Option<pb::IdempotencyKey>,
        loaded: Loaded,
    ) -> Result<Gated<pb::Operation>, Status> {
        let id = loaded.record.id.clone();
        let steps = [
            ("launch".to_owned(), "Starting the agent".to_owned()),
            ("verify".to_owned(), "Checking it started".to_owned()),
        ];
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.clone() })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            target,
            &steps,
            false,
            key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        tokio::spawn(run_launch(
            self.starter.clone(),
            self.tell.clone(),
            self.tokens.clone(),
            identity::actor(caller),
            id,
            progress,
        ));
        Ok(Gated::Done(operation))
    }
}

/// Starts the first agent of `session` for `progress`.
async fn run_launch(
    starter: Starter,
    tell: Tell,
    tokens: Arc<Tokens>,
    by: pb::Actor,
    session: String,
    progress: Progress,
) {
    let keys = vec!["launch".to_owned(), "verify".to_owned()];
    let mut current = "launch".to_owned();
    progress.set_step("launch", pb::StepState::Running, None);
    let env = |tab: &TabRecord| tokens.env(tab);
    let launched = {
        let mut note = |event: &str| current = advance(&progress, &keys, event);
        starter.launch_unlaunched(&session, COLS, ROWS, &env, &mut note).await
    };
    match launched {
        Ok(_) => {
            progress.set_step("verify", pb::StepState::Done, None);
            progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: session.clone(),
            })));
            tell.session(&session, pb::Change::Updated, Some(by)).await;
            watch::agent(tell, session);
        }
        Err(error) => {
            let (_, failure) = start_error(&error);
            progress.set_step(&current, pb::StepState::Failed, Some(status::chain(&error)));
            progress.fail(failure);
        }
    }
}

/// The one step a fork failed at, and why.
struct Failure {
    step: &'static str,
    status: Status,
}

struct Forker {
    svc: SessionSvc,
    carry: Carry,
    caller: Caller,
    operation: String,
    /// The fork's id.
    session: String,
    fork: pb::CarryFork,
    parent: SessionRecord,
    parent_detail: SessionDetail,
    channel: ChannelRecord,
    from: String,
    to: String,
    at_from: Place,
    at_to: Place,
    progress: Mutex<Option<Progress>>,
    /// Something of the fork may be on the target.
    placing: AtomicBool,
    /// The host's record of the fork is written.
    recorded: AtomicBool,
    /// `sessions/<name>`, once the pack named it.
    staged: Mutex<String>,
}

impl Forker {
    async fn run(self) {
        match self.go().await {
            Ok(()) => {
                let by = Some(identity::actor(&self.caller));
                self.svc.tell.session(&self.session, pb::Change::Created, by.clone()).await;
                publish_family(&self.svc.tell, &self.parent.id, by).await;
                if let Some(progress) = self.take_progress() {
                    progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                        value: self.session.clone(),
                    })));
                }
            }
            Err(failure) => self.undo(failure).await,
        }
    }

    fn take_progress(&self) -> Option<Progress> {
        self.progress.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn set_step(&self, key: &str, state: pb::StepState, tail: Option<String>) {
        if let Some(progress) = self.progress.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            progress.set_step(key, state, tail);
        }
    }

    /// Runs one step: shown running, then done, or failed with `why`.
    async fn step<T>(
        &self,
        key: &'static str,
        work: impl std::future::Future<Output = Result<T, Status>>,
    ) -> Result<T, Failure> {
        self.set_step(key, pb::StepState::Running, None);
        match work.await {
            Ok(done) => {
                self.set_step(key, pb::StepState::Done, None);
                Ok(done)
            }
            Err(status) => Err(Failure { step: key, status }),
        }
    }

    async fn go(&self) -> Result<(), Failure> {
        let fork = pb::CarryFork {
            session: Some(pb::SessionId {
                value: self.session.clone(),
            }),
            ..self.fork.clone()
        };
        let packed = self
            .step("pack", self.carry.pack_fork(&self.from, &self.operation, &self.parent.id, fork))
            .await?;
        let name = packed.folder.rsplit('/').next().unwrap_or_default().to_owned();
        let folder = format!("sessions/{name}");
        *self.staged.lock().unwrap_or_else(PoisonError::into_inner) = folder.clone();
        self.step("copy", self.copy(&packed, &folder)).await?;
        self.step("unpack", self.unpack(&folder)).await?;
        self.step("launch", self.record_and_launch(&packed.branch, &name)).await?;
        self.set_step("verify", pb::StepState::Done, None);
        Ok(())
    }

    /// The target has nothing under the folder's name; then the fork's
    /// folder, and the staged files, onto it.
    async fn copy(&self, packed: &pb::CarryPackResponse, folder: &str) -> Result<(), Status> {
        if self.carry.exists(&self.at_to, folder).await? {
            return Err(failed(
                Code::FailedPrecondition,
                &ForkElsewhereError::FolderTaken {
                    folder: folder.to_owned(),
                    machine: self.to.clone(),
                },
                "move or remove that folder on the machine, then fork again",
                None,
            ));
        }
        self.placing.store(true, Ordering::SeqCst);
        self.carry
            .copy(
                &format!("{}_folder", self.operation),
                (&self.at_from, &packed.folder),
                (&self.at_to, "sessions"),
            )
            .await?;
        self.carry
            .copy(
                &format!("{}_stage", self.operation),
                (&self.at_from, &packed.stage),
                (&self.at_to, &format!("{}/{}/in", humata2_sessions::CARRY_DIR, self.operation)),
            )
            .await
    }

    /// The target checks every hash and takes the fork in; what was staged
    /// is no longer needed on either machine.
    async fn unpack(&self, folder: &str) -> Result<(), Status> {
        self.carry
            .unpack_new(&self.to, &self.operation, &self.session, &self.channel.id)
            .await?;
        for (machine, place) in [(&self.from, &self.at_from), (&self.to, &self.at_to)] {
            // Best effort: the fork is in; what is left is staging.
            let _ = self
                .carry
                .abort(machine, &self.operation, &self.session, place, folder, false)
                .await;
        }
        Ok(())
    }

    /// The host's own record of a fork that lives on a box, then the
    /// agent's start there.
    async fn record_and_launch(&self, branch: &str, name: &str) -> Result<(), Status> {
        let folder = self.svc.starter.layout.session(&self.channel.folder, name);
        let origin = ForkOrigin {
            session_id: self.session.clone(),
            parent: self.parent.id.clone(),
            kind: self.fork.kind,
            before_turn: self.fork.before_turn,
            created_unix_ms: now_ms(),
        };
        let boxed = !self.carry.is_host(&self.to);
        let record = SessionRecord {
            id: self.session.clone(),
            channel: self.channel.id.clone(),
            machine: self.to.clone(),
            title: self.fork.title.clone(),
            agent: self.parent.agent.clone(),
            folder: folder.clone(),
            created_unix_ms: origin.created_unix_ms,
        };
        let detail = SessionDetail {
            session: self.session.clone(),
            repos: self
                .parent_detail
                .repos
                .iter()
                .map(|repo| SessionRepo {
                    owner: repo.owner.clone(),
                    name: repo.name.clone(),
                    branch: branch.to_owned(),
                    path: folder.join(&repo.name),
                })
                .collect(),
            model: self.parent_detail.model.clone(),
            first_message: None,
            start: StartStep::Done,
            operation: None,
            revision: 1,
        };
        let store = self.svc.starter.store.clone();
        self.recorded.store(true, Ordering::SeqCst);
        blocking(move || -> Result<(), humata2_store::StoreError> {
            if boxed {
                store.put_session(&record)?;
                store.put_session_detail(&detail)?;
            }
            store.put_fork_origin(&origin)
        })
        .await?
        .map_err(|error| status::store(error, &self.svc.machine, "save the fork"))?;
        self.resume().await
    }

    /// Starts the fork's agent on the target through its own ResumeSession,
    /// and waits for that to end.
    async fn resume(&self) -> Result<(), Status> {
        let request = pb::ResumeSessionRequest {
            id: Some(pb::SessionId {
                value: self.session.clone(),
            }),
            ..pb::ResumeSessionRequest::default()
        };
        let started = match self.svc.resume(&self.caller, request, true).await? {
            Gated::Done(operation) => operation,
            Gated::Ask(_) => {
                return Err(status::internal("a resume the host made itself asked for approval", ""));
            }
        };
        let id = started.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let mut watch = self.svc.operations.progress_of(&self.caller, &id)?.watch();
        let ended = loop {
            let now = watch.borrow_and_update().clone();
            if now.state != pb::OperationState::Running as i32 {
                break now;
            }
            if watch.changed().await.is_err() {
                return Err(status::internal("the fork's start operation was lost", ""));
            }
        };
        if ended.state == pb::OperationState::Succeeded as i32 {
            return Ok(());
        }
        Err(match ended.error {
            Some(error) => status::failure(Code::FailedPrecondition, error),
            None => failed(
                Code::FailedPrecondition,
                &ForkElsewhereError::NotStarted {
                    machine: self.to.clone(),
                },
                "look at the machine, then fork again",
                None,
            ),
        })
    }

    /// A fork that failed: what it put on the target and what it staged is
    /// taken back, and the operation ends FAILED naming the step. The
    /// parent was never touched.
    async fn undo(&self, failure: Failure) {
        let Failure { step, status: cause } = failure;
        if self.placing.load(Ordering::SeqCst) {
            let folder = self.fork_folder();
            let _ = self
                .carry
                .abort_new(&self.to, &self.operation, &self.session, &self.at_to, &folder)
                .await;
        }
        let _ = self
            .carry
            .abort(&self.from, &self.operation, &self.parent.id, &self.at_from, "", false)
            .await;
        if self.recorded.load(Ordering::SeqCst) {
            let (store, id) = (self.svc.starter.store.clone(), self.session.clone());
            let _ = blocking(move || store.remove_session(&id)).await;
        }
        let why = status::error_of(&cause);
        let reason = why
            .as_ref()
            .map_or_else(|| cause.message().to_owned(), |error| error.message.clone());
        let fix = why
            .as_ref()
            .map(|error| error.fix.clone())
            .filter(|fix| !fix.is_empty())
            .unwrap_or_else(|| "try the fork again".to_owned());
        let title = STEPS
            .iter()
            .find(|(key, _)| *key == step)
            .map_or(step, |(_, title)| *title);
        self.set_step(step, pb::StepState::Failed, Some(reason.clone()));
        let error = pb::Error {
            message: format!("forking {} onto {} failed at \"{title}\": {reason}", self.parent.title, self.to),
            fix: format!(
                "{} is untouched and still runs on {}, and nothing of the fork was left on {}. {fix}",
                self.parent.title, self.from, self.to
            ),
            retry: None,
            kind: None,
        };
        if let Some(progress) = self.take_progress() {
            progress.fail(error);
        }
    }

    /// `sessions/<name>` of the folder the fork is copied to, from what the
    /// pack named; empty before it did.
    fn fork_folder(&self) -> String {
        self.staged.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}
