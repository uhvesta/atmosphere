//! D784: every running holder of ours on this machine, listed so the user
//! can account for it, attach it and end it. "Ours" is a holder build
//! serving a socket right in this daemon's own holders folder: a holder of
//! another state folder (another daemon, the terminal lab) is never listed
//! or touched. A holder no session names is an orphan, listed as "Not in a
//! channel", never hidden; one with no tab is given a tab as it is listed,
//! so every entry attaches by its tab. Nothing here ends a holder unless
//! the user asked to with [`end`].

mod adopt;
mod let_go;

pub(crate) use adopt::adopt;
pub(crate) use let_go::let_go;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_grpc::humata::v1 as pb;
use humata2_holder::process::{Snapshot, holder_names};
use humata2_sessions::DESK_CHANNEL;
use humata2_store::{AgentRecord, AgentState, SessionRecord, Store, StoreError, TabKind, TabRecord};
use tonic::Status;

use crate::status;

/// Why the running holders couldn't be listed or one couldn't be ended.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RunningError {
    #[error("couldn't list this machine's processes to find its holders: {source}")]
    Processes {
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't {doing}")]
    Store {
        doing: &'static str,
        #[source]
        source: StoreError,
    },
    #[error("no running holder of this machine has tab {tab}")]
    NoTab { tab: String },
    #[error("couldn't end the holder of tab {tab}")]
    End {
        tab: String,
        #[source]
        source: crate::holder_end::HolderEndError,
    },
    #[error("tab {tab} is session {session}'s agent, not a shell")]
    AgentTab {
        tab: String,
        session: String,
        /// What to use for it instead.
        instead: &'static str,
    },
    #[error("couldn't stop the holder of tab {tab}")]
    Stop {
        tab: String,
        #[source]
        source: humata2_holder::EndError,
    },
    #[error("couldn't mark tab {tab} stopped at {}", path.display())]
    Mark {
        tab: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't start tab {tab}'s shell again")]
    Resume {
        tab: String,
        #[source]
        source: Status,
    },
    #[error("a background task failed while {doing}")]
    Task { doing: &'static str },
    #[error("tab {tab} already belongs to session {session}, so it isn't a holder no session names")]
    AlreadyInSession { tab: String, session: String },
    #[error("the holder of tab {tab} isn't running on this machine")]
    NotRunning { tab: String },
    #[error("channel {channel} isn't one this machine knows")]
    NoChannel { channel: String },
    #[error("this machine has no {agent} agent to adopt the holder as")]
    NoAgent {
        agent: String,
        #[source]
        source: humata2_sessions::AgentError,
    },
    #[error("this machine has no plain shell agent to adopt the holder as")]
    Shell {
        #[source]
        source: humata2_sessions::AgentError,
    },
}

impl RunningError {
    pub(crate) fn into_status(self, machine: &str) -> Status {
        match self {
            Self::Store { doing, source } => status::store(source, machine, doing),
            Self::NoTab { ref tab } => status::not_found(
                pb::EntityKind::Tab,
                tab,
                status::chain(&self),
                "list the machine's running holders again: this one may have ended already",
            ),
            Self::Processes { .. } => status::internal(
                format!("{}; check that `ps` runs for this user", status::chain(&self)),
                "",
            ),
            Self::End { .. } => status::internal(
                format!(
                    "{}; end it again, and if it stays, check with `ps` that it is a humata2 holder before ending it by hand",
                    status::chain(&self)
                ),
                "",
            ),
            Self::AgentTab { instead, .. } => crate::channel_service::failed(
                tonic::Code::FailedPrecondition,
                &self,
                instead,
                None,
            ),
            Self::Stop { .. } => status::internal(
                format!(
                    "{}; stop it again, and if it stays, check with `ps` that it is a humata2 holder before ending it by hand",
                    status::chain(&self)
                ),
                "",
            ),
            Self::Mark { .. } => status::internal(
                format!(
                    "{}; check that the daemon's holders folder is writable, then stop it again. Its holder wasn't touched",
                    status::chain(&self)
                ),
                "",
            ),
            Self::Resume { source, .. } => source,
            Self::Task { .. } => status::internal(status::chain(&self), ""),
            Self::AlreadyInSession { .. } => crate::channel_service::failed(
                tonic::Code::FailedPrecondition,
                &self,
                "list the machine's running holders again: this one already has a session",
                None,
            ),
            Self::NotRunning { .. } => crate::channel_service::failed(
                tonic::Code::FailedPrecondition,
                &self,
                "list the machine's running holders again: it has ended, so there is nothing to adopt",
                None,
            ),
            Self::NoChannel { .. } => crate::channel_service::failed(
                tonic::Code::NotFound,
                &self,
                "pick a channel from the channel list, or leave it empty to adopt into the Desk",
                None,
            ),
            Self::NoAgent { ref agent, .. } => status::internal(
                format!(
                    "{}; add the {agent} agent to Settings > Agents, then adopt it again",
                    status::chain(&self)
                ),
                "",
            ),
            Self::Shell { .. } => status::internal(
                format!(
                    "{}; add the plain shell (zsh) to Settings > Agents, then adopt it again",
                    status::chain(&self)
                ),
                "",
            ),
        }
    }
}

/// A holder process as `ps` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    pid: u32,
    socket: PathBuf,
    folder: String,
    agent: &'static str,
    /// How long it has run.
    elapsed: Duration,
}

/// The holders this machine runs right now, with sockets in `holders`.
async fn seen(holders: &Path) -> Result<Vec<Seen>, RunningError> {
    let output = tokio::process::Command::new("ps")
        .args(["-A", "-ww", "-o", "pid=,etime=,args="])
        .output()
        .await
        .map_err(|source| RunningError::Processes { source })?;
    let snapshot = Snapshot::take()
        .await
        .map_err(|source| RunningError::Processes { source })?;
    let names = holder_names(Path::new(""));
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| parse_line(line, holders))
        .filter(|seen| snapshot.is_holder(seen.pid, &names))
        .collect())
}

