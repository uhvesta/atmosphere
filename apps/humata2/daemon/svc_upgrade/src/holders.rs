//! The holders of this Mac, across a daemon upgrade (U6, D373, PER-5, PER-6).
//!
//! A daemon upgrade never touches a running holder, so a session started
//! under release `a` keeps `a`'s holder when the daemon becomes release `b`.
//! Two parts make that visible and let the user end it:
//!
//! - [`Builds`] keeps, for each holder this daemon spawns, the sha256 of the
//!   `--holder` file it was spawned from, as `<state>/holder_builds/<name>.sha`
//!   (`<session>.sha` for an agent, `<tab>.shell.sha` for a shell tab), and a
//!   second line `moves` when that file can move a running holder onto
//!   another build in place. It is written just before the holder starts,
//!   through a temporary file renamed into place, and never when a holder is
//!   only reattached.
//! - [`Holders`] compares those records with the `humata2_holder` the
//!   release names. A terminal whose holder differs, or has no record (it
//!   began before D373), is on an old holder. While this daemon's own
//!   `--holder` file is the release's, one pending Holders item for this
//!   machine lists them. Migrate moves each holder whose record says `moves`
//!   onto the current build in place, one at a time, the first being the
//!   canary: same process, same program, same screen, nothing restarts, and
//!   the round stops at the first move that fails. An agent whose holder is
//!   too old to move is restarted onto the current holder at its next idle
//!   moment, in the same session and conversation; a shell whose holder is
//!   too old stays until it is closed and never holds the item open. The item
//!   ends Approved once nothing is left to move or restart.
//!
//! A machine's own daemon looks the same way at its own holders (D378). It
//! posts no item: it pushes the list to the host as a [`Reports`] value, which
//! the uplink sends on each change and again on every link, and the host
//! posts the item from it. An Upgrade of the holders from the host starts the
//! machine's restart loop; asking again changes nothing.
//!
//! Nothing here polls. A look is made when the daemon's start is done, after
//! each holder spawn, when a session changes (an exit, a restart), when any
//! agent's status changes (the idle moment Migrate waits for), and when the
//! bin folder changes (`Local`).

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use humata2_machines::InstalledFile;
use humata2_revive::{HolderBuilds, ReviveError, Revived};
use humata2_rpc_kit::helpers as status;
use humata2_sessions::Starter;
use humata2_store::{AgentRecord, AgentState, ResumeFailure, StoreError, TabKind, TabRecord};
use tokio::sync::{Notify, broadcast, watch};

use super::build::Release;
use super::items::{Carry, Detail, Migrations, Progress, outstanding, waiting_sessions};
use super::run::{RunError, hash_file};
use super::tools;
use crate::ports::{
    HolderEventsPort, HolderSessionPort, HolderStartedPort, HolderTokensPort, HolderTurnsPort,
};

/// Shown first in the terminal of an agent restarted onto the current
/// holder, after `[Humata] `.
const RESTARTED: &str = "Restarted on the current release's holder; the conversation continues.";

/// Why a record of a holder's build couldn't be read or kept. Never stops a
/// holder from starting.
#[derive(Debug, thiserror::Error)]
enum BuildsError {
    #[error(
        "couldn't read the holder file {} to note which release it is, so sessions started from it count as on an old holder. Make the file readable by your user, or install Humata again",
        .path.display()
    )]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't keep {}, which says which holder file a session's holder was spawned from, so that session counts as on an old holder. Check that your user can write the state folder and that it has room",
        .path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// What the holder file looked like when it was last hashed, so an
/// unchanged file isn't read again for every spawn.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    inode: u64,
    len: u64,
    modified: (i64, i64),
}

impl Stamp {
    fn of(meta: &fs::Metadata) -> Self {
        Self {
            inode: meta.ino(),
            len: meta.len(),
            modified: (meta.mtime(), meta.mtime_nsec()),
        }
    }
}

/// Which holder file each holder was spawned from.
#[derive(Debug)]
pub struct Builds {
    dir: PathBuf,
    holder: PathBuf,
    /// Woken when an agent's record is kept: what waits may have changed.
    wake: Arc<Notify>,
    /// The holder file's stamp, sha256 and whether it can move a holder.
    hashed: Mutex<Option<(Stamp, String, bool)>>,
}

/// What a record says of the holder file a holder was spawned from.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Recorded {
    sha: String,
    /// The file could move a running holder onto another build in place.
    /// A record written before moving existed says nothing here, so it is
    /// too old to move.
    moves: bool,
}

/// How long `--handoff-versions` has to answer.
const PROBE_LIMIT: Duration = Duration::from_secs(20);

/// Whether the holder file at `holder` can move a running holder onto another
/// build: it answers `--handoff-versions` with at least one version number.
/// Any failure says no, which is the safe answer: the holder is restarted
/// instead.
fn can_move(holder: &Path) -> bool {
    let spawned = Command::new(holder)
        .arg("--handoff-versions")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn();
    let Ok(mut child) = spawned else {
        return false;
    };
    // Read on its own thread so a full pipe never stalls the child.
    let reader = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = out.read_to_string(&mut text);
            text
        })
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < PROBE_LIMIT => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let Some(reader) = reader else {
        return false;
    };
    let Ok(text) = reader.join() else {
        return false;
    };
    status.is_some_and(|status| status.success()) && text.split_whitespace().any(|word| word.parse::<u32>().is_ok())
}

impl Builds {
    /// The records of the daemon whose state folder is `state`, which spawns
    /// holders from `holder`.
    pub fn new(state: &Path, holder: &Path) -> Self {
        Self {
            dir: state.join("holder_builds"),
            holder: holder.to_owned(),
            wake: Arc::new(Notify::new()),
            hashed: Mutex::new(None),
        }
    }

    /// The `--holder` file this daemon spawns holders from, and moves them onto.
    fn holder_file(&self) -> &Path {
        &self.holder
    }

    /// Woken after each agent holder is about to spawn.
    fn wake(&self) -> Arc<Notify> {
        self.wake.clone()
    }

    /// The sha256 of the `--holder` file now.
    fn current(&self) -> Result<String, BuildsError> {
        self.current_build().map(|(sha, _)| sha)
    }

    /// The sha256 of the `--holder` file now, and whether it can move a
    /// holder in place. Both are worked out once per change of the file.
    fn current_build(&self) -> Result<(String, bool), BuildsError> {
        let read = |source| BuildsError::Read {
            path: self.holder.clone(),
            source,
        };
        let stamp = Stamp::of(&fs::metadata(&self.holder).map_err(read)?);
        if let Some((seen, sha, moves)) = &*self.hashed.lock().unwrap_or_else(PoisonError::into_inner)
            && *seen == stamp
        {
            return Ok((sha.clone(), *moves));
        }
        let sha = hash_file(&self.holder).map_err(read)?;
        let moves = can_move(&self.holder);
        *self.hashed.lock().unwrap_or_else(PoisonError::into_inner) = Some((stamp, sha.clone(), moves));
        Ok((sha, moves))
    }

