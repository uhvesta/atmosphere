//! Which turn a running agent is in (SE-21, SE-22, D169).
//!
//! A running agent is WORKING until something says otherwise. Two things do:
//! the agent's own hooks, which `humata turn <state>` reports through
//! `ReportTurn`, and the output fallback, which `watch.rs` feeds: output
//! that has gone quiet for 3 s. [`turn_status`] is the rule that combines
//! them; the rest keeps each running agent's inputs in memory, saves the
//! last hook state so a restart keeps it, and tells the feed when the
//! status changes.
//!
//! The inputs belong to one launch of the agent: they are keyed by the
//! holder's pid, so a launch or a resume (a new holder) starts them over,
//! and the saved hook state is cleared by the store at the same moment.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_access::Refusal;
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{Agents, TurnEnd};
use humata2_store::{AgentState, Store, StoreError};
use pb::invalid_field::Problem;
use tokio::sync::watch;
use tokio::time::Instant;
use tonic::{Code, Request, Response, Status};

use super::SessionSvc;
use crate::channel_service::blocking;
use crate::identity::{self, Caller};
use crate::status;

/// No output for this long: the turn is taken to have ended (SE-21).
const QUIET_AFTER: Duration = Duration::from_secs(3);
/// No output for this long while a hook says the agent works: the turn
/// ended without the agent saying so (an interrupt fires no hook).
const NET_AFTER: Duration = Duration::from_secs(10);
/// A hook heard while the agent launches or resumes is kept this long for
/// the launch to take up.
const PENDING_MAX_NS: i64 = 60_000_000_000;

/// What the agent's own hooks last said (D169).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hook {
    Working,
    Idle,
    NeedsInput,
}

impl Hook {
    /// The `TurnState` the store keeps it as.
    fn code(self) -> i32 {
        match self {
            Self::Working => pb::TurnState::Working as i32,
            Self::Idle => pb::TurnState::Idle as i32,
            Self::NeedsInput => pb::TurnState::NeedsInput as i32,
        }
    }

    /// The hook a saved `TurnState` stands for; none for a value this
    /// build doesn't know.
    pub(super) fn from_code(code: i32) -> Option<Self> {
        match pb::TurnState::try_from(code).ok()? {
            pb::TurnState::Working => Some(Self::Working),
            pb::TurnState::Idle => Some(Self::Idle),
            pb::TurnState::NeedsInput => Some(Self::NeedsInput),
            pb::TurnState::Unspecified => None,
        }
    }
}

/// How long an agent's output has been quiet, as far as the timers have
/// got.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Quiet {
    /// Output within the last [`QUIET_AFTER`].
    Talking,
    /// None for [`QUIET_AFTER`].
    Quiet,
    /// None for [`NET_AFTER`].
    Long,
}

/// The status of a running agent from what its hooks said (none: no hook
/// heard since this launch), how long its output has been quiet, and
/// whether output came back after a quiet spell since the hook spoke.
///
/// - no hook: the output fallback decides, WORKING until it goes quiet,
///   then PROBABLY_IDLE;
/// - a hook said idle: IDLE, however the output looks;
/// - a hook said needs input: NEEDS_INPUT, until output comes back after a
///   quiet spell (the user answered, and no hook said so), then as working;
/// - a hook said working: WORKING until the output has been quiet for
///   [`NET_AFTER`], then PROBABLY_IDLE.
pub(super) fn turn_status(hook: Option<Hook>, quiet: Quiet, resumed: bool) -> pb::SessionStatus {
    match hook {
        Some(Hook::Idle) => pb::SessionStatus::Idle,
        Some(Hook::NeedsInput) if !resumed => pb::SessionStatus::NeedsInput,
        Some(Hook::Working | Hook::NeedsInput) => {
            if quiet == Quiet::Long {
                pb::SessionStatus::ProbablyIdle
            } else {
                pb::SessionStatus::Working
            }
        }
        None => {
            if quiet == Quiet::Talking {
                pb::SessionStatus::Working
            } else {
                pb::SessionStatus::ProbablyIdle
            }
        }
    }
}

/// Times an agent's output for the fallback: the level of quiet, and when
/// the next level is due.
pub(super) struct Quieting {
    last_output: Instant,
    quiet: Quiet,
    /// False for an agent with no turns: nothing is ever due.
    timed: bool,
}