/// One `ps` line, `pid etime args`, as a holder serving a socket right in
/// `holders`; `None` for any other process.
fn parse_line(line: &str, holders: &Path) -> Option<Seen> {
    let mut fields = line.split_whitespace();
    let pid = fields.next()?.parse().ok()?;
    let elapsed = parse_etime(fields.next()?)?;
    let (_, args) = line.split_once(" --socket ")?;
    let (socket, rest) = args.split_once(" --")?;
    let socket = PathBuf::from(socket);
    if socket.parent() != Some(holders) {
        return None;
    }
    let folder = rest
        .strip_prefix("cwd ")
        .or_else(|| rest.split_once(" --cwd ").map(|(_, after)| after))
        .and_then(|after| after.split(" --").next())
        .unwrap_or_default()
        .to_owned();
    let program = rest.split_once(" -- ").map(|(_, program)| program).unwrap_or("");
    Some(Seen {
        pid,
        socket,
        folder,
        agent: agent_of(program),
        elapsed,
    })
}

/// `ps`'s elapsed time, `[[dd-]hh:]mm:ss`.
fn parse_etime(text: &str) -> Option<Duration> {
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, text),
    };
    let mut seconds = 0u64;
    for part in clock.split(':') {
        seconds = seconds.checked_mul(60)?.checked_add(part.parse().ok()?)?;
    }
    Some(Duration::from_secs(days * 86_400 + seconds))
}

/// The agent kind a holder's program line names; "shell" for anything else.
fn agent_of(program: &str) -> &'static str {
    let first = program.split_whitespace().take(3).collect::<Vec<_>>();
    for word in first {
        let name = Path::new(word)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        for kind in ["copilot", "claude", "codex"] {
            if name == kind || name.starts_with(&format!("{kind}-")) {
                return kind;
            }
        }
    }
    "shell"
}

/// The kind of agent a session's own tab runs.
fn session_agent(record: &SessionRecord) -> &'static str {
    let agent = record.agent.to_ascii_lowercase();
    ["copilot", "claude", "codex"]
        .into_iter()
        .find(|kind| agent.contains(kind))
        .unwrap_or("shell")
}

/// Every running holder of ours, sessions' first (by title), then orphans
/// (oldest first). An orphan with no tab is given one.
pub(crate) async fn list(
    store: &Arc<Store>,
    holders: &Path,
    machine: &str,
) -> Result<Vec<pb::RunningHolder>, RunningError> {
    let seen = seen(holders).await?;
    let store = store.clone();
    let machine = machine.to_owned();
    tokio::task::spawn_blocking(move || listed(&store, seen, &machine, SystemTime::now()))
        .await
        .map_err(|_| RunningError::Task {
            doing: "listing the holders",
        })?
}