    /// What is recorded under `name`, if there is a record.
    fn recorded(&self, name: &str) -> Option<Recorded> {
        let text = fs::read_to_string(self.dir.join(name)).ok()?;
        let mut lines = text.lines().map(str::trim);
        let sha = lines.next().filter(|sha| !sha.is_empty())?.to_owned();
        Some(Recorded {
            sha,
            moves: lines.any(|line| line == "moves"),
        })
    }

    /// Whether the holder recorded under `name` is on the current holder file.
    fn on_current(&self, name: &str) -> bool {
        match (self.recorded(name), self.current()) {
            (Some(recorded), Ok(current)) => recorded.sha.eq_ignore_ascii_case(&current),
            _ => false,
        }
    }

    fn keep(&self, name: &str) -> Result<(), BuildsError> {
        let (sha, moves) = self.current_build()?;
        self.write(name, &sha, moves)
    }

    /// The record under `name` no longer says its holder can move, because a
    /// move found it too old. Its sha stays. Nothing is written when there is
    /// no record.
    fn too_old(&self, name: &str) -> Result<(), BuildsError> {
        match self.recorded(name) {
            Some(recorded) if recorded.moves => self.write(name, &recorded.sha, false),
            _ => Ok(()),
        }
    }

    fn write(&self, name: &str, sha: &str, moves: bool) -> Result<(), BuildsError> {
        let path = self.dir.join(name);
        let failed = |source| BuildsError::Write {
            path: path.clone(),
            source,
        };
        humata2_durable::create_dir_all(&self.dir, 0o700).map_err(failed)?;
        let temp = self.dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let written = File::create(&temp).and_then(|mut file| {
            file.write_all(sha.as_bytes())?;
            if moves {
                file.write_all(b"\nmoves\n")?;
            }
            file.sync_all()
        });
        written
            .and_then(|()| humata2_durable::rename(&temp, &path))
            .and_then(|()| humata2_durable::sync_dir(&self.dir))
            .map_err(|source| {
                let _ = fs::remove_file(&temp);
                failed(source)
            })
    }

    fn keep_logged(&self, name: &str) {
        if let Err(error) = self.keep(name) {
            humata2_rpc_kit::log::say!("humatad2: warning: {}", status::chain(&error));
        }
    }

    /// A holder is about to start for `session`'s agent.
    pub fn record_agent(&self, session: &str) {
        self.keep_logged(&format!("{session}.sha"));
        self.wake.notify_one();
    }

    /// A holder is about to start for the shell tab `tab`.
    pub fn record_shell(&self, tab: &str) {
        self.keep_logged(&format!("{tab}.shell.sha"));
        self.wake.notify_one();
    }
}

impl HolderBuilds for Builds {
    fn agent_spawned(&self, session: &str) {
        self.record_agent(session);
    }

    fn shell_spawned(&self, tab: &str) {
        self.record_shell(tab);
    }
}

/// An agent session as one look found it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Waiting {
    session: String,
    /// The session's title, when the store has it.
    title: String,
    /// The agent's holder process while it runs; none while it launches or
    /// resumes.
    holder_pid: Option<u32>,
    /// Its holder can be moved onto the current build in place; only a running
    /// agent's can.
    moves: bool,
    /// The agent's holder socket, to move it.
    socket: PathBuf,
}

/// A terminal whose holder can be moved onto the current build in place.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Mover {
    /// Its record under `holder_builds`.
    name: String,
    /// Its holder's socket.
    socket: PathBuf,
    /// The session, for an agent or a Terminal session's shell; none for a
    /// shell tab.
    session: Option<String>,
    title: String,
}

impl Mover {
    fn of_agent(waiting: &Waiting) -> Self {
        Self {
            name: format!("{}.sha", waiting.session),
            socket: waiting.socket.clone(),
            session: Some(waiting.session.clone()),
            title: waiting.title.clone(),
        }
    }

    /// Which terminal this is, for what is said when it can't move.
    fn what(&self) -> String {
        match &self.session {
            Some(session) => format!("session {session}'s terminal"),
            None => format!("the shell tab {}", self.name.trim_end_matches(".shell.sha")),
        }
    }
}

/// What one look found on old holders.
#[derive(Debug, Default, PartialEq, Eq)]
struct Found {
    /// The agent sessions, by session id.
    waiting: Vec<Waiting>,
    /// The shells (shell tabs and Terminal sessions) whose holder is too old
    /// to move: they stay until closed.
    shells: usize,
    /// The shells whose holder can move.
    movable_shells: Vec<Mover>,
}

impl Found {
    /// Everything a move round would move, in order: agents first, then
    /// shells.
    fn movers(&self) -> Vec<Mover> {
        self.waiting
            .iter()
            .filter(|waiting| waiting.moves)
            .map(Mover::of_agent)
            .chain(self.movable_shells.iter().cloned())
            .collect()
    }
}

/// One agent record and what is known about it, for [`classify`].
struct Seen {
    record: AgentRecord,
    /// A Terminal session's plain shell, which has no conversation to resume.
    is_shell: bool,
    recorded: Option<Recorded>,
}

/// One shell tab and what is known about it, for [`classify`].
struct SeenTab {
    tab: TabRecord,
    recorded: Option<Recorded>,
}

