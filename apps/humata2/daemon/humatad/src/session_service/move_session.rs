//! MoveSession (MV-1..MV-6, MV-13, MV-14, flow 26.6; D195): a channel
//! session goes from the machine it is on to another online machine of its
//! channel, and nothing of the old copy is deleted until the agent runs on
//! the new one.
//!
//! The steps, in the operation: **check** (the new machine has no folder of
//! that name), **pause** (the session shows Moving, MV-4), **pack** (the old
//! machine stages the manifest, records and the agent's files), **copy** (the
//! folder and the staged files, through the transfer engine), **verify** (the
//! new machine checks every hash and takes the copy in), **resume** (the old
//! agent stops, the session is packed again to see nothing changed meanwhile,
//! the new agent starts in its conversation), **switch** (the host's record
//! names the new machine, the old report is forgotten) and **release** (the old
//! machine's copy goes to its trash).
//!
//! A failure before the switch takes the new machine's copy back, starts the
//! old agent again if it was stopped, and ends the operation FAILED with the
//! one step that failed and a fix that says nothing was deleted (MV-3). The
//! agent keeps running on the old machine until the copy is verified, so a
//! copy that fails leaves the same agent running there.
//!
//! The host's journal `<state>/carry/<operation>.json` says how far the move
//! got (MV-14). A move a restart cut short is undone while the agent still
//! runs on the old machine and finished once it runs on the new one (see
//! `move_recover.rs`).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use humata2_access::{Capability, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{AgentState, ChannelRecord, SessionRecord};
use pb::invalid_field::Problem;
use tokio::sync::{OwnedMutexGuard, watch};
use tonic::{Code, Status};

use super::{Gated, Loaded, SessionSvc, busy, run_resume};
use super::fork::{hold_point, stop_point};
use crate::carry::{Carry, Journaled, Place};
use crate::channel_service::{blocking, failed};
use crate::identity::{self, Caller, UserVia};
use crate::operations::{Progress, Started};
use crate::remote_work::{HostOp, RemoteWork};
use crate::status;

const MOVE_SESSION: &str = "humata.v1.SessionService.MoveSession";

/// The steps, as `(key, title)`.
const STEPS: [(&str, &str); 8] = [
    ("check", "Checking the new machine can take it"),
    ("pause", "Pausing the agent"),
    ("pack", "Listing what moves"),
    ("copy", "Copying the folder and conversation"),
    ("verify", "Checking every file arrived"),
    ("resume", "Starting the agent on the new machine"),
    ("switch", "Switching to the new machine"),
    ("release", "Moving the old copy to the old machine's trash"),
];

#[derive(Debug, thiserror::Error)]
enum MoveError {
    #[error("session {session} is already on machine {machine}")]
    SameMachine { session: String, machine: String },
    #[error("machine {machine} isn't one of channel {channel}'s machines, so session {session} can't move there")]
    NotInChannel {
        session: String,
        machine: String,
        channel: String,
    },
    #[error("{folder} already exists on {machine}, so the session's folder can't be copied there")]
    FolderTaken { folder: String, machine: String },
    #[error("the session's folder {folder} isn't one Humata made, so it can't be moved")]
    NotMovable { folder: String },
    #[error("the agent stopped by the move didn't start again on {machine}")]
    NotResumed { machine: String },
}

/// Sessions with a move running on this daemon, and a count that moves
/// each time one starts or ends, for terminals to show it (MV-4).
struct MovingSet {
    sessions: Mutex<HashSet<String>>,
    changes: watch::Sender<u64>,
}

fn moving() -> &'static MovingSet {
    static MOVING: OnceLock<MovingSet> = OnceLock::new();
    MOVING.get_or_init(|| MovingSet {
        sessions: Mutex::default(),
        changes: watch::channel(0).0,
    })
}