fn listed(
    store: &Store,
    seen: Vec<Seen>,
    machine: &str,
    now: SystemTime,
) -> Result<Vec<pb::RunningHolder>, RunningError> {
    let read = |doing| move |source| RunningError::Store { doing, source };
    let mut by_socket: HashMap<PathBuf, TabRecord> = store
        .tabs()
        .map_err(read("list the tabs"))?
        .into_iter()
        .map(|tab| (tab.holder_socket.clone(), tab))
        .collect();
    // A resume saves its agent's tab only once the agent took up its
    // conversation (a restore's agent has no tab until then), so a holder
    // still being checked is found by the agent's own socket. It is that
    // session's, and gets no tab here: the resume saves it, or ends the
    // holder when it fails.
    let agents: HashMap<PathBuf, AgentRecord> = store
        .agents()
        .map_err(read("list the agents"))?
        .into_iter()
        .map(|agent| (agent.launch.holder_socket.clone(), agent))
        .collect();
    let mut sessions = Vec::new();
    let mut orphans = Vec::new();
    for holder in seen {
        let tab = match by_socket.remove(&holder.socket) {
            Some(tab) => tab,
            None => match agents.get(&holder.socket) {
                Some(agent) => TabRecord {
                    id: agent.tab.clone(),
                    session: Some(agent.session.clone()),
                    kind: TabKind::Agent,
                    title: agent.launch.agent.clone(),
                    holder_socket: holder.socket.clone(),
                    holder_pid: holder.pid,
                    created_unix_ms: unix_ms(now.checked_sub(holder.elapsed).unwrap_or(UNIX_EPOCH)),
                    holder_generation: 0,
                },
                None => {
                    let tab = orphan_tab(store, &holder, now)?;
                    store.put_tab(&tab).map_err(read("save a tab for a holder no session names"))?;
                    tab
                }
            },
        };
        let record = match tab.session.as_deref() {
            Some(id) => store.session(id).map_err(read("read a holder's session"))?,
            None => None,
        };
        let started = now.checked_sub(holder.elapsed).unwrap_or(UNIX_EPOCH);
        // A session's agent tab, a plain shell's included, ends through
        // its session: the running-holder calls refuse it (`shell_tab`).
        let agent_tab = record.is_some()
            && (tab.kind == TabKind::Agent || agents.get(&holder.socket).is_some_and(|agent| agent.tab == tab.id));
        let entry = pb::RunningHolder {
            tab: Some(pb::TabId { value: tab.id.clone() }),
            session: record.as_ref().map(|record| pb::SessionId { value: record.id.clone() }),
            title: record.as_ref().map(|record| record.title.clone()).unwrap_or_default(),
            channel: record
                .as_ref()
                .filter(|record| record.channel != DESK_CHANNEL)
                .map(|record| pb::ChannelId { value: record.channel.clone() }),
            desk: record.as_ref().is_some_and(|record| record.channel == DESK_CHANNEL),
            agent: match (&record, tab.kind) {
                (Some(record), TabKind::Agent) => session_agent(record),
                (Some(_), _) => "shell",
                (None, _) => holder.agent,
            }
            .to_owned(),
            pid: holder.pid,
            folder: holder.folder.clone(),
            started: Some(crate::grant_target::timestamp(started)),
            attached: record
                .as_ref()
                .is_some_and(|record| crate::read_marks::is_viewed(&record.id)),
            machine: Some(pb::MachineId { value: machine.to_owned() }),
            stopped: false,
            agent_tab,
        };
        match record {
            Some(record) => sessions.push((record.title, entry)),
            None => orphans.push((started, entry)),
        }
    }
    let running: HashSet<String> = sessions
        .iter()
        .filter(|(_, entry)| entry.agent_tab)
        .filter_map(|(_, entry)| entry.session.as_ref().map(|session| session.value.clone()))
        .collect();
    let mut stopped = stopped_sessions(store, machine, &running)?;
    // A shell the user stopped keeps its tab: it is listed stopped, by its
    // tab, until it is resumed or ended (D784).
    let mut shells: Vec<TabRecord> = by_socket
        .into_values()
        .filter(|tab| tab.kind == TabKind::Shell)
        .filter(|tab| humata2_holder::stopped_mark_path(&tab.holder_socket).exists())
        .collect();
    shells.sort_by(|a, b| (a.created_unix_ms, &a.id).cmp(&(b.created_unix_ms, &b.id)));
    for tab in shells {
        let record = match tab.session.as_deref() {
            Some(id) => store.session(id).map_err(read("read a stopped shell's session"))?,
            None => None,
        };
        let folder = store.tab_program(&tab.id).map(|program| program.pwd).unwrap_or_default();
        stopped.push(pb::RunningHolder {
            tab: Some(pb::TabId { value: tab.id.clone() }),
            session: record.as_ref().map(|record| pb::SessionId { value: record.id.clone() }),
            title: record.as_ref().map(|record| record.title.clone()).unwrap_or_default(),
            channel: record
                .as_ref()
                .filter(|record| record.channel != DESK_CHANNEL)
                .map(|record| pb::ChannelId { value: record.channel.clone() }),
            desk: record.as_ref().is_some_and(|record| record.channel == DESK_CHANNEL),
            agent: "shell".to_owned(),
            pid: 0,
            folder,
            started: None,
            attached: false,
            machine: Some(pb::MachineId { value: machine.to_owned() }),
            stopped: true,
            agent_tab: false,
        });
    }
    sessions.sort_by(|a, b| a.0.cmp(&b.0));
    orphans.sort_by_key(|(started, _)| *started);
    Ok(sessions
        .into_iter()
        .map(|(_, entry)| entry)
        .chain(stopped)
        .chain(orphans.into_iter().map(|(_, entry)| entry))
        .collect())
}

