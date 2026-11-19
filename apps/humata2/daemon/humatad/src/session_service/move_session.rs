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

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use humata2_access::{Capability, Target};
use humata2_holder::HolderError;
use humata2_grpc::humata::v1 as pb;
use humata2_store::{AgentState, ChannelRecord, DeskFolderKind, SessionRecord, StoreError};
use pb::invalid_field::Problem;
use tokio::sync::{OwnedMutexGuard, watch};
use tonic::{Code, Status};

use super::{Gated, Loaded, SessionSvc, busy, busy_running, run_resume};
use crate::carry::{Carry, Chunks, Journaled, Place, SessionCopy, Tally};
use crate::channel_service::{blocking, failed};
use crate::holder_end;
use crate::identity::{self, Caller, UserVia};
use crate::operations::{Progress, Started};
use crate::remote_work::{HostOp, MachineOutcome, RemoteError, RemoteWork, channel_steps};
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
    #[error("{machine} couldn't be given the PR reviews channel that review {session} lives in: {reason}")]
    ReviewChannelNotMade {
        session: String,
        machine: String,
        reason: String,
    },
    #[error("{machine} was removed from Humata while the PR reviews channel was being set up on it for review {session}")]
    ReviewMachineRemoved { session: String, machine: String },
    /// D801 J: the old agent's holder can't pause its program, so nothing
    /// was carried and the agent keeps running where it is.
    #[error("the holder of session {session}'s agent on {machine} is from an older Humata build that can't pause its program for a move, so nothing was copied and the agent keeps running there")]
    HolderTooOldToFreeze {
        session: String,
        machine: String,
        #[source]
        source: HolderError,
    },
    #[error("{machine} runs an older humatad2 that can't pause session {session}'s agent for a move, so nothing was copied and the agent keeps running there")]
    MachineTooOldToFreeze {
        session: String,
        machine: String,
        #[source]
        source: Status,
    },
    #[error("couldn't pause session {session}'s agent on {machine} for the move; the agent was left running")]
    FreezeFailed {
        session: String,
        machine: String,
        #[source]
        source: HolderError,
    },
    #[error("the holder of session {session}'s agent on {machine} didn't answer within {seconds} seconds when asked to pause or resume the agent; it may be wedged")]
    FreezeTimeout {
        session: String,
        machine: String,
        seconds: u64,
        #[source]
        source: tokio::time::error::Elapsed,
    },
}

/// Says what to do about a holder or machine too old to pause an agent.
const MIGRATE_FIRST: &str = "click Migrate on the \"Migrate the holders\" item in the Inbox, which moves each holder onto the current build without restarting its agent where it can and updates the machine's tools, then move the session again";

/// How long a holder gets to answer a freeze or thaw.
const FREEZE_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// The holders (socket and pid) this daemon froze for a move and hasn't
/// thawed: the running list reads them as "Paused for move" (D801 J). A
/// holder ended with its move is never listed again, as the pid is its.
static FROZEN: std::sync::Mutex<std::collections::BTreeSet<(std::path::PathBuf, u32)>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Whether the holder at `socket` with process `pid` is frozen for a move
/// this daemon is making.
pub(crate) fn paused_for_move(socket: &std::path::Path, pid: u32) -> bool {
    FROZEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&(socket.to_owned(), pid))
}

fn note_frozen(socket: &std::path::Path, pid: u32, frozen: bool) {
    let mut held = FROZEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if frozen {
        held.insert((socket.to_owned(), pid));
    } else {
        held.remove(&(socket.to_owned(), pid));
    }
}

