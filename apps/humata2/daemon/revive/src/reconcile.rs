//! What the daemon does with every saved session when it starts (PER-5,
//! PER-13, PER-16).

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use humata2_holder::{HolderSpec, SavedOutput};
use humata2_store::fault::point;
use humata2_store::{
    AgentRecord, AgentState, Interruption, Restored, ResumeFailure, SessionRecord, Store, TabKind,
    TabRecord,
};

use crate::agent::{
    Why, blocking, fresh_replies, probe, relaunch, resume, running, spawn, stop_holder,
    store_error, unix_ms,
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
    /// It never finished starting, or it was moved before it wrote any
    /// conversation, so it was started again from scratch.
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
    Restarted {
        saved: Option<SavedOutput>,
    },
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
    reconcile_then(store, config, env, restored, Box::new(|_| {})).await
}

/// [`reconcile`], with `before_json` run on the report's folder after the
/// report's other files are written and before its `restart.json`. A
/// reader takes `restart.json` as the sign the report is whole, so what
/// the caller writes beside it must be there first.
pub async fn reconcile_then(
    store: &Arc<Store>,
    config: &ReviveConfig,
    env: Env<'_>,
    restored: Option<&Restored>,
    before_json: Box<dyn FnOnce(&Path) + Send>,
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
        let outcome = revive_agent(store, config, agent, tab, &giving, Why::Interrupted).await;
        report.mask(&std::mem::take(&mut *lock(&given)));
        report.agent_outcome(&outcome);
        reconciled.agents.push((session, outcome));
    }
    let mut shells: Vec<TabRecord> = by_id
        .drain()
        .map(|(_, tab)| tab)
        .filter(|tab| tab.kind == TabKind::Shell && !agent_tabs.contains(&tab.id))
        // D784: a shell the user stopped stays stopped until Resume.
        .filter(|tab| !humata2_holder::stopped_mark_path(&tab.holder_socket).exists())
        .collect();
    shells.sort_by_key(|tab| (tab.created_unix_ms, tab.id.clone()));
    for tab in shells {
        let folder = tab.session.as_ref().and_then(|id| folders.get(id)).cloned();
        let id = tab.id.clone();
        report.shell(config, &tab);
        let outcome = revive_shell(store, config, tab, folder, &giving, None).await;
        report.mask(&std::mem::take(&mut *lock(&given)));
        report.shell_outcome(&outcome);
        reconciled.shells.push((id, outcome));
    }
    let config = config.clone();
    reconciled.report = blocking(move || report.write(&config, before_json))
        .await
        .and_then(|written| written);
    Ok(reconciled)
}

/// Brings back one agent the way [`reconcile`] does when the daemon is
/// already running: for an agent whose record says it should run and whose
/// holder is gone, it is resumed with its pinned conversation and the
/// resume is checked; one whose holder still answers while it is saved as
/// launching or resuming is stopped and resumed again. An agent that
/// shouldn't run is left as it is ([`Revived::Left`]). Unlike `reconcile`,
/// no restart report is written and no other session is touched.
/// `Revived::ResumeFailed` says the resume was tried and why it failed.
pub async fn resume_agent(
    store: &Arc<Store>,
    config: &ReviveConfig,
    record: AgentRecord,
    env: Env<'_>,
) -> Result<Revived, ReviveError> {
    revive_listed(store, config, record, env, Why::Interrupted).await
}

/// Resumes an agent the person stopped, or that exited, because they asked
/// to (D671, Provisional): as [`resume_agent`], but no interruption is
/// recorded and the agent isn't told it was interrupted. Its resume is
/// checked only for coming up, staying up and starting no other
/// conversation, so an agent that writes nothing until prompted resumes.
pub async fn resume_agent_asked(
    store: &Arc<Store>,
    config: &ReviveConfig,
    record: AgentRecord,
    env: Env<'_>,
) -> Result<Revived, ReviveError> {
    revive_listed(store, config, record, env, Why::Asked).await
}

