//! What the daemon does with the sessions it finds when it starts: brings
//! back each agent and shell whose holder is gone (humata2_revive), writes
//! the restart report, and keeps what wasn't brought back for the app to
//! show (PER-25–PER-29, PER-31).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_revive::{Reconciled, RestartReports, ReviveConfig, Revived, ReviveError};
use humata2_sessions::{Agents, Layout, Leftovers, Recovery, Starter, recover};
use humata2_store::{Restored, ResumeFailure, Store, StoreError, TabKind, TabRecord};

use crate::host_vars::HostVars;
use crate::identity::{Identities, TabIdentity};
use crate::status;

mod operations;

pub(crate) use operations::{Operations, State as OperationState};

/// The ids a report folder's `operations.jsonl` has a `pending` line for.
#[cfg(test)]
pub(crate) fn pending_in(folder: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(folder.join("operations.jsonl")) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value["state"] == "pending")
        .filter_map(|value| value["id"].as_str().map(str::to_owned))
        .collect()
}

/// How long a resumed agent has to show it took up its conversation.
const RESUME_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a resumed agent's output stays quiet before it's told it was
/// interrupted.
const SETTLE: Duration = Duration::from_secs(1);

/// Where and how the sessions are brought back.
pub(crate) struct Revive {
    /// SEC-6: what contains agents when their setting says so.
    pub(crate) contain: Option<Arc<dyn humata2_revive::Contain>>,
    pub(crate) holder: PathBuf,
    /// D373: keeps which holder file each agent and shell holder was spawned from.
    pub(crate) builds: Option<Arc<dyn humata2_revive::HolderBuilds>>,
    pub(crate) logs: PathBuf,
    pub(crate) scrollback_bytes: usize,
    pub(crate) shell: PathBuf,
    pub(crate) shell_args: Vec<OsString>,
    /// Where a revived shell starts when its folder is gone: the daemon's root.
    pub(crate) home: PathBuf,
    /// The home the agents' CLIs run in (`$HOME`), where their own dirs
    /// (`{home}/.claude`, `.codex`, `.copilot`) and so their conversation
    /// files are (B5): not the root, which `--root` may move elsewhere.
    pub(crate) agent_home: PathBuf,
    /// The data folder's `restarts/`.
    pub(crate) reports: PathBuf,
    /// This machine's id, named in the report when it has no host name.
    pub(crate) machine: String,
    /// The daemon's log from the run before, if there was one.
    pub(crate) daemon_log: Option<PathBuf>,
}

/// Where channels live and what a session starts, besides how it is
/// brought back.
pub(crate) struct Sessions {
    /// The folder holding every channel's folder (FC-1).
    pub(crate) channels: Layout,
    /// Where a repository is cloned from, with `{owner}` and `{name}`.
    pub(crate) remote_template: String,
    pub(crate) agents: Agents,
    /// Where each terminal's holder keeps its socket.
    pub(crate) holders: PathBuf,
}

/// What starts sessions on this machine and brings them back: `revive`
/// says how, `sessions` where and what. The same one serves
/// SessionService and the restart, so the two never launch an agent twice.
pub(crate) fn starter(store: &Arc<Store>, revive: Revive, sessions: Sessions) -> Starter {
    Starter {
        store: store.clone(),
        layout: sessions.channels,
        remote_template: sessions.remote_template,
        agents: sessions.agents.clone(),
        home: revive.agent_home,
        revive: ReviveConfig {
            contain: revive.contain,
            builds: revive.builds,
            // CV-3: a resume asks each agent's CLI for its version again.
            versions: Some(Arc::new(sessions.agents)),
            // D763: a resume's interrupted note waits for a question
            // the agent asks to be answered.
            gate: Some(Arc::new(crate::input_gate::StartGate)),
            holder: revive.holder,
            logs: revive.logs,
            scrollback_bytes: revive.scrollback_bytes,
            shell: revive.shell,
            shell_args: revive.shell_args,
            home: revive.home,
            resume_timeout: RESUME_TIMEOUT,
            settle: SETTLE,
            reports: RestartReports {
                folder: revive.reports,
                version: env!("CARGO_PKG_VERSION").to_owned(),
                machine: host_name().unwrap_or(revive.machine),
                os: os_name(),
                daemon_log: revive.daemon_log,
            },
        },
        holders: sessions.holders,
        first_message: humata2_sessions::FirstMessage::default(),
    }
}

/// How long a request waits for this daemon's restart to finish before it is
/// answered Unavailable. Provisional: 15 minutes, longer than any one agent's
/// resume; recommend keeping it. Tests give [`Started::bounded`] a short one.
pub(crate) const RESTART_WAIT: Duration = Duration::from_secs(15 * 60);

/// What to do about a machine whose restart hasn't finished. Said by the
/// machine to whoever asked, and by the host of an operation the machine lost.
pub(crate) const RESTART_FIX: &str = "the machine's restart hasn't finished; check `humata doctor restarts` on that machine, then try again";

/// What a request was waiting to do when the restart hadn't finished.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Awaited {
    Start,
    DeskStart,
    Resume,
    Read,
    Attach,
}

