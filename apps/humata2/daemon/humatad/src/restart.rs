//! What the daemon does with the sessions it finds when it starts: brings
//! back each agent and shell whose holder is gone (humata2_revive), writes
//! the restart report, and keeps what wasn't brought back for the app to
//! show (PER-25–PER-29, PER-31).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_revive::{Reconciled, RestartReports, ReviveConfig, Revived};
use humata2_sessions::{Agents, Layout, Leftovers, Recovery, Starter, recover};
use humata2_store::{Restored, ResumeFailure, Store, StoreError, TabKind, TabRecord};

use crate::identity::{Identities, TabIdentity};
use crate::status;

mod operations;

pub(crate) use operations::{Operations, State as OperationState};

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
    pub(crate) logs: PathBuf,
    pub(crate) scrollback_bytes: usize,
    pub(crate) shell: PathBuf,
    pub(crate) shell_args: Vec<OsString>,
    pub(crate) home: PathBuf,
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
    pub(crate) channels: PathBuf,
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
        layout: Layout {
            root: sessions.channels,
        },
        remote_template: sessions.remote_template,
        agents: sessions.agents,
        home: revive.home.clone(),
        revive: ReviveConfig {
            contain: revive.contain,
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
    tokio::spawn(async move {
        let start = bring_back(
            &store,
            &identities,
            &starter,
            restored.as_ref(),
            left,
            cut_rebuilds,
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

/// Brings back what the last run left, and says what it couldn't. Never
/// fails the start: a reconcile that can't run is itself the one line.
#[allow(clippy::too_many_arguments)]
async fn bring_back(
    store: &Arc<Store>,
    identities: &Arc<Identities>,
    starter: &Starter,
    restored: Option<&Restored>,
    left: Result<Leftovers, StoreError>,
    cut_rebuilds: Vec<crate::session_service::CutRebuild>,
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
    match humata2_revive::reconcile(store, &starter.revive, &env, restored).await {
        Ok(reconciled) => describe(reconciled, &mut start, operations),
        Err(error) => {
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
    crate::session_service::undo_cut_rebuilds(starter, cut_rebuilds).await;
    match left {
        Ok(left) => {
            for recovery in recover(starter, &env, &left).await {
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
        ResumeFailure::FolderMissing => {
            "its folder is gone, so nothing was started in it; restore the folder, then resume the session"
        }
    };
    match exit_code {
        Some(code) => format!("the session wasn't resumed: {why} (the agent exited with {code})"),
        None => format!("the session wasn't resumed: {why}"),
    }
}

fn describe(reconciled: Reconciled, start: &mut pb::LastStart, operations: &Operations) {
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
                problem: status::chain(&error),
            });
        }
    }
    for (tab, outcome) in reconciled.shells {
        if let Err(error) = outcome {
            start.not_brought_back.push(pb::NotBroughtBack {
                session: String::new(),
                tab,
                problem: status::chain(&error),
            });
        }
    }
    match reconciled.report {
        Ok(folder) => {
            operations.set_folder(&folder);
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

    pub(crate) fn env(&self, tab: &TabRecord) -> Vec<(OsString, OsString)> {
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
mod tests {
    use std::sync::Arc;

    use humata2_grpc::humata::v1 as pb;

    use super::{Started, pretty_name};

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

    #[test]
    fn an_os_release_names_its_distribution() {
        let text = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(pretty_name(text).as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(pretty_name("ID=alpine\n"), None);
    }
}