/// Starts `record`'s agent again with its conversation, as [`resume_agent`]
/// does, for a change the person asked for rather than an interruption: no
/// interruption is recorded and the agent isn't told it was interrupted.
/// `line` is shown first in its terminal, after `[Humata] `. A failure is
/// saved as [`Revived::ResumeFailed`], as a resume's is, and so is a
/// conversation file that is gone: it can't be resumed (D757).
pub async fn restart_agent(
    store: &Arc<Store>,
    config: &ReviveConfig,
    record: AgentRecord,
    env: Env<'_>,
    line: &str,
) -> Result<Revived, ReviveError> {
    revive_listed(store, config, record, env, Why::Restarted(line)).await
}

/// [`restart_agent`] after a move that carried every file the agent wrote
/// and checked them here: a conversation file missing after it is one the
/// agent never wrote, so the agent is started fresh with its first command
/// ([`Revived::Relaunched`]) rather than failed (D757).
pub async fn resume_moved_agent(
    store: &Arc<Store>,
    config: &ReviveConfig,
    record: AgentRecord,
    env: Env<'_>,
    line: &str,
) -> Result<Revived, ReviveError> {
    revive_listed(store, config, record, env, Why::Moved(line)).await
}

/// Starts each of `session`'s shell tabs whose holder is gone again, as
/// [`reconcile`] does, as a new shell in the session's folder with the
/// output its old holder saved; one whose holder answers is left. `line`
/// is shown after that output, after `[Humata] `. Run when a session's
/// agent is resumed, so the shell tabs a move brought in come back with it
/// (MV-13). One that fails doesn't stop the rest.
pub async fn restart_shells(
    store: &Arc<Store>,
    config: &ReviveConfig,
    session: &str,
    env: Env<'_>,
    line: &str,
) -> Result<Vec<(String, Result<ShellRevived, ReviveError>)>, ReviveError> {
    let (tabs, record) = blocking({
        let (store, id) = (store.clone(), session.to_owned());
        move || (store.tabs(), store.session(&id))
    })
    .await?;
    let tabs = tabs.map_err(|source| store_error(session, "list the session's tabs", source))?;
    let folder = record
        .map_err(|source| store_error(session, "read the session", source))?
        .map(|record| record.folder);
    let agent_tab = blocking({
        let (store, id) = (store.clone(), session.to_owned());
        move || store.agent(&id)
    })
    .await?
    .map_err(|source| store_error(session, "read the session's agent", source))?
    .map(|agent| agent.tab);
    let mut shells: Vec<TabRecord> = tabs
        .into_iter()
        .filter(|tab| {
            tab.kind == TabKind::Shell
                && tab.session.as_deref() == Some(session)
                && agent_tab.as_deref() != Some(tab.id.as_str())
        })
        .collect();
    shells.sort_by_key(|tab| (tab.created_unix_ms, tab.id.clone()));
    let mut restarted = Vec::new();
    for tab in shells {
        let id = tab.id.clone();
        let outcome = revive_shell(store, config, tab, folder.clone(), env, Some(line)).await;
        restarted.push((id, outcome));
    }
    Ok(restarted)
}

/// Starts one shell tab again as [`restart_shells`] does for a session's:
/// a new shell in `folder` (its session's, if it has one) with the output
/// its old holder saved shown above `line`; a tab whose holder answers is
/// reattached. For the restart doctor (PER-30), which repairs a tab the
/// last start didn't bring back. No restart report is written.
pub async fn restart_shell(
    store: &Arc<Store>,
    config: &ReviveConfig,
    tab: TabRecord,
    folder: Option<PathBuf>,
    env: Env<'_>,
    line: &str,
) -> Result<ShellRevived, ReviveError> {
    revive_shell(store, config, tab, folder, env, Some(line)).await
}

/// Looks up the agent's tab, then brings the agent back for `why`.
async fn revive_listed(
    store: &Arc<Store>,
    config: &ReviveConfig,
    record: AgentRecord,
    env: Env<'_>,
    why: Why<'_>,
) -> Result<Revived, ReviveError> {
    let tab = {
        let (store, id) = (store.clone(), record.tab.clone());
        blocking(move || store.tab(&id)).await?
    }
    .map_err(|source| store_error(&record.session, "read the agent's tab", source))?;
    revive_agent(store, config, record, tab, env, why).await
}