impl Awaited {
    fn doing(self) -> &'static str {
        match self {
            Self::Start => "start the session",
            Self::DeskStart => "start the Desk session",
            Self::Resume => "resume the session",
            Self::Read => "read the session",
            Self::Attach => "open the tab's terminal",
        }
    }
}

/// A request that waited for this machine's restart and gave up.
#[derive(Debug, thiserror::Error)]
enum RestartUnfinished {
    #[error("this machine's restart hasn't finished after {waited:?}, so it can't {doing} yet")]
    NotDone {
        doing: &'static str,
        waited: Duration,
        #[source]
        source: tokio::time::error::Elapsed,
    },
}

impl From<RestartUnfinished> for tonic::Status {
    fn from(error: RestartUnfinished) -> Self {
        status::failure(
            tonic::Code::Unavailable,
            pb::Error {
                message: error.to_string(),
                fix: RESTART_FIX.to_owned(),
                retry: None,
                kind: None,
            },
        )
    }
}

/// What this start brought back, set once when it's done. Waiters wake
/// when it is set: nothing looks again on a timer.
#[derive(Default)]
pub(crate) struct Started {
    done: OnceLock<pb::LastStart>,
    set: tokio::sync::Notify,
    /// The operations this start found cut short and how each ended (D241).
    operations: Operations,
    /// Set once every recovery of the last run's operations has said it is
    /// pending, before anything is served.
    recorded: std::sync::atomic::AtomicBool,
    recording: tokio::sync::Notify,
    /// How long [`Started::wait_for`] waits; [`RESTART_WAIT`] when unset.
    bound: Option<Duration>,
}

impl Started {
    /// What the start brought back, once it's done.
    pub(crate) fn get(&self) -> Option<&pb::LastStart> {
        self.done.get()
    }

    /// Where the recoveries of this start say what they found and did.
    pub(crate) fn operations(&self) -> &Operations {
        &self.operations
    }

    pub(crate) fn set(&self, start: pb::LastStart) {
        let _ = self.done.set(start);
        self.set.notify_waiters();
    }

    /// Says that every recovery of the last run's cut operations has noted
    /// itself pending: the restart report, written after this, names each
    /// of them, so a report never reads as finished while one is owed.
    pub(crate) fn recoveries_noted(&self) {
        self.recorded.store(true, std::sync::atomic::Ordering::SeqCst);
        self.recording.notify_waiters();
    }

    /// Returns once [`Started::recoveries_noted`] was called.
    async fn noted(&self) {
        loop {
            let woken = self.recording.notified();
            tokio::pin!(woken);
            woken.as_mut().enable();
            if self.recorded.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            woken.await;
        }
    }

    /// A start that waits only this long for the restart.
    #[cfg(test)]
    fn bounded(waited: Duration) -> Self {
        Self { bound: Some(waited), ..Self::default() }
    }

    /// [`Started::wait`] for a request that holds a caller until the restart
    /// is done: past the bound the caller is answered Unavailable with a fix,
    /// not held for ever. Only the restart is bounded; whatever the request
    /// does after it, however slow, is not.
    pub(crate) async fn wait_for(&self, doing: Awaited) -> Result<(), tonic::Status> {
        let waited = self.bound.unwrap_or(RESTART_WAIT);
        tokio::time::timeout(waited, self.wait()).await.map_err(|source| {
            RestartUnfinished::NotDone { doing: doing.doing(), waited, source }.into()
        })
    }

    /// Returns once the start is done; at once if it already is.
    pub(crate) async fn wait(&self) {
        loop {
            let woken = self.set.notified();
            tokio::pin!(woken);
            woken.as_mut().enable();
            if self.done.get().is_some() {
                return;
            }
            woken.await;
        }
    }
}

/// Brings back what the last run left while the daemon serves: each agent
/// may take [`RESUME_TIMEOUT`] to resume, longer than a bridge waits for
/// the daemon to listen. Then finishes or undoes the channel setups and
/// session starts the last run cut short, in this same task so nothing
/// launches an agent twice. The lock is set once it's done.
pub(crate) fn in_background(
    store: Arc<Store>,
    identities: Arc<Identities>,
    starter: Starter,
    restored: Option<Restored>,
    tell: crate::session_service::Tell,
    socket: PathBuf,
    after_stop: Option<crate::stop::boot::AfterStop>,
) -> Arc<Started> {
    let done = Arc::new(Started::default());
    let setting = done.clone();
    let operations = done.operations().clone();
    // Read now, before anything is served, so a channel or session a call
    // makes while this runs is left to that call.
    let left = Leftovers::read(&store);
    // Likewise the rebuilds the last run began here and didn't end.
    let cut_rebuilds = crate::session_service::cut_rebuilds(&starter);
    // D241: each is named pending now, before the report is written.
    for rebuild in &cut_rebuilds {
        rebuild.note(&operations);
    }
    // D241: so is each session start this run will finish or undo; serve.rs
    // calls `recoveries_noted` only after this returns.
    let cut_starts = cut_starts(&store, &left);
    note_cut_starts(&operations, &cut_starts);
    // D241: and each agent saved as launching, which the reconcile starts again.
    let cut_launches = launching_agents(&store);
    note_cut_launches(&operations, &cut_launches);
    tokio::spawn(async move {
        // D241: the report is written once every recovery said what it owes.
        setting.noted().await;
        let start = bring_back(
            &store,
            &identities,
            &starter,
            restored.as_ref(),
            left,
            cut_launches,
            cut_rebuilds,
            cut_starts,
            socket,
            &operations,
        )
        .await;
        // D265: a start after a stop names it in the report and the start.
        let start = match after_stop {
            Some(after) => crate::stop::boot::finish_start(start, after).await,
            None => start,
        };
        setting.set(start);
        crate::session_service::after_restart(tell).await;
    });
    done
}