impl Quieting {
    pub(super) fn new(now: Instant, timed: bool) -> Self {
        Self {
            last_output: now,
            quiet: Quiet::Talking,
            timed,
        }
    }

    /// Output at `now`. Returns `Talking` if that ended a quiet spell.
    pub(super) fn output(&mut self, now: Instant) -> Option<Quiet> {
        self.last_output = now;
        (self.quiet != Quiet::Talking).then(|| {
            self.quiet = Quiet::Talking;
            Quiet::Talking
        })
    }

    /// Waits for the next level of quiet and returns it. Cancel safe: the
    /// level changes only once the wait is over.
    pub(super) async fn next_level(&mut self) -> Quiet {
        let due = match (self.timed, self.quiet) {
            (false, _) | (true, Quiet::Long) => None,
            (true, Quiet::Talking) => Some(self.last_output + QUIET_AFTER),
            (true, Quiet::Quiet) => Some(self.last_output + NET_AFTER),
        };
        match due {
            Some(at) => tokio::time::sleep_until(at).await,
            None => std::future::pending().await,
        }
        self.quiet = match self.quiet {
            Quiet::Talking => Quiet::Quiet,
            Quiet::Quiet | Quiet::Long => Quiet::Long,
        };
        self.quiet
    }
}

/// One running agent's inputs.
#[derive(Clone, Copy, Debug)]
struct Entry {
    /// The holder this launch runs in: a different one is another launch.
    holder_pid: u32,
    hook: Option<Hook>,
    /// When the hook's report was made (unix ns); an older report is dropped.
    hook_at: i64,
    quiet: Quiet,
    /// Output came back after a quiet spell since the hook spoke.
    resumed: bool,
    /// False for an agent with no turns (SE-21): it shows no turn.
    timed: bool,
}

impl Entry {
    fn status(&self) -> pb::SessionStatus {
        if self.timed {
            turn_status(self.hook, self.quiet, self.resumed)
        } else {
            pb::SessionStatus::RunningUnknownTurn
        }
    }
}

#[derive(Default)]
struct Inner {
    agents: HashMap<String, Entry>,
    /// Hooks heard while an agent launches or resumes, for `enter`.
    pending: HashMap<String, (Hook, i64)>,
}

struct Registry {
    inner: Mutex<Inner>,
    /// Counts every status change, for a tab that shows its agent's.
    changes: watch::Sender<u64>,
    sources: OnceLock<HashMap<String, pb::TurnEndSource>>,
    hooks: OnceLock<HooksFile>,
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(|| Registry {
    inner: Mutex::default(),
    changes: watch::channel(0).0,
    sources: OnceLock::new(),
    hooks: OnceLock::new(),
});

impl Registry {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Applies `edit` to the entry of `session`'s launch in `holder_pid`,
    /// if there is one. Says whether the status changed (and tells the
    /// tabs when it did), and what `edit` returned.
    fn change<T>(
        &self,
        session: &str,
        holder_pid: u32,
        edit: impl FnOnce(&mut Entry) -> T,
    ) -> Option<(bool, T)> {
        let found = {
            let mut inner = self.lock();
            let entry = inner
                .agents
                .get_mut(session)
                .filter(|entry| entry.holder_pid == holder_pid)?;
            let before = entry.status();
            let result = edit(entry);
            (entry.status() != before, result)
        };
        if found.0 {
            self.bump();
        }
        Some(found)
    }

    fn bump(&self) {
        self.changes.send_modify(|count| *count += 1);
    }
}

fn unix_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_nanos()).unwrap_or(i64::MAX))
}

/// The status of `session`'s running agent in `holder_pid`: what its hooks
/// and its output say, WORKING when nothing is known yet.
pub(crate) fn running(session: &str, holder_pid: u32) -> pb::SessionStatus {
    REGISTRY
        .lock()
        .agents
        .get(session)
        .filter(|entry| entry.holder_pid == holder_pid)
        .map_or(pb::SessionStatus::Working, Entry::status)
}

/// Counts up whenever any running agent's status changes.
pub(crate) fn changes() -> watch::Receiver<u64> {
    REGISTRY.changes.subscribe()
}

