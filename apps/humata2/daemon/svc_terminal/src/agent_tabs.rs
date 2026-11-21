//! D858 items 1–2: daemon-kept extra terminal tab metadata (`label` + `agent`)
//! and launching a fresh extra agent tab directly under its own holder instead
//! of typing a command into a shell.
//!
//! Extra tabs (plain shells and added agents) are stored in `tabs` with
//! `TabKind::Shell` so they sit after tab 0, stay closable via `CloseTab` /
//! `End for good`, and never hijack session-wide `typing_tab`. The tab's
//! user-facing label lives in `TabRecord::title` (persisted in SQLite), and
//! an added agent's kind lives under `tab_agent:<tab_id>` in the same SQLite
//! store so both survive daemon restarts, power cuts, and app restarts.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::pb;
use humata2_holder::{HolderError, HolderSpec};
use humata2_rpc_kit::{chain, internal, invalid_field};
use humata2_sessions::{AgentError, Agents};
use humata2_store::{Store, StoreError, TabKind, TabRecord, TabReplies};
use pb::invalid_field::Problem;
use tonic::Status;

const TAB_AGENT_PREFIX: &str = "tab_agent:";

/// D671: pause between typing the opening prompt and pressing return.
const PROMPT_SUBMIT_PAUSE: Duration = Duration::from_millis(400);

/// How long an opening prompt waits for the agent to read keys and show the
/// text before giving up quietly (D857, D858).
const PROMPT_DELIVER_TIMEOUT: Duration = Duration::from_secs(30);

/// Why resolving, saving, or spawning an extra agent tab failed.
#[derive(Debug, thiserror::Error)]
pub enum AgentTabError {
    #[error("agent is missing: pick an installed agent such as claude, codex, or copilot")]
    MissingAgent,
    #[error(
        "agent {agent} is a plain shell, not an agent; use OpenShellTab to open a shell tab"
    )]
    IsShell { agent: String },
    #[error("agent {agent} is not configured on this machine: pick an agent configured in Settings")]
    UnknownAgent {
        agent: String,
        #[source]
        source: AgentError,
    },
    #[error("agent {agent} has an empty launch command; check its configuration in Settings")]
    EmptyCommand { agent: String },
    #[error("couldn't {doing} in the store")]
    Store {
        doing: &'static str,
        #[source]
        source: StoreError,
    },
    #[error("couldn't start the agent tab's holder (see log at {log})")]
    Spawn {
        log: PathBuf,
        #[source]
        source: HolderError,
    },
}

/// Maps an [`AgentTabError`] to a structured gRPC [`Status`] with actionable
/// fix guidance.
#[must_use]
pub fn to_status(error: &AgentTabError) -> Status {
    match error {
        AgentTabError::MissingAgent => invalid_field(
            "agent",
            Problem::Missing,
            error.to_string(),
            "set agent to a configured agent such as claude, codex, or copilot, then try again",
        ),
        AgentTabError::IsShell { .. } => invalid_field(
            "agent",
            Problem::NotAllowedHere,
            error.to_string(),
            "call OpenShellTab for a plain shell, or pick an agent such as claude, codex, or copilot",
        ),
        AgentTabError::UnknownAgent { .. } => invalid_field(
            "agent",
            Problem::UnknownValue,
            chain(error),
            "pick one of the configured agents, or add this agent in Settings",
        ),
        AgentTabError::EmptyCommand { .. } => invalid_field(
            "agent",
            Problem::BadFormat,
            error.to_string(),
            "check the agent's program and arguments in Settings, then try again",
        ),
        AgentTabError::Store { .. } => internal(chain(error), ""),
        AgentTabError::Spawn { log, .. } => internal(chain(error), &log.display().to_string()),
    }
}

/// One extra terminal tab of a session (shell or added agent), as kept on the
/// daemon so its label and agent survive daemon and app restarts (D858).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellTabEntry {
    pub tab: String,
    pub label: String,
    /// `None` for a plain shell tab; `Some(agent_name)` for an agent tab
    /// opened through `TerminalService.OpenAgentTab`.
    pub agent: Option<String>,
}

/// Parameters for launching a fresh extra agent tab under its own holder.
#[derive(Clone, Debug)]
pub struct AgentTabLaunch {
    pub holder: PathBuf,
    pub holder_socket: PathBuf,
    pub log: PathBuf,
    pub home: PathBuf,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub cols: u16,
    pub rows: u16,
    pub scrollback_bytes: usize,
}

fn tab_agent_key(tab: &str) -> String {
    format!("{TAB_AGENT_PREFIX}{tab}")
}

/// Persists `agent` as the agent kind running in extra tab `tab`.
pub fn set_tab_agent(store: &Store, tab: &str, agent: &str) -> Result<(), AgentTabError> {
    store
        .put_setting(&tab_agent_key(tab), agent, 0)
        .map_err(|source| AgentTabError::Store {
            doing: "save the tab's agent kind",
            source,
        })
}