impl MovingSet {
    fn set(&self, session: &str, on: bool) {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        if on {
            sessions.insert(session.to_owned());
        } else {
            sessions.remove(session);
        }
        drop(sessions);
        self.changes.send_modify(|count| *count += 1);
    }

    fn has(&self, session: &str) -> bool {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner).contains(session)
    }
}

/// Whether a move of `session` runs on this daemon now: its terminal is
/// read-only until the agent resumes (MV-4).
pub(crate) fn is_moving(session: &str) -> bool {
    moving().has(session)
}

/// Changes whenever a move starts or ends.
pub(crate) fn moving_changes() -> watch::Receiver<u64> {
    moving().changes.subscribe()
}

/// A session's place in the set of those being moved, for as long as the
/// move runs.
pub(super) struct Moving(String);

impl Moving {
    pub(super) fn begin(session: &str) -> Self {
        moving().set(session, true);
        Self(session.to_owned())
    }
}

impl Drop for Moving {
    fn drop(&mut self) {
        moving().set(&self.0, false);
    }
}

/// Shows a session whose move is running as Moving (MV-4), whatever else is
/// known of it.
pub(super) fn show_moving(session: &mut pb::Session) {
    if session.id.as_ref().is_some_and(|id| is_moving(&id.value)) {
        session.status = pb::SessionStatus::Moving as i32;
    }
}

/// The machine `record` lives on, from the host's point of view.
fn machine_of(record: &SessionRecord, here: &str) -> String {
    if record.machine.is_empty() {
        here.to_owned()
    } else {
        record.machine.clone()
    }
}

impl SessionSvc {
    /// Gives the host's SessionService the means to move sessions. Once.
    pub(crate) fn use_carry(&self, carry: Carry) {
        let _ = self.carry.set(carry);
    }

    pub(super) async fn move_session_of(
        &self,
        caller: &Caller,
        request: pb::MoveSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        Self::refuse_peer(caller)?;
        let id = request
            .id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("id"))?;
        let to = request
            .to
            .map(|machine| machine.value)
            .filter(|machine| !machine.is_empty())
            .ok_or_else(|| status::missing("to"))?;
        let loaded = self.load(&id).await?;
        if let Some(asked) = self.gate(
            caller,
            Capability::SessionMove,
            Target::Session(self.session_ref(&loaded.record)),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self.operations.existing(caller, request.idempotency_key.as_ref())? {
            return Ok(Gated::Done(operation));
        }
        let (Some(carry), Some(remote)) = (self.carry.get().cloned(), self.remote.clone()) else {
            return Err(status::not_built(MOVE_SESSION));
        };
        let channel = self.find_channel(&loaded.record.channel).await?;
        let from = machine_of(&loaded.record, &self.machine);
        if from == to {
            return Err(status::invalid_field(
                "to",
                Problem::NotAllowedHere,
                MoveError::SameMachine {
                    session: id.clone(),
                    machine: to,
                }
                .to_string(),
                "pick another machine of the channel to move it to",
            ));
        }
        if !channel.machines.iter().any(|machine| machine == &to) {
            return Err(failed(
                Code::FailedPrecondition,
                &MoveError::NotInChannel {
                    session: id.clone(),
                    machine: to,
                    channel: channel.name.clone(),
                },
                "add the machine to the channel first, or pick one of the channel's machines",
                None,
            ));
        }
        for machine in [&to, &from] {
            if !carry.is_host(machine) {
                remote.require_link(machine).map_err(|error| offline(Status::from(error)))?;
            }
        }
        match loaded.agent.as_ref().map(|agent| agent.state) {
            Some(AgentState::Launching) => return Err(busy(&id, "still starting")),
            Some(AgentState::Resuming) => return Err(busy(&id, "being resumed")),
            _ => {}
        }
        let Ok(held) = self.lifecycle.lock_of(&id).try_lock_owned() else {
            return Err(busy(&id, "being moved, archived or restored"));
        };
        let folder = relative_folder(&loaded.record, &self.starter.layout.sessions(&channel.folder)).ok_or_else(|| {
            failed(
                Code::FailedPrecondition,
                &MoveError::NotMovable {
                    folder: loaded.record.folder.display().to_string(),
                },
                "only a session Humata started in a channel can move",
                None,
            )
        })?;

        let steps: Vec<(String, String)> = STEPS
            .iter()
            .map(|(key, title)| ((*key).to_owned(), (*title).to_owned()))
            .collect();
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.clone() })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::MoveSession,
            target,
            &steps,
            true,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        let (channel_id, channel_folder) = (channel.id.clone(), channel.folder.clone());
        let place = |machine: &str| Place {
            machine: machine.to_owned(),
            channel: channel_id.clone(),
            folder: channel_folder.clone(),
        };
        let (at_from, at_to) = (place(&from), place(&to));
        let mover = Mover {
            svc: self.clone(),
            carry,
            remote,
            caller: caller.clone(),
            operation: operation.id.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
            session: id,
            title: loaded.record.title.clone(),
            record: loaded.record,
            channel,
            at_from,
            at_to,
            from,
            to,
            folder,
            progress: Mutex::new(Some(progress)),
            stopped: AtomicBool::new(false),
            pass: std::sync::atomic::AtomicU32::new(0),
            placing: AtomicBool::new(false),
            agent_ran: AtomicBool::new(true),
            rebased: AtomicBool::new(false),
        };
        tokio::spawn(async move { mover.run(held).await });
        Ok(Gated::Done(operation))
    }
}

