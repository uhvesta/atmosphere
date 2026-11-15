//! What the daemon does with every saved session when it starts (PER-5,
//! PER-13, PER-16).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::ffi::OsString;
use std::sync::{Arc, Mutex, PoisonError};

use humata2_holder::{HolderSpec, SavedOutput};
use humata2_store::fault::point;
use humata2_store::{
    AgentRecord, AgentState, Interruption, Restored, ResumeFailure, SessionRecord, Store,
    TabKind, TabRecord,
};

use crate::agent::{
    announced, blocking, probe, relaunch, resume, running, spawn, stop_holder, store_error,
    unix_ms,
};
use crate::report::Restart;
use crate::{Env, ReviveConfig, ReviveError};

/// What became of one session's agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Revived {
    /// Its holder still answers; nothing was lost.
    Reattached,
    /// Its holder still answers, and the agent in it had ended.
    Exited { code: i32 },
    /// It never finished starting, so it was started again from scratch.
    Relaunched,
    /// Resumed with its conversation, which it was seen to take up.
    Resumed { interruption: Interruption },
    /// Couldn't be resumed; nothing runs in its place (PER-15).
    ResumeFailed {
        reason: ResumeFailure,
        exit_code: Option<i32>,
    },
    /// It wasn't running: it had exited, been stopped, or failed before.
    Left,
}

/// What became of one shell tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellRevived {
    Reattached,
    /// A fresh shell in the same folder; what ran in the old one is gone.
    /// `saved` is what's left of the old one's output, shown above.
    Restarted { saved: Option<SavedOutput> },
}

/// Each session's agent and each shell tab, and what became of it. One
/// that failed doesn't stop the rest.
#[derive(Debug)]
pub struct Reconciled {
    /// By session id.
    pub agents: Vec<(String, Result<Revived, ReviveError>)>,
    /// By tab id.
    pub shells: Vec<(String, Result<ShellRevived, ReviveError>)>,
    /// The restart report's folder (PER-29). Failing to write it changes
    /// nothing above.
    pub report: Result<PathBuf, ReviveError>,
}

/// Brings back every saved agent and shell whose holder is gone, and
/// reattaches to those whose holder still answers. Run when the daemon
/// starts, before it serves; running it again changes nothing but the
/// sessions it brings back. Writes a restart report with what it found and
/// did, including what `restored` lost when the store was just restored
/// from its last good copy (PER-29, PER-31).
pub async fn reconcile(
    store: &Arc<Store>,
    config: &ReviveConfig,
    env: Env<'_>,
    restored: Option<&Restored>,
) -> Result<Reconciled, ReviveError> {
    let at_unix_ms = unix_ms();
    let listing = |doing, source| ReviveError::Store {
        what: "this machine's sessions".to_owned(),
        doing,
        source,
    };
    let (agents, tabs, sessions) = blocking({
        let store = store.clone();
        move || (store.agents(), store.tabs(), store.sessions())
    })
    .await?;
    let agents = agents.map_err(|source| listing("list the saved agents", source))?;
    let tabs = tabs.map_err(|source| listing("list the saved tabs", source))?;
    let sessions = sessions.map_err(|source| listing("list the saved sessions", source))?;
    let mut report = Restart::new(config, at_unix_ms, &sessions, restored);
    let folders: HashMap<String, PathBuf> = sessions
        .into_iter()
        .map(|SessionRecord { id, folder, .. }| (id, folder))
        .collect();
    let mut by_id: HashMap<String, TabRecord> =
        tabs.into_iter().map(|tab| (tab.id.clone(), tab)).collect();

    let mut reconciled = Reconciled {
        agents: Vec::new(),
        shells: Vec::new(),
        report: Ok(PathBuf::new()),
    };
    // `env` is asked only for a holder this run starts, since a tab's
    // variables can carry a token that replaces the one its holder was
    // started with: a holder that still answers keeps its own. What it
    // gave out, the report masks.
    let given: Mutex<Vec<(OsString, OsString)>> = Mutex::new(Vec::new());
    let giving = |tab: &TabRecord| {
        let variables = env(tab);
        lock(&given).extend(variables.iter().cloned());
        variables
    };
    let agent_tabs: HashSet<String> = agents.iter().map(|agent| agent.tab.clone()).collect();
    for agent in agents {
        let tab = by_id.get(&agent.tab).cloned();
        let session = agent.session.clone();
        report.agent(config, &agent, tab.as_ref());
        let outcome = revive_agent(store, config, agent, tab, &giving).await;
        report.mask(&std::mem::take(&mut *lock(&given)));
        report.agent_outcome(&outcome);
        reconciled.agents.push((session, outcome));
    }
    let mut shells: Vec<TabRecord> = by_id
        .drain()
        .map(|(_, tab)| tab)
        .filter(|tab| tab.kind == TabKind::Shell && !agent_tabs.contains(&tab.id))
        .collect();
    shells.sort_by_key(|tab| (tab.created_unix_ms, tab.id.clone()));
    for tab in shells {
        let folder = tab.session.as_ref().and_then(|id| folders.get(id)).cloned();
        let id = tab.id.clone();
        report.shell(config, &tab);
        let outcome = revive_shell(store, config, tab, folder, &giving).await;
        report.mask(&std::mem::take(&mut *lock(&given)));
        report.shell_outcome(&outcome);
        reconciled.shells.push((id, outcome));
    }
    let config = config.clone();
    reconciled.report = blocking(move || report.write(&config)).await.and_then(|written| written);
    Ok(reconciled)
}