/// Reads the persisted agent kind of extra tab `tab`. Returns `Ok(None)` for a
/// plain shell tab (or when no agent was recorded).
pub fn tab_agent(store: &Store, tab: &str) -> Result<Option<String>, AgentTabError> {
    let saved = store
        .setting(&tab_agent_key(tab))
        .map_err(|source| AgentTabError::Store {
            doing: "read the tab's agent kind",
            source,
        })?;
    Ok(saved
        .map(|entry| entry.value)
        .filter(|value| !value.is_empty()))
}

/// Removes the persisted agent kind for `tab` when the tab is closed or ended.
pub fn clear_tab_agent(store: &Store, tab: &str) -> Result<(), AgentTabError> {
    store
        .delete_setting(&tab_agent_key(tab))
        .map_err(|source| AgentTabError::Store {
            doing: "remove the tab's agent kind",
            source,
        })
}

/// Lists every extra tab (`TabKind::Shell`) of `session`, oldest first, with
/// each tab's persisted label (`TabRecord::title`) and agent kind.
pub fn shell_tabs(store: &Store, session: &str) -> Result<Vec<ShellTabEntry>, AgentTabError> {
    let tabs = store.tabs().map_err(|source| AgentTabError::Store {
        doing: "list the session's tabs",
        source,
    })?;
    let mut entries = Vec::new();
    for tab in tabs {
        if tab.kind != TabKind::Shell || tab.session.as_deref() != Some(session) {
            continue;
        }
        let agent = tab_agent(store, &tab.id)?;
        entries.push(ShellTabEntry {
            tab: tab.id,
            label: tab.title,
            agent,
        });
    }
    Ok(entries)
}

/// Lists every extra tab (`TabKind::Shell`) grouped by session id, oldest first
/// within each session, in a single pass over `Store::tabs`.
pub fn shell_tabs_by_session(
    store: &Store,
) -> Result<HashMap<String, Vec<ShellTabEntry>>, AgentTabError> {
    let tabs = store.tabs().map_err(|source| AgentTabError::Store {
        doing: "list the sessions' tabs",
        source,
    })?;
    let mut by_session: HashMap<String, Vec<ShellTabEntry>> = HashMap::new();
    for tab in tabs {
        if tab.kind != TabKind::Shell {
            continue;
        }
        let Some(session) = tab.session else {
            continue;
        };
        let agent = tab_agent(store, &tab.id)?;
        by_session.entry(session).or_default().push(ShellTabEntry {
            tab: tab.id,
            label: tab.title,
            agent,
        });
    }
    Ok(by_session)
}

/// The 1-based index of extra tab `tab_id` in `session`'s tab strip (tab 0 is
/// the session's primary agent tab; extra tabs are 1..=N in creation order).
/// Returns `Ok(None)` if `tab_id` is not an extra tab of `session`.
pub fn tab_index(
    store: &Store,
    session: &str,
    tab_id: &str,
) -> Result<Option<u32>, AgentTabError> {
    let entries = shell_tabs(store, session)?;
    Ok(entries
        .iter()
        .position(|entry| entry.tab == tab_id)
        .and_then(|pos| u32::try_from(pos + 1).ok()))
}

/// Resolves `agent` in `agents` (rejecting empty, unknown, or plain-shell
/// kinds) and returns `(canonical_name, program, args)` to launch a fresh
/// conversation in `cwd`.
pub fn resolve_launch(
    agents: &Agents,
    agent: &str,
    home: &Path,
    cwd: &Path,
) -> Result<(String, PathBuf, Vec<OsString>), AgentTabError> {
    let agent = agent.trim();
    if agent.is_empty() {
        return Err(AgentTabError::MissingAgent);
    }
    let kind = agents
        .get(agent)
        .map_err(|source| AgentTabError::UnknownAgent {
            agent: agent.to_owned(),
            source,
        })?;
    if kind.is_shell() {
        return Err(AgentTabError::IsShell {
            agent: kind.name.clone(),
        });
    }
    let conversation = uuid::Uuid::new_v4().to_string();
    let plan = kind.launch(home, cwd, &conversation);
    let mut argv = plan.argv.into_iter();
    let Some(program) = argv.next() else {
        return Err(AgentTabError::EmptyCommand {
            agent: kind.name.clone(),
        });
    };
    Ok((kind.name.clone(), PathBuf::from(program), argv.collect()))
}