/// The sessions whose agent the user stopped, or whose resume failed, and
/// that aren't archived, by title: nothing runs for them, and each stays
/// listed so it is accounted for until it is resumed, started fresh or
/// archived (D784 "Stop"). One whose agent's own holder is listed already
/// isn't listed twice.
fn stopped_sessions(
    store: &Store,
    machine: &str,
    listed: &HashSet<String>,
) -> Result<Vec<pb::RunningHolder>, RunningError> {
    let read = |doing| move |source| RunningError::Store { doing, source };
    let mut rows = Vec::new();
    for agent in store.agents().map_err(read("list the agents"))? {
        if !matches!(agent.state, AgentState::Stopped | AgentState::ResumeFailed { .. })
            || listed.contains(&agent.session)
            || store.archive(&agent.session).map_err(read("read an archive"))?.is_some()
        {
            continue;
        }
        let Some(record) = store.session(&agent.session).map_err(read("read a stopped session"))? else {
            continue;
        };
        rows.push(pb::RunningHolder {
            tab: None,
            session: Some(pb::SessionId { value: record.id.clone() }),
            channel: (record.channel != DESK_CHANNEL).then(|| pb::ChannelId { value: record.channel.clone() }),
            desk: record.channel == DESK_CHANNEL,
            agent: session_agent(&record).to_owned(),
            pid: 0,
            folder: agent.launch.folder.display().to_string(),
            started: None,
            attached: false,
            machine: Some(pb::MachineId { value: machine.to_owned() }),
            stopped: true,
            agent_tab: true,
            title: record.title,
        });
    }
    rows.sort_by(|a, b| a.title.cmp(&b.title));
    Ok(rows)
}

/// A tab for a holder no tab names: its socket's own name when that is a
/// tab id no tab has, so the holder's terminal keeps its id.
fn orphan_tab(store: &Store, holder: &Seen, now: SystemTime) -> Result<TabRecord, RunningError> {
    let stem = holder
        .socket
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| stem.starts_with("tb_"))
        .map(str::to_owned);
    let id = match stem {
        Some(stem)
            if store
                .tab(&stem)
                .map_err(|source| RunningError::Store {
                    doing: "look up a tab",
                    source,
                })?
                .is_none() =>
        {
            stem
        }
        _ => format!("tb_{}", uuid::Uuid::new_v4().simple()),
    };
    let created = now.checked_sub(holder.elapsed).unwrap_or(UNIX_EPOCH);
    Ok(TabRecord {
        id,
        session: None,
        kind: TabKind::Shell,
        title: holder.agent.to_owned(),
        holder_socket: holder.socket.clone(),
        holder_pid: holder.pid,
        created_unix_ms: unix_ms(created),
        holder_generation: 0,
    })
}