/// D670: before bringing agents back, each whose CLI picked its own
/// conversation id and wrote it learns it, so it resumes that conversation.
/// One that can't be read is resumed as saved, and says why.
async fn learn_conversations(store: &Arc<Store>, starter: &Starter) {
    let (store, agents, home) = (store.clone(), starter.agents.clone(), starter.home.clone());
    let learned = tokio::task::spawn_blocking(move || -> Result<(), StoreError> {
        for agent in store.agents()? {
            if let Err(error) = agents.learn_conversation(&store, &home, &agent.session) {
                crate::log::say!(
                    "humatad2: couldn't learn session {}'s conversation, so it resumes as saved: {}",
                    agent.session,
                    status::chain(&error)
                );
            }
        }
        Ok(())
    })
    .await;
    match learned {
        Ok(Ok(())) => {}
        Ok(Err(error)) => crate::log::say!(
            "humatad2: couldn't read the agents to learn their conversations; they resume as saved: {}",
            status::chain(&error)
        ),
        Err(error) => crate::log::say!(
            "humatad2: learning the agents' conversations stopped; they resume as saved: {error}"
        ),
    }
}

/// Brings back what the last run left, and says what it couldn't. Never
/// fails the start: a reconcile that can't run is itself the one line.
#[allow(clippy::too_many_arguments)]
async fn bring_back(
    store: &Arc<Store>,
    identities: &Arc<Identities>,
    starter: &Starter,
    restored: Option<&Restored>,
    left: Result<Leftovers, StoreError>,
    cut_launches: Vec<String>,
    cut_rebuilds: Vec<crate::session_service::CutRebuild>,
    cut_starts: Vec<String>,
    socket: PathBuf,
    operations: &Operations,
) -> pb::LastStart {
    let tokens = Tokens::new(store.clone(), identities.clone(), socket);
    let env = |tab: &TabRecord| tokens.env(tab);
    let mut start = pb::LastStart {
        started_at: Some(crate::attach::now()),
        ..pb::LastStart::default()
    };
    if let Some(restored) = restored {
        start.restored_copy_taken_unix_ms = restored.copy_taken_unix_ms;
        start.restored_rows_changed = u32::try_from(restored.changes.len()).unwrap_or(u32::MAX);
    }
    // The operations' lines go into the report before its restart.json: a
    // reader that finds restart.json takes the report as whole (D432).
    learn_conversations(store, starter).await;
    let ops = operations.clone();
    let before_json = Box::new(move |folder: &Path| ops.set_folder(folder));
    match humata2_revive::reconcile_then(store, &starter.revive, &env, restored, before_json).await {
        Ok(reconciled) => {
            note_relaunched(operations, &cut_launches, &reconciled.agents, |session| {
                store.agent(session).ok().flatten().map(|agent| agent.state)
            });
            describe(reconciled, &mut start);
        }
        Err(error) => {
            note_relaunched(operations, &cut_launches, &[], |_| None);
            crate::log::say!(
                "humatad2: couldn't bring back this machine's sessions: {}",
                status::chain(&error)
            );
            start.not_brought_back.push(pb::NotBroughtBack {
                session: String::new(),
                tab: String::new(),
                problem: status::chain(&error),
            });
        }
    }
    // Before the starts are finished: a rebuild cut short would be started
    // with no conversation.
    crate::session_service::undo_cut_rebuilds(starter, cut_rebuilds, operations).await;
    match left {
        Ok(left) => {
            let recovered = recover(starter, &env, &left).await;
            note_ended(operations, &cut_starts, &recovered);
            for recovery in recovered {
                if let Some(line) = recovery_line(recovery) {
                    start.not_brought_back.push(line);
                }
            }
        }
        Err(error) => start.not_brought_back.push(pb::NotBroughtBack {
            session: String::new(),
            tab: String::new(),
            problem: format!(
                "couldn't read which channel setups and session starts the last run cut short ({}), so none were finished; the store may be damaged or its disk full, so check the data folder's disk, then start humatad2 again",
                status::chain(&error)
            ),
        }),
    }
    start
}

/// What the restart report calls the start of session `session`. The name
/// keeps it apart from other operations on the same session (Provisional:
/// `start:<id>` beside `rebuild:<id>` and `restore_channel:<id>`; recommend
/// keeping it).
fn start_operation(session: &str) -> String {
    format!("start:{session}")
}