/// Builds a [`HolderSpec`] that runs `agent` directly in `launch.cwd` under a
/// fresh conversation, returning `(canonical_agent_name, HolderSpec)`.
pub fn build_holder_spec(
    agents: &Agents,
    agent: &str,
    launch: &AgentTabLaunch,
) -> Result<(String, HolderSpec), AgentTabError> {
    let (name, program, args) = resolve_launch(agents, agent, &launch.home, &launch.cwd)?;
    let spec = HolderSpec {
        holder: launch.holder.clone(),
        socket: launch.holder_socket.clone(),
        program,
        args,
        cwd: launch.cwd.clone(),
        env: launch.env.clone(),
        cols: launch.cols,
        rows: launch.rows,
        scrollback_bytes: launch.scrollback_bytes,
        log: launch.log.clone(),
    };
    Ok((name, spec))
}

/// Saves a newly spawned extra agent tab (`TabRecord` + initial `TabReplies` +
/// persisted `tab_agent` entry) and returns its 1-based index in the session's
/// tab strip.
pub fn save_agent_tab(
    store: &Store,
    tab: &TabRecord,
    agent: &str,
) -> Result<u32, AgentTabError> {
    let first = TabReplies {
        generation: tab.holder_generation,
        ..TabReplies::default()
    };
    store
        .put_tab_with_replies(tab, &first)
        .map_err(|source| AgentTabError::Store {
            doing: "save the new agent tab",
            source,
        })?;
    if let Err(error) = set_tab_agent(store, &tab.id, agent) {
        let _ = store.remove_tab(&tab.id);
        return Err(error);
    }
    let Some(session) = tab.session.as_deref() else {
        return Ok(1);
    };
    Ok(tab_index(store, session, &tab.id)?.unwrap_or(1))
}

/// Delivers an optional opening prompt to the newly spawned agent tab in the
/// background once the agent reads keys and shows the text (D857, D858). Does
/// nothing when `prompt` is empty.
pub fn deliver_prompt(socket: PathBuf, prompt: String) {
    if prompt.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let _ = humata2_holder::deliver(
            &socket,
            prompt.as_bytes(),
            PROMPT_SUBMIT_PAUSE,
            None,
            None,
            PROMPT_DELIVER_TIMEOUT,
        )
        .await;
    });
}

#[cfg(test)]
mod tests {
    use humata2_rpc_kit::status_error_of;
    use humata2_store::SessionRecord;
    use tonic::Code;

    use super::*;

    fn seed_session(store: &Store, id: &str, title: &str) {
        store
            .put_session(&SessionRecord {
                id: id.to_owned(),
                channel: "ch_main".to_owned(),
                machine: "ma_host".to_owned(),
                title: title.to_owned(),
                agent: "claude".to_owned(),
                folder: PathBuf::from("/tmp/sessions").join(id),
                created_unix_ms: 100,
            })
            .unwrap();
    }

    fn tab_record(
        id: &str,
        session: Option<&str>,
        kind: TabKind,
        title: &str,
        created_unix_ms: i64,
    ) -> TabRecord {
        TabRecord {
            id: id.to_owned(),
            session: session.map(str::to_owned),
            kind,
            title: title.to_owned(),
            holder_socket: PathBuf::from("/tmp/holders").join(format!("{id}.sock")),
            holder_pid: 0,
            created_unix_ms,
            holder_generation: 0,
        }
    }

