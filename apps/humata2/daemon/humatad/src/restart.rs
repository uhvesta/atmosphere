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
use humata2_revive::{Reconciled, RestartReports, ReviveConfig};
use humata2_store::{Restored, Store, TabRecord};

use crate::identity::Identities;
use crate::status;

/// How long a resumed agent has to show it took up its conversation.
const RESUME_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a resumed agent's output stays quiet before it's told it was
/// interrupted.
const SETTLE: Duration = Duration::from_secs(1);

/// Where and how the sessions are brought back.
pub(crate) struct Revive {
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
}

/// Brings back what the last run left while the daemon serves: each agent
/// may take [`RESUME_TIMEOUT`] to resume, longer than a bridge waits for
/// the daemon to listen. The lock is set once it's done.
pub(crate) fn in_background(
    store: Arc<Store>,
    identities: Arc<Identities>,
    revive: Revive,
    restored: Option<Restored>,
) -> Arc<OnceLock<pb::LastStart>> {
    let done = Arc::new(OnceLock::new());
    let setting = done.clone();
    tokio::spawn(async move {
        let start = bring_back(&store, &identities, revive, restored.as_ref()).await;
        let _ = setting.set(start);
    });
    done
}

/// Brings back what the last run left, and says what it couldn't. Never
/// fails the start: a reconcile that can't run is itself the one line.
async fn bring_back(
    store: &Arc<Store>,
    identities: &Arc<Identities>,
    revive: Revive,
    restored: Option<&Restored>,
) -> pb::LastStart {
    let config = ReviveConfig {
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
        },
    };
    let tokens = Tokens::new(store.clone(), identities.clone());
    let env = |tab: &TabRecord| tokens.env(tab);
    let mut start = pb::LastStart::default();
    if let Some(restored) = restored {
        start.restored_copy_taken_unix_ms = restored.copy_taken_unix_ms;
        start.restored_rows_changed = u32::try_from(restored.changes.len()).unwrap_or(u32::MAX);
    }
    match humata2_revive::reconcile(store, &config, &env, restored).await {
        Ok(reconciled) => describe(reconciled, &mut start),
        Err(error) => {
            eprintln!("humatad2: couldn't bring back this machine's sessions: {}", status::chain(&error));
            start.not_brought_back.push(pb::NotBroughtBack {
                session: String::new(),
                tab: String::new(),
                problem: status::chain(&error),
            });
        }
    }
    start
}

fn describe(reconciled: Reconciled, start: &mut pb::LastStart) {
    for (session, outcome) in reconciled.agents {
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
        Ok(folder) => start.report = folder.display().to_string(),
        Err(error) => eprintln!("humatad2: couldn't write the restart report: {}", status::chain(&error)),
    }
}

/// A fresh session token for each session tab brought back, the same one
/// each time reviving asks for that tab, since the report masks it too.
struct Tokens {
    store: Arc<Store>,
    identities: Arc<Identities>,
    minted: Mutex<HashMap<String, Vec<(OsString, OsString)>>>,
}

impl Tokens {
    fn new(store: Arc<Store>, identities: Arc<Identities>) -> Self {
        Self {
            store,
            identities,
            minted: Mutex::new(HashMap::new()),
        }
    }

    fn env(&self, tab: &TabRecord) -> Vec<(OsString, OsString)> {
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
                eprintln!(
                    "humatad2: couldn't read session {session} to give tab {} a token, so its agent can't reach Humata until the tab is restarted: {}",
                    tab.id,
                    status::chain(&error)
                );
                return Vec::new();
            }
        };
        // The old holder's token went with it.
        if let Err(error) = self.identities.revoke(&tab.id) {
            eprintln!("humatad2: couldn't drop tab {}'s old token: {}", tab.id, status::chain(&error));
        }
        let env = match self.identities.mint(&tab.id, &record) {
            Ok(token) => vec![(OsString::from(humata2_mcp::TOKEN_VAR), OsString::from(token))],
            Err(error) => {
                eprintln!(
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

/// This machine's host name, as `uname -n` gives it.
fn host_name() -> Option<String> {
    let output = std::process::Command::new("uname").arg("-n").output().ok()?;
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
    use super::pretty_name;

    #[test]
    fn an_os_release_names_its_distribution() {
        let text = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(pretty_name(text).as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(pretty_name("ID=alpine\n"), None);
    }
}