/// Freezes (`thaw` false) or thaws session `session`'s agent on this
/// machine for a move (D801 J): its program is stopped with SIGSTOP and its
/// holder serves the frozen screen, or the program runs again. Shells are
/// never frozen: only the agent's own holder is asked. Answers whether an
/// agent ran here to freeze or thaw; thawing an agent whose holder is gone
/// is fine. A holder from a build before freezing is refused as Outdated,
/// naming Migrate; nothing was stopped.
pub(crate) async fn freeze_agent_here(
    svc: &SessionSvc,
    session: &str,
    machine: &str,
    thaw: bool,
) -> Result<bool, Status> {
    let loaded = svc.load(session).await?;
    let Some(agent) = loaded.agent else {
        return Ok(false);
    };
    let AgentState::Running { holder_pid } = agent.state else {
        return Ok(false);
    };
    let socket = &agent.launch.holder_socket;
    let asked = async {
        if thaw {
            humata2_holder::thaw(socket).await
        } else {
            humata2_holder::freeze(socket).await
        }
    };
    let (session, machine) = (session.to_owned(), machine.to_owned());
    match tokio::time::timeout(FREEZE_WAIT, asked).await {
        Ok(Ok(())) => {
            note_frozen(socket, holder_pid, !thaw);
            Ok(true)
        }
        // The holder is gone, so there is nothing of it to let run.
        Ok(Err(HolderError::Connect { .. })) if thaw => {
            note_frozen(socket, holder_pid, false);
            Ok(false)
        }
        Ok(Err(source @ HolderError::TooOldToFreeze { .. })) => Err(failed(
            Code::FailedPrecondition,
            &MoveError::HolderTooOldToFreeze { session, machine, source },
            MIGRATE_FIRST,
            None,
        )),
        Ok(Err(source)) => Err(failed(
            Code::FailedPrecondition,
            &MoveError::FreezeFailed { session, machine, source },
            "check that the machine is up and the session's terminal answers, then try the move again",
            None,
        )),
        Err(source) => Err(failed(
            Code::DeadlineExceeded,
            &MoveError::FreezeTimeout {
                session,
                machine,
                seconds: FREEZE_WAIT.as_secs(),
                source,
            },
            "the agent was left as it was; try the move again, and if it keeps happening end the session's terminal and resume it",
            None,
        )),
    }
}

/// Sessions with a move running on this daemon, and a count that moves
/// each time one starts or ends, for terminals to show it (MV-4).
struct MovingSet {
    /// How many holders each session has: the running move, and a recovery that
    /// still has to start its agent again.
    sessions: Mutex<HashMap<String, usize>>,
    /// Each session's running move: where it goes and the operation that moves it (MV-4).
    runs: Mutex<HashMap<String, MoveRun>>,
    changes: watch::Sender<u64>,
}

fn moving() -> &'static MovingSet {
    static MOVING: OnceLock<MovingSet> = OnceLock::new();
    MOVING.get_or_init(|| MovingSet {
        sessions: Mutex::default(),
        runs: Mutex::default(),
        changes: watch::channel(0).0,
    })
}

impl MovingSet {
    fn set(&self, session: &str, on: bool) {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        if on {
            *sessions.entry(session.to_owned()).or_default() += 1;
        } else if let Some(count) = sessions.get_mut(session) {
            *count -= 1;
            if *count == 0 {
                sessions.remove(session);
            }
        }
        drop(sessions);
        self.changes.send_modify(|count| *count += 1);
    }

    fn has(&self, session: &str) -> bool {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner).contains_key(session)
    }
}

/// The move a session's running holder belongs to: where it goes and the
/// operation that moves it (MV-4). Kept while the move runs.
#[derive(Clone)]
struct MoveRun {
    to: String,
    operation: String,
}

/// Forgets a session's move once no holder of the session is left.
fn forget_run(session: &str) {
    let mut runs = moving().runs.lock().unwrap_or_else(PoisonError::into_inner);
    if !moving().has(session) {
        runs.remove(session);
    }
}