/// Makes an offline answer a failed precondition: the move is refused as it
/// stands, and nothing changes.
fn offline(status: Status) -> Status {
    match status::error_of(&status) {
        Some(error) => status::failure(Code::FailedPrecondition, error),
        None => status,
    }
}

/// `sessions/<name>` for `record`'s folder, when it is directly in `sessions`.
fn relative_folder(record: &SessionRecord, sessions: &std::path::Path) -> Option<String> {
    let parent = record.folder.parent()?;
    (parent == sessions).then_some(())?;
    let name = record.folder.file_name()?.to_str()?;
    Some(format!("sessions/{name}"))
}

/// The one step a move failed at, and why.
struct Failure {
    step: &'static str,
    status: Status,
    /// CancelOperation stopped the move here rather than a step failing.
    cancelled: bool,
}

struct Mover {
    svc: SessionSvc,
    carry: Carry,
    remote: RemoteWork,
    caller: Caller,
    operation: String,
    session: String,
    title: String,
    record: SessionRecord,
    channel: ChannelRecord,
    from: String,
    to: String,
    at_from: Place,
    at_to: Place,
    /// `sessions/<name>`.
    folder: String,
    progress: Mutex<Option<Progress>>,
    /// The old agent was stopped by this move.
    stopped: AtomicBool,
    /// How many times the copy was made, so each copy is a transfer of its own.
    pass: std::sync::atomic::AtomicU32,
    /// Something of the copy may be on the new machine.
    placing: AtomicBool,
    /// The agent ran on the old machine when the move packed it; until
    /// then nothing was stopped, so an undo has nothing to start again.
    agent_ran: AtomicBool,
    /// The host's mirror was pointed at the new machine's copy.
    rebased: AtomicBool,
}

