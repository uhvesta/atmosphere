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

pub mod scripts;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use humata2_daemon_ports::Caller;
use humata2_store::{ChannelRecord, SessionRecord};
use humata2_svc_sessions_common::pb;
use pb::invalid_field::Problem;
use tokio::sync::watch;
use tonic::Status;

use crate::SessionTransferSvc;
use crate::ports::{invalid_field, missing, require_user};

pub const MOVE_SESSION: &str = "humata.v1.SessionService.MoveSession";

/// The steps, as `(key, title)`.
pub const STEPS: [(&str, &str); 8] = [
    ("check", "Checking the new machine can take it"),
    ("pause", "Pausing the agent"),
    ("pack", "Listing what moves"),
    ("copy", "Copying the folder and conversation"),
    ("verify", "Checking every file arrived"),
    ("resume", "Starting the agent on the new machine"),
    ("switch", "Switching to the new machine"),
    ("release", "Moving the old copy to the old machine's trash"),
];

/// Says what to do about a holder or machine too old to pause an agent.
pub const MIGRATE_FIRST: &str = "click Migrate on the \"Migrate the holders\" item in the Inbox, which moves each holder onto the current build without restarting its agent where it can and updates the machine's tools, then move the session again";

/// How long a holder gets to answer a freeze or thaw.
pub const FREEZE_WAIT: Duration = Duration::from_secs(15);

/// The line a session's shell tab starts with when its agent's resume
/// starts it again, after the output its old holder saved.
pub const SHELL_BACK: &str = "This terminal's earlier shell is gone: the session moved to this machine, or its shell had stopped. \
     Programs that ran in it didn't come along; the output above is what it had saved.";

#[derive(Debug, thiserror::Error)]
pub enum MoveError {
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
    #[error("the holder of session {session}'s agent on {machine} is from an older Humata build that can't pause its program for a move, so nothing was copied and the agent keeps running there: {detail}")]
    HolderTooOldToFreeze {
        session: String,
        machine: String,
        detail: String,
    },
    #[error("{machine} runs an older humatad2 that can't pause session {session}'s agent for a move, so nothing was copied and the agent keeps running there: {detail}")]
    MachineTooOldToFreeze {
        session: String,
        machine: String,
        detail: String,
    },
    #[error("couldn't pause session {session}'s agent on {machine} for the move; the agent was left running: {detail}")]
    FreezeFailed {
        session: String,
        machine: String,
        detail: String,
    },
    #[error("the holder of session {session}'s agent on {machine} didn't answer within {seconds} seconds when asked to pause or resume the agent; it may be wedged")]
    FreezeTimeout {
        session: String,
        machine: String,
        seconds: u64,
    },
}

/// The holders (socket and pid) this daemon froze for a move and hasn't
/// thawed: the running list reads them as "Paused for move" (D801 J). A
/// holder ended with its move is never listed again, as the pid is its.
static FROZEN: Mutex<BTreeSet<(PathBuf, u32)>> = Mutex::new(BTreeSet::new());

/// Whether the holder at `socket` with process `pid` is frozen for a move
/// this daemon is making.
pub fn paused_for_move(socket: &Path, pid: u32) -> bool {
    FROZEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(&(socket.to_owned(), pid))
}

/// Records or clears the frozen state of `(socket, pid)`.
pub fn note_frozen(socket: &Path, pid: u32, frozen: bool) {
    let mut held = FROZEN.lock().unwrap_or_else(PoisonError::into_inner);
    if frozen {
        held.insert((socket.to_owned(), pid));
    } else {
        held.remove(&(socket.to_owned(), pid));
    }
}

/// Sessions with a move running on this daemon, and a count that moves
/// each time one starts or ends, for terminals to show it (MV-4).
struct MovingSet {
    sessions: Mutex<HashMap<String, usize>>,
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
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(session)
    }
}

/// The move a session's running holder belongs to: where it goes and the
/// operation that moves it (MV-4). Kept while the move runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveRun {
    pub to: String,
    pub operation: String,
}

/// Forgets a session's move once no holder of the session is left.
fn forget_run(session: &str) {
    let mut runs = moving().runs.lock().unwrap_or_else(PoisonError::into_inner);
    if !moving().has(session) {
        runs.remove(session);
    }
}

/// The move a session's running holder belongs to, while it runs.
pub fn moving_run(session: &str) -> Option<MoveRun> {
    let run = moving()
        .runs
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(session)
        .cloned()?;
    moving().has(session).then_some(run)
}

/// Whether a move of `session` runs on this daemon now: its terminal is
/// read-only until the agent resumes (MV-4).
pub fn is_moving(session: &str) -> bool {
    moving().has(session)
}

/// Changes whenever a move starts or ends.
pub fn moving_changes() -> watch::Receiver<u64> {
    moving().changes.subscribe()
}

/// A session's place in the set of those being moved, for as long as the
/// move runs.
pub struct Moving(String);