/// The sessions whose starts the last run cut short and this run will
/// finish or undo, read with `left` before anything is served. When they
/// can't be read, the starts are still recovered and named as they end.
fn cut_starts(store: &Store, left: &Result<Leftovers, StoreError>) -> Vec<String> {
    let Ok(left) = left else {
        return Vec::new();
    };
    left.cut_starts(store).unwrap_or_else(|error| {
        crate::log::say!(
            "humatad2: couldn't read which session starts the last run cut short, so the restart report names them only as they end: {}",
            status::chain(&error)
        );
        Vec::new()
    })
}

/// Names each of `sessions` pending in the restart report. Called before
/// the report is written, so it never reads as finished while a start is
/// owed (D241).
fn note_cut_starts(operations: &Operations, sessions: &[String]) {
    for session in sessions {
        operations.record(
            &start_operation(session),
            "start",
            "recovering",
            OperationState::Pending { waits_for: "the restart to finish or undo it".to_owned() },
        );
    }
}

/// Names how each start of `recovered` ended, and every one of `cut` that
/// recovery never reported as stuck, so none stays pending. A start taken
/// back is `Undone` and one left with its folder `Stuck` (Provisional:
/// `Undone`, not `Finished`, for a start taken back, as the move, fork and
/// rebuild recoveries record theirs; recommend keeping it).
fn note_ended(operations: &Operations, cut: &[String], recovered: &[Recovery]) {
    let mut ended: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for recovery in recovered {
        let (session, state) = match recovery {
            Recovery::SessionFinished { session } => (session, OperationState::Finished),
            Recovery::SessionUndone { session, reason } => (session, OperationState::Undone { reason: reason.clone() }),
            Recovery::SessionStuck { session, reason } => (
                session,
                OperationState::Stuck { reason: format!("{reason}; delete its folder by hand, then start it again") },
            ),
            Recovery::ChannelFinished { .. } | Recovery::ChannelStuck { .. } => continue,
        };
        // A stuck line with no session is the listing that failed: the
        // starts it hid are named below.
        if session.is_empty() {
            continue;
        }
        operations.record(&start_operation(session), "start", "recovering", state);
        ended.insert(session.as_str());
    }
    for session in cut.iter().filter(|session| !ended.contains(session.as_str())) {
        operations.record(
            &start_operation(session),
            "start",
            "recovering",
            OperationState::Stuck {
                reason: "the restart ended before it looked at this start; start humatad2 again to try it again".to_owned(),
            },
        );
    }
}

/// What recovery did that the person should hear of: a channel or session
/// that couldn't be finished. A finished one needs nothing said.
fn recovery_line(recovery: Recovery) -> Option<pb::NotBroughtBack> {
    let (session, problem) = match recovery {
        Recovery::ChannelFinished { .. } | Recovery::SessionFinished { .. } => return None,
        Recovery::ChannelStuck { channel, reason } => (
            String::new(),
            format!(
                "channel {channel} couldn't be finished setting up ({reason}); its clones are marked as they stand, so check the repository addresses and that git can reach them, then start humatad2 again"
            ),
        ),
        Recovery::SessionUndone { session, reason } => {
            let problem = format!(
                "session {session} was being started when humatad2 stopped and couldn't be finished ({reason}); it and its folder were removed, so start it again"
            );
            (session, problem)
        }
        Recovery::SessionStuck { session, reason } => {
            let problem = format!(
                "session {session} was being started when humatad2 stopped and couldn't be finished or removed ({reason}); delete its folder by hand, then start it again"
            );
            (session, problem)
        }
    };
    Some(pb::NotBroughtBack {
        session,
        tab: String::new(),
        problem,
    })
}

/// What the restart report calls the relaunch of session `session`'s agent
/// (Provisional: `relaunch:<id>` beside `start:<id>`; recommend keeping it).
fn relaunch_operation(session: &str) -> String {
    format!("relaunch:{session}")
}

/// The sessions whose agent is saved as launching, read before anything is
/// served: the reconcile starts each again if its holder is gone.
fn launching_agents(store: &Store) -> Vec<String> {
    match store.agents() {
        Ok(agents) => agents
            .into_iter()
            .filter(|agent| agent.state == humata2_store::AgentState::Launching)
            .map(|agent| agent.session)
            .collect(),
        Err(error) => {
            crate::log::say!(
                "humatad2: couldn't read which agents were still starting, so the restart report names them only as they end: {}",
                status::chain(&error)
            );
            Vec::new()
        }
    }
}

/// Names each of `sessions` pending in the restart report, before the
/// report is written (D241).
fn note_cut_launches(operations: &Operations, sessions: &[String]) {
    for session in sessions {
        operations.record(
            &relaunch_operation(session),
            "relaunch",
            "recovering",
            OperationState::Pending { waits_for: "the restart to start the agent again".to_owned() },
        );
    }
}