/// Which of `agents` and `shell_tabs` are on a holder other than the one with
/// sha256 `wanted`, and which of those can move in place. An agent that isn't
/// meant to run is never waiting. A holder moves only while it runs.
fn classify(wanted: &str, agents: Vec<Seen>, shell_tabs: Vec<SeenTab>) -> Found {
    let old = |recorded: &Option<Recorded>| {
        !recorded
            .as_ref()
            .is_some_and(|recorded| recorded.sha.eq_ignore_ascii_case(wanted))
    };
    let can = |recorded: &Option<Recorded>| recorded.as_ref().is_some_and(|recorded| recorded.moves);
    let mut found = Found::default();
    for seen in shell_tabs {
        if !old(&seen.recorded) {
            continue;
        }
        if seen.tab.holder_pid != 0 && can(&seen.recorded) {
            found.movable_shells.push(Mover {
                name: format!("{}.shell.sha", seen.tab.id),
                socket: seen.tab.holder_socket,
                session: None,
                title: String::new(),
            });
        } else {
            found.shells += 1;
        }
    }
    for seen in agents {
        if !seen.record.state.should_run() || !old(&seen.recorded) {
            continue;
        }
        let running = matches!(seen.record.state, AgentState::Running { .. });
        let moves = running && can(&seen.recorded);
        if seen.is_shell {
            if moves {
                found.movable_shells.push(Mover {
                    name: format!("{}.sha", seen.record.session),
                    socket: seen.record.launch.holder_socket,
                    session: Some(seen.record.session),
                    title: String::new(),
                });
            } else {
                found.shells += 1;
            }
        } else {
            found.waiting.push(Waiting {
                session: seen.record.session,
                title: String::new(),
                holder_pid: match seen.record.state {
                    AgentState::Running { holder_pid } => Some(holder_pid),
                    _ => None,
                },
                moves,
                socket: seen.record.launch.holder_socket,
            });
        }
    }
    found.waiting.sort_by(|a, b| a.session.cmp(&b.session));
    found.movable_shells.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// The line that says the shells on holders too old to move keep running.
pub(crate) fn shell_line(shells: usize) -> Option<String> {
    match shells {
        0 => None,
        1 => Some("1 shell on a holder too old to move stays on it until it is closed".to_owned()),
        many => Some(format!(
            "{many} shells on holders too old to move stay on them until they are closed"
        )),
    }
}

/// Why an agent couldn't be restarted onto the current holder.
#[derive(Debug, thiserror::Error)]
enum HoldersError {
    #[error(
        "couldn't {doing} for session {session} to move its agent onto the current holder. Click Migrate again; if it keeps failing, restart humatad2"
    )]
    Store {
        session: String,
        doing: &'static str,
        #[source]
        source: StoreError,
    },

    #[error("restarting session {session}'s agent was cut short. Click Migrate again; if it keeps failing, restart humatad2")]
    Task {
        session: String,
        #[source]
        source: tokio::task::JoinError,
    },

    #[error(
        "couldn't start session {session}'s agent again on the current holder. Its terminal is where it printed why: open it, fix the cause, then click Migrate again"
    )]
    Restart {
        session: String,
        #[source]
        source: ReviveError,
    },

    #[error(
        "couldn't move {what} onto the current holder in place. If the holder refused, it keeps running on its old holder and nothing changed; if it was lost in the move, open the terminal to see what is left of it. Click Migrate again"
    )]
    Move {
        what: String,
        #[source]
        source: humata2_holder::HolderError,
    },

    #[error(
        "session {session}'s agent didn't take its conversation up on the current holder ({reason:?}, exit code {exit_code:?}); nothing runs in its place. Open the session's terminal to read what it printed, fix the cause, resume the session, then click Migrate for the others"
    )]
    ResumeFailed {
        session: String,
        reason: ResumeFailure,
        exit_code: Option<i32>,
    },
}

/// What a machine's daemon says of its holders: the whole list each time, and
/// none until its first look.
pub type Reports = watch::Sender<Option<pb::HoldersReport>>;

struct Inner {
    machine: pb::MachineId,
    release: Release,
    builds: Arc<Builds>,
    migrations: Migrations,
    starter: Starter,
    tokens: Arc<dyn HolderTokensPort>,
    sessions: Arc<dyn HolderSessionPort>,
    events: Arc<dyn HolderEventsPort>,
    turns: Arc<dyn HolderTurnsPort>,
    /// Sessions being restarted now, so one restart is never begun twice.
    restarting: Mutex<HashSet<String>>,
    /// The records of the holders being moved now, so their rows show Moving.
    moving: Mutex<HashSet<String>>,
    /// Set while a round of moves runs, so only one runs at a time.
    moving_round: AtomicBool,
    /// How far the current or last round of moves got (PROGRESS T6), shown
    /// as "3 of 7 terminals moved". None until the first round.
    round: Mutex<Option<pb::HoldersMoved>>,
    /// Woken by a click on Migrate and by the bin folder changing.
    wake: Arc<Notify>,
    /// Held across one look and what it does about it.
    one_look: tokio::sync::Mutex<()>,
    /// Set on a machine's daemon: where it reports its holders to the host.
    /// None on the host, which posts the item itself.
    report: Option<Arc<Reports>>,
    /// Set on a machine's daemon while the host's Migrate is carried out.
    migrating: AtomicBool,
    /// Set on a machine's daemon when a restart failed; reported to the host
    /// so its item shows Failed. Cleared when the next round begins.
    failure: Mutex<Option<pb::Error>>,
    /// The titles of the sessions last found waiting, so a row keeps its
    /// title while its agent restarts.
    titles: Mutex<HashMap<String, String>>,
}

/// The Holders item of this machine and the restarts it asks for.
#[derive(Clone)]
pub struct Holders {
    inner: Arc<Inner>,
}

/// What [`Holders`] is made of.
pub struct Deps {
    /// This machine's id.
    pub machine: String,
    /// The release this daemon runs.
    pub release: Release,
    /// What records the holders this daemon spawns.
    pub builds: Arc<Builds>,
    pub migrations: Migrations,
    pub starter: Starter,
    pub tokens: Arc<dyn HolderTokensPort>,
    pub sessions: Arc<dyn HolderSessionPort>,
    pub events: Arc<dyn HolderEventsPort>,
    pub turns: Arc<dyn HolderTurnsPort>,
    /// Some on a machine's daemon (D378): the list is reported, not posted.
    pub report: Option<Arc<Reports>>,
}