async fn revive_agent(
    store: &Arc<Store>,
    config: &ReviveConfig,
    mut record: AgentRecord,
    tab: Option<TabRecord>,
    env: Env<'_>,
    why: Why<'_>,
) -> Result<Revived, ReviveError> {
    if !record.state.should_run() {
        return Ok(Revived::Left);
    }
    // A launching record being brought back is a launch in flight: a stop
    // that ends its old holder must wait for this to be done.
    let _in_flight = (record.state == AgentState::Launching)
        .then(|| crate::agent::LaunchInFlight::enter(&record.session));
    let socket = record.launch.holder_socket.clone();
    let created = tab.as_ref().map_or_else(unix_ms, |tab| tab.created_unix_ms);
    let Some(welcome) = probe(&socket).await else {
        // Its holder is gone (PER-24).
        return match record.state {
            AgentState::Launching => {
                launch_or_resume(store, config, record, created, env, why).await
            }
            _ => resume(store, config, record, created, env, why).await,
        };
    };
    // D801 I: this branch undoes a start Humata never finished, and that is
    // all it may do. A holder moving onto a new build (a live `.handoff`
    // beside its socket) is a running terminal, so a stale record never
    // ends it: it is left for the next reconcile.
    if matches!(record.state, AgentState::Launching | AgentState::Resuming) && is_moving(&socket) {
        return Ok(Revived::Left);
    }
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
            // Shutting down deletes the saved output. A session that
            // started fresh keeps it: it is the old conversation's terminal.
            let session = record.session.clone();
            let started_fresh = blocking({
                let store = store.clone();
                move || store.previous_conversations(&session)
            })
            .await?
            .map(|previous| !previous.is_empty())
            // Unreadable: keep the output rather than lose it.
            .unwrap_or(true);
            if !started_fresh {
                // Found by its pid file, or by the socket it serves if that
                // file is gone, so a holder that won't shut down is ended.
                match humata2_holder::find_holder(&socket).await {
                    Ok(Some(pid)) => {
                        let ended = humata2_holder::shutdown_or_end(&socket, pid)
                            .await
                            .map(|_| ())
                            .map_err(|source| ReviveError::EndHolder {
                                what: format!("session {}'s agent", record.session),
                                source,
                            });
                        settle_failed_end(store, &record, welcome.exited.is_some(), ended).await?;
                    }
                    found => {
                        if let Err(error) = found {
                            eprintln!("humata2 revive: {error}");
                        }
                        // No holder to find: a socket nobody answers on is
                        // how a holder that already stopped looks.
                        if let Err(error) = humata2_holder::shutdown(&socket).await
                            && !matches!(error, humata2_holder::HolderError::Connect { .. })
                        {
                            eprintln!("humata2 revive: {error}");
                        }
                    }
                }
            }
            stop_holder(&socket, None).await;
            launch_or_resume(store, config, record, created, env, why).await
        }
        // Stopped part way through a resume, before it was checked: the
        // resume is done again, keeping the output so far.
        _ => {
            stop_holder(&socket, None).await;
            resume(store, config, record, created, env, why).await
        }
    }
}

/// Whether the holder serving `socket` is in the middle of moving onto a
/// new build: its `.handoff` marker is there and the holder's process is
/// alive.
fn is_moving(socket: &Path) -> bool {
    if !humata2_holder::handoff_path(socket).exists() {
        return false;
    }
    humata2_holder::holder_pid(socket).is_some_and(pid_alive)
}

fn pid_alive(pid: u32) -> bool {
    let Ok(raw) = i32::try_from(pid) else {
        return false;
    };
    let Some(pid) = rustix::process::Pid::from_raw(raw) else {
        return false;
    };
    matches!(
        rustix::process::test_kill_process(pid),
        Ok(()) | Err(rustix::io::Errno::PERM)
    )
}

#[cfg(test)]
mod moving_tests {
    use std::fs;

    use super::is_moving;

    #[test]
    fn a_marker_beside_a_live_holder_is_moving() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("t.sock");
        fs::write(humata2_holder::handoff_path(&socket), b"H2HANDOFF").unwrap();
        // This test's own process stands in for the live holder.
        fs::write(socket.with_extension("pid"), std::process::id().to_string()).unwrap();
        assert!(is_moving(&socket));
    }

    #[test]
    fn no_marker_is_not_moving() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("t.sock");
        fs::write(socket.with_extension("pid"), std::process::id().to_string()).unwrap();
        assert!(!is_moving(&socket));
    }

    #[test]
    fn a_marker_with_no_live_holder_is_not_moving() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("t.sock");
        fs::write(humata2_holder::handoff_path(&socket), b"H2HANDOFF").unwrap();
        assert!(!is_moving(&socket));
    }
}