async fn revive_agent(
    store: &Arc<Store>,
    config: &ReviveConfig,
    mut record: AgentRecord,
    tab: Option<TabRecord>,
    env: Env<'_>,
) -> Result<Revived, ReviveError> {
    if !record.state.should_run() {
        return Ok(Revived::Left);
    }
    let socket = record.launch.holder_socket.clone();
    let created = tab.as_ref().map_or_else(unix_ms, |tab| tab.created_unix_ms);
    let Some(welcome) = probe(&socket).await else {
        // Its holder is gone (PER-24).
        return match record.state {
            AgentState::Launching => launch_or_resume(store, config, record, created, env).await,
            _ => resume(store, config, record, created, env).await,
        };
    };
    match record.state {
        AgentState::Running { holder_pid } => {
            if let Some(code) = welcome.exited {
                record.state = AgentState::Exited { code };
                record.updated_unix_ms = unix_ms();
                crate::agent::save(store, &record, "save that the agent exited").await?;
                return Ok(Revived::Exited { code });
            }
            let pid = humata2_holder::holder_pid(&socket).unwrap_or(holder_pid);
            let saved = tab.as_ref().is_some_and(|tab| tab.holder_pid == pid);
            if pid != holder_pid || !saved {
                running(store, record, humata2_holder::Spawned { pid }, created).await?;
            }
            Ok(Revived::Reattached)
        }
        // Started, but the daemon stopped before it saved that: no one
        // has seen it, so it starts over.
        AgentState::Launching => {
            let _ = humata2_holder::shutdown(&socket).await;
            stop_holder(&socket, None).await;
            launch_or_resume(store, config, record, created, env).await
        }
        // Stopped part way through a resume, before it was checked: the
        // resume is done again, keeping the output so far.
        _ => {
            stop_holder(&socket, None).await;
            resume(store, config, record, created, env).await
        }
    }
}

/// A launch that never finished: resumed if it got as far as starting a
/// conversation, started again if not.
async fn launch_or_resume(
    store: &Arc<Store>,
    config: &ReviveConfig,
    record: AgentRecord,
    created: i64,
    env: Env<'_>,
) -> Result<Revived, ReviveError> {
    if record.launch.conversation_file.is_file() {
        resume(store, config, record, created, env).await
    } else {
        relaunch(store, config, record, created, env).await
    }
}

/// A fresh shell in the tab's folder when its holder is gone (PER-16).
async fn revive_shell(
    store: &Arc<Store>,
    config: &ReviveConfig,
    mut tab: TabRecord,
    folder: Option<PathBuf>,
    env: Env<'_>,
) -> Result<ShellRevived, ReviveError> {
    let what = format!("shell tab {}", tab.id);
    if probe(&tab.holder_socket).await.is_some() {
        // A daemon stopped after it started this holder but before it
        // saved the tab left the old pid: saved now, so the tab names the
        // holder that answers.
        let socket = tab.holder_socket.clone();
        let pid = blocking(move || humata2_holder::holder_pid(&socket)).await?;
        if let Some(pid) = pid.filter(|pid| *pid != tab.holder_pid) {
            tab.holder_pid = pid;
            let saving = tab.clone();
            blocking({
                let store = store.clone();
                move || store.put_tab(&saving)
            })
            .await?
            .map_err(|source| {
                store_error(
                    tab.session.as_deref().unwrap_or("none"),
                    "save the shell tab's holder",
                    source,
                )
            })?;
        }
        return Ok(ShellRevived::Reattached);
    }
    let path = humata2_holder::saved_scrollback_path(&tab.holder_socket);
    let saved = blocking(move || humata2_holder::read_saved(&path))
        .await?
        .map_err(|source| ReviveError::Holder {
            what: what.clone(),
            source,
        })?;
    let cwd = folder
        .filter(|folder| folder.is_dir())
        .unwrap_or_else(|| config.home.clone());
    let line = format!(
        "[Humata] This terminal stopped: its machine restarted or its holder was killed. Programs that were running in it are gone, and output after this point wasn't saved. This is a new shell in {}.",
        cwd.display()
    );
    let (program, args) = announced(Some(&line), config.shell.as_os_str(), &config.shell_args);
    let spec = HolderSpec {
        holder: config.holder.clone(),
        socket: tab.holder_socket.clone(),
        program,
        args,
        cwd,
        env: env(&tab),
        cols: 80,
        rows: 24,
        scrollback_bytes: config.scrollback_bytes,
        log: config.logs.join(format!("{}.log", tab.id)),
    };
    let spawned = spawn(spec, what).await?;
    point("revive.shell.spawned");
    tab.holder_pid = spawned.pid;
    let (saving, socket) = (tab.clone(), tab.holder_socket.clone());
    let put = blocking({
        let store = store.clone();
        move || store.put_tab(&saving)
    })
    .await?;
    if let Err(source) = put {
        let _ = humata2_holder::shutdown(&socket).await;
        return Err(store_error(
            tab.session.as_deref().unwrap_or("none"),
            "save the restarted shell's tab",
            source,
        ));
    }
    Ok(ShellRevived::Restarted { saved })
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