/// The move a session's running holder belongs to, while it runs.
fn moving_run(session: &str) -> Option<MoveRun> {
    let run = moving().runs.lock().unwrap_or_else(PoisonError::into_inner).get(session).cloned()?;
    moving().has(session).then_some(run)
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
pub(crate) struct Moving(String);

impl Moving {
    pub(crate) fn begin(session: &str, to: &str, operation: &str) -> Self {
        moving().set(session, true);
        let run = MoveRun {
            to: to.to_owned(),
            operation: operation.to_owned(),
        };
        moving().runs.lock().unwrap_or_else(PoisonError::into_inner).insert(session.to_owned(), run);
        Self(session.to_owned())
    }
}

impl Drop for Moving {
    fn drop(&mut self) {
        moving().set(&self.0, false);
        forget_run(&self.0);
    }
}

/// Shows a session whose move is running as Moving (MV-4), whatever else is
/// known of it, with the machine it moves to and the move's operation. Neither
/// is set once the move has ended.
pub(super) fn show_moving(session: &mut pb::Session) {
    let run = session.id.as_ref().and_then(|id| moving_run(&id.value));
    if let Some(run) = run {
        session.status = pb::SessionStatus::Moving as i32;
        session.moving_to = Some(pb::MachineId { value: run.to });
        session.running_operation = Some(pb::OperationId { value: run.operation });
    } else {
        session.moving_to = None;
        session.running_operation = None;
    }
}

/// Refuses input to a session whose move runs on this daemon (MV-4): the
/// refusal names the session and the move, which must finish first.
pub(crate) fn refuse_input_while_moving(session: &str) -> Result<(), Status> {
    match moving_run(session) {
        Some(run) => Err(busy_running(
            session,
            &format!("moving to {}", run.to),
            Some(run.operation.as_str()),
        )),
        None => Ok(()),
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

// MACHINESCRIPTS (D681): the move's script triggers.
mod scripts;

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
        // UP-1, D281 e, bug 34: nothing moves to a machine at another release.
        crate::upgrade::gate::machine_ready(&self.migrations, &pb::MachineId { value: to.clone() })?;
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
        let home = self.home_of(&loaded.record).await?;
        let channel = home.channel.clone();
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
        // UP-1: nothing moves to a machine at another release.
        crate::upgrade::gate::machine_ready(&self.migrations, &pb::MachineId { value: to.clone() })?;
        // A review's channel is Humata's own (PR-21): the machine is given it
        // by the move, in its check, so it needn't be one of its machines yet.
        let review = loaded.review.is_some();
        if home.limited && !review && !channel.machines.iter().any(|machine| machine == &to) {
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
        let folder = home.folder.clone();

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
            review,
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
            frozen: AtomicBool::new(false),
        };
        tokio::spawn(async move { mover.run(held).await });
        Ok(Gated::Done(operation))
    }
}

/// Where a session lives in the channels tree, for a move (MV-16, MV-17).
struct Home {
    /// The channel a carry names: the session's own, or the channels folder
    /// itself for a Desk session.
    channel: ChannelRecord,
    /// The session's folder under the channel's folder: `sessions/<name>`,
    /// `desk/<name>` or `<channel folder>/desk/<name>`.
    folder: String,
    /// Only the channel's machines may take it (a channel session).
    limited: bool,
}

impl SessionSvc {
    /// Where `record`'s session lives, by what kind of session it is: a
    /// channel session under its channel, a plain Desk session in the Desk,
    /// a channel's Desk session in that channel's Desk. A Desk session works
    /// in the same place on every machine, so any linked machine may take it.
    async fn home_of(&self, record: &SessionRecord) -> Result<Home, Status> {
        let not_movable = || {
            failed(
                Code::FailedPrecondition,
                &MoveError::NotMovable {
                    folder: record.folder.display().to_string(),
                },
                "only a session Humata started in a channel, or in a scratch folder of a Desk, can move; a folder you picked stays where it is",
                None,
            )
        };
        if record.channel != humata2_sessions::DESK_CHANNEL {
            let channel = self.find_channel(&record.channel).await?;
            let folder = relative_folder(record, &self.starter.layout.sessions(&channel.folder)).ok_or_else(not_movable)?;
            return Ok(Home {
                channel,
                folder,
                limited: true,
            });
        }
        let (store, id) = (self.starter.store.clone(), record.id.clone());
        let (plain, own) = blocking(move || -> Result<_, StoreError> { Ok((store.desk(&id)?, store.channel_desk(&id)?)) })
            .await?
            .map_err(|error| status::store(error, &self.machine, "look up the Desk session"))?;
        let name = record
            .folder
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(not_movable)?;
        let folder = match (plain, own) {
            (_, Some(row)) => format!("{}/desk/{name}", self.find_channel(&row.channel).await?.folder),
            (Some(row), None) if matches!(row.kind, DeskFolderKind::Scratch { .. }) => format!("desk/{name}"),
            _ => return Err(not_movable()),
        };
        let channel = ChannelRecord {
            id: humata2_sessions::CHANNELS_ROOT.to_owned(),
            name: "Desk".to_owned(),
            folder: String::new(),
            branch_prefix: None,
            default_agent: String::new(),
            repos: Vec::new(),
            machines: Vec::new(),
            revision: 0,
            created_unix_ms: 0,
        };
        Ok(Home {
            channel,
            folder,
            limited: false,
        })
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
    /// The session is a pull request review: its target is given the PR
    /// reviews channel before anything is copied (PR-21).
    review: bool,
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
    /// The old agent's program is frozen (SIGSTOP) for this move, so it
    /// can't overlap the copy; the journal says so, and an undo or a
    /// restart thaws it (D801 J).
    frozen: AtomicBool,
}

impl Mover {
    async fn run(self, held: OwnedMutexGuard<()>) {
        let moving = Moving::begin(&self.session, &self.to, &self.operation);
        self.tell().await;
        self.reached("started").await;
        match self.go().await {
            Ok(()) => {
                // The journal stays until the new machine's copy of the
                // staged files is gone too; a link that is down then gets
                // the clean-up when it comes back (D207).
                match self.unstage_target().await {
                    Ok(()) => {
                        self.carry.journal_done(&self.operation).await;
                        // A clean-up that must wait is finished by `recover`, which fires this then.
                        self.script_arrived();
                    }
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
        let mut entry =
            Carry::entry(&self.operation, &self.session, &self.from, &self.to, phase, self.agent_ran.load(Ordering::SeqCst));
        // The move note says the old agent is frozen, so a restart thaws it
        // or finishes the move (D801 J).
        entry["frozen"] = serde_json::Value::Bool(self.frozen.load(Ordering::SeqCst));
        self.carry.journal(&self.operation, entry).await;
    }

    /// The move got as far as `phase`: the journal says so, and a test
    /// daemon may be killed here.
    async fn reached(&self, phase: &str) {
        self.journal(phase).await;
        crate::fault::point(&format!("move-{phase}"));
        crate::fault::hold_point(&format!("move-{phase}")).await;
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
        self.step("pause", self.freeze_old()).await?;
        self.go_on("pack")?;
        let packed = self
            .step("pack", self.pack_session())
            .await?;
        self.agent_ran.store(packed.agent_running, Ordering::SeqCst);
        self.reached("packed").await;
        self.go_on("copy")?;
        // Something of the copy may be on the new machine from here, so the
        // journal says so before the first byte goes there (MV-14).
        self.placing.store(true, Ordering::SeqCst);
        self.reached("copying").await;
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

    /// A review's target machine holds the PR reviews channel, with a main
    /// clone of each repository, as a review started there would have made
    /// it, and is one of the channel's machines (PR-4, PR-21). The host has
    /// them already. Nothing of the session is touched.
    async fn prepare_review_target(&self) -> Result<(), Status> {
        if !self.review || self.carry.is_host(&self.to) {
            return Ok(());
        }
        let repos: Vec<pb::RepoRef> = self
            .channel
            .repos
            .iter()
            .map(|repo| pb::RepoRef {
                owner: repo.owner.clone(),
                name: repo.name.clone(),
            })
            .collect();
        let steps = channel_steps(&self.to, &repos);
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                value: self.channel.id.clone(),
            })),
        };
        let Started::New(progress) =
            self.svc
                .operations
                .start_steps(&self.caller, pb::OperationKind::StartPrReview, target, &steps, false, None)?
        else {
            return Err(status::internal("a channel setup without a key was taken for a repeat", ""));
        };
        let host = Arc::new(HostOp::new(humata2_svc_peer::ports::Progress::new(
            Arc::new(crate::wire_peer::ProgressAdapter(std::sync::Mutex::new(Some(progress)))),
        )));
        let create = pb::CreateChannelRequest {
            idempotency_key: None,
            name: self.channel.name.clone(),
            repos,
            machines: Vec::new(),
        };
        let ended = match self.remote.create_channel_on(&self.to, &self.channel.id, create, host.clone()).await {
            MachineOutcome::Ready => Ok(()),
            MachineOutcome::Failed(error) => Err(failed(
                Code::FailedPrecondition,
                &MoveError::ReviewChannelNotMade {
                    session: self.session.clone(),
                    machine: self.to.clone(),
                    reason: error.message,
                },
                "check that the machine is online, has git, and can reach GitHub to clone the repositories, then move the review again; nothing was changed",
                None,
            )),
            MachineOutcome::Gone => Err(failed(
                Code::FailedPrecondition,
                &MoveError::ReviewMachineRemoved {
                    session: self.session.clone(),
                    machine: self.to.clone(),
                },
                "add the machine again from the Machines page, or move the review to another machine",
                None,
            )),
        };
        if let Some(progress) = host.take() {
            match &ended {
                Ok(()) => progress.succeed(None),
                Err(status) => progress.fail(status::error_of(status).unwrap_or_else(|| pb::Error {
                    message: status.message().to_owned(),
                    fix: String::new(),
                    retry: None,
                    kind: None,
                })),
            }
        }
        ended?;
        self.svc.add_machine_to(&self.channel, &self.to, &self.caller).await
    }

    /// The new machine can hold the session, and has nothing under the
    /// folder's name.
    async fn check(&self) -> Result<(), Status> {
        self.prepare_review_target().await?;
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

    /// The session's folder, then the staged files, onto the new machine,
    /// leaving out what the new machine rebuilds or holds (MV-17). The copy
    /// step counts the bytes it carries, and says what crossed and what was
    /// reused once it is done.
    async fn copy(&self, packed: &pb::CarryPackResponse) -> Result<(), Status> {
        let pass = self.pass.fetch_add(1, Ordering::SeqCst);
        let units = self
            .progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(Progress::units);
        let tally = Arc::new(Tally::default());
        if let Some(units) = units.clone() {
            let tally = tally.clone();
            self.carry.hear(&self.operation, move |snapshot| {
                tally.note(snapshot);
                units.set("copy", tally.done(), tally.total());
            });
        }
        // The chunks of big files count too, as each is sent or reused.
        let chunks: Option<Arc<dyn Fn(Chunks) + Send + Sync>> = units.clone().map(|units| {
            let tally = tally.clone();
            let tell: Arc<dyn Fn(Chunks) + Send + Sync> = Arc::new(move |event: Chunks| {
                tally.chunks(event);
                units.set("copy", tally.done(), tally.total());
            });
            tell
        });
        let before = self.carry.crossed(&self.to);
        let copied = self
            .carry
            .copy_session(SessionCopy {
                id: format!("{}_{pass}", self.operation),
                from: &self.at_from,
                to: &self.at_to,
                folder: &packed.folder,
                stage: &packed.stage,
                into: format!("{}/{}/in", humata2_sessions::CARRY_DIR, self.operation),
                chunks,
            })
            .await;
        self.carry.unhear(&self.operation);
        let copied = copied?;
        if let Some(units) = units {
            let whole = tally.total();
            units.set("copy", whole, whole);
            units.add_fact("copy", "sent_bytes", self.carry.crossed(&self.to).saturating_sub(before));
            units.add_fact("copy", "reused_bytes", copied.reused_bytes);
            units.add_fact("copy", "chunk_bytes", copied.chunk_bytes);
        }
        Ok(())
    }

    /// Packs the session on the old machine, its conversation written for
    /// the folder the new machine will put it in, which that machine says
    /// (MV-17): what it holds of an earlier copy then matches.
    async fn pack_session(&self) -> Result<pb::CarryPackResponse, Status> {
        let target = self.carry.target(&self.to, &self.channel.id, &self.folder).await?;
        self.carry.pack_for(&self.from, &self.operation, &self.session, &target).await
    }

    async fn unpack(&self) -> Result<(), Status> {
        self.carry
            .unpack(&self.to, &self.operation, &self.session, &self.channel.id)
            .await
            .map(drop)
    }

    /// Freezes the old agent before anything is packed (D801 J): its program
    /// stops where it is, so it can't write while it is copied nor run
    /// beside the new agent. The note says so first, so a restart that finds
    /// the program stopped thaws it. A machine whose agent can't be frozen
    /// refuses the move here, before anything is carried.
    async fn freeze_old(&self) -> Result<(), Status> {
        self.frozen.store(true, Ordering::SeqCst);
        self.journal("started").await;
        match self.freeze_on(&self.from, false).await {
            Ok(true) => Ok(()),
            Ok(false) => {
                // No agent ran: nothing was frozen.
                self.frozen.store(false, Ordering::SeqCst);
                self.journal("started").await;
                Ok(())
            }
            Err(error) => {
                // A freeze that failed part way is thawed; one the holder was
                // too old to take stopped nothing.
                match self.freeze_on(&self.from, true).await {
                    Err(thaw) if thaw.code() != Code::FailedPrecondition => {}
                    _ => {
                        self.frozen.store(false, Ordering::SeqCst);
                        self.journal("started").await;
                    }
                }
                Err(error)
            }
        }
    }

    /// Freezes or thaws the session's agent on `machine`. Whether an agent
    /// ran there.
    async fn freeze_on(&self, machine: &str, thaw: bool) -> Result<bool, Status> {
        if self.carry.is_host(machine) {
            return freeze_agent_here(&self.svc, &self.session, machine, thaw).await;
        }
        match self.remote.freeze_session_on(machine, &self.session, thaw).await {
            Ok(ran) => Ok(ran),
            // The machine's daemon is from a build before freezing: it
            // stopped nothing.
            Err(RemoteError::Machine { source, .. }) if source.code() == Code::Unimplemented => Err(failed(
                Code::FailedPrecondition,
                &MoveError::MachineTooOldToFreeze {
                    session: self.session.clone(),
                    machine: machine.to_owned(),
                    source,
                },
                MIGRATE_FIRST,
                None,
            )),
            // Nothing of the session is there to let run.
            Err(RemoteError::Machine { source, .. }) if thaw && source.code() == Code::NotFound => Ok(false),
            Err(error) => Err(Status::from(error)),
        }
    }

    /// The new agent runs and answered (the resume's check), so the frozen
    /// old one is ended. It is the end of the overlap-free hand-over: until
    /// here an undo only thaws it.
    async fn end_frozen_old(&self) -> Result<(), Status> {
        self.stop_on(&self.from).await?;
        self.stopped.store(self.agent_ran.load(Ordering::SeqCst), Ordering::SeqCst);
        self.frozen.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// The new agent starts in the copied conversation, and once it has
    /// answered the old one, frozen since the move began, is ended. An old
    /// agent that wasn't frozen is stopped after the copy is checked, so a
    /// failed copy leaves it running, and the session is packed again to
    /// see that nothing changed while it ran.
    async fn hand_over(&self, digest: &str) -> Result<(), Status> {
        // MACHINESCRIPTS (D681): the old machine's leaving rules, a minute at most; never fails the move.
        self.script_leaving().await;
        if self.frozen.load(Ordering::SeqCst) {
            // Frozen, it wrote nothing since it was packed: what was copied is all of it.
            self.resume_on(&self.to).await?;
            return self.end_frozen_old().await;
        }
        self.stop_old().await?;
        let again = self.pack_session().await?;
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
            return self.remote.stop_moving_session_on(machine, &self.session).await.map(drop).map_err(Status::from);
        }
        let loaded = self.svc.load(&self.session).await?;
        let Some(mut agent) = loaded.agent else {
            return Ok(());
        };
        let AgentState::Running { holder_pid } = agent.state else {
            return Ok(());
        };
        if let Err(error) = holder_end::shut_or_end(
            &agent.launch.holder_socket,
            holder_pid,
            &format!("session {}'s terminal", self.session),
        )
        .await
        {
            crate::log::say!("humatad2: a move of session {}: {error}", self.session);
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
            let loaded = Loaded { moved: Some(super::MOVED), ..self.svc.load(&self.session).await? };
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
                mode: pb::resume_session_request::Mode::AfterMove as i32,
                ..pb::ResumeSessionRequest::default()
            };
            let host = Arc::new(HostOp::new(humata2_svc_peer::ports::Progress::new(
            Arc::new(crate::wire_peer::ProgressAdapter(std::sync::Mutex::new(Some(progress)))),
        )));
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
        if self.channel.id == humata2_sessions::CHANNELS_ROOT {
            // A Desk session's folder is where the machine keeps it.
            record.folder = self.folder_after().await?;
        }
        let store = self.svc.starter.store.clone();
        blocking(move || store.put_session(&record))
            .await?
            .map_err(|error| status::store(error, &self.svc.machine, "save the session's new machine"))?;
        if !self.carry.is_host(&self.from) {
            self.remote.forget_session(&self.from, &self.session).await;
        }
        Ok(())
    }

    /// Where the session's folder is on the new machine, for a Desk session,
    /// whose host record names it: the host's own place, or the one the
    /// machine says it took the session into.
    async fn folder_after(&self) -> Result<std::path::PathBuf, Status> {
        if self.carry.is_host(&self.to) {
            return Ok(self.svc.starter.layout.channel(&self.channel.folder).join(&self.folder));
        }
        let (store, id) = (self.svc.starter.store.clone(), self.session.clone());
        let own = blocking(move || store.channel_desk(&id))
            .await?
            .map_err(|error| status::store(error, &self.svc.machine, "look up the channel's Desk session"))?;
        let asked = pb::ListDeskSessionsRequest {
            channel: own.map(|row| pb::ChannelId { value: row.channel }),
            target: None,
        };
        let relayed = self.remote.relayed(&self.to).map_err(Status::from)?;
        let call = crate::host_relay::assigned(asked, None).map_err(Status::from)?;
        let listed = relayed.desk().list_desk_sessions(call).await?.into_inner();
        listed
            .sessions
            .into_iter()
            .find(|shown| shown.session.as_ref().is_some_and(|session| session.value == self.session))
            .and_then(|shown| shown.folder)
            .map(|folder| std::path::PathBuf::from(folder.path))
            .ok_or_else(|| {
                status::internal(
                    format!("{} didn't say where session {} works now", self.to, self.session),
                    "",
                )
            })
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
                crate::fault::point("move-released");
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

    /// Takes back what the move put on the new machine once its copy began
    /// there, and what it staged on the old one. Before that the new
    /// machine's folder of that name is not the move's (MV-3, MV-14).
    async fn take_copy_back(&self) -> (Result<(), Status>, Result<(), Status>) {
        let incoming = if self.placing.load(Ordering::SeqCst) {
            self.carry
                .abort(&self.to, &self.operation, &self.session, &self.at_to, &self.folder, true)
                .await
        } else {
            Ok(())
        };
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
        let placed = self.placing.load(Ordering::SeqCst);
        let mut taken_back = true;
        if placed {
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
        // The old agent was stopped and didn't start again: the journal says
        // so, and the host starts it once both machines' links are up; a
        // restart's report names the session if that fails too (MV-14, MV-17).
        let mut resume_pending = false;
        // The old agent was frozen for the move and still is: it runs again,
        // as it was. Its holder being gone leaves nothing to thaw.
        if self.frozen.load(Ordering::SeqCst) {
            match self.freeze_on(&self.from, true).await {
                Ok(_) => self.frozen.store(false, Ordering::SeqCst),
                Err(error) => {
                    resume_pending = true;
                    left = format!(
                        " Its agent was paused for the move and didn't run again on {}: {}. Humata tries again once the machines are linked.",
                        self.from,
                        error.message()
                    );
                }
            }
        }
        if !resume_pending
            && self.stopped.load(Ordering::SeqCst)
            && let Err(error) = self.resume_on(&self.from).await
        {
            resume_pending = true;
            left = format!(
                " Its agent was stopped for the move and didn't start again on {}: {}. Humata tries again once the machines are linked; or resume it from the session's menu.",
                self.from,
                error.message()
            );
        }
        // Only a copy that began on the new machine needs its journal kept for
        // a restart to take back; otherwise the journal keeps the phase it had,
        // which says nothing reached the new machine.
        if resume_pending {
            // `unpacked` is the phase whose recovery takes the copy back and
            // starts the old agent again; the agent only stops after a copy.
            self.journal("unpacked").await;
            self.recover_resume_later();
        } else if placed {
            self.journal("aborted").await;
        }
        // With a resume pending the journal stays until the retry settles it.
        if taken_back && !resume_pending {
            self.carry.journal_done(&self.operation).await;
        } else if !resume_pending {
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

/// Whether a journal at `phase` says the copy into the new machine began, so
/// a restart takes that copy back (MV-14). `copying` is journaled before the
/// first byte goes there; `packed` comes before it, when the new machine
/// has nothing of the move yet.
fn copy_began(phase: &str) -> bool {
    matches!(phase, "copying" | "copied" | "unpacked" | "resumed" | "switched" | "aborted")
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
    let home = svc.home_of(&loaded.record).await?;
    let (channel, folder) = (home.channel, home.folder);
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
        review: loaded.review.is_some(),
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
        placing: AtomicBool::new(copy_began(&journal.phase)),
        agent_ran: AtomicBool::new(journal.agent_ran),
        rebased: AtomicBool::new(false),
        frozen: AtomicBool::new(journal.frozen),
    };
    let outcome = match journal.phase.as_str() {
        "resumed" => mover.finish(true).await,
        "switched" => mover.finish(false).await,
        phase => mover.undo_cut_short(phase == "unpacked").await,
    };
    if outcome.is_ok() {
        mover.carry.journal_done(&mover.operation).await;
        // MACHINESCRIPTS (D681): a move finished by recovery has arrived too.
        if matches!(journal.phase.as_str(), "resumed" | "switched") {
            mover.script_arrived();
        }
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
                frozen: self.frozen.load(Ordering::SeqCst),
            },
            None,
        );
    }

    /// The old agent didn't start again after an undo: the journal's
    /// recovery runs once both links are up, and the session shows Moving
    /// until it has.
    fn recover_resume_later(&self) {
        self.svc.recover_when_linked(
            Journaled {
                operation: self.operation.clone(),
                session: self.session.clone(),
                from: self.from.clone(),
                to: self.to.clone(),
                phase: "unpacked".to_owned(),
                agent_ran: true,
                frozen: self.frozen.load(Ordering::SeqCst),
            },
            Some(Moving::begin(&self.session, &self.to, &self.operation)),
        );
    }

    /// The agent was never taken over by the new machine: whatever started
    /// there stops, the new machine's copy is taken back and the agent runs
    /// on the old one again. `handing_over` says the old agent may have been
    /// stopped for the move.
    async fn undo_cut_short(&self, handing_over: bool) -> Result<(), Status> {
        if self.placing.load(Ordering::SeqCst) {
            // An agent the resume started there stops before its copy goes.
            let started_there = self.stop_on(&self.to).await;
            if handing_over {
                match started_there {
                    // The new machine holds no record of the session, as when it
                    // was cut before it took the copy in: no agent runs there,
                    // and its folder still has to be taken back.
                    Err(status) if status.code() == Code::NotFound => {}
                    other => other?,
                }
            }
        }
        // The old agent was frozen for the move: it runs again before the
        // copy is taken back, so a target that is offline doesn't keep it
        // stopped (D801 J).
        if self.frozen.load(Ordering::SeqCst) {
            self.freeze_on(&self.from, true).await?;
            self.frozen.store(false, Ordering::SeqCst);
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
