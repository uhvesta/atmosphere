//! The holders of this Mac, across a daemon upgrade (U6, D373, PER-5, PER-6).
//!
//! A daemon upgrade never touches a running holder, so a session started
//! under release `a` keeps `a`'s holder when the daemon becomes release `b`.
//! Two parts make that visible and let the user end it:
//!
//! - [`Builds`] keeps, for each holder this daemon spawns, the sha256 of the
//!   `--holder` file it was spawned from, as `<state>/holder_builds/<name>.sha`
//!   (`<session>.sha` for an agent, `<tab>.shell.sha` for a shell tab). It is
//!   written just before the holder starts, through a temporary file renamed
//!   into place, and never when a holder is only reattached.
//! - [`Holders`] compares those records with the `humata2_holder` the
//!   release names. A session whose holder differs, or has no record (it
//!   began before D373), is on an old holder. While this daemon's own
//!   `--holder` file is the release's, one pending Holders item for this
//!   machine lists the agent sessions on old holders; shells are only said
//!   to keep running, and never hold the item open. Migrate restarts each
//!   listed agent onto the current holder at its next idle moment, in the
//!   same session and conversation, and the item ends Approved once none
//!   waits.
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
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use humata2_grpc::humata::v1 as pb;
use humata2_machines::InstalledFile;
use humata2_revive::{HolderBuilds, ReviveError, Revived};
use humata2_sessions::Starter;
use humata2_store::{AgentRecord, AgentState, ResumeFailure, StoreError, TabKind, TabRecord};
use tokio::sync::{Notify, broadcast, watch};

use super::build::Release;
use super::items::{Carry, Detail, Migrations, Progress};
use super::run::{RunError, hash_file};
use super::tools;
use crate::events::Feed;
use crate::identity::Identities;
use crate::restart::Tokens;
use crate::session_service::{Tell, turns, watch as agent_watch};
use crate::status;

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
pub(crate) struct Builds {
    dir: PathBuf,
    holder: PathBuf,
    /// Woken when an agent's record is kept: what waits may have changed.
    wake: Arc<Notify>,
    hashed: Mutex<Option<(Stamp, String)>>,
}

impl Builds {
    /// The records of the daemon whose state folder is `state`, which spawns
    /// holders from `holder`.
    pub(crate) fn new(state: &Path, holder: &Path) -> Self {
        Self {
            dir: state.join("holder_builds"),
            holder: holder.to_owned(),
            wake: Arc::new(Notify::new()),
            hashed: Mutex::new(None),
        }
    }

    /// Woken after each agent holder is about to spawn.
    fn wake(&self) -> Arc<Notify> {
        self.wake.clone()
    }

    /// The sha256 of the `--holder` file now.
    fn current(&self) -> Result<String, BuildsError> {
        let read = |source| BuildsError::Read {
            path: self.holder.clone(),
            source,
        };
        let stamp = Stamp::of(&fs::metadata(&self.holder).map_err(read)?);
        if let Some((seen, sha)) = &*self.hashed.lock().unwrap_or_else(PoisonError::into_inner)
            && *seen == stamp
        {
            return Ok(sha.clone());
        }
        let sha = hash_file(&self.holder).map_err(read)?;
        *self.hashed.lock().unwrap_or_else(PoisonError::into_inner) = Some((stamp, sha.clone()));
        Ok(sha)
    }