/// What a failed end of an old holder leaves of the agent saved as
/// launching. `agent_exited` is what the holder's welcome said: the agent in
/// it had already ended, so none can be running and the agent is saved as
/// stopped, where resume and start fresh take it. If the agent may still be
/// running in that holder, it stays launching: stopped would let a resume
/// start a second agent beside it, and the error (which says how to end the
/// holder) is the way out. Provisional: recommend keeping this; a stop that
/// ends a launching agent's holder by its pid would let the person do it from
/// the app. The save is best effort: if it fails too, the next restart tries
/// again.
async fn settle_failed_end(
    store: &Arc<Store>,
    record: &AgentRecord,
    agent_exited: bool,
    ended: Result<(), ReviveError>,
) -> Result<(), ReviveError> {
    let Err(error) = ended else {
        return Ok(());
    };
    if agent_exited {
        let mut stopped = record.clone();
        stopped.state = AgentState::Stopped;
        stopped.updated_unix_ms = unix_ms();
        let _ = crate::agent::save(store, &stopped, "save that the agent didn't start again").await;
    }
    Err(error)
}

#[cfg(test)]
mod end_failed_tests {
    //! A holder that couldn't be ended leaves the agent somewhere the person
    //! can act on, unless the agent might still be running.

    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use humata2_holder::EndError;
    use humata2_store::{AgentLaunch, AgentRecord, AgentState, SessionRecord, Store};

    use super::settle_failed_end;
    use crate::ReviveError;

    fn failed_end() -> Result<(), ReviveError> {
        Err(ReviveError::EndHolder {
            what: "session se_a's agent".to_owned(),
            source: EndError::Survived { socket: PathBuf::from("/nowhere/h.sock"), pid: 1 },
        })
    }

    fn saved_launching(root: &Path) -> (Arc<Store>, AgentRecord) {
        std::fs::create_dir_all(root.join("state")).unwrap();
        let store = Arc::new(Store::open(root.join("state")).unwrap());
        store
            .put_session(&SessionRecord {
                id: "se_a".to_owned(),
                channel: "ch_1".to_owned(),
                machine: "ma_1".to_owned(),
                title: "Fix the build".to_owned(),
                agent: "fake".to_owned(),
                folder: root.to_owned(),
                created_unix_ms: 1,
            })
            .unwrap();
        let record = AgentRecord {
            session: "se_a".to_owned(),
            tab: "tb_a".to_owned(),
            launch: AgentLaunch {
                agent: "fake".to_owned(),
                folder: root.to_owned(),
                argv: vec![OsString::from("/bin/sh")],
                resume_argv: vec![OsString::from("/bin/sh")],
                variable_names: Vec::new(),
                conversation_id: "c1".to_owned(),
                conversation_file: root.join("c1.jsonl"),
                cli_version: None,
                holder_socket: root.join("h.sock"),
                cols: 80,
                rows: 24,
            },
            state: AgentState::Launching,
            generation: 1,
            interruptions: Vec::new(),
            updated_unix_ms: 1,
        };
        store.put_agent(&record).unwrap();
        (store, record)
    }

    #[tokio::test]
    async fn an_old_holder_that_cannot_be_ended_with_its_agent_already_gone_leaves_the_agent_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let (store, record) = saved_launching(dir.path());

        let outcome = settle_failed_end(&store, &record, true, failed_end()).await;