/// The shell tab `tab`, or why it can't be stopped, resumed or ended here:
/// no such tab, or an agent's, which its session's calls handle.
async fn shell_tab(store: &Arc<Store>, tab: &str, instead: &'static str) -> Result<TabRecord, RunningError> {
    let (store, key) = (store.clone(), tab.to_owned());
    let found = tokio::task::spawn_blocking(move || {
        let tab = store.tab(&key)?;
        let agent = match tab.as_ref().and_then(|tab| tab.session.clone()) {
            Some(session) => store.agent(&session)?,
            None => None,
        };
        Ok::<_, StoreError>((tab, agent))
    })
    .await
    .map_err(|_| RunningError::Task {
        doing: "looking up the tab",
    })?
    .map_err(|source| RunningError::Store {
        doing: "look up the tab",
        source,
    })?;
    match found {
        (None, _) => Err(RunningError::NoTab { tab: tab.to_owned() }),
        (Some(record), agent)
            if record.kind == TabKind::Agent || agent.as_ref().is_some_and(|agent| agent.tab == record.id) =>
        {
            Err(RunningError::AgentTab {
                tab: record.id,
                session: record.session.unwrap_or_default(),
                instead,
            })
        }
        (Some(record), _) => Ok(record),
    }
}

/// "Stop" for a shell tab (D784): its holder and the program's whole
/// process group end, as a reboot would end them, and the tab is kept with
/// what the holder saved and a stopped mark, so a start leaves it stopped
/// and only [`resume`] starts it again. The mark is made first: a start
/// that ran between the two would otherwise bring it back.
pub(crate) async fn stop(store: &Arc<Store>, tab: &str) -> Result<(), RunningError> {
    let record = shell_tab(
        store,
        tab,
        "stop the session instead (`humata sessions stop`); its agent is stopped with it",
    )
    .await?;
    let mark = humata2_holder::stopped_mark_path(&record.holder_socket);
    std::fs::write(&mark, b"").map_err(|source| RunningError::Mark {
        tab: record.id.clone(),
        path: mark.clone(),
        source,
    })?;
    let stopping = |source| RunningError::Stop {
        tab: record.id.clone(),
        source,
    };
    // Ended by its pid, not shut down: a holder told to shut down drops
    // what it saved, which Resume shows above the new shell.
    if let Some(pid) = humata2_holder::find_holder(&record.holder_socket)
        .await
        .map_err(stopping)?
    {
        humata2_holder::end_by_pid(&record.holder_socket, pid, STOP_GRACE)
            .await
            .map_err(stopping)?;
    }
    Ok(())
}

/// How long a stopped holder, then its programs, get to end on their own.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// "Resume" for a stopped shell tab (D784): a new shell in the folder it
/// was in, with what its holder saved shown above, as after a reboot; a
/// shell whose holder answers is left. Starts it through the restart
/// doctor's repair, which also takes the stopped mark away.
pub(crate) async fn resume(
    store: &Arc<Store>,
    sessions: &crate::session_service::SessionSvc,
    caller: &crate::identity::Caller,
    tab: &str,
) -> Result<(), RunningError> {
    let record = shell_tab(
        store,
        tab,
        "resume the session instead (`humata sessions resume`); its agent comes back in its conversation",
    )
    .await?;
    let started = sessions
        .repair_tab(caller, record.id.clone(), true)
        .await
        .map_err(|source| RunningError::Resume {
            tab: record.id.clone(),
            source,
        })?;
    match started {
        crate::channel_service::Gated::Done(()) => Ok(()),
        crate::channel_service::Gated::Ask(_) => Err(RunningError::Resume {
            tab: record.id,
            source: Status::permission_denied(
                "starting this shell again needs the user's approval; resume it from the app",
            ),
        }),
    }
}

/// "End for good" for a shell tab (D784): its holder and the program's
/// whole process group end, and the tab, its saved output and any stopped
/// mark are gone. A stopped shell has no holder left to end.
pub(crate) async fn end(store: &Arc<Store>, tab: &str) -> Result<(), RunningError> {
    let record = shell_tab(
        store,
        tab,
        "archive the session to end it for good (`humata sessions end`); its history stays readable",
    )
    .await?;
    let owner = match &record.session {
        Some(session) => format!("session {session}'s terminal {}", record.id),
        None => format!("terminal {}", record.id),
    };
    crate::holder_end::end_holder_at(&record.holder_socket, &owner)
        .await
        .map_err(|source| RunningError::End {
            tab: record.id.clone(),
            source,
        })?;
    // Gone already is fine: a holder shut down takes its saved output.
    let _ = std::fs::remove_file(humata2_holder::stopped_mark_path(&record.holder_socket));
    let _ = std::fs::remove_file(humata2_holder::saved_scrollback_path(&record.holder_socket));
    let (store, id) = (store.clone(), record.id);
    tokio::task::spawn_blocking(move || store.remove_tab(&id))
        .await
        .map_err(|_| RunningError::Task {
            doing: "removing the ended tab",
        })?
        .map_err(|source| RunningError::Store {
            doing: "remove the ended tab",
            source,
        })
}