    /// The sha256 recorded under `name`, if there is one.
    fn recorded(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.dir.join(name))
            .ok()
            .map(|text| text.trim().to_owned())
            .filter(|sha| !sha.is_empty())
    }

    fn keep(&self, name: &str) -> Result<(), BuildsError> {
        let sha = self.current()?;
        let path = self.dir.join(name);
        let failed = |source| BuildsError::Write {
            path: path.clone(),
            source,
        };
        humata2_durable::create_dir_all(&self.dir, 0o700).map_err(failed)?;
        let temp = self.dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let written = File::create(&temp).and_then(|mut file| {
            file.write_all(sha.as_bytes())?;
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
            crate::log::say!("humatad2: warning: {}", status::chain(&error));
        }
    }

    /// A holder is about to start for `session`'s agent.
    pub(crate) fn record_agent(&self, session: &str) {
        self.keep_logged(&format!("{session}.sha"));
        self.wake.notify_one();
    }

    /// A holder is about to start for the shell tab `tab`.
    pub(crate) fn record_shell(&self, tab: &str) {
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
}

/// What one look found on old holders.
#[derive(Debug, Default, PartialEq, Eq)]
struct Found {
    /// The agent sessions, by session id.
    waiting: Vec<Waiting>,
    /// The shells (shell tabs and Terminal sessions).
    shells: usize,
}

/// One agent record and what is known about it, for [`classify`].
struct Seen {
    record: AgentRecord,
    /// A Terminal session's plain shell, which has no conversation to resume.
    is_shell: bool,
    recorded: Option<String>,
}

/// Which of `agents` and `shell_tabs` (each with its recorded sha256) are on
/// a holder other than the one with sha256 `wanted`. An agent that isn't
/// meant to run, and one that is a plain shell, is never waiting.
fn classify(wanted: &str, agents: Vec<Seen>, shell_tabs: &[Option<String>]) -> Found {
    let old = |recorded: &Option<String>| !recorded.as_deref().is_some_and(|sha| sha.eq_ignore_ascii_case(wanted));
    let mut found = Found {
        shells: shell_tabs.iter().filter(|recorded| old(*recorded)).count(),
        ..Found::default()
    };
    for seen in agents {
        if !seen.record.state.should_run() || !old(&seen.recorded) {
            continue;
        }
        if seen.is_shell {
            found.shells += 1;
        } else {
            found.waiting.push(Waiting {
                session: seen.record.session,
                title: String::new(),
                holder_pid: match seen.record.state {
                    AgentState::Running { holder_pid } => Some(holder_pid),
                    _ => None,
                },
            });
        }
    }
    found.waiting.sort_by(|a, b| a.session.cmp(&b.session));
    found
}

/// The line that says the shells keep running.
pub(super) fn shell_line(shells: usize) -> Option<String> {
    match shells {
        0 => None,
        1 => Some("1 shell stays on its holder until it is closed".to_owned()),
        many => Some(format!("{many} shells stay on their holders until they are closed")),
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
pub(crate) type Reports = watch::Sender<Option<pb::HoldersReport>>;

struct Inner {
    machine: pb::MachineId,
    release: Release,
    builds: Arc<Builds>,
    migrations: Migrations,
    starter: Starter,
    tokens: Tokens,
    tell: Tell,
    feed: Arc<Feed>,
    /// Sessions being restarted now, so one restart is never begun twice.
    restarting: Mutex<HashSet<String>>,
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
pub(crate) struct Holders {
    inner: Arc<Inner>,
}

/// What [`Holders`] is made of.
pub(crate) struct Deps {
    /// This machine's id.
    pub(crate) machine: String,
    /// The release this daemon runs.
    pub(crate) release: Release,
    /// What records the holders this daemon spawns.
    pub(crate) builds: Arc<Builds>,
    pub(crate) migrations: Migrations,
    pub(crate) starter: Starter,
    pub(crate) identities: Arc<Identities>,
    /// The socket this daemon serves on, given to session tabs.
    pub(crate) socket: PathBuf,
    pub(crate) tell: Tell,
    pub(crate) feed: Arc<Feed>,
    /// Some on a machine's daemon (D378): the list is reported, not posted.
    pub(crate) report: Option<Arc<Reports>>,
}

impl Holders {
    pub(crate) fn new(deps: Deps) -> Self {
        let tokens = Tokens::new(deps.starter.store.clone(), deps.identities, deps.socket);
        Self {
            inner: Arc::new(Inner {
                machine: pb::MachineId { value: deps.machine },
                release: deps.release,
                builds: deps.builds,
                migrations: deps.migrations,
                starter: deps.starter,
                tokens,
                tell: deps.tell,
                feed: deps.feed,
                restarting: Mutex::new(HashSet::new()),
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
    pub(crate) fn wake(&self) -> Arc<Notify> {
        self.inner.wake.clone()
    }

    /// Migrate was clicked: each agent waiting is restarted at its next idle
    /// moment, beginning with the ones idle now.
    pub(super) fn begin(&self) {
        self.inner.migrating.store(true, Ordering::SeqCst);
        // A new round: what failed last time is reported again only if it fails again.
        *self.inner.failure.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.inner.wake.notify_one();
    }

    /// Looks once `started` says the daemon's start has brought back what it
    /// could (so a session about to be resumed isn't shown as waiting), then
    /// again at every change that can alter what waits. Returns at once.
    pub(crate) fn start(&self, started: Arc<crate::restart::Started>) {
        if self.inner.release.bundle().is_empty() {
            return;
        }
        let me = self.clone();
        // Subscribed now, so a change during the start is looked at too.
        let (_, mut events) = self.inner.feed.subscribe(None);
        let mut statuses = turns::changes();
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
                crate::log::say!(
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
        if found.waiting.is_empty() {
            if let Err(error) = migrations.resolved(pb::MigrateThing::Holders, machine).await {
                crate::log::say!("humatad2: warning: {}", status::chain(&error));
            }
            return;
        }
        let detail = Detail {
            waiting: found.waiting.iter().map(|waiting| waiting.session.clone()).collect(),
            keeps_running: shell_line(found.shells).into_iter().collect(),
            rows,
            failed: None,
        };
        let build = tools::release_only(&self.inner.release.info());
        if let Err(error) = migrations.observe_holders(machine, build, detail).await {
            crate::log::say!("humatad2: warning: {}", status::chain(&error));
            return;
        }
        if migrations.is_migrating(pb::MigrateThing::Holders, machine) {
            for waiting in found.waiting.into_iter().filter(idle) {
                self.restart_in_background(waiting.session);
            }
        }
    }

    /// The rows of the card (D379): a row for each agent waiting, with what
    /// Migrate does with it, then one for each shell, which stays until it is
    /// closed.
    fn rows(&self, found: &Found) -> Vec<pb::HolderSession> {
        let restarting = self.inner.restarting.lock().unwrap_or_else(PoisonError::into_inner);
        let agents = found.waiting.iter().map(|waiting| {
            let note = if restarting.contains(&waiting.session) {
                pb::HolderNote::Restarting
            } else if waiting
                .holder_pid
                .is_some_and(|pid| turns::running(&waiting.session, pid) == pb::SessionStatus::RunningUnknownTurn)
            {
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
        let shells = (0..found.shells).map(|_| pb::HolderSession {
            session: None,
            title: String::new(),
            note: pb::HolderNote::StaysUntilClosed as i32,
        });
        agents.chain(shells).collect()
    }

    /// A machine's look (D378): the list goes to the host when it changed,
    /// and what the host asked for goes on.
    fn report(&self, report: &Reports, found: Found, rows: Vec<pb::HolderSession>) {
        // With no old holder left there is nothing to retry, so no failure.
        let failed = if found.waiting.is_empty() {
            None
        } else {
            self.inner.failure.lock().unwrap_or_else(PoisonError::into_inner).clone()
        };
        let next = pb::HoldersReport { holders: rows, failed };
        report.send_if_modified(|current| {
            if current.as_ref() == Some(&next) {
                return false;
            }
            *current = Some(next);
            true
        });
        if !self.inner.migrating.load(Ordering::SeqCst) {
            return;
        }
        if found.waiting.is_empty() {
            self.inner.migrating.store(false, Ordering::SeqCst);
            return;
        }
        for waiting in found.waiting.into_iter().filter(idle) {
            self.restart_in_background(waiting.session);
        }
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
                crate::log::say!("humatad2: warning: {}", status::chain(&error));
                return None;
            }
        }
        let store = &inner.starter.store;
        let (agents, tabs) = match (store.agents(), store.tabs()) {
            (Ok(agents), Ok(tabs)) => (agents, tabs),
            (Err(error), _) | (_, Err(error)) => {
                crate::log::say!(
                    "humatad2: warning: couldn't read the sessions to look for old holders: {}",
                    status::chain(&error)
                );
                return None;
            }
        };
        let agent_tabs: HashSet<&str> = agents.iter().map(|agent| agent.tab.as_str()).collect();
        let shell_tabs: Vec<Option<String>> = tabs
            .iter()
            .filter(|tab| tab.kind == TabKind::Shell && !agent_tabs.contains(tab.id.as_str()))
            .map(|tab| inner.builds.recorded(&format!("{}.shell.sha", tab.id)))
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
        let mut found = classify(&wanted, seen, &shell_tabs);
        for waiting in &mut found.waiting {
            if let Ok(Some(session)) = store.session(&waiting.session) {
                waiting.title = session.title;
            }
        }
        Some(found)
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
                crate::log::say!("humatad2: warning: {}", status::chain(&error));
                let problem = holders_problem(&session, &error);
                if me.inner.report.is_some() {
                    // The host's item shows it Failed through the next report.
                    me.inner.migrating.store(false, Ordering::SeqCst);
                    *me.inner.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(problem);
                } else {
                    me.inner.migrations.fail_holders(&me.inner.machine, problem).await;
                }
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
        if !turns_idle(session, holder_pid) {
            return Ok(());
        }
        agent.state = AgentState::Resuming;
        agent.updated_unix_ms = now_ms();
        let saved = {
            let (store, saving) = (starter.store.clone(), agent.clone());
            blocking(session, move || store.put_agent(&saving)).await?
        };
        saved.map_err(store_error("save that the agent is restarting"))?;
        let tell = &self.inner.tell;
        tell.session(session, pb::Change::Updated, None).await;
        let env = |tab: &TabRecord| self.inner.tokens.env(tab);
        let outcome = humata2_revive::restart_agent(&starter.store, &starter.revive, agent, &env, RESTARTED).await;
        let result = match outcome {
            Ok(Revived::Resumed { .. } | Revived::Reattached | Revived::Relaunched) => {
                agent_watch::agent(tell.clone(), session.to_owned());
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
        tell.session(session, pb::Change::Updated, None).await;
        result
    }
}

/// What the Holders item shows when `session`'s agent couldn't be restarted:
/// the cause, and the two ways out.
fn holders_problem(session: &str, error: &HoldersError) -> pb::Error {
    pb::Error {
        message: status::chain(error),
        fix: format!(
            "open session {session}'s terminal to read what it printed and fix the cause, then click Migrate again; or close session {session} to free its old holder"
        ),
        retry: None,
        kind: None,
    }
}

/// Whether the agent is at an idle moment: its status is Idle or
/// ProbablyIdle. One that is working, needs input, or has no turns to tell
/// (and so no idle moment) waits.
fn idle(waiting: &Waiting) -> bool {
    waiting
        .holder_pid
        .is_some_and(|holder_pid| turns_idle(&waiting.session, holder_pid))
}

fn turns_idle(session: &str, holder_pid: u32) -> bool {
    matches!(
        turns::running(session, holder_pid),
        pb::SessionStatus::Idle | pb::SessionStatus::ProbablyIdle
    )
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
pub(crate) struct Split {
    machine: String,
    daemon: Arc<dyn Carry>,
    holders: Arc<OnceLock<Holders>>,
}

impl Split {
    /// `daemon` carries what isn't the holders of `machine`, which `holders`
    /// carries once it is set.
    pub(crate) fn new(machine: &str, daemon: Arc<dyn Carry>, holders: Arc<OnceLock<Holders>>) -> Self {
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

    fn seen(session: &str, state: AgentState, is_shell: bool, recorded: Option<&str>) -> Seen {
        Seen {
            record: agent(session, state),
            is_shell,
            recorded: recorded.map(str::to_owned),
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
                seen("b", running(2), false, Some("aa")),
                seen("a", running(1), false, None),
                seen("c", running(3), false, Some("BB")),
            ],
            &[],
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
            &[],
        );

        let sessions: Vec<&str> = found.waiting.iter().map(|waiting| waiting.session.as_str()).collect();
        assert_eq!(sessions, ["resuming"]);
        assert_eq!(found.waiting[0].holder_pid, None, "an agent that isn't running has no holder to be idle in");
    }

    #[test]
    fn shells_are_counted_and_never_wait() {
        let found = classify(
            NEW,
            vec![
                seen("terminal", running(4), true, Some("aa")),
                seen("current-terminal", running(5), true, Some(NEW)),
            ],
            &[None, Some("aa".to_owned()), Some(NEW.to_owned())],
        );

        assert!(found.waiting.is_empty());
        assert_eq!(found.shells, 3, "two shell tabs and one Terminal session are on old holders");
    }

    #[test]
    fn the_shell_line_is_said_only_for_shells() {
        assert_eq!(shell_line(0), None);
        assert!(shell_line(1).is_some_and(|line| line.starts_with("1 shell ")));
        assert!(shell_line(3).is_some_and(|line| line.starts_with("3 shells ")));
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

        assert_eq!(builds.recorded("tb_1.shell.sha").unwrap(), builds.current().unwrap());
        assert_ne!(builds.recorded("tb_1.shell.sha").unwrap(), first, "the new file was hashed");
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
}