/// What `enter` did.
pub(super) struct Entered {
    /// The launch had no entry (or another launch's): there is one now.
    pub(super) inserted: bool,
    /// A hook heard while the agent launched, now taken up and still to be
    /// saved.
    pub(super) pending: Option<(Hook, i64)>,
}

/// Starts the inputs of `session`'s launch in `holder_pid` from the hook
/// state saved for it and any hook heard while it launched, unless they
/// are already kept. The only way an entry comes to be.
pub(super) fn enter(
    session: &str,
    holder_pid: u32,
    saved: Option<(Hook, i64)>,
    timed: bool,
) -> Entered {
    let (entered, changed) = {
        let mut inner = REGISTRY.lock();
        if inner
            .agents
            .get(session)
            .is_some_and(|entry| entry.holder_pid == holder_pid)
        {
            return Entered {
                inserted: false,
                pending: None,
            };
        }
        let mut entry = Entry {
            holder_pid,
            hook: saved.map(|(hook, _)| hook),
            hook_at: saved.map_or(0, |(_, at)| at),
            quiet: Quiet::Talking,
            resumed: false,
            timed,
        };
        let now = unix_ns();
        let pending = inner
            .pending
            .remove(session)
            .filter(|&(_, at)| now.saturating_sub(at) < PENDING_MAX_NS && at > entry.hook_at);
        if let Some((hook, at)) = pending {
            entry.hook = Some(hook);
            entry.hook_at = at;
        }
        let changed = entry.status() != pb::SessionStatus::Working;
        inner.agents.insert(session.to_owned(), entry);
        (
            Entered {
                inserted: true,
                pending,
            },
            changed,
        )
    };
    if changed {
        REGISTRY.bump();
    }
    entered
}

/// Keeps a hook heard while `session`'s agent launches or resumes.
fn keep_pending(session: &str, hook: Hook, at: i64) {
    let mut inner = REGISTRY.lock();
    let newer = inner.pending.get(session).is_none_or(|&(_, kept)| at > kept);
    if newer {
        inner.pending.insert(session.to_owned(), (hook, at));
    }
}

/// Whether a report of `hook` made at `at` changes nothing, from what is
/// kept (the cheap check before the store is read). A same-state report is
/// noted so an older one is still dropped.
fn already_known(session: &str, hook: Hook, at: i64) -> bool {
    let mut inner = REGISTRY.lock();
    let Some(entry) = inner.agents.get_mut(session) else {
        return false;
    };
    if at <= entry.hook_at {
        return true;
    }
    if entry.hook == Some(hook) {
        entry.hook_at = at;
        return true;
    }
    false
}

/// Whether applying `hook` made at `at` to the launch in `holder_pid`
/// would change its hook.
fn would_apply(session: &str, holder_pid: u32, hook: Hook, at: i64) -> bool {
    REGISTRY
        .lock()
        .agents
        .get(session)
        .is_some_and(|entry| {
            entry.holder_pid == holder_pid && at > entry.hook_at && entry.hook != Some(hook)
        })
}

/// Output has been quiet as far as `quiet` (or came back: `Talking`).
/// Says whether the status changed. No entry for the launch: nothing.
pub(super) fn set_quiet(session: &str, holder_pid: u32, quiet: Quiet) -> bool {
    REGISTRY
        .change(session, holder_pid, |entry| {
            if quiet == Quiet::Talking && entry.quiet != Quiet::Talking {
                entry.resumed = true;
            }
            entry.quiet = quiet;
        })
        .is_some_and(|(changed, ())| changed)
}

/// The agent's hook said `hook` in a report made at `at`. Dropped when an
/// older report arrives after a newer one. Says whether the status changed.
fn set_hook(session: &str, holder_pid: u32, hook: Hook, at: i64) -> bool {
    REGISTRY
        .change(session, holder_pid, |entry| {
            if at > entry.hook_at {
                entry.hook = Some(hook);
                entry.hook_at = at;
                entry.resumed = false;
            }
        })
        .is_some_and(|(changed, ())| changed)
}

/// Drops the inputs of `session`'s launch in `holder_pid`, once it exited.
pub(super) fn forget(session: &str, holder_pid: u32) {
    let mut inner = REGISTRY.lock();
    if inner
        .agents
        .get(session)
        .is_some_and(|entry| entry.holder_pid == holder_pid)
    {
        inner.agents.remove(session);
        inner.pending.remove(session);
    }
}