impl Holders {
    pub fn new(deps: Deps) -> Self {
        Self {
            inner: Arc::new(Inner {
                machine: pb::MachineId { value: deps.machine },
                release: deps.release,
                builds: deps.builds,
                migrations: deps.migrations,
                starter: deps.starter,
                tokens: deps.tokens,
                sessions: deps.sessions,
                events: deps.events,
                turns: deps.turns,
                restarting: Mutex::new(HashSet::new()),
                moving: Mutex::new(HashSet::new()),
                moving_round: AtomicBool::new(false),
                round: Mutex::new(None),
                wake: Arc::new(Notify::new()),
                one_look: tokio::sync::Mutex::new(()),
                report: deps.report,
                migrating: AtomicBool::new(false),
                failure: Mutex::new(None),
                titles: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// What a change of the bin folder wakes.
    pub fn wake(&self) -> Arc<Notify> {
        self.inner.wake.clone()
    }

    /// Migrate was clicked: each holder that can move is moved in place, and
    /// each agent whose holder can't is restarted at its next idle moment,
    /// beginning with the ones idle now.
    pub(crate) fn begin(&self) {
        self.inner.migrating.store(true, Ordering::SeqCst);
        // A new round: what failed last time is reported again only if it fails again.
        *self.inner.failure.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.inner.wake.notify_one();
    }

    /// Looks once `started` says the daemon's start has brought back what it
    /// could (so a session about to be resumed isn't shown as waiting), then
    /// again at every change that can alter what waits. Returns at once.
    pub fn start(&self, started: Arc<dyn HolderStartedPort>) {
        if self.inner.release.bundle().is_empty() {
            return;
        }
        let me = self.clone();
        // Subscribed now, so a change during the start is looked at too.
        let mut events = self.inner.events.subscribe();
        let mut statuses = self.inner.turns.changes();
        let spawned = self.inner.builds.wake();
        let woken = self.inner.wake.clone();
        tokio::spawn(async move {
            started.wait().await;
            loop {
                me.look().await;
                // Waits for the next change, then takes in everything that
                // arrived with it, so a burst is one look.
                let again = loop {
                    tokio::select! {
                        () = spawned.notified() => break true,
                        () = woken.notified() => break true,
                        changed = statuses.changed() => break changed.is_ok(),
                        event = events.recv() => match event {
                            Ok(event) if !matches!(&event.payload, Some(pb::event::Payload::Session(_))) => {}
                            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => break true,
                            Err(broadcast::error::RecvError::Closed) => break false,
                        },
                    }
                };
                if !again {
                    return;
                }
                while !matches!(
                    events.try_recv(),
                    Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed)
                ) {}
            }
        });
    }

    /// One look, and what it asks for.
    async fn look(&self) {
        let _one = self.inner.one_look.lock().await;
        let me = self.clone();
        let found = match tokio::task::spawn_blocking(move || me.found()).await {
            Ok(found) => found,
            Err(error) => {
                humata2_rpc_kit::log::say!(
                    "humatad2: warning: looking at which holders are old failed ({error}), so the Holders item isn't updated until the next change"
                );
                return;
            }
        };
        let Some(mut found) = found else {
            return;
        };
        {
            let mut titles = self.inner.titles.lock().unwrap_or_else(PoisonError::into_inner);
            for waiting in found.waiting.iter().filter(|waiting| !waiting.title.is_empty()) {
                titles.insert(waiting.session.clone(), waiting.title.clone());
            }
        }
        // An agent being restarted waits until its restart is over, though
        // its holder was recorded when the new one began to spawn.
        for session in self
            .inner
            .restarting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            if !found.waiting.iter().any(|waiting| waiting.session == *session) {
                let title = self
                    .inner
                    .titles
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(session)
                    .cloned()
                    .unwrap_or_default();
                found.waiting.push(Waiting {
                    session: session.clone(),
                    title,
                    holder_pid: None,
                    moves: false,
                    socket: PathBuf::new(),
                });
            }
        }
        found.waiting.sort_by(|a, b| a.session.cmp(&b.session));
        let rows = self.rows(&found);
        if let Some(report) = &self.inner.report {
            self.report(report, found, rows);
            return;
        }
        let (migrations, machine) = (&self.inner.migrations, &self.inner.machine);
        if !outstanding(&rows) {
            if let Err(error) = migrations.resolved(pb::MigrateThing::Holders, machine).await {
                humata2_rpc_kit::log::say!("humatad2: warning: {}", status::chain(&error));
            }
            return;
        }
        let detail = Detail {
            waiting: waiting_sessions(&rows),
            keeps_running: shell_line(found.shells).into_iter().collect(),
            rows,
            failed: None,
            moved: self.round(),
        };
        let build = tools::release_only(&self.inner.release.info());
        if let Err(error) = migrations.observe_holders(machine, build.clone(), detail).await {
            humata2_rpc_kit::log::say!("humatad2: warning: {}", status::chain(&error));
            return;
        }
        if !migrations.is_migrating(pb::MigrateThing::Holders, machine) {
            migrations
                .auto_migrate(pb::MigrateThing::Holders, machine, &build.bundle)
                .await;
        }
        if migrations.is_migrating(pb::MigrateThing::Holders, machine) {
            self.carry_on(found);
        }
    }

    /// Migrate is under way: starts a round of moves, and restarts the
    /// agents whose holders can't move and are idle now.
    fn carry_on(&self, found: Found) {
        self.move_in_background(found.movers());
        for waiting in found
            .waiting
            .into_iter()
            .filter(|waiting| !waiting.moves && self.idle(waiting))
        {
            self.restart_in_background(waiting.session);
        }
    }

    /// The rows of the card (D379): a row for each agent waiting, with what
    /// Migrate does with it, then one for each shell that moves, then one for
    /// each shell that stays until it is closed.
    fn rows(&self, found: &Found) -> Vec<pb::HolderSession> {
        let restarting = self.inner.restarting.lock().unwrap_or_else(PoisonError::into_inner);
        let moving = self.inner.moving.lock().unwrap_or_else(PoisonError::into_inner);
        let agents = found.waiting.iter().map(|waiting| {
            let note = if moving.contains(&format!("{}.sha", waiting.session)) {
                pb::HolderNote::Moving
            } else if waiting.moves {
                pb::HolderNote::Moves
            } else if restarting.contains(&waiting.session) {
                pb::HolderNote::Restarting
            } else if waiting.holder_pid.is_some_and(|pid| {
                self.inner.turns.running(&waiting.session, pid)
                    == pb::SessionStatus::RunningUnknownTurn
            }) {
                pb::HolderNote::NoIdleMoment
            } else {
                pb::HolderNote::RestartsWhenIdle
            };
            pb::HolderSession {
                session: Some(pb::SessionId {
                    value: waiting.session.clone(),
                }),
                title: waiting.title.clone(),
                note: note as i32,
            }
        });
        let movable = found.movable_shells.iter().map(|mover| pb::HolderSession {
            session: mover.session.as_ref().map(|session| pb::SessionId { value: session.clone() }),
            title: mover.title.clone(),
            note: if moving.contains(&mover.name) {
                pb::HolderNote::Moving
            } else {
                pb::HolderNote::Moves
            } as i32,
        });
        let stays = (0..found.shells).map(|_| pb::HolderSession {
            session: None,
            title: String::new(),
            note: pb::HolderNote::StaysUntilClosed as i32,
        });
        agents.chain(movable).chain(stays).collect()
    }

    /// A machine's look (D378): the list goes to the host when it changed,
    /// and what the host asked for goes on.
    fn report(&self, report: &Reports, found: Found, rows: Vec<pb::HolderSession>) {
        // With no old holder left there is nothing to retry, so no failure.
        let failed = if !outstanding(&rows) {
            None
        } else {
            self.inner.failure.lock().unwrap_or_else(PoisonError::into_inner).clone()
        };
        let next = pb::HoldersReport {
            holders: rows,
            failed,
            moved: self.round(),
        };
        report.send_if_modified(|current| {
            if current.as_ref() == Some(&next) {
                return false;
            }
            *current = Some(next.clone());
            true
        });
        if !self.inner.migrating.load(Ordering::SeqCst) {
            return;
        }
        if !outstanding(&next.holders) {
            self.inner.migrating.store(false, Ordering::SeqCst);
            return;
        }
        self.carry_on(found);
    }

    /// What is on old holders, or none while it can't be said: the release
    /// names no holder, or this daemon doesn't spawn from the one it names.
    fn found(&self) -> Option<Found> {
        let inner = &self.inner;
        let info = inner.release.info();
        let wanted = info
            .files
            .iter()
            .find(|file| file.name == InstalledFile::Holder.file_name())?
            .sha256
            .clone();
        if info.bundle.is_empty() {
            return None;
        }
        match inner.builds.current() {
            Ok(own) if own.eq_ignore_ascii_case(&wanted) => {}
            Ok(_) => return None,
            Err(error) => {
                humata2_rpc_kit::log::say!("humatad2: warning: {}", status::chain(&error));
                return None;
            }
        }
        let store = &inner.starter.store;
        let (agents, tabs) = match (store.agents(), store.tabs()) {
            (Ok(agents), Ok(tabs)) => (agents, tabs),
            (Err(error), _) | (_, Err(error)) => {
                humata2_rpc_kit::log::say!(
                    "humatad2: warning: couldn't read the sessions to look for old holders: {}",
                    status::chain(&error)
                );
                return None;
            }
        };
        let agent_tabs: HashSet<&str> = agents.iter().map(|agent| agent.tab.as_str()).collect();
        let shell_tabs: Vec<SeenTab> = tabs
            .iter()
            .filter(|tab| tab.kind == TabKind::Shell && !agent_tabs.contains(tab.id.as_str()))
            .map(|tab| SeenTab {
                recorded: inner.builds.recorded(&format!("{}.shell.sha", tab.id)),
                tab: tab.clone(),
            })
            .collect();
        let seen = agents
            .into_iter()
            .filter_map(|record| {
                let kind = inner.starter.agents.get(&record.launch.agent).ok()?;
                Some(Seen {
                    is_shell: kind.conversation_file.is_empty(),
                    recorded: inner.builds.recorded(&format!("{}.sha", record.session)),
                    record,
                })
            })
            .collect();
        let mut found = classify(&wanted, seen, shell_tabs);
        for waiting in &mut found.waiting {
            if let Ok(Some(session)) = store.session(&waiting.session) {
                waiting.title = session.title;
            }
        }
        for mover in &mut found.movable_shells {
            if let Some(session) = &mover.session
                && let Ok(Some(session)) = store.session(session)
            {
                mover.title = session.title;
            }
        }
        Some(found)
    }

    /// Starts a round of moves of `movers`, unless one is running. The first
    /// is the canary: the round stops at the first move that fails (D801).
    /// Nothing here ends a holder: a move that isn't made leaves it running.
    fn move_in_background(&self, movers: Vec<Mover>) {
        if movers.is_empty() || self.inner.moving_round.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let outcome = me.move_all(movers).await;
            me.inner.moving_round.store(false, Ordering::SeqCst);
            if let Err(error) = outcome {
                humata2_rpc_kit::log::say!("humatad2: warning: {}", status::chain(&error));
                me.fail(holders_problem("", &error)).await;
            }
            // The item may have more to do, or be done.
            me.inner.wake.notify_one();
        });
    }