impl Mover {
    async fn run(self, held: OwnedMutexGuard<()>) {
        let moving = Moving::begin(&self.session);
        self.tell().await;
        self.reached("started").await;
        match self.go().await {
            Ok(()) => {
                // The journal stays until the new machine's copy of the
                // staged files is gone too; a link that is down then gets
                // the clean-up when it comes back (D207).
                match self.unstage_target().await {
                    Ok(()) => self.carry.journal_done(&self.operation).await,
                    Err(_) => self.recover_later("switched"),
                }
                if let Some(progress) = self.take_progress() {
                    progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                        value: self.session.clone(),
                    })));
                }
            }
            Err(failure) => self.undo(failure).await,
        }
        drop(moving);
        self.tell().await;
        drop(held);
    }

    fn take_progress(&self) -> Option<Progress> {
        self.progress.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn set_step(&self, key: &str, state: pb::StepState, tail: Option<String>) {
        if let Some(progress) = self.progress.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            progress.set_step(key, state, tail);
        }
    }

    async fn tell(&self) {
        self.svc
            .tell
            .session(&self.session, pb::Change::Updated, Some(identity::actor(&self.caller)))
            .await;
    }

    async fn journal(&self, phase: &str) {
        self.carry
            .journal(
                &self.operation,
                Carry::entry(&self.operation, &self.session, &self.from, &self.to, phase, self.agent_ran.load(Ordering::SeqCst)),
            )
            .await;
    }

    /// The move got as far as `phase`: the journal says so, and a test
    /// daemon may be killed here.
    async fn reached(&self, phase: &str) {
        self.journal(phase).await;
        stop_point(&format!("move-{phase}"));
        hold_point(&format!("move-{phase}")).await;
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
            Err(status) => Err(Failure {
                step: key,
                status,
                cancelled: false,
            }),
        }
    }

    async fn go(&self) -> Result<(), Failure> {
        self.step("check", self.check()).await?;
        self.go_on("pause")?;
        self.step("pause", async { Ok(()) }).await?;
        self.go_on("pack")?;
        let packed = self
            .step("pack", self.carry.pack(&self.from, &self.operation, &self.session))
            .await?;
        self.agent_ran.store(packed.agent_running, Ordering::SeqCst);
        self.reached("packed").await;
        self.go_on("copy")?;
        self.step("copy", self.copy(&packed)).await?;
        self.reached("copied").await;
        self.go_on("verify")?;
        self.step("verify", self.unpack()).await?;
        self.reached("unpacked").await;
        self.past_no_return()?;
        self.step("resume", self.hand_over(&packed.digest)).await?;
        self.reached("resumed").await;
        self.step("switch", self.switch()).await?;
        self.reached("switched").await;
        self.release().await;
        Ok(())
    }

    /// Stops before `next` when CancelOperation asked: the move is undone
    /// as a failed one is, and ends cancelled (GRPC.md §7).
    fn go_on(&self, next: &'static str) -> Result<(), Failure> {
        let asked = self
            .progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(Progress::cancelled);
        if asked { Err(cancelled_at(next)) } else { Ok(()) }
    }

    /// Stopping the old agent is the point of no return: a cancel asked
    /// before it undoes the move; one asked after is refused.
    fn past_no_return(&self) -> Result<(), Failure> {
        let past = self
            .progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_none_or(Progress::past_no_return);
        if past { Ok(()) } else { Err(cancelled_at("resume")) }
    }

    /// The new machine has nothing under the folder's name.
    async fn check(&self) -> Result<(), Status> {
        if self.carry.exists(&self.at_to, &self.folder).await? {
            return Err(failed(
                Code::FailedPrecondition,
                &MoveError::FolderTaken {
                    folder: self.folder.clone(),
                    machine: self.to.clone(),
                },
                "move or remove that folder on the machine, then move the session again",
                None,
            ));
        }
        Ok(())
    }

    /// The session's folder, then the staged files, onto the new machine.
    async fn copy(&self, packed: &pb::CarryPackResponse) -> Result<(), Status> {
        self.placing.store(true, Ordering::SeqCst);
        let pass = self.pass.fetch_add(1, Ordering::SeqCst);
        self.carry
            .copy(
                &format!("{}_{pass}_folder", self.operation),
                (&self.at_from, &packed.folder),
                (&self.at_to, "sessions"),
            )
            .await?;
        self.carry
            .copy(
                &format!("{}_{pass}_stage", self.operation),
                (&self.at_from, &packed.stage),
                (&self.at_to, &format!("{}/{}/in", humata2_sessions::CARRY_DIR, self.operation)),
            )
            .await
    }

    async fn unpack(&self) -> Result<(), Status> {
        self.carry
            .unpack(&self.to, &self.operation, &self.session, &self.channel.id)
            .await
            .map(drop)
    }

    /// The old agent stops (after the copy is checked, so a failed copy
    /// leaves it running), the session is packed again to see that nothing
    /// changed while it ran, and the new agent starts in its conversation.
    async fn hand_over(&self, digest: &str) -> Result<(), Status> {
        self.stop_old().await?;
        let again = self.carry.pack(&self.from, &self.operation, &self.session).await?;
        if again.digest != digest {
            // The agent wrote while it was being copied. It is stopped now,
            // so what is copied this time is all of it.
            self.carry
                .abort(&self.to, &self.operation, &self.session, &self.at_to, &self.folder, true)
                .await?;
            self.copy(&again).await?;
            self.unpack().await?;
        }
        self.resume_on(&self.to).await
    }

    /// Stops the agent without withdrawing its Inbox requests: they stay
    /// (MV-5).
    async fn stop_old(&self) -> Result<(), Status> {
        self.stopped.store(self.agent_ran.load(Ordering::SeqCst), Ordering::SeqCst);
        self.stop_on(&self.from).await
    }

    /// Stops the session's agent on `machine`, if one runs there.
    async fn stop_on(&self, machine: &str) -> Result<(), Status> {
        if !self.carry.is_host(machine) {
            return self.remote.stop_session_on(machine, &self.session).await.map(drop).map_err(Status::from);
        }
        let loaded = self.svc.load(&self.session).await?;
        let Some(mut agent) = loaded.agent else {
            return Ok(());
        };
        if !matches!(agent.state, AgentState::Running { .. }) {
            return Ok(());
        }
        if let Err(error) = humata2_holder::shutdown(&agent.launch.holder_socket).await {
            crate::log::say!(
                "humatad2: stopping session {}'s terminal for a move: {}",
                self.session,
                status::chain(&error)
            );
        }
        agent.state = AgentState::Stopped;
        agent.updated_unix_ms = super::now_ms();
        let (store, saving) = (self.svc.starter.store.clone(), agent);
        blocking(move || store.put_agent(&saving))
            .await?
            .map_err(|error| status::store(error, &self.svc.machine, "save that the agent stopped"))
    }

    /// Resumes the session's agent on `machine` and waits until it has.
    async fn resume_on(&self, machine: &str) -> Result<(), Status> {
        let steps = [
            ("launch".to_owned(), "Starting the agent again".to_owned()),
            ("verify".to_owned(), "Checking it took up its conversation".to_owned()),
        ];
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: self.session.clone(),
            })),
        };
        let Started::New(progress) = self.svc.operations.start_steps(
            &self.caller,
            pb::OperationKind::StartSession,
            target,
            &steps,
            false,
            None,
        )?
        else {
            return Err(status::internal("a resume without a key was taken for a repeat", ""));
        };
        let mut watch = progress.watch();
        if self.carry.is_host(machine) {
            let loaded: Loaded = self.svc.load(&self.session).await?;
            tokio::spawn(run_resume(
                self.svc.starter.clone(),
                self.svc.tell.clone(),
                identity::actor(&self.caller),
                self.svc.tokens.clone(),
                loaded,
                progress,
            ));
        } else {
            let request = pb::ResumeSessionRequest {
                id: Some(pb::SessionId {
                    value: self.session.clone(),
                }),
                ..pb::ResumeSessionRequest::default()
            };
            let host = Arc::new(HostOp::new(progress));
            if let Err(error) = self
                .remote
                .resume_session_on(machine, &self.session, request, host.clone())
                .await
            {
                if let Some(progress) = host.take() {
                    progress.end_cancelled();
                }
                return Err(Status::from(error));
            }
        }
        let ended = loop {
            let now = watch.borrow_and_update().clone();
            if now.state != pb::OperationState::Running as i32 {
                break now;
            }
            if watch.changed().await.is_err() {
                return Err(status::internal("the resume's operation was lost", ""));
            }
        };
        if ended.state == pb::OperationState::Succeeded as i32 {
            return Ok(());
        }
        Err(match ended.error {
            Some(error) => status::failure(Code::FailedPrecondition, error),
            None => failed(
                Code::FailedPrecondition,
                &MoveError::NotResumed {
                    machine: machine.to_owned(),
                },
                "look at the session on the new machine, then try the move again",
                None,
            ),
        })
    }

    /// The host's record names the new machine, and what the old one last
    /// reported is forgotten (SL-1, SL-3).
    async fn switch(&self) -> Result<(), Status> {
        // Before the record says so: the mirror then reads the new
        // machine's copy alone, and the history it holds is not added twice.
        self.rebased.store(true, Ordering::SeqCst);
        self.svc.rebase_mirror(&self.session, &self.to, None).await;
        let mut record = self.record.clone();
        record.machine.clone_from(&self.to);
        let store = self.svc.starter.store.clone();
        blocking(move || store.put_session(&record))
            .await?
            .map_err(|error| status::store(error, &self.svc.machine, "save the session's new machine"))?;
        if !self.carry.is_host(&self.from) {
            self.remote.forget_session(&self.from, &self.session).await;
        }
        Ok(())
    }

    /// The old machine's copy goes to its trash. The session already runs on
    /// the new machine, so a failure here doesn't undo the move: the step says
    /// what is left.
    async fn release(&self) {
        self.set_step("release", pb::StepState::Running, None);
        match self.release_copy().await {
            Ok(()) => {
                self.set_step("release", pb::StepState::Done, None);
                // The old copy is gone, the journal not yet: a restart here
                // releases again.
                stop_point("move-released");
            }
            Err(status) => {
                let shown = status::error_of(&status).map_or_else(|| status.message().to_owned(), |error| error.message);
                self.set_step(
                    "release",
                    pb::StepState::Failed,
                    Some(format!(
                        "the session runs on {} now, but its old copy on {} is still there: {shown}",
                        self.to, self.from
                    )),
                );
            }
        }
    }

    /// The old machine's copy goes to its trash.
    async fn release_copy(&self) -> Result<(), Status> {
        self.carry
            .release(&self.from, &self.operation, &self.session, &self.channel.id)
            .await
    }

    /// Takes back what the move put on the new machine, and what it staged
    /// on the old one.
    async fn take_copy_back(&self) -> (Result<(), Status>, Result<(), Status>) {
        let incoming = self
            .carry
            .abort(&self.to, &self.operation, &self.session, &self.at_to, &self.folder, true)
            .await;
        let staged = self
            .carry
            .abort(&self.from, &self.operation, &self.session, &self.at_from, &self.folder, false)
            .await;
        (incoming, staged)
    }

    /// A move that failed before it switched: the new machine's copy is
    /// taken back, the old agent starts again if it was stopped, and the
    /// operation ends FAILED naming the step (MV-3).
    async fn undo(&self, failure: Failure) {
        let Failure { step, status: cause, cancelled } = failure;
        if self.rebased.load(Ordering::SeqCst) {
            self.svc.rebase_mirror(&self.session, &self.from, None).await;
        }
        let mut taken_back = true;
        if self.placing.load(Ordering::SeqCst) {
            // An agent the resume started there stops before its copy goes.
            let _ = self.stop_on(&self.to).await;
            taken_back = self
                .carry
                .abort(&self.to, &self.operation, &self.session, &self.at_to, &self.folder, true)
                .await
                .is_ok();
        }
        let _ = self
            .carry
            .abort(&self.from, &self.operation, &self.session, &self.at_from, &self.folder, false)
            .await;
        let mut left = String::new();
        if self.stopped.load(Ordering::SeqCst)
            && let Err(error) = self.resume_on(&self.from).await
        {
            left = format!(
                " Its agent was stopped for the move and didn't start again on {}: {}. Resume it from the session's menu.",
                self.from,
                error.message()
            );
        }
        self.journal("aborted").await;
        if taken_back {
            self.carry.journal_done(&self.operation).await;
        } else {
            // The new machine keeps a stopped copy until its link is back;
            // the journal stays so the host takes it back then (D207).
            self.recover_later("aborted");
        }
        if cancelled {
            if let Some(progress) = self.take_progress() {
                progress.end_cancelled();
            }
            return;
        }
        let why = status::error_of(&cause);
        let reason = why.as_ref().map_or_else(|| cause.message().to_owned(), |error| error.message.clone());
        let fix = why
            .as_ref()
            .map(|error| error.fix.clone())
            .filter(|fix| !fix.is_empty())
            .unwrap_or_else(|| "try the move again".to_owned());
        let title = STEPS.iter().find(|(key, _)| *key == step).map_or(step, |(_, title)| *title);
        self.set_step(step, pb::StepState::Failed, Some(reason.clone()));
        let error = pb::Error {
            message: format!("moving {} to {} failed at \"{title}\": {reason}", self.title, self.to),
            fix: format!(
                "Nothing was deleted: {} is on {} as before.{left} {fix}",
                self.title, self.from
            ),
            retry: None,
            kind: None,
        };
        if let Some(progress) = self.take_progress() {
            progress.fail(error);
        }
    }
}