/// Saves a hook `enter` took up from those heard while the agent launched.
pub(super) async fn save_pending(
    store: Arc<Store>,
    session: &str,
    holder_pid: u32,
    pending: Option<(Hook, i64)>,
) {
    let Some((hook, at)) = pending else {
        return;
    };
    let key = session.to_owned();
    let saved = blocking(move || store.set_hook_turn(&key, holder_pid, hook.code(), at)).await;
    if let Ok(Err(error)) = saved {
        crate::log::say!(
            "humatad2: session {session}'s agent said it is {hook:?} as it started, but saving that failed, so a restart forgets it: {}",
            status::chain(&error)
        );
    }
}

/// Where `agent`'s turn ends come from (SE-21): its kind's say, and the
/// output fallback for a kind this daemon doesn't know.
pub(super) fn source_of(agent: &str) -> pb::TurnEndSource {
    REGISTRY
        .sources
        .get()
        .and_then(|sources| sources.get(agent))
        .copied()
        .unwrap_or(pb::TurnEndSource::OutputFallback)
}

/// Remembers where each kind's turn ends come from, for the sessions shown.
/// Once: a second call changes nothing.
pub(crate) fn set_sources(agents: &Agents) {
    let sources = agents
        .kinds()
        .iter()
        .map(|kind| {
            let source = match kind.turn_end {
                TurnEnd::Hooks => pb::TurnEndSource::Hooks,
                TurnEnd::Output => pb::TurnEndSource::OutputFallback,
                TurnEnd::None => pb::TurnEndSource::None,
            };
            (kind.name.clone(), source)
        })
        .collect();
    let _ = REGISTRY.sources.set(sources);
    crate::inbox::permission::codex_trust::set_kinds(agents);
}

// ---------------------------------------------------------------------------
// ReportTurn: the agent's hooks.

impl SessionSvc {
    /// An agent's hook reports its turn state, for its own session only
    /// (D169). Repeating the state it has, or an older report, is a no-op.
    pub(super) async fn report_turn_of(
        &self,
        request: Request<pb::ReportTurnRequest>,
    ) -> Result<Response<pb::ReportTurnResponse>, Status> {
        let caller = identity::caller(&request)?;
        let agent = match &caller {
            Caller::Agent(agent) => agent,
            Caller::Script(_) => return Err(status::refused(&Refusal::ScriptTokenHasNoSession)),
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::User(_) => {
                return Err(status::invalid_field(
                    "state",
                    Problem::NotAllowedHere,
                    "only a session's agent reports its turn; this connection is the user",
                    "the agent's hooks report it with `humata turn`, from the session's own terminal, where HUMATA_SESSION_TOKEN is set",
                ));
            }
        };
        let hook = match pb::TurnState::try_from(request.get_ref().state) {
            Ok(pb::TurnState::Working) => Hook::Working,
            Ok(pb::TurnState::Idle) => Hook::Idle,
            Ok(pb::TurnState::NeedsInput) => Hook::NeedsInput,
            _ => {
                return Err(status::invalid_field(
                    "state",
                    Problem::UnknownValue,
                    "the turn state is missing or one this daemon doesn't know",
                    "report working, idle or needs-input",
                ));
            }
        };
        let at = match request.get_ref().reported_at_unix_ns {
            at if at > 0 => at,
            _ => unix_ns(),
        };
        self.inbox.hook_heard(&agent.agent).await;
        let session = agent.session.id.to_string();
        let timed = source_of(&agent.agent) != pb::TurnEndSource::None;
        let ok = || Ok(Response::new(pb::ReportTurnResponse {}));
        if already_known(&session, hook, at) {
            return ok();
        }

        let (store, key) = (self.starter.store.clone(), session.clone());
        let read = blocking(move || -> Result<Option<(AgentState, Option<(i32, i64)>)>, StoreError> {
            let Some(agent) = store.agent(&key)? else {
                return Ok(None);
            };
            let saved = store.hook_turn(&key)?;
            Ok(Some((agent.state, saved)))
        })
        .await?;
        let (state, saved) = match read {
            Ok(Some(found)) => found,
            Ok(None) => return Err(not_running(&session)),
            Err(error) => {
                return Err(status::store(
                    error,
                    &self.machine,
                    "read the agent's turn state",
                ));
            }
        };
        let holder_pid = match state {
            AgentState::Running { holder_pid } => holder_pid,
            // The launch takes the hook up when it is running.
            AgentState::Launching | AgentState::Resuming => {
                keep_pending(&session, hook, at);
                return ok();
            }
            _ => return Err(not_running(&session)),
        };

        let saved = saved.and_then(|(code, at)| Some((Hook::from_code(code)?, at)));
        let entered = enter(&session, holder_pid, saved, timed);
        save_pending(
            self.starter.store.clone(),
            &session,
            holder_pid,
            entered.pending,
        )
        .await;
        let mut changed = entered.inserted;
        if would_apply(&session, holder_pid, hook, at) {
            // Saved before it shows, so what the feed said survives a crash.
            let (store, key) = (self.starter.store.clone(), session.clone());
            blocking(move || store.set_hook_turn(&key, holder_pid, hook.code(), at))
                .await?
                .map_err(|error| {
                    status::store(error, &self.machine, "save the agent's turn state")
                })?;
            changed |= set_hook(&session, holder_pid, hook, at);
        }
        if changed {
            self.tell
                .session(&session, pb::Change::Updated, None)
                .await;
        }
        ok()
    }
}