    /// How far the current or last round of moves got.
    fn round(&self) -> Option<pb::HoldersMoved> {
        *self.inner.round.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Moves each of `movers` onto the current holder file, one at a time,
    /// counting each one handled for the item (PROGRESS T6).
    async fn move_all(&self, movers: Vec<Mover>) -> Result<(), HoldersError> {
        let shown = |count: pb::HoldersMoved| {
            *self.inner.round.lock().unwrap_or_else(PoisonError::into_inner) = Some(count);
            // Looks again, so the card shows the count.
            self.inner.wake.notify_one();
        };
        counted_round(movers, now_ms(), shown, |mover| self.move_one(mover)).await
    }

    /// Moves `mover` onto the current holder file. Ok when it is handled: it
    /// moved, or it stays running where it is because its holder is too old
    /// to move or can't be reached. An error stops the round.
    async fn move_one(&self, mover: Mover) -> Result<(), HoldersError> {
        let builds = &self.inner.builds;
        // A look made before an earlier round ended may list one that has
        // moved since.
        if builds.on_current(&mover.name) {
            return Ok(());
        }
        self.inner
            .moving
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(mover.name.clone());
        // Looks again, so the card shows it moving.
        self.inner.wake.notify_one();
        let outcome = humata2_holder::move_to(&mover.socket, builds.holder_file()).await;
        self.inner
            .moving
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&mover.name);
        match after_move(&mover, outcome)? {
            AfterMove::Moved => {
                builds.keep_logged(&mover.name);
                if let Some(session) = &mover.session {
                    // The old image's connection closed with its move, which
                    // ended the watch on this agent.
                    self.inner.sessions.watch_agent(session.clone());
                    self.inner
                        .sessions
                        .publish_session(session, pb::Change::Updated, None)
                        .await;
                }
            }
            AfterMove::TooOld => {
                if let Err(error) = builds.too_old(&mover.name) {
                    humata2_rpc_kit::log::say!("humatad2: warning: {}", status::chain(&error));
                }
            }
            AfterMove::Unreached(error) => {
                humata2_rpc_kit::log::say!(
                    "humatad2: warning: couldn't reach {}'s holder to move it, so it is left as it is: {}",
                    mover.what(),
                    status::chain(&error)
                );
            }
        }
        Ok(())
    }

    /// The round stopped on `problem`: this machine's item shows it Failed
    /// (the host's, through the next report, when this is a machine's daemon).
    async fn fail(&self, problem: pb::Error) {
        if self.inner.report.is_some() {
            // The host's item shows it Failed through the next report.
            self.inner.migrating.store(false, Ordering::SeqCst);
            *self.inner.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(problem);
        } else {
            self.inner.migrations.fail_holders(&self.inner.machine, problem).await;
        }
    }