fn cancelled_at(step: &'static str) -> Failure {
    Failure {
        step,
        status: Status::cancelled("the move was cancelled"),
        cancelled: true,
    }
}

/// Finishes or undoes the move `journal` says a restart cut short (MV-14),
/// under the session's lifecycle lock (`held`). Succeeds once the session is
/// settled on one machine with one agent and the journal is gone; fails with
/// the journal kept, so the next start tries again.
pub(super) async fn recover(
    svc: SessionSvc,
    carry: Carry,
    remote: RemoteWork,
    journal: Journaled,
    held: OwnedMutexGuard<()>,
) -> Result<(), Status> {
    let loaded = svc.load(&journal.session).await?;
    let channel = svc.find_channel(&loaded.record.channel).await?;
    let folder = relative_folder(&loaded.record, &svc.starter.layout.sessions(&channel.folder)).ok_or_else(|| {
        failed(
            Code::FailedPrecondition,
            &MoveError::NotMovable {
                folder: loaded.record.folder.display().to_string(),
            },
            "only a session Humata started in a channel can move",
            None,
        )
    })?;
    let place = |machine: &str| Place {
        machine: machine.to_owned(),
        channel: channel.id.clone(),
        folder: channel.folder.clone(),
    };
    let mover = Mover {
        svc,
        carry,
        remote,
        caller: Caller::User(UserVia::Host),
        operation: journal.operation.clone(),
        session: journal.session.clone(),
        title: loaded.record.title.clone(),
        record: loaded.record,
        at_from: place(&journal.from),
        at_to: place(&journal.to),
        channel,
        from: journal.from.clone(),
        to: journal.to.clone(),
        folder,
        progress: Mutex::new(None),
        stopped: AtomicBool::new(false),
        pass: std::sync::atomic::AtomicU32::new(0),
        placing: AtomicBool::new(true),
        agent_ran: AtomicBool::new(journal.agent_ran),
        rebased: AtomicBool::new(false),
    };
    let outcome = match journal.phase.as_str() {
        "resumed" => mover.finish(true).await,
        "switched" => mover.finish(false).await,
        phase => mover.undo_cut_short(phase == "unpacked").await,
    };
    if outcome.is_ok() {
        mover.carry.journal_done(&mover.operation).await;
    }
    drop(held);
    outcome
}