fn not_running(session: &str) -> Status {
    status::failure(
        Code::FailedPrecondition,
        pb::Error {
            message: format!("session {session}'s agent isn't running, so its turn state wasn't taken"),
            fix: "resume the session if its agent stopped; a running agent's next hook reports again".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

// ---------------------------------------------------------------------------
// The hooks file Claude Code is started with.

/// Where the hooks file goes and what its hooks run.
struct HooksFile {
    path: PathBuf,
    humata: PathBuf,
    socket: PathBuf,
}

/// Why the hooks file couldn't be written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HooksError {
    #[error(
        "couldn't write the agent hooks file {}; check that the state folder belongs to you and the disk isn't full. Claude Code sessions can't start without it, so humatad2 didn't start",
        path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: StoreError,
    },
}

/// Writes the hooks file at `path` for `humata` talking to `socket`, and
/// remembers all three, so each launch can write it again. At the daemon's
/// start, before it serves.
pub(crate) fn install_hooks(
    path: PathBuf,
    humata: PathBuf,
    socket: PathBuf,
) -> Result<PathBuf, HooksError> {
    write_hooks(&path, &humata, &socket)?;
    let _ = REGISTRY.hooks.set(HooksFile {
        path: path.clone(),
        humata,
        socket,
    });
    Ok(path)
}

/// Writes the hooks file again, before a launch: it may have been deleted
/// or changed. A failure is logged and the file from the last write stays.
pub(super) async fn refresh_hooks() {
    let Some(file) = REGISTRY.hooks.get() else {
        return;
    };
    let (path, humata, socket) = (file.path.clone(), file.humata.clone(), file.socket.clone());
    match blocking(move || write_hooks(&path, &humata, &socket)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            crate::log::say!(
                "humatad2: couldn't rewrite the agent hooks file before a launch, so the agent starts with the last one written: {}",
                status::chain(&error)
            );
        }
        Err(_) => {}
    }
}

/// Writes the file whole (0600), unless it already says this.
fn write_hooks(path: &Path, humata: &Path, socket: &Path) -> Result<(), HooksError> {
    let failed = |source| HooksError::Write {
        path: path.to_owned(),
        source,
    };
    let text = serde_json::to_string_pretty(&hooks_json(humata, socket))
        .map_err(|error| failed(StoreError::Write {
            path: path.to_owned(),
            source: io::Error::other(error),
        }))?;
    if std::fs::read(path).is_ok_and(|have| have == text.as_bytes()) {
        return Ok(());
    }
    humata2_store::write_atomic(path, text.as_bytes()).map_err(failed)
}

/// `text` as one shell word.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// How long Claude Code lets a turn hook run; `humata turn` takes well
/// under a second, and a slower one is dropped, not waited on.
const TURN_HOOK_SECONDS: u64 = 10;