impl Moving {
    pub fn begin(session: &str, to: &str, operation: &str) -> Self {
        moving().set(session, true);
        let run = MoveRun {
            to: to.to_owned(),
            operation: operation.to_owned(),
        };
        moving()
            .runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session.to_owned(), run);
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
pub fn show_moving(session: &mut pb::Session) {
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
pub fn refuse_input_while_moving(session: &str) -> Result<(), Status> {
    match moving_run(session) {
        Some(run) => Err(Status::failed_precondition(format!(
            "session {session} is moving to {} (operation {})",
            run.to, run.operation
        ))),
        None => Ok(()),
    }
}

/// The machine `record` lives on, from the host's point of view.
pub fn machine_of(record: &SessionRecord, here: &str) -> String {
    if record.machine.is_empty() {
        here.to_owned()
    } else {
        record.machine.clone()
    }
}

/// `sessions/<name>` for `record`'s folder, when it is directly in `sessions`.
pub fn relative_folder(record: &SessionRecord, sessions: &Path) -> Option<String> {
    let parent = record.folder.parent()?;
    (parent == sessions).then_some(())?;
    let name = record.folder.file_name()?.to_str()?;
    Some(format!("sessions/{name}"))
}

/// Whether a journal at `phase` says the copy into the new machine began, so
/// a restart takes that copy back (MV-14). `copying` is journaled before the
/// first byte goes there; `packed` comes before it, when the new machine
/// has nothing of the move yet.
pub fn copy_began(phase: &str) -> bool {
    matches!(
        phase,
        "copying" | "copied" | "unpacked" | "resumed" | "switched" | "aborted"
    )
}

/// Where a session lives in the channels tree, for a move (MV-16, MV-17).
#[derive(Clone, Debug)]
pub struct Home {
    /// The channel a carry names: the session's own, or the channels folder
    /// itself for a Desk session.
    pub channel: ChannelRecord,
    /// The session's folder under the channel's folder: `sessions/<name>`,
    /// `desk/<name>` or `<channel folder>/desk/<name>`.
    pub folder: String,
    /// Only the channel's machines may take it (a channel session).
    pub limited: bool,
}

impl SessionTransferSvc {
    /// Handles `MoveSession` for `SessionTransferPort`.
    pub(crate) async fn handle_move_session(
        &self,
        caller: &Caller,
        req: pb::MoveSessionRequest,
    ) -> Result<pb::Operation, Status> {
        require_user(caller)?;
        let id = req
            .id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| missing("id"))?;
        let to = req
            .to
            .map(|machine| machine.value)
            .filter(|machine| !machine.is_empty())
            .ok_or_else(|| missing("to"))?;

        let loaded = self.env.load_session(&self.core, &id)?;
        let from = machine_of(&loaded.record, &self.core.machine);
        if from == to {
            return Err(invalid_field(
                "to",
                Problem::NotAllowedHere,
                MoveError::SameMachine {
                    session: id,
                    machine: to,
                }
                .to_string(),
                "pick another machine of the channel to move it to",
            ));
        }
        if self.env.is_moving(&id) || is_moving(&id) {
            return Err(Status::failed_precondition(format!(
                "session {id} is already being moved"
            )));
        }

        // Carry/remote transfer orchestration is wired by the host adapter.
        Err(self.env.not_wired(MOVE_SESSION))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(machine: &str, folder: &str) -> SessionRecord {
        SessionRecord {
            id: "se_move_test".to_owned(),
            channel: "ch_1".to_owned(),
            machine: machine.to_owned(),
            title: "Move test".to_owned(),
            agent: "claude".to_owned(),
            folder: PathBuf::from(folder),
            created_unix_ms: 0,
        }
    }

    #[test]
    fn copy_began_distinguishes_pre_copy_and_post_copy_phases() {
        assert!(!copy_began("started"));
        assert!(!copy_began("packed"));
        for phase in ["copying", "copied", "unpacked", "resumed", "switched", "aborted"] {
            assert!(copy_began(phase), "{phase} should count as copy begun");
        }
    }

    #[test]
    fn machine_of_defaults_empty_machine_to_here() {
        let local = sample_record("", "/ch/sessions/a");
        assert_eq!(machine_of(&local, "mac-1"), "mac-1");
        let remote = sample_record("linux-2", "/ch/sessions/a");
        assert_eq!(machine_of(&remote, "mac-1"), "linux-2");
    }

    #[test]
    fn relative_folder_matches_direct_children_of_sessions() {
        let rec = sample_record("m1", "/h/ch/pay/sessions/fix-1");
        assert_eq!(
            relative_folder(&rec, Path::new("/h/ch/pay/sessions")),
            Some("sessions/fix-1".to_owned())
        );
        assert_eq!(
            relative_folder(&rec, Path::new("/h/ch/other/sessions")),
            None
        );
    }

    #[test]
    fn moving_guard_tracks_and_clears_session_state() {
        let id = format!("se_mv_{}", uuid::Uuid::new_v4().simple());
        assert!(!is_moving(&id));
        assert!(refuse_input_while_moving(&id).is_ok());

        let guard = Moving::begin(&id, "m2", "op_1");
        assert!(is_moving(&id));
        assert!(refuse_input_while_moving(&id).is_err());

        let mut pb_sess = pb::Session {
            id: Some(pb::SessionId { value: id.clone() }),
            ..Default::default()
        };
        show_moving(&mut pb_sess);
        assert_eq!(pb_sess.status, pb::SessionStatus::Moving as i32);
        assert_eq!(pb_sess.moving_to.as_ref().unwrap().value, "m2");
        assert_eq!(pb_sess.running_operation.as_ref().unwrap().value, "op_1");

        drop(guard);
        assert!(!is_moving(&id));
        show_moving(&mut pb_sess);
        assert!(pb_sess.moving_to.is_none());
        assert!(pb_sess.running_operation.is_none());
    }

    #[test]
    fn frozen_set_tracks_socket_and_pid() {
        let sock = PathBuf::from(format!("/tmp/h2-freeze-{}.sock", uuid::Uuid::new_v4().simple()));
        assert!(!paused_for_move(&sock, 42));
        note_frozen(&sock, 42, true);
        assert!(paused_for_move(&sock, 42));
        note_frozen(&sock, 42, false);
        assert!(!paused_for_move(&sock, 42));
    }
}