impl Mover {
    /// The agent runs on the new machine: nothing may run on the old one, the
    /// record names the new machine, and the old copy goes to the old
    /// machine's trash. `switch` is false when the journal says it was done.
    async fn finish(&self, switch: bool) -> Result<(), Status> {
        match self.stop_on(&self.from).await {
            // The old machine already released the session: nothing of it
            // runs there.
            Err(status) if status.code() == Code::NotFound && !self.carry.is_host(&self.from) => {}
            stopped => stopped?,
        }
        if switch {
            self.switch().await?;
        }
        self.release_copy().await?;
        self.unstage_target().await
    }

    /// Removes what was staged on the new machine for the move, once the
    /// session runs there; the session itself stays.
    async fn unstage_target(&self) -> Result<(), Status> {
        self.carry
            .abort(&self.to, &self.operation, &self.session, &self.at_to, &self.folder, false)
            .await
    }

    /// The clean-up of a machine failed: the journal says how far the move
    /// got, and the host finishes it once both machines' links are up.
    fn recover_later(&self, phase: &str) {
        self.svc.recover_when_linked(
            Journaled {
                operation: self.operation.clone(),
                session: self.session.clone(),
                from: self.from.clone(),
                to: self.to.clone(),
                phase: phase.to_owned(),
                agent_ran: self.agent_ran.load(Ordering::SeqCst),
            },
            None,
        );
    }