/// Claude Code's settings with the hooks D169 lists. Every turn hook is async
/// and runs `humata turn <state>` against this daemon's socket with its
/// output thrown away and its failure ignored, so it can't block a prompt
/// or put anything in the context. Beside its async report, the
/// PermissionRequest group has the one blocking hook: it asks in the Inbox
/// and holds for the answer (HOOKS, D191).
fn hooks_json(humata: &Path, socket: &Path) -> serde_json::Value {
    let group = |matcher: Option<&str>, state: &str| {
        let command = format!(
            "HUMATA_SOCKET={} {} turn {state} >/dev/null 2>&1 || true",
            quote(&socket.to_string_lossy()),
            quote(&humata.to_string_lossy()),
        );
        let hooks = serde_json::json!([{
            "type": "command",
            "command": command,
            "async": true,
            "timeout": TURN_HOOK_SECONDS,
        }]);
        let mut group = serde_json::Map::new();
        if let Some(matcher) = matcher {
            group.insert("matcher".to_owned(), matcher.into());
        }
        group.insert("hooks".to_owned(), hooks);
        serde_json::Value::Array(vec![serde_json::Value::Object(group)])
    };
    let mut permission = group(None, "needs-input");
    // HOOKS (D191): after the async needs-input report, the blocking hook
    // that asks in the Inbox and holds for the answer.
    if let Some(hooks) = permission[0]["hooks"].as_array_mut() {
        hooks.push(humata2_agent_hooks::claude_permission_hook(humata, socket));
    }
    serde_json::json!({
        "hooks": {
            "SessionStart": group(Some("startup|resume"), "idle"),
            "UserPromptSubmit": group(None, "working"),
            "PostToolBatch": group(None, "working"),
            "Stop": group(None, "idle"),
            "StopFailure": group(None, "idle"),
            "PermissionRequest": permission,
            "PermissionDenied": group(None, "working"),
            "Elicitation": group(None, "needs-input"),
            "ElicitationResult": group(None, "working"),
        },
    })
}

#[cfg(test)]
mod tests {
    use pb::SessionStatus as S;

    use super::*;

    const ALL_QUIET: [Quiet; 3] = [Quiet::Talking, Quiet::Quiet, Quiet::Long];

    #[test]
    fn with_no_hook_the_output_decides() {
        assert_eq!(turn_status(None, Quiet::Talking, false), S::Working);
        assert_eq!(turn_status(None, Quiet::Quiet, false), S::ProbablyIdle);
        assert_eq!(turn_status(None, Quiet::Long, false), S::ProbablyIdle);
    }

    #[test]
    fn a_hook_that_says_idle_wins_over_the_output() {
        for quiet in ALL_QUIET {
            assert_eq!(turn_status(Some(Hook::Idle), quiet, false), S::Idle);
            assert_eq!(turn_status(Some(Hook::Idle), quiet, true), S::Idle);
        }
    }

    #[test]
    fn needs_input_holds_until_output_comes_back_after_a_quiet_spell() {
        for quiet in ALL_QUIET {
            assert_eq!(
                turn_status(Some(Hook::NeedsInput), quiet, false),
                S::NeedsInput
            );
        }
        // The user answered: no hook said so, but the agent writes again.
        assert_eq!(
            turn_status(Some(Hook::NeedsInput), Quiet::Talking, true),
            S::Working
        );
        assert_eq!(
            turn_status(Some(Hook::NeedsInput), Quiet::Long, true),
            S::ProbablyIdle
        );
    }

    #[test]
    fn a_hook_that_says_working_holds_until_the_long_quiet() {
        assert_eq!(turn_status(Some(Hook::Working), Quiet::Talking, false), S::Working);
        assert_eq!(turn_status(Some(Hook::Working), Quiet::Quiet, false), S::Working);
        assert_eq!(
            turn_status(Some(Hook::Working), Quiet::Long, false),
            S::ProbablyIdle
        );
    }

    #[test]
    fn saved_hook_states_round_trip_and_unknown_ones_are_none() {
        for hook in [Hook::Working, Hook::Idle, Hook::NeedsInput] {
            assert_eq!(Hook::from_code(hook.code()), Some(hook));
        }
        assert_eq!(Hook::from_code(0), None);
        assert_eq!(Hook::from_code(99), None);
    }