fn unix_ms(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_store::ResumeFailure;

    fn session_with_agent(store: &Store, id: &str, title: &str, state: AgentState) {
        store
            .put_session(&SessionRecord {
                id: id.to_owned(),
                channel: DESK_CHANNEL.to_owned(),
                machine: "ma_1".to_owned(),
                title: title.to_owned(),
                agent: "copilot".to_owned(),
                folder: PathBuf::from("/w/desk").join(title),
                created_unix_ms: 1,
            })
            .unwrap();
        store
            .put_agent(&AgentRecord {
                session: id.to_owned(),
                tab: format!("tb_{id}"),
                launch: humata2_store::AgentLaunch {
                    agent: "copilot".to_owned(),
                    folder: PathBuf::from("/w/desk").join(title),
                    ..Default::default()
                },
                state,
                generation: 1,
                interruptions: Vec::new(),
                updated_unix_ms: 1,
            })
            .unwrap();
    }

    /// D784: a session whose resume failed has nothing running, but it is
    /// still listed with the stopped ones, so it is accounted for; one with
    /// a holder listed already isn't listed twice.
    #[test]
    fn a_session_whose_resume_failed_is_listed_with_the_stopped_ones() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let failed = AgentState::ResumeFailed {
            reason: ResumeFailure::ConversationFileMissing,
            exit_code: None,
        };
        session_with_agent(&store, "se_failed", "copilot1", failed);
        session_with_agent(&store, "se_stopped", "codex1", AgentState::Stopped);
        session_with_agent(&store, "se_running", "claude1", AgentState::Running { holder_pid: 7 });
        session_with_agent(&store, "se_listed", "zed1", AgentState::Stopped);
        let listed = HashSet::from(["se_listed".to_owned()]);

        let rows = stopped_sessions(&store, "ma_1", &listed).unwrap();
        let sessions: Vec<_> = rows
            .iter()
            .map(|row| (row.session.as_ref().map(|id| id.value.as_str()), row.stopped))
            .collect();
        assert_eq!(sessions, vec![(Some("se_stopped"), true), (Some("se_failed"), true)]);
    }

    #[test]
    fn a_holder_in_this_folder_is_read_with_its_folder_and_agent() {
        let line = "  4242     01:02:03 /opt/h/humata2-holder --socket /s/holders/tb_1.sock --cwd /w/fix --cols 80 -- /usr/local/bin/copilot --resume";
        let seen = parse_line(line, Path::new("/s/holders")).unwrap();
        assert_eq!(seen.pid, 4242);
        assert_eq!(seen.socket, PathBuf::from("/s/holders/tb_1.sock"));
        assert_eq!(seen.folder, "/w/fix");
        assert_eq!(seen.agent, "copilot");
        assert_eq!(seen.elapsed, Duration::from_secs(3723));
    }

    #[test]
    fn a_holder_of_another_folder_is_not_ours() {
        let line = "1 00:05 /opt/h/humata2-holder --socket /other/holders/tb_1.sock --cwd /w -- /bin/zsh";
        assert_eq!(parse_line(line, Path::new("/s/holders")), None);
        let nested = "1 00:05 /opt/h/humata2-holder --socket /s/holders/lab/tb_1.sock --cwd /w -- /bin/zsh";
        assert_eq!(parse_line(nested, Path::new("/s/holders")), None);
    }

    #[test]
    fn a_program_that_is_no_agent_is_a_shell() {
        assert_eq!(agent_of("/bin/zsh -l"), "shell");
        assert_eq!(agent_of("node /x/claude --x"), "claude");
        assert_eq!(agent_of("codex resume"), "codex");
    }

    #[test]
    fn elapsed_times_read_with_and_without_days() {
        assert_eq!(parse_etime("00:07"), Some(Duration::from_secs(7)));
        assert_eq!(parse_etime("2-00:00:01"), Some(Duration::from_secs(2 * 86_400 + 1)));
        assert_eq!(parse_etime("x"), None);
    }
}