    /// The agent was never taken over by the new machine: whatever started
    /// there stops, the new machine's copy is taken back and the agent runs
    /// on the old one again. `handing_over` says the old agent may have been
    /// stopped for the move.
    async fn undo_cut_short(&self, handing_over: bool) -> Result<(), Status> {
        let started_there = self.stop_on(&self.to).await;
        if handing_over {
            started_there?;
        }
        let (incoming, staged) = self.take_copy_back().await;
        incoming?;
        staged?;
        if handing_over && self.agent_ran.load(Ordering::SeqCst) && !self.running_here(&self.from).await? {
            self.resume_on(&self.from).await?;
        }
        Ok(())
    }

    /// Whether an agent of the session is running on the host itself;
    /// always false for another machine, whose resume leaves a running
    /// agent as it is.
    async fn running_here(&self, machine: &str) -> Result<bool, Status> {
        if !self.carry.is_host(machine) {
            return Ok(false);
        }
        let loaded = self.svc.load(&self.session).await?;
        Ok(loaded
            .agent
            .is_some_and(|agent| matches!(agent.state, AgentState::Running { .. })))
    }
}

/// The line a session's shell tab starts with when its agent's resume
/// starts it again, after the output its old holder saved.
const SHELL_BACK: &str = "This terminal's earlier shell is gone: the session moved to this machine, or its shell had stopped. \
     Programs that ran in it didn't come along; the output above is what it had saved.";

/// Starts the session's shell tabs whose holders are gone, such as those a
/// move brought in (MV-13). A shell that can't start is logged and leaves
/// the resume as it is: its tab stays, and the next resume or restart tries
/// again.
pub(super) async fn restart_session_shells(
    starter: &humata2_sessions::Starter,
    session: &str,
    env: humata2_revive::Env<'_>,
) {
    match humata2_revive::restart_shells(&starter.store, &starter.revive, session, env, SHELL_BACK).await {
        Ok(shells) => {
            for (tab, outcome) in shells {
                if let Err(error) = outcome {
                    crate::log::say!(
                        "humatad2: session {session}'s shell tab {tab} didn't start again: {}",
                        status::chain(&error)
                    );
                }
            }
        }
        Err(error) => crate::log::say!(
            "humatad2: couldn't start session {session}'s shell tabs again: {}",
            status::chain(&error)
        ),
    }
}