    #[test]
    fn only_entering_makes_an_entry_and_reports_to_another_launch_do_nothing() {
        let session = "turns-unit-a";
        assert_eq!(running(session, 10), S::Working);
        // No entry yet: nothing to change.
        assert!(!set_hook(session, 10, Hook::Idle, 5));
        assert!(!set_quiet(session, 10, Quiet::Quiet));
        assert_eq!(running(session, 10), S::Working);

        assert!(enter(session, 10, None, true).inserted);
        assert!(!enter(session, 10, None, true).inserted);
        assert!(set_hook(session, 10, Hook::Idle, 5));
        assert_eq!(running(session, 10), S::Idle);
        // Another holder is another launch: it has no entry, and a report
        // for it doesn't make one.
        assert!(!set_hook(session, 11, Hook::Working, 6));
        assert_eq!(running(session, 11), S::Working);
        // The ended launch's entry goes; a late report doesn't bring it back.
        forget(session, 10);
        assert!(!set_hook(session, 10, Hook::Working, 7));
        assert_eq!(running(session, 10), S::Working);
    }

    #[test]
    fn an_older_report_arriving_late_is_dropped() {
        let session = "turns-unit-b";
        enter(session, 1, None, true);
        assert!(set_hook(session, 1, Hook::Idle, 20));
        assert!(!set_hook(session, 1, Hook::Working, 10));
        assert_eq!(running(session, 1), S::Idle);
        assert!(already_known(session, Hook::Working, 10));
        // A same-state report moves the last time forward to 30.
        assert!(already_known(session, Hook::Idle, 30));
        assert!(already_known(session, Hook::Working, 25));
        assert!(!already_known(session, Hook::Working, 40));
        assert!(!would_apply(session, 1, Hook::Working, 25));
        assert!(would_apply(session, 1, Hook::Working, 40));
    }

    #[test]
    fn a_saved_hook_and_one_heard_while_launching_are_taken_up_on_entering() {
        let session = "turns-unit-c";
        let entered = enter(session, 5, Some((Hook::NeedsInput, 100)), true);
        assert!(entered.inserted && entered.pending.is_none());
        assert_eq!(running(session, 5), S::NeedsInput);
        forget(session, 5);

        let at = unix_ns();
        keep_pending(session, Hook::Idle, at);
        keep_pending(session, Hook::Working, at - 1);
        let entered = enter(session, 6, None, true);
        assert_eq!(entered.pending, Some((Hook::Idle, at)));
        assert_eq!(running(session, 6), S::Idle);
        // Taken up once.
        forget(session, 6);
        assert!(enter(session, 7, None, true).pending.is_none());
        // One that is long old isn't.
        forget(session, 7);
        keep_pending(session, Hook::Idle, at - 2 * PENDING_MAX_NS);
        assert!(enter(session, 8, None, true).pending.is_none());
        assert_eq!(running(session, 8), S::Working);
    }

    #[test]
    fn an_agent_with_no_turns_shows_none() {
        let session = "turns-unit-d";
        enter(session, 2, None, false);
        assert_eq!(running(session, 2), S::RunningUnknownTurn);
        set_hook(session, 2, Hook::Idle, 9);
        assert_eq!(running(session, 2), S::RunningUnknownTurn);
    }

    #[test]
    fn a_quiet_spell_then_output_while_waiting_for_input_shows_working() {
        let session = "turns-unit-e";
        enter(session, 3, None, true);
        set_hook(session, 3, Hook::NeedsInput, 1);
        assert!(!set_quiet(session, 3, Quiet::Quiet));
        assert_eq!(running(session, 3), S::NeedsInput);
        assert!(set_quiet(session, 3, Quiet::Talking));
        assert_eq!(running(session, 3), S::Working);
        // A new hook speaks again.
        assert!(set_hook(session, 3, Hook::NeedsInput, 2));
        assert_eq!(running(session, 3), S::NeedsInput);
    }