    fn restart_in_background(&self, session: String) {
        let inserted = self
            .inner
            .restarting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session.clone());
        if !inserted {
            return;
        }
        // Looks again, so the card shows it restarting.
        self.inner.wake.notify_one();
        let me = self.clone();
        tokio::spawn(async move {
            let outcome = me.restart(&session).await;
            me.inner
                .restarting
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&session);
            if let Err(error) = outcome {
                humata2_rpc_kit::log::say!("humatad2: warning: {}", status::chain(&error));
                me.fail(holders_problem(&session, &error)).await;
            }
            // The item may have more to restart, or be done.
            me.inner.wake.notify_one();
        });
    }

    /// Starts `session`'s agent again on the current holder if it is
    /// running and idle: the same session and conversation, saved as
    /// resuming first so revive stops its holder and resumes it.
    async fn restart(&self, session: &str) -> Result<(), HoldersError> {
        let starter = &self.inner.starter;
        let store_error = |doing: &'static str| {
            let session = session.to_owned();
            move |source| HoldersError::Store { session, doing, source }
        };
        let read = {
            let (store, id) = (starter.store.clone(), session.to_owned());
            blocking(session, move || store.agent(&id)).await?
        };
        let Some(mut agent) = read.map_err(store_error("read the agent"))? else {
            return Ok(());
        };
        let AgentState::Running { holder_pid } = agent.state else {
            return Ok(());
        };
        // It may have gone back to work since the look.
        if !self.turns_idle(session, holder_pid) {
            return Ok(());
        }
        agent.state = AgentState::Resuming;
        agent.updated_unix_ms = now_ms();
        let saved = {
            let (store, saving) = (starter.store.clone(), agent.clone());
            blocking(session, move || store.put_agent(&saving)).await?
        };
        saved.map_err(store_error("save that the agent is restarting"))?;
        self.inner
            .sessions
            .publish_session(session, pb::Change::Updated, None)
            .await;
        let env = |tab: &TabRecord| self.inner.tokens.env(tab);
        let outcome = humata2_revive::restart_agent(&starter.store, &starter.revive, agent, &env, RESTARTED).await;
        let result = match outcome {
            Ok(Revived::Resumed { .. } | Revived::Reattached | Revived::Relaunched) => {
                self.inner.sessions.watch_agent(session.to_owned());
                Ok(())
            }
            Ok(Revived::ResumeFailed { reason, exit_code }) => Err(HoldersError::ResumeFailed {
                session: session.to_owned(),
                reason,
                exit_code,
            }),
            Ok(Revived::Exited { .. } | Revived::Left) => Ok(()),
            Err(source) => Err(HoldersError::Restart {
                session: session.to_owned(),
                source,
            }),
        };
        self.inner
            .sessions
            .publish_session(session, pb::Change::Updated, None)
            .await;
        result
    }

    /// Whether the agent is at an idle moment: its status is Idle or
    /// ProbablyIdle. One that is working, needs input, or has no turns to tell
    /// (and so no idle moment) waits.
    fn idle(&self, waiting: &Waiting) -> bool {
        waiting
            .holder_pid
            .is_some_and(|holder_pid| self.turns_idle(&waiting.session, holder_pid))
    }

    fn turns_idle(&self, session: &str, holder_pid: u32) -> bool {
        matches!(
            self.inner.turns.running(session, holder_pid),
            pb::SessionStatus::Idle | pb::SessionStatus::ProbablyIdle
        )
    }
}

/// What one move left, when it doesn't stop the round.
#[derive(Debug)]
enum AfterMove {
    /// The holder runs the current build now.
    Moved,
    /// Its holder is too old to move: it keeps running, and an agent is
    /// restarted when idle instead.
    TooOld,
    /// Its holder couldn't be reached to ask; it is left as it is.
    Unreached(humata2_holder::HolderError),
}

/// What `outcome`, the move of `mover`, means for the round: a refused or
/// lost move stops it (D801: the first failure stops the round); anything
/// else counts the terminal as handled, so the count never sticks on it.
fn after_move(
    mover: &Mover,
    outcome: Result<humata2_holder::Moved, humata2_holder::HolderError>,
) -> Result<AfterMove, HoldersError> {
    match outcome {
        Ok(humata2_holder::Moved) => Ok(AfterMove::Moved),
        Err(humata2_holder::HolderError::TooOldToMove { .. }) => Ok(AfterMove::TooOld),
        Err(
            source @ (humata2_holder::HolderError::Refused { .. }
            | humata2_holder::HolderError::LostInMove { .. }),
        ) => Err(HoldersError::Move {
            what: mover.what(),
            source,
        }),
        Err(error) => Ok(AfterMove::Unreached(error)),
    }
}

/// One round of moves: `move_one` for each of `movers` in order, `shown`
/// told the count before the first and after each one handled. The first
/// error stops the round, and the count stays where it got to.
async fn counted_round<F, Fut>(
    movers: Vec<Mover>,
    started_unix_ms: i64,
    shown: impl Fn(pb::HoldersMoved),
    mut move_one: F,
) -> Result<(), HoldersError>
where
    F: FnMut(Mover) -> Fut,
    Fut: std::future::Future<Output = Result<(), HoldersError>>,
{
    let mut count = pb::HoldersMoved {
        done: 0,
        total: u32::try_from(movers.len()).unwrap_or(u32::MAX),
        started_unix_ms,
    };
    shown(count);
    for mover in movers {
        move_one(mover).await?;
        count.done = count.done.saturating_add(1);
        shown(count);
    }
    Ok(())
}

/// What the Holders item shows when a terminal couldn't be moved or
/// `session`'s agent couldn't be restarted: the cause, and the way out.
fn holders_problem(session: &str, error: &HoldersError) -> pb::Error {
    let fix = match error {
        HoldersError::Move {
            source: humata2_holder::HolderError::LostInMove { .. },
            ..
        } => "open the terminal's tab to see what is left of it, then click Migrate again for the others".to_owned(),
        HoldersError::Move { .. } => {
            "click Migrate again; the terminal keeps running on its old holder until a move works".to_owned()
        }
        _ => format!(
            "open session {session}'s terminal to read what it printed and fix the cause, then click Migrate again; or close session {session} to free its old holder"
        ),
    };
    pb::Error {
        message: status::chain(error),
        fix,
        retry: None,
        kind: None,
    }
}

async fn blocking<T: Send + 'static>(
    session: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, HoldersError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|source| HoldersError::Task {
            session: session.to_owned(),
            source,
        })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

/// Carries this machine's Migrate items: the Holders one is the holders'
/// own, every other this daemon's `Runner`.
pub struct Split {
    machine: String,
    daemon: Arc<dyn Carry>,
    holders: Arc<OnceLock<Holders>>,
}

impl Split {
    /// `daemon` carries what isn't the holders of `machine`, which `holders`
    /// carries once it is set.
    pub fn new(machine: &str, daemon: Arc<dyn Carry>, holders: Arc<OnceLock<Holders>>) -> Self {
        Self {
            machine: machine.to_owned(),
            daemon,
            holders,
        }
    }
}

#[tonic::async_trait]
impl Carry for Split {
    async fn migrate(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
        to: &pb::BuildInfo,
        progress: &Progress,
    ) -> Result<(), RunError> {
        if thing != pb::MigrateThing::Holders {
            return self.daemon.migrate(thing, machine, to, progress).await;
        }
        match self.holders.get() {
            Some(holders) => {
                holders.begin();
                Ok(())
            }
            None => Err(RunError::NotHere {
                machine: self.machine.clone(),
            }),
        }
    }