/// How the relaunch of an agent ended, for the report. A start that failed
/// leaves the agent stopped (`Undone`: taken back, with what to do), or, if
/// even that couldn't be saved, as it was (`Stuck`).
fn relaunch_end(
    outcome: Option<&Result<Revived, humata2_revive::ReviveError>>,
    state: Option<humata2_store::AgentState>,
) -> OperationState {
    use humata2_store::AgentState;
    match outcome {
        Some(Ok(Revived::ResumeFailed { reason, exit_code })) => {
            OperationState::Undone { reason: resume_failed(*reason, *exit_code) }
        }
        Some(Ok(_)) => OperationState::Finished,
        Some(Err(error)) => {
            let why = status::chain(error);
            match state {
                Some(AgentState::Stopped) => OperationState::Undone {
                    reason: format!(
                        "{why}; the agent is marked stopped, so resume the session to try again, or start it fresh with a summary"
                    ),
                },
                Some(AgentState::Resuming) => OperationState::Stuck {
                    reason: format!("{why}; resume the session to try again"),
                },
                _ => OperationState::Stuck {
                    reason: format!("{why}; start humatad2 again to try it again"),
                },
            }
        }
        None => OperationState::Stuck {
            reason: "the restart ended before it looked at this agent; start humatad2 again to try it again".to_owned(),
        },
    }
}

/// Names how each of `cut` ended, so none stays pending. `state_of` reads an
/// agent's saved state.
fn note_relaunched(
    operations: &Operations,
    cut: &[String],
    agents: &[(String, Result<Revived, humata2_revive::ReviveError>)],
    state_of: impl Fn(&str) -> Option<humata2_store::AgentState>,
) {
    for session in cut {
        let outcome = agents.iter().find(|(id, _)| id == session).map(|(_, outcome)| outcome);
        let state = match outcome {
            Some(Err(_)) => state_of(session),
            _ => None,
        };
        operations.record(&relaunch_operation(session), "relaunch", "recovering", relaunch_end(outcome, state));
    }
}

/// Why a session's agent couldn't be resumed, and what the person can do.
fn resume_failed(reason: ResumeFailure, exit_code: Option<i32>) -> String {
    let why = match reason {
        ResumeFailure::ConversationFileMissing => {
            "its conversation file is gone, so it can't be resumed; start it fresh with a summary, or restore the file from a backup and resume it"
        }
        ResumeFailure::AgentStartedAnotherConversation => {
            "the agent started another conversation instead of resuming its own, so it was stopped; resume it again, or start it fresh with a summary"
        }
        ResumeFailure::AgentExited => {
            "the agent exited while resuming; read its tab's output for why, then resume it again"
        }
        ResumeFailure::AgentNotInstalled => {
            "its agent program isn't installed on this machine; install it, then resume the session"
        }
        ResumeFailure::ConversationUnknown => {
            "it was adopted and its conversation isn't known, so it can't be resumed; End it for good, or keep it running"
        }
        ResumeFailure::FolderMissing => {
            "its folder is gone, so nothing was started in it; restore the folder, then resume the session"
        }
    };
    match exit_code {
        Some(code) => format!("the session wasn't resumed: {why} (the agent exited with {code})"),
        None => format!("the session wasn't resumed: {why}"),
    }
}

/// What went wrong with an agent or shell that wasn't brought back, and what
/// to do about it.
fn problem_of(error: &ReviveError) -> String {
    format!("{}; {}", status::chain(error), fix(error))
}

/// What the person does next, by what kind of error it was. Some errors
/// already end in their own fix; this is then the next step if that doesn't
/// work. Provisional: recommend keeping this wording's shape (one line per
/// kind, mapped from the variant); the exact steps are open to taste.
fn fix(error: &ReviveError) -> &'static str {
    match error {
        ReviveError::Store { .. } => {
            "check that the data folder's disk has space and is readable, then restart humatad2; if it still fails, resume the session from Humata"
        }
        ReviveError::Holder { .. } => {
            "check that humata2_holder is installed next to humatad2 and runs, then restart humatad2; if it still fails, start a new session from Humata with a summary of this one"
        }
        ReviveError::EndHolder { .. } => {
            "if restarting humatad2 doesn't clear it, end that terminal's process by hand as the cause says, then resume the session from Humata"
        }
        ReviveError::NoCommand { .. } => {
            "there is nothing to resume: start the session fresh from Humata with a summary of the old conversation"
        }
        ReviveError::Injected { .. } => "start humatad2 again without HUMATA2_FAIL_AT, then resume the session from Humata",
        ReviveError::Background { .. } => {
            "resume the session from Humata once humatad2 is back; if it fails again, report it as a bug"
        }
        ReviveError::Contain { .. } => {
            "resume the session once the cause is fixed; Humata won't run the agent uncontained, so start it fresh only if resuming keeps being refused"
        }
        ReviveError::ScrubReport { .. } | ReviveError::WriteReport { .. } | ReviveError::RestartJson { .. } => {
            "no session is affected; the next restart writes a new report"
        }
    }
}

fn describe(reconciled: Reconciled, start: &mut pb::LastStart) {
    for (session, outcome) in reconciled.agents {
        if let Ok(Revived::Resumed { .. }) = &outcome {
            start.resumed.push(pb::SessionId {
                value: session.clone(),
            });
        }
        if let Ok(Revived::ResumeFailed { reason, exit_code }) = &outcome {
            // PER-15, PER-29: nothing runs in its place, so the start names it.
            start.not_brought_back.push(pb::NotBroughtBack {
                session: session.clone(),
                tab: String::new(),
                problem: resume_failed(*reason, *exit_code),
            });
            continue;
        }
        if let Err(error) = outcome {
            start.not_brought_back.push(pb::NotBroughtBack {
                session,
                tab: String::new(),
                problem: problem_of(&error),
            });
        }
    }
    for (tab, outcome) in reconciled.shells {
        if let Err(error) = outcome {
            start.not_brought_back.push(pb::NotBroughtBack {
                session: String::new(),
                tab,
                problem: problem_of(&error),
            });
        }
    }
    match reconciled.report {
        Ok(folder) => {
            start.report = folder.display().to_string();
        }
        Err(error) => crate::log::say!(
            "humatad2: couldn't write the restart report: {}",
            status::chain(&error)
        ),
    }
}