        assert!(matches!(outcome, Err(ReviveError::EndHolder { .. })), "{outcome:?}");
        assert_eq!(store.agent("se_a").unwrap().expect("still saved").state, AgentState::Stopped);
    }

    #[tokio::test]
    async fn an_old_holder_that_cannot_be_ended_with_its_agent_maybe_running_keeps_the_agent_launching() {
        let dir = tempfile::tempdir().unwrap();
        let (store, record) = saved_launching(dir.path());

        let outcome = settle_failed_end(&store, &record, false, failed_end()).await;

        assert!(matches!(outcome, Err(ReviveError::EndHolder { .. })), "{outcome:?}");
        // Stopped here would let a resume run a second agent beside the one
        // still running in the old holder.
        assert_eq!(store.agent("se_a").unwrap().expect("still saved").state, AgentState::Launching);
    }

    #[tokio::test]
    async fn an_old_holder_that_ended_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (store, record) = saved_launching(dir.path());

        settle_failed_end(&store, &record, true, Ok(())).await.expect("nothing failed");

        assert_eq!(store.agent("se_a").unwrap().expect("still saved").state, AgentState::Launching);
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
    why: Why<'_>,
) -> Result<Revived, ReviveError> {
    if record.launch.conversation_file.is_file() {
        resume(store, config, record, created, env, why).await
    } else {
        relaunch(store, config, record, created, env).await
    }
}

/// A fresh shell in the tab's folder when its holder is gone (PER-16).
/// `why`, if given, is the line shown after `[Humata] ` in place of the
/// one that says the machine restarted.
async fn revive_shell(
    store: &Arc<Store>,
    config: &ReviveConfig,
    mut tab: TabRecord,
    folder: Option<PathBuf>,
    env: Env<'_>,
    why: Option<&str>,
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
            // Another process than the one saved: a new holder.
            tab.holder_generation = tab.holder_generation.saturating_add(1);
            let saving = tab.clone();
            let fresh = fresh_replies(saving.holder_generation);
            blocking({
                let store = store.clone();
                move || store.put_tab_with_replies(&saving, &fresh)
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
    let reported = {
        let (store, id) = (store.clone(), tab.id.clone());
        match blocking(move || store.tab_program(&id)).await? {
            Ok(program) => Some(program.pwd),
            Err(error) => {
                eprintln!(
                    "humata2 revive: couldn't read where tab {}'s program was, so its new shell starts in the session folder: {error}",
                    tab.id
                );
                None
            }
        }
    };
    let cwd = crate::shell_start::start_folder(reported.as_deref(), folder, &config.home);
    let line = match why {
        Some(why) => format!("[Humata] {why} This is a new shell in {}.", cwd.display()),
        None => format!(
            "[Humata] This terminal stopped: its machine restarted or its holder was killed. Programs that were running in it are gone, and output after this point wasn't saved. This is a new shell in {}.",
            cwd.display()
        ),
    };
    let spec = HolderSpec {
        holder: config.holder.clone(),
        socket: tab.holder_socket.clone(),
        program: config.shell.clone(),
        args: config.shell_args.clone(),
        cwd,
        env: env(&tab),
        cols: crate::START_COLS,
        rows: crate::START_ROWS,
        scrollback_bytes: config.scrollback_bytes,
        log: config.logs.join(format!("{}.log", tab.id)),
    };
    if let Some(builds) = &config.builds {
        let (builds, tab) = (builds.clone(), tab.id.clone());
        let _ = blocking(move || builds.shell_spawned(&tab)).await;
    }
    let spawned = spawn(spec, Some(&line), what).await?;
    point("revive.shell.spawned");
    // D784: started again, so no longer stopped. Gone already is fine.
    let _ = std::fs::remove_file(humata2_holder::stopped_mark_path(&tab.holder_socket));
    tab.holder_pid = spawned.pid;
    tab.holder_generation = tab.holder_generation.saturating_add(1);
    let (saving, socket) = (tab.clone(), tab.holder_socket.clone());
    let fresh = fresh_replies(saving.holder_generation);
    let put = blocking({
        let store = store.clone();
        move || store.put_tab_with_replies(&saving, &fresh)
    })
    .await;
    let session = tab.session.clone().unwrap_or_else(|| "none".to_owned());
    let saved_tab = match put {
        Ok(Ok(())) => Ok(()),
        Ok(Err(source)) => Err(store_error(&session, "save the restarted shell's tab", source)),
        Err(error) => Err(error),
    };
    if let Err(error) = saved_tab {
        // Nothing could find the new holder again: it is ended, and if it
        // stays that is said, beside the error that is returned.
        if let Err(end) = humata2_holder::shutdown_or_end(&socket, spawned.pid).await {
            eprintln!("humata2 revive: {end}");
        }
        return Err(error);
    }
    Ok(ShellRevived::Restarted { saved })
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