    fn deadline(&self, machine: &pb::MachineId) -> Option<std::time::Duration> {
        self.daemon.deadline(machine)
    }

    fn steps(&self, thing: pb::MigrateThing, machine: &pb::MachineId) -> &'static [&'static str] {
        self.daemon.steps(thing, machine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW: &str = "bb";

    fn agent(session: &str, state: AgentState) -> AgentRecord {
        AgentRecord {
            session: session.to_owned(),
            tab: format!("tb_{session}"),
            launch: humata2_store::AgentLaunch {
                agent: "claude".to_owned(),
                folder: PathBuf::new(),
                argv: Vec::new(),
                resume_argv: Vec::new(),
                variable_names: Vec::new(),
                conversation_id: String::new(),
                conversation_file: PathBuf::new(),
                holder_socket: PathBuf::new(),
                cols: 80,
                rows: 24,
                cli_version: None,
            },
            state,
            generation: 1,
            interruptions: Vec::new(),
            updated_unix_ms: 0,
        }
    }

    fn sha(sha: &str, moves: bool) -> Option<Recorded> {
        Some(Recorded {
            sha: sha.to_owned(),
            moves,
        })
    }

    fn seen(session: &str, state: AgentState, is_shell: bool, recorded: Option<Recorded>) -> Seen {
        let mut record = agent(session, state);
        record.launch.holder_socket = PathBuf::from(format!("/sock/{session}"));
        Seen {
            record,
            is_shell,
            recorded,
        }
    }

    fn shell_tab(id: &str, holder_pid: u32, recorded: Option<Recorded>) -> SeenTab {
        SeenTab {
            tab: TabRecord {
                id: id.to_owned(),
                session: None,
                kind: TabKind::Shell,
                title: String::new(),
                holder_socket: PathBuf::from(format!("/sock/{id}")),
                holder_pid,
                created_unix_ms: 0,
                holder_generation: 0,
            },
            recorded,
        }
    }

    fn running(pid: u32) -> AgentState {
        AgentState::Running { holder_pid: pid }
    }

    #[test]
    fn an_agent_whose_record_differs_or_is_missing_waits_and_one_on_the_current_holder_does_not() {
        let found = classify(
            NEW,
            vec![
                seen("b", running(2), false, sha("aa", false)),
                seen("a", running(1), false, None),
                seen("c", running(3), false, sha("BB", false)),
            ],
            Vec::new(),
        );

        let sessions: Vec<&str> = found.waiting.iter().map(|waiting| waiting.session.as_str()).collect();
        assert_eq!(sessions, ["a", "b"], "sorted, and the one on the current holder is left out");
        assert_eq!(found.waiting[0].holder_pid, Some(1));
    }

    #[test]
    fn an_agent_that_is_not_meant_to_run_never_waits() {
        let found = classify(
            NEW,
            vec![
                seen("stopped", AgentState::Stopped, false, None),
                seen("exited", AgentState::Exited { code: 0 }, false, None),
                seen("resuming", AgentState::Resuming, false, None),
            ],
            Vec::new(),
        );

        let sessions: Vec<&str> = found.waiting.iter().map(|waiting| waiting.session.as_str()).collect();
        assert_eq!(sessions, ["resuming"]);
        assert_eq!(found.waiting[0].holder_pid, None, "an agent that isn't running has no holder to be idle in");
    }

    #[test]
    fn a_running_agent_whose_record_says_moves_moves_and_one_without_it_is_too_old() {
        let found = classify(
            NEW,
            vec![
                seen("can", running(1), false, sha("aa", true)),
                seen("old", running(2), false, sha("aa", false)),
                seen("unrecorded", running(3), false, None),
                seen("launching", AgentState::Resuming, false, sha("aa", true)),
            ],
            Vec::new(),
        );

        let moves: Vec<(&str, bool)> = found
            .waiting
            .iter()
            .map(|waiting| (waiting.session.as_str(), waiting.moves))
            .collect();
        assert_eq!(
            moves,
            [("can", true), ("launching", false), ("old", false), ("unrecorded", false)]
        );
        let movers = found.movers();
        assert_eq!(movers.len(), 1);
        assert_eq!(movers[0].socket, PathBuf::from("/sock/can"));
        assert_eq!(movers[0].session.as_deref(), Some("can"));
    }

    #[test]
    fn shells_that_can_move_are_movers_and_the_rest_are_counted_and_stay() {
        let found = classify(
            NEW,
            vec![
                seen("terminal", running(4), true, sha("aa", true)),
                seen("old-terminal", running(5), true, sha("aa", false)),
                seen("current-terminal", running(6), true, sha(NEW, true)),
            ],
            vec![
                shell_tab("tb_old", 7, None),
                shell_tab("tb_move", 8, sha("aa", true)),
                shell_tab("tb_current", 9, sha(NEW, true)),
                shell_tab("tb_gone", 0, sha("aa", true)),
            ],
        );

        assert!(found.waiting.is_empty());
        assert_eq!(found.shells, 3, "a tab and a Terminal session on too old holders, and a tab with no holder");
        let movers: Vec<(&str, Option<&str>)> = found
            .movable_shells
            .iter()
            .map(|mover| (mover.name.as_str(), mover.session.as_deref()))
            .collect();
        assert_eq!(movers, [("tb_move.shell.sha", None), ("terminal.sha", Some("terminal"))]);
        assert_eq!(found.movable_shells[0].socket, PathBuf::from("/sock/tb_move"));
    }

    #[test]
    fn a_terminal_session_that_is_not_running_yet_stays_until_it_runs() {
        let found = classify(
            NEW,
            vec![seen("terminal", AgentState::Resuming, true, sha("aa", true))],
            Vec::new(),
        );

        assert!(found.movable_shells.is_empty());
        assert_eq!(found.shells, 1);
    }

    #[test]
    fn a_shell_on_a_holder_too_old_to_move_is_counted_and_gives_nothing_to_move() {
        let found = classify(NEW, Vec::new(), vec![shell_tab("tb_old", 7, None)]);
        assert!(found.movers().is_empty());
        assert_eq!(found.shells, 1);
    }

    #[test]
    fn the_shell_line_is_said_only_for_shells() {
        assert_eq!(shell_line(0), None);
        assert!(shell_line(1).is_some_and(|line| line.starts_with("1 shell ")));
        assert!(shell_line(3).is_some_and(|line| line.starts_with("3 shells ")));
    }

    fn write_holder(path: &Path, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, script).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn a_record_is_kept_whole_and_read_back_and_a_changed_holder_file_is_hashed_again() {
        let dir = tempfile::tempdir().unwrap();
        let holder = dir.path().join("humata2_holder");
        fs::write(&holder, b"release a").unwrap();
        let builds = Builds::new(dir.path(), &holder);

        builds.record_agent("ss_1");
        let first = builds.recorded("ss_1.sha").unwrap();
        fs::write(&holder, b"release b is longer").unwrap();
        builds.record_shell("tb_1");

        assert_eq!(builds.recorded("tb_1.shell.sha").unwrap().sha, builds.current().unwrap());
        assert_ne!(builds.recorded("tb_1.shell.sha").unwrap().sha, first.sha, "the new file was hashed");
        assert_eq!(builds.recorded("ss_1.sha").unwrap(), first, "a shell's record leaves an agent's alone");
        let left: Vec<_> = fs::read_dir(dir.path().join("holder_builds")).unwrap().collect();
        assert_eq!(left.len(), 2, "no temporary file is left behind");
    }

    #[test]
    fn a_holder_file_that_cannot_be_read_keeps_no_record_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let builds = Builds::new(dir.path(), &dir.path().join("missing"));

        assert!(matches!(builds.keep("ss_1.sha"), Err(BuildsError::Read { .. })));
        assert_eq!(builds.recorded("ss_1.sha"), None);
    }

    #[test]
    fn a_holder_file_that_names_its_handoff_versions_is_recorded_as_able_to_move() {
        let dir = tempfile::tempdir().unwrap();
        let holder = dir.path().join("humata2_holder");
        write_holder(&holder, "#!/bin/sh\necho 1 2\n");
        let builds = Builds::new(dir.path(), &holder);

        builds.record_agent("ss_1");

        assert!(builds.recorded("ss_1.sha").unwrap().moves);
    }

    #[test]
    fn a_holder_file_that_cannot_say_its_handoff_versions_is_recorded_as_unable_to_move() {
        let dir = tempfile::tempdir().unwrap();
        let fails = dir.path().join("fails");
        let silent = dir.path().join("silent");
        let words = dir.path().join("words");
        write_holder(&fails, "#!/bin/sh\necho 1\nexit 3\n");
        write_holder(&silent, "#!/bin/sh\nexit 0\n");
        write_holder(&words, "#!/bin/sh\necho none\n");

        for holder in [fails, silent, words, dir.path().join("missing")] {
            assert!(!can_move(&holder), "{} must not count as able to move", holder.display());
        }
    }

    #[test]
    fn a_record_with_no_moves_line_is_too_old_to_move_and_too_old_keeps_the_sha() {
        let dir = tempfile::tempdir().unwrap();
        let holder = dir.path().join("humata2_holder");
        write_holder(&holder, "#!/bin/sh\necho 1\n");
        let builds = Builds::new(dir.path(), &holder);
        fs::create_dir_all(dir.path().join("holder_builds")).unwrap();
        fs::write(dir.path().join("holder_builds/old.sha"), "abc\n").unwrap();

        assert_eq!(builds.recorded("old.sha"), sha("abc", false));

        builds.record_agent("ss_1");
        let before = builds.recorded("ss_1.sha").unwrap();
        assert!(before.moves);
        builds.too_old("ss_1.sha").unwrap();

        assert_eq!(builds.recorded("ss_1.sha"), Some(Recorded { sha: before.sha, moves: false }));
        builds.too_old("nothing.sha").unwrap();
        assert_eq!(builds.recorded("nothing.sha"), None, "no record is made up");
    }

    fn movers(count: usize) -> Vec<Mover> {
        (0..count)
            .map(|index| Mover {
                name: format!("ss_{index}.sha"),
                socket: PathBuf::from(format!("/nowhere/ss_{index}.sock")),
                session: Some(format!("ss_{index}")),
                title: String::new(),
            })
            .collect()
    }

    fn counts(shown: &Mutex<Vec<pb::HoldersMoved>>) -> Vec<(u32, u32)> {
        shown.lock().unwrap().iter().map(|count| (count.done, count.total)).collect()
    }

    #[tokio::test]
    async fn a_round_of_n_moves_reports_each_terminal_in_order_and_ends_at_n_of_n() {
        let shown = Mutex::new(Vec::new());
        let moved = Mutex::new(Vec::new());

        let outcome = counted_round(
            movers(3),
            1_700,
            |count| shown.lock().unwrap().push(count),
            |mover| {
                moved.lock().unwrap().push(mover.name);
                async { Ok(()) }
            },
        )
        .await;

        assert!(outcome.is_ok());
        assert_eq!(counts(&shown), vec![(0, 3), (1, 3), (2, 3), (3, 3)]);
        assert!(shown.lock().unwrap().iter().all(|count| count.started_unix_ms == 1_700));
        assert_eq!(*moved.lock().unwrap(), vec!["ss_0.sha", "ss_1.sha", "ss_2.sha"]);
    }

    #[tokio::test]
    async fn a_round_that_stops_keeps_the_count_it_got_to() {
        let shown = Mutex::new(Vec::new());

        let outcome = counted_round(
            movers(3),
            0,
            |count| shown.lock().unwrap().push(count),
            |mover| async move {
                match mover.name.as_str() {
                    "ss_1.sha" => after_move(
                        &mover,
                        Err(humata2_holder::HolderError::Refused {
                            socket: mover.socket.clone(),
                            message: "no".to_owned(),
                        }),
                    )
                    .map(|_| ()),
                    _ => Ok(()),
                }
            },
        )
        .await;

        assert!(matches!(outcome, Err(HoldersError::Move { .. })));
        assert_eq!(counts(&shown), vec![(0, 3), (1, 3)]);
    }

    #[test]
    fn a_holder_that_cant_hand_over_counts_as_handled_and_a_refused_or_lost_move_stops_the_round() {
        let mover = &movers(1)[0];
        let socket = || mover.socket.clone();

        assert!(matches!(after_move(mover, Ok(humata2_holder::Moved)), Ok(AfterMove::Moved)));
        assert!(matches!(
            after_move(mover, Err(humata2_holder::HolderError::TooOldToMove { socket: socket() })),
            Ok(AfterMove::TooOld)
        ));
        assert!(matches!(
            after_move(
                mover,
                Err(humata2_holder::HolderError::Connect {
                    socket: socket(),
                    source: std::io::Error::from(std::io::ErrorKind::NotFound),
                })
            ),
            Ok(AfterMove::Unreached(_))
        ));
        assert!(after_move(mover, Err(humata2_holder::HolderError::LostInMove { socket: socket() })).is_err());
        assert!(
            after_move(
                mover,
                Err(humata2_holder::HolderError::Refused { socket: socket(), message: String::new() })
            )
            .is_err()
        );
    }
}