/// A fresh session token for each session tab brought back, the same one
/// each time reviving asks for that tab, since the report masks it too.
pub(crate) struct Tokens {
    store: Arc<Store>,
    identities: Arc<Identities>,
    /// The socket this daemon serves on, given to session tabs.
    socket: PathBuf,
    minted: Mutex<HashMap<String, Vec<(OsString, OsString)>>>,
}

impl Tokens {
    pub(crate) fn new(store: Arc<Store>, identities: Arc<Identities>, socket: PathBuf) -> Self {
        Self {
            store,
            identities,
            socket,
            minted: Mutex::new(HashMap::new()),
        }
    }

    /// What a tab is started with: the host variables for an agent (HO-13),
    /// then the session's token and socket, last so a variable can never
    /// replace them.
    pub(crate) fn env(&self, tab: &TabRecord) -> Vec<(OsString, OsString)> {
        let mut env = self.host_variables(tab);
        let mut token = self.token_env(tab);
        // D748: a PATH among the host variables keeps Humata's tools first.
        let path_var = crate::identity::PATH_VAR;
        if let Some((_, host)) = env.iter().rev().find(|(name, _)| name == path_var)
            && let Some(path) = crate::identity::tools_path(Some(host.clone()))
        {
            token.retain(|(name, _)| name != path_var);
            token.push((OsString::from(path_var), path));
        }
        env.extend(token);
        env
    }

    /// The host variables a session's agent is started with. When they
    /// can't be read the agent starts without them, and the log says why.
    fn host_variables(&self, tab: &TabRecord) -> Vec<(OsString, OsString)> {
        if tab.session.is_none() || !matches!(tab.kind, TabKind::Agent) {
            return Vec::new();
        }
        // Read each time an agent is launched, so a change reaches the agents
        // started after it and never one already running (HO-13).
        match HostVars::installed().and_then(|variables| variables.env()) {
            Ok(env) => env,
            Err(error) => {
                crate::log::say!(
                    "humatad2: tab {}'s agent starts without this machine's host variables: {}",
                    tab.id,
                    status::chain(&error)
                );
                Vec::new()
            }
        }
    }

    fn token_env(&self, tab: &TabRecord) -> Vec<(OsString, OsString)> {
        let Some(session) = &tab.session else {
            return Vec::new();
        };
        let mut minted = self.minted.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(env) = minted.get(&tab.id) {
            return env.clone();
        }
        let record = match self.store.session(session) {
            Ok(Some(record)) => record,
            Ok(None) => return Vec::new(),
            Err(error) => {
                crate::log::say!(
                    "humatad2: couldn't read session {session} to give tab {} a token, so its agent can't reach Humata until the tab is restarted: {}",
                    tab.id,
                    status::chain(&error)
                );
                return Vec::new();
            }
        };
        // The old holder's token went with it.
        if let Err(error) = self.identities.revoke(&tab.id) {
            crate::log::say!(
                "humatad2: couldn't drop tab {}'s old token: {}",
                tab.id,
                status::chain(&error)
            );
        }
        let env = match self
            .identities
            .mint_as(&tab.id, &record, tab_identity(tab.kind))
        {
            Ok(token) => crate::identity::tab_env(token, &self.socket),
            Err(error) => {
                crate::log::say!(
                    "humatad2: couldn't save a token for tab {}, so its agent can't reach Humata until the tab is restarted: {}",
                    tab.id,
                    status::chain(&error)
                );
                Vec::new()
            }
        };
        minted.insert(tab.id.clone(), env.clone());
        env
    }
}

/// What a token minted for a tab of `kind` says its holder is. The
/// session's agent and a shell in its tree act as the session (SEC-8); an
/// added agent has no identity (SE-29, D103).
///
/// Waits for seam S4: the store's `TabKind` has no `AddedAgent` yet, and no
/// RPC opens one. When it does, add `TabKind::AddedAgent =>
/// TabIdentity::AddedAgent` here; nothing else changes.
fn tab_identity(kind: TabKind) -> TabIdentity {
    match kind {
        TabKind::Agent | TabKind::Shell => TabIdentity::Session,
    }
}

/// This machine's host name, as `uname -n` gives it.
fn host_name() -> Option<String> {
    let output = std::process::Command::new("uname")
        .arg("-n")
        .output()
        .ok()?;
    let name = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !name.is_empty()).then_some(name)
}