    #[tokio::test(start_paused = true)]
    async fn the_fallback_flips_at_three_seconds_and_the_net_at_ten() {
        let start = Instant::now();
        let mut quieting = Quieting::new(start, true);
        assert_eq!(quieting.next_level().await, Quiet::Quiet);
        assert_eq!(start.elapsed(), QUIET_AFTER);
        assert_eq!(quieting.next_level().await, Quiet::Long);
        assert_eq!(start.elapsed(), NET_AFTER);
        // Nothing more is due.
        let more = tokio::time::timeout(Duration::from_secs(60), quieting.next_level()).await;
        assert!(more.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn output_restarts_the_timers_and_says_when_it_ended_a_quiet_spell() {
        let mut quieting = Quieting::new(Instant::now(), true);
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(quieting.output(Instant::now()), None);
        tokio::time::advance(Duration::from_secs(2)).await;
        // 4 s since the start but 2 s since the output: not quiet yet.
        let early = tokio::time::timeout(Duration::from_millis(500), quieting.next_level()).await;
        assert!(early.is_err());
        assert_eq!(quieting.next_level().await, Quiet::Quiet);
        assert_eq!(quieting.output(Instant::now()), Some(Quiet::Talking));
    }

    #[tokio::test(start_paused = true)]
    async fn an_agent_with_no_turns_is_never_timed() {
        let mut quieting = Quieting::new(Instant::now(), false);
        let due = tokio::time::timeout(Duration::from_secs(3600), quieting.next_level()).await;
        assert!(due.is_err());
    }

    #[test]
    fn the_hooks_file_lists_every_hook_async_with_the_matchers_d169_gives() {
        let json = hooks_json(Path::new("/bin/humata"), Path::new("/s/d.sock"));
        let hooks = json["hooks"].as_object().unwrap();
        let state_of = |event: &str| {
            let command = hooks[event][0]["hooks"][0]["command"].as_str().unwrap();
            command
                .split(" turn ")
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next())
                .unwrap()
                .to_owned()
        };
        for (event, state) in [
            ("SessionStart", "idle"),
            ("UserPromptSubmit", "working"),
            ("PostToolBatch", "working"),
            ("Stop", "idle"),
            ("StopFailure", "idle"),
            ("PermissionRequest", "needs-input"),
            ("PermissionDenied", "working"),
            ("Elicitation", "needs-input"),
            ("ElicitationResult", "working"),
        ] {
            assert_eq!(state_of(event), state, "{event}");
            assert_eq!(hooks[event][0]["hooks"][0]["async"], true, "{event}");
        }
        assert_eq!(hooks["SessionStart"][0]["matcher"], "startup|resume");
        assert!(hooks["Stop"][0].get("matcher").is_none());
        assert_eq!(hooks.len(), 9);
    }

    #[test]
    fn the_generated_command_runs_humata_turn_quietly_and_never_fails() {
        use std::os::unix::fs::PermissionsExt;
        let folder = tempfile::tempdir().unwrap();
        let bin = folder.path().join("it's here");
        std::fs::create_dir(&bin).unwrap();
        let seen = folder.path().join("seen");
        let humata = bin.join("humata");
        let socket = folder.path().join("so'cket");
        let script = |exit: i32| {
            format!(
                "#!/bin/sh\necho \"$HUMATA_SOCKET|$*\" > '{}'\necho loud\necho loud >&2\nexit {exit}\n",
                seen.display()
            )
        };
        let command_for = |event: &str| {
            let json = hooks_json(&humata, &socket);
            json["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        for exit in [0, 1] {
            std::fs::write(&humata, script(exit)).unwrap();
            std::fs::set_permissions(&humata, std::fs::Permissions::from_mode(0o755)).unwrap();
            let _ = std::fs::remove_file(&seen);
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(command_for("Stop"))
                .output()
                .unwrap();
            assert!(output.status.success(), "exit {exit}");
            assert!(output.stdout.is_empty() && output.stderr.is_empty());
            assert_eq!(
                std::fs::read_to_string(&seen).unwrap().trim(),
                format!("{}|turn idle", socket.display())
            );
        }
    }

    #[test]
    fn the_hooks_file_is_written_whole_private_and_not_again_when_unchanged() {
        use std::os::unix::fs::PermissionsExt;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("claude-hooks.json");
        write_hooks(&path, Path::new("/bin/humata"), Path::new("/s")).unwrap();
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();
        write_hooks(&path, Path::new("/bin/humata"), Path::new("/s")).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), first);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(json["hooks"]["Stop"].is_array());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        write_hooks(&path, Path::new("/other/humata"), Path::new("/s")).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("/other/humata"));
    }
}