    #[test]
    fn tab_agent_and_label_survive_store_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = Store::open(dir.path()).unwrap();
            seed_session(&store, "se_1", "Review PR");
            store
                .put_tab(&tab_record("tb_primary", Some("se_1"), TabKind::Agent, "claude", 1))
                .unwrap();
            store
                .put_tab(&tab_record("tb_sh", Some("se_1"), TabKind::Shell, "zsh", 2))
                .unwrap();
            let agent_tab = tab_record("tb_rev", Some("se_1"), TabKind::Shell, "codex", 3);
            let index = save_agent_tab(&store, &agent_tab, "codex").unwrap();
            assert_eq!(index, 2);
        }

        // Simulate daemon restart by reopening the store from disk.
        let reopened = Store::open(dir.path()).unwrap();
        assert_eq!(tab_agent(&reopened, "tb_sh").unwrap(), None);
        assert_eq!(tab_agent(&reopened, "tb_rev").unwrap(), Some("codex".to_owned()));
        assert_eq!(
            shell_tabs(&reopened, "se_1").unwrap(),
            vec![
                ShellTabEntry {
                    tab: "tb_sh".to_owned(),
                    label: "zsh".to_owned(),
                    agent: None,
                },
                ShellTabEntry {
                    tab: "tb_rev".to_owned(),
                    label: "codex".to_owned(),
                    agent: Some("codex".to_owned()),
                },
            ]
        );
        assert_eq!(tab_index(&reopened, "se_1", "tb_sh").unwrap(), Some(1));
        assert_eq!(tab_index(&reopened, "se_1", "tb_rev").unwrap(), Some(2));
        assert_eq!(tab_index(&reopened, "se_1", "tb_primary").unwrap(), None);
    }

    #[test]
    fn clearing_tab_agent_removes_persisted_kind() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        seed_session(&store, "se_1", "Work");
        let tab = tab_record("tb_agent2", Some("se_1"), TabKind::Shell, "copilot", 10);
        save_agent_tab(&store, &tab, "copilot").unwrap();
        assert_eq!(tab_agent(&store, "tb_agent2").unwrap(), Some("copilot".to_owned()));

        clear_tab_agent(&store, "tb_agent2").unwrap();
        assert_eq!(tab_agent(&store, "tb_agent2").unwrap(), None);
    }

    #[test]
    fn shell_tabs_by_session_groups_extra_tabs_per_session_excluding_primary_and_machine_shells() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        seed_session(&store, "se_a", "Alpha");
        seed_session(&store, "se_b", "Beta");
        store
            .put_tab(&tab_record("tb_a0", Some("se_a"), TabKind::Agent, "claude", 1))
            .unwrap();
        store
            .put_tab(&tab_record("tb_machine", None, TabKind::Shell, "zsh", 2))
            .unwrap();
        let a1 = tab_record("tb_a1", Some("se_a"), TabKind::Shell, "claude", 3);
        let a2 = tab_record("tb_a2", Some("se_a"), TabKind::Shell, "claude", 4);
        let b1 = tab_record("tb_b1", Some("se_b"), TabKind::Shell, "zsh", 5);
        assert_eq!(save_agent_tab(&store, &a1, "claude").unwrap(), 1);
        assert_eq!(save_agent_tab(&store, &a2, "claude").unwrap(), 2);
        store.put_tab(&b1).unwrap();

        let grouped = shell_tabs_by_session(&store).unwrap();
        assert_eq!(grouped.len(), 2);
        assert_eq!(
            grouped["se_a"],
            vec![
                ShellTabEntry {
                    tab: "tb_a1".to_owned(),
                    label: "claude".to_owned(),
                    agent: Some("claude".to_owned()),
                },
                ShellTabEntry {
                    tab: "tb_a2".to_owned(),
                    label: "claude".to_owned(),
                    agent: Some("claude".to_owned()),
                },
            ]
        );
        assert_eq!(
            grouped["se_b"],
            vec![ShellTabEntry {
                tab: "tb_b1".to_owned(),
                label: "zsh".to_owned(),
                agent: None,
            }]
        );
    }

    #[test]
    fn resolve_launch_rejects_missing_shell_and_unknown_agents() {
        let agents = Agents::builtin().with_shell(Path::new("/bin/zsh"), &[]);
        let home = Path::new("/tmp/home");
        let cwd = Path::new("/tmp/session");

        assert!(matches!(
            resolve_launch(&agents, "   ", home, cwd),
            Err(AgentTabError::MissingAgent)
        ));
        assert!(matches!(
            resolve_launch(&agents, "zsh", home, cwd),
            Err(AgentTabError::IsShell { .. })
        ));
        assert!(matches!(
            resolve_launch(&agents, "nonexistent-agent", home, cwd),
            Err(AgentTabError::UnknownAgent { .. })
        ));

        for err in [
            AgentTabError::MissingAgent,
            AgentTabError::IsShell {
                agent: "zsh".to_owned(),
            },
        ] {
            let status = to_status(&err);
            assert_eq!(status.code(), Code::InvalidArgument);
            let detail = status_error_of(&status).expect("structured error detail");
            assert!(!detail.fix.is_empty());
        }
    }

    #[test]
    fn resolve_launch_and_build_holder_spec_start_fresh_agent_directly() {
        let agents = Agents::builtin();
        let launch = AgentTabLaunch {
            holder: PathBuf::from("/usr/local/bin/humata2_holder"),
            holder_socket: PathBuf::from("/tmp/holders/tab.sock"),
            log: PathBuf::from("/tmp/logs/tab.log"),
            home: PathBuf::from("/tmp/home"),
            cwd: PathBuf::from("/tmp/session-folder"),
            env: vec![(OsString::from("HUMATA_SESSION_TOKEN"), OsString::from("tok"))],
            cols: 80,
            rows: 24,
            scrollback_bytes: 1024 * 1024,
        };

        let (name_1, spec_1) = build_holder_spec(&agents, "claude", &launch).unwrap();
        let (name_2, spec_2) = build_holder_spec(&agents, "claude", &launch).unwrap();

        assert_eq!(name_1, "claude");
        assert_eq!(name_2, "claude");
        assert_eq!(spec_1.cwd, PathBuf::from("/tmp/session-folder"));
        assert!(!spec_1.program.as_os_str().is_empty());
        // Each open gets its own fresh conversation id (no shared history).
        assert_ne!(spec_1.args, spec_2.args);
    }
}