/// The OS and its version, such as `macOS 26.7` or `Ubuntu 24.04.1 LTS`.
fn os_name() -> String {
    if cfg!(target_os = "macos") {
        let version = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|version| version.trim().to_owned())
            .filter(|version| !version.is_empty());
        return match version {
            Some(version) => format!("macOS {version}"),
            None => "macOS".to_owned(),
        };
    }
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| pretty_name(&text))
        .unwrap_or_else(|| std::env::consts::OS.to_owned())
}

/// `PRETTY_NAME` from an os-release file.
fn pretty_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map(|value| value.trim().trim_matches('"').to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod fix_tests {
    //! Every agent or shell the report says wasn't brought back says what to
    //! do. Presence only: the wording is not asserted.

    use std::path::PathBuf;

    use humata2_grpc::humata::v1 as pb;
    use humata2_holder::{EndError, HolderError};
    use humata2_revive::{ContainRefused, Reconciled, ReviveError};
    use humata2_store::StoreError;

    use super::{describe, fix};

    fn io() -> std::io::Error {
        std::io::Error::other("disk")
    }

    /// One error of each kind a start can put against an agent or a shell.
    async fn every_kind() -> Vec<ReviveError> {
        let task = tokio::spawn(std::future::pending::<()>());
        task.abort();
        let source = task.await.unwrap_err();
        vec![
            ReviveError::Store {
                what: "s".to_owned(),
                doing: "read",
                source: StoreError::Read { path: PathBuf::from("x"), source: io() },
            },
            ReviveError::Holder {
                what: "s".to_owned(),
                source: HolderError::Connect { socket: PathBuf::from("x"), source: io() },
            },
            ReviveError::EndHolder {
                what: "s".to_owned(),
                source: EndError::Survived { socket: PathBuf::from("x"), pid: 1 },
            },
            ReviveError::NoCommand { session: "s".to_owned() },
            ReviveError::Injected { point: "p" },
            ReviveError::Background { source },
            ReviveError::Contain {
                session: "s".to_owned(),
                source: ContainRefused::NoSession { session: "s".to_owned() },
            },
            ReviveError::RestartJson { path: PathBuf::from("x"), source: io() },
        ]
    }

    #[tokio::test]
    async fn every_kind_of_error_has_a_fix() {
        for error in every_kind().await {
            assert!(!fix(&error).is_empty(), "{error:?}");
        }
    }

    #[tokio::test]
    async fn an_agent_and_a_shell_that_were_not_brought_back_carry_their_fix() {
        let kinds = every_kind().await;
        let agents: Vec<_> = every_kind().await.into_iter().enumerate().map(|(n, error)| (format!("s{n}"), Err(error))).collect();
        let shells: Vec<_> = every_kind().await.into_iter().enumerate().map(|(n, error)| (format!("t{n}"), Err(error))).collect();
        let mut start = pb::LastStart::default();
        describe(Reconciled { agents, shells, report: Ok(PathBuf::from("r")) }, &mut start);
        assert_eq!(start.not_brought_back.len(), kinds.len() * 2);
        for (line, error) in start.not_brought_back.iter().zip(kinds.iter().chain(kinds.iter())) {
            assert!(line.problem.contains(fix(error)), "{line:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use humata2_grpc::humata::v1 as pb;

    use humata2_sessions::Recovery;

    use super::{
        OperationState, Operations, Revived, Started, note_cut_launches, note_cut_starts, note_ended, note_relaunched, pending_in,
        pretty_name,
    };

    /// A recovery that waits for the start runs once the start is done,
    /// woken by it: paused time never moves, so no timer ran.
    #[tokio::test(start_paused = true)]
    async fn a_wait_for_the_start_ends_when_the_start_does_with_no_timer() {
        let started = Arc::new(Started::default());
        let before = tokio::time::Instant::now();
        let waiter = tokio::spawn({
            let started = started.clone();
            async move { started.wait().await }
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(!waiter.is_finished(), "nothing runs before the start is done");

        started.set(pb::LastStart::default());
        waiter.await.unwrap();
        started.wait().await;
        assert_eq!(tokio::time::Instant::now(), before, "no timer ran");
    }

    const BOUND: std::time::Duration = std::time::Duration::from_secs(60);

    /// A request waiting on a restart that never finishes is answered
    /// Unavailable with a fix once the bound runs out, never before, whichever
    /// kind of request it is.
    #[tokio::test(start_paused = true)]
    async fn a_request_waiting_on_a_restart_that_never_finishes_is_answered_unavailable_with_a_fix_at_the_bound() {
        use super::Awaited::{DeskStart, Read, Resume, Start};
        for doing in [Start, DeskStart, Resume, Read] {
            let started = Arc::new(Started::bounded(BOUND));
            let asked = tokio::spawn({
                let started = started.clone();
                async move { started.wait_for(doing).await }
            });
            tokio::time::sleep(BOUND / 2).await;
            assert!(!asked.is_finished(), "nothing is answered before the bound");
            let refused = tokio::time::timeout(BOUND, asked).await.expect("answered at the bound, not held for ever").unwrap();
            let Err(status) = refused else {
                panic!("the request was refused");
            };
            assert_eq!(status.code(), tonic::Code::Unavailable);
            assert!(crate::status::error_of(&status).is_some_and(|error| !error.fix.is_empty()), "the answer says what to do");
        }
    }

    /// A restart that finishes just before the bound, or already has, lets
    /// the request through, with no timer when it already has.
    #[tokio::test(start_paused = true)]
    async fn a_request_waiting_on_a_restart_that_finishes_in_time_is_let_through() {
        let started = Arc::new(Started::bounded(BOUND));
        let asked = tokio::spawn({
            let started = started.clone();
            async move { started.wait_for(super::Awaited::Start).await }
        });
        tokio::time::sleep(BOUND - std::time::Duration::from_secs(5)).await;
        started.set(pb::LastStart::default());
        assert!(asked.await.unwrap().is_ok());

        let before = tokio::time::Instant::now();
        assert!(started.wait_for(super::Awaited::Resume).await.is_ok());
        assert_eq!(tokio::time::Instant::now(), before, "no timer ran");
    }

    /// Each agent saved as launching is named pending before the report is
    /// written. One whose relaunch failed ends where the person can act on
    /// it (stopped, or resuming), and one the reconcile never looked at is
    /// stuck rather than left pending.
    #[test]
    fn every_launching_agent_is_pending_first_and_ends_as_its_relaunch_left_it() {
        use humata2_revive::ReviveError;
        use humata2_store::AgentState;

        let dir = tempfile::tempdir().unwrap();
        let operations = Operations::default();
        operations.set_folder(dir.path());
        let cut: Vec<String> = ["se_ok", "se_stopped", "se_resuming", "se_unseen"].map(str::to_owned).to_vec();

        note_cut_launches(&operations, &cut);
        let mut pending = pending_in(dir.path());
        pending.sort();
        let mut want: Vec<String> = cut.iter().map(|session| format!("relaunch:{session}")).collect();
        want.sort();
        assert_eq!(pending, want, "every relaunch is pending before any of them ends");

        let failed = || ReviveError::NoCommand { session: "se".to_owned() };
        let agents = vec![
            ("se_ok".to_owned(), Ok(Revived::Relaunched)),
            ("se_stopped".to_owned(), Err(failed())),
            ("se_resuming".to_owned(), Err(failed())),
        ];
        note_relaunched(&operations, &cut, &agents, |session| match session {
            "se_stopped" => Some(AgentState::Stopped),
            "se_resuming" => Some(AgentState::Resuming),
            _ => None,
        });
        let entries = super::operations::read(&dir.path().join("operations.jsonl")).unwrap();
        let state = |session: &str| {
            entries.iter().rev().find(|entry| entry.id == format!("relaunch:{session}")).map(|entry| entry.state.clone())
        };

        assert_eq!(state("se_ok"), Some(OperationState::Finished));
        assert!(matches!(state("se_stopped"), Some(OperationState::Undone { .. })), "a stopped agent was taken back");
        assert!(matches!(state("se_resuming"), Some(OperationState::Stuck { .. })));
        assert!(matches!(state("se_unseen"), Some(OperationState::Stuck { .. })), "an agent never looked at is not left pending");
    }

    #[test]
    fn an_os_release_names_its_distribution() {
        let text = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(pretty_name(text).as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(pretty_name("ID=alpine\n"), None);
    }

    /// Each start the restart will bring back or undo is named pending
    /// before the report is written, and ends as recovery left it; one it
    /// never reported is stuck rather than left pending.
    #[test]
    fn every_cut_start_is_pending_first_and_then_ends_as_recovery_left_it() {
        let dir = tempfile::tempdir().unwrap();
        let operations = Operations::default();
        operations.set_folder(dir.path());
        let cut: Vec<String> = ["se_done", "se_undone", "se_stuck", "se_unseen"].map(str::to_owned).to_vec();

        note_cut_starts(&operations, &cut);
        let mut pending = pending_in(dir.path());
        pending.sort();
        let mut want: Vec<String> = cut.iter().map(|session| format!("start:{session}")).collect();
        want.sort();
        assert_eq!(pending, want, "every start is pending before any of them ends");

        note_ended(
            &operations,
            &cut,
            &[
                Recovery::SessionFinished { session: "se_done".to_owned() },
                Recovery::SessionUndone { session: "se_undone".to_owned(), reason: "its clone failed".to_owned() },
                Recovery::SessionStuck { session: "se_stuck".to_owned(), reason: "the disk is read-only".to_owned() },
                Recovery::SessionStuck { session: String::new(), reason: "the store is damaged".to_owned() },
                Recovery::ChannelFinished { channel: "ch_1".to_owned() },
            ],
        );
        let entries = super::operations::read(&dir.path().join("operations.jsonl")).unwrap();
        let state = |session: &str| entries.iter().find(|entry| entry.id == format!("start:{session}")).map(|entry| entry.state.clone());

        assert_eq!(entries.len(), 4, "a recovery that names no session adds no operation");
        assert_eq!(state("se_done"), Some(OperationState::Finished));
        assert!(matches!(state("se_undone"), Some(OperationState::Undone { reason }) if reason == "its clone failed"));
        assert!(matches!(state("se_stuck"), Some(OperationState::Stuck { reason }) if reason.contains("the disk is read-only")));
        assert!(matches!(state("se_unseen"), Some(OperationState::Stuck { .. })), "a start recovery never reported is not left pending");
    }
}
