//! Contain agents (SEC-6, D217): whether a session's agent runs contained,
//! from its channel's override or else Settings, and the sandbox it runs in.
//! Every launch and resume asks here, through revive's `Contain` hook, so a
//! change applies when a session next launches or resumes. A session that
//! must be contained and can't be stays stopped: it never runs uncontained
//! instead.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use humata2_contain::{
    Agent, Containment, Login, TOKEN_VARIABLES, choose_login, flow_back, hide_from_git, home_rules, parse_version,
    seed_claude_copy,
};
use humata2_revive::{Contain, ContainRefused};
use humata2_sessions::Layout;
use humata2_store::{AgentRecord, Store, StoreError};

use crate::guard::Guard;

/// The store key of Settings › Security › Contain agents: "true" when on.
const GLOBAL_KEY: &str = "security.contain_agents";

/// The store key of a channel's override: "true", "false", or "" for none.
fn channel_key(channel: &str) -> String {
    format!("channel.{channel}.contain_agents")
}

/// Whether Settings contains agents, and the revision it was saved at (0
/// for the default, off).
pub(crate) fn global(store: &Store) -> Result<(bool, u64), StoreError> {
    Ok(store
        .setting(GLOBAL_KEY)?
        .map_or((false, 0), |saved| (saved.value == "true", saved.revision)))
}

/// Saves Settings › Contain agents at `revision`.
pub(crate) fn put_global(store: &Store, on: bool, revision: u64) -> Result<(), StoreError> {
    store.put_setting(GLOBAL_KEY, if on { "true" } else { "false" }, revision)
}

/// A channel's own choice, if it made one.
pub(crate) fn channel(store: &Store, channel: &str) -> Result<Option<bool>, StoreError> {
    Ok(store
        .setting(&channel_key(channel))?
        .and_then(|saved| match saved.value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }))
}

/// Saves a channel's choice; `None` goes back to Settings.
pub(crate) fn put_channel(store: &Store, channel: &str, on: Option<bool>, revision: u64) -> Result<(), StoreError> {
    let value = match on {
        Some(true) => "true",
        Some(false) => "false",
        None => "",
    };
    store.put_setting(&channel_key(channel), value, revision)
}

/// Whether a session in `channel` launches contained now.
fn applies(store: &Store, channel_id: &str) -> Result<bool, StoreError> {
    match channel(store, channel_id)? {
        Some(on) => Ok(on),
        None => global(store).map(|(on, _)| on),
    }
}

/// The agent whose home-folder state a session's agent name stands for.
fn agent_of(name: &str) -> Agent {
    match name {
        "claude" => Agent::ClaudeCode,
        "codex" => Agent::Codex,
        "copilot" => Agent::CopilotCli,
        "opencode" => Agent::OpenCode,
        _ => Agent::Other,
    }
}

/// Runs agents contained when their session's setting says so.
pub(crate) struct Contained {
    store: Arc<Store>,
    /// Where every channel's folder is; hidden but for the session's own.
    channels: Layout,
    /// The daemon's state folder: hidden, but for the hook files agents run.
    state: PathBuf,
    /// The daemon's socket: the one local connection allowed.
    socket: PathBuf,
    home: PathBuf,
    /// Where to ask whether the host's rules are in force: a machine with
    /// no policy copy launches nothing, since it would read Contain agents
    /// as off (D266).
    host_rules: Option<Arc<Guard>>,
}

impl Contained {
    pub(crate) fn new(store: Arc<Store>, channels: impl Into<Layout>, state: PathBuf, socket: PathBuf, home: PathBuf) -> Self {
        Self {
            store,
            channels: channels.into(),
            state,
            socket,
            home,
            host_rules: None,
        }
    }

    /// Refuses every launch and resume while `guard` says the host's rules
    /// aren't in force.
    pub(crate) fn needing_host_rules(mut self, guard: Arc<Guard>) -> Self {
        self.host_rules = Some(guard);
        self
    }

    fn containment(
        &self,
        session: &humata2_store::SessionRecord,
        shared: PathBuf,
        login: Login,
    ) -> Result<Containment, ContainRefused> {
        // Its temp folder and config copy live in `.humata/`, which git
        // must not show as the session's changes.
        hide_from_git(&session.folder)?;
        let agent = agent_of(&session.agent);
        let own = own_state(agent, &self.home, &self.state, &session.folder, login)?;
        let mut hidden = own.hidden;
        hidden.push(self.channels.root());
        hidden.extend(self.channels.older_roots());
        hidden.push(self.state.clone());
        Ok(Containment {
            session_folder: session.folder.clone(),
            shared,
            also_writable: Vec::new(),
            also_readable: vec![
                // The hooks agents run to tell Humata their turns (D169, D193).
                self.state.join("claude-hooks.json"),
                self.state.join("copilot-plugin"),
            ],
            hidden,
            socket: self.socket.clone(),
            agent_state: own.agent_state,
            read_only: own.read_only,
            temp: session.folder.join(".humata").join("tmp"),
            env: own.env,
        })
    }
}

impl Contained {
    /// Moves the session's own todos and checkpoints out of its contained
    /// copy into the real config. It never stops the start: what can't move
    /// stays in the copy for the next uncontained start.
    fn flow_back(&self, folder: &Path, conversation: &str) {
        match flow_back(&self.home, &folder.join(".humata").join("claude"), conversation) {
            Ok(report) => {
                for path in &report.conflicts {
                    crate::log::say!(
                        "humatad2: left {} in the session's contained Claude Code copy: your real config has a newer file in its place",
                        path.display()
                    );
                }
            }
            Err(error) => crate::log::say!("humatad2: warning: {}", crate::status::chain(&error)),
        }
    }
}

/// How a contained Claude Code signs in: the usual keychain login when the
/// installed version is verified to find it, else a token the user set.
fn claude_login(program: Option<&OsString>) -> Result<Login, ContainRefused> {
    let version = program.and_then(|program| installed_version(Path::new(program)));
    let token_set = TOKEN_VARIABLES
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()));
    Ok(choose_login(version.as_deref(), token_set)?)
}

/// The version `program --version` prints, if it answers within a few
/// seconds. An unreadable version is treated as unverified.
fn installed_version(program: &Path) -> Option<String> {
    let mut child = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut text = String::new();
    child.stdout.take()?.read_to_string(&mut text).ok()?;
    parse_version(&text)
}

/// What one agent keeps of its own when contained, and how it is told.
struct OwnState {
    hidden: Vec<PathBuf>,
    agent_state: Vec<PathBuf>,
    read_only: Vec<PathBuf>,
    env: Vec<(OsString, OsString)>,
}

/// The home-folder rules for `agent` in the session at `folder`. Claude Code
/// runs on a fresh copy of its config inside the session (D217 amendment):
/// the real `~/.claude/` and `~/.claude.json` stay readable but not
/// writable, apart from this session's own conversation folders.
fn own_state(
    agent: Agent,
    home: &Path,
    state: &Path,
    folder: &Path,
    login: Login,
) -> Result<OwnState, ContainRefused> {
    let rules = home_rules(agent, home)?;
    let mut own = OwnState {
        hidden: rules.hidden,
        agent_state: rules.agent_state,
        read_only: rules.read_only,
        env: Vec::new(),
    };
    match agent {
        Agent::ClaudeCode => {
            let copy = seed_claude_copy(home, &folder.join(".humata").join("claude"), folder)?;
            own.env = copy.env(login);
            own.agent_state = copy.projects;
            own.read_only = Vec::new();
        }
        // The `humata opencode` wrapper keeps its server's state here.
        Agent::OpenCode => own.agent_state.push(state.join("opencode")),
        Agent::Codex | Agent::CopilotCli | Agent::Other => {}
    }
    Ok(own)
}

impl std::fmt::Debug for Contained {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Contained")
            .field("channels", &self.channels)
            .field("state", &self.state)
            .field("socket", &self.socket)
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}

impl Contain for Contained {
    fn wrap(&self, record: &AgentRecord, argv: &[OsString]) -> Result<Vec<OsString>, ContainRefused> {
        if self.host_rules.as_ref().is_some_and(|guard| !guard.has_host_rules()) {
            return Err(ContainRefused::HostRulesMissing {
                session: record.session.clone(),
            });
        }
        let setting = |source| ContainRefused::Setting {
            session: record.session.clone(),
            source,
        };
        let session = self
            .store
            .session(&record.session)
            .map_err(setting)?
            .ok_or_else(|| ContainRefused::NoSession {
                session: record.session.clone(),
            })?;
        let claude = matches!(agent_of(&session.agent), Agent::ClaudeCode);
        if !applies(&self.store, &session.channel).map_err(setting)? {
            // D217: the mirror of seeding, at an uncontained start.
            if claude {
                self.flow_back(&session.folder, &record.launch.conversation_id);
            }
            return Ok(argv.to_vec());
        }
        // Refused before anything is seeded: no login, no contained launch.
        let login = if claude {
            claude_login(argv.first())?
        } else {
            Login::Keychain
        };
        let channel = self
            .store
            .channel(&session.channel)
            .map_err(setting)?
            .ok_or_else(|| ContainRefused::NoSession {
                session: record.session.clone(),
            })?;
        let shared = self.channels.shared(&channel.folder);
        Ok(self.containment(&session, shared, login)?.wrap(argv)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        (directory, store)
    }

    #[test]
    fn a_contained_claude_code_writes_its_copy_and_only_its_own_conversations() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(home.join(".claude.json"), "{}\n").unwrap();
        let folder = directory.path().join("channels/acme/fix-login");
        std::fs::create_dir_all(&folder).unwrap();
        let state = directory.path().join("state");

        let own = own_state(Agent::ClaudeCode, &home, &state, &folder, Login::Keychain).unwrap();

        let copy = folder.join(".humata/claude");
        assert!(own.env.contains(&("CLAUDE_CONFIG_DIR".into(), copy.clone().into_os_string())));
        assert!(copy.join(".claude.json").is_file());
        assert!(!own.agent_state.contains(&home.join(".claude")));
        assert!(!own.agent_state.contains(&home.join(".claude.json")));
        assert!(!own.agent_state.is_empty());
        for path in &own.agent_state {
            assert!(path.starts_with(home.join(".claude/projects")), "{}", path.display());
        }
    }

    fn agent_of_session(session: &str) -> AgentRecord {
        AgentRecord {
            session: session.to_owned(),
            tab: "tab".to_owned(),
            launch: humata2_store::AgentLaunch {
                agent: "codex".to_owned(),
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
            state: humata2_store::AgentState::Launching,
            generation: 1,
            interruptions: Vec::new(),
            updated_unix_ms: 0,
        }
    }

    #[test]
    fn a_machine_with_no_copy_launches_nothing_and_does_once_it_has_the_hosts_rules() {
        let (directory, store) = store();
        let store = Arc::new(store);
        store
            .put_session(&humata2_store::SessionRecord {
                id: "se_a".to_owned(),
                channel: "ch_a".to_owned(),
                machine: String::new(),
                title: "t".to_owned(),
                agent: "codex".to_owned(),
                folder: directory.path().to_owned(),
                created_unix_ms: 0,
            })
            .unwrap();
        let guard = Arc::new(Guard::new("m1", humata2_grpc::humata::v1::DaemonRole::Machine));
        let contained = Contained::new(
            store,
            directory.path().join("ch"),
            directory.path().join("state"),
            directory.path().join("d.sock"),
            directory.path().to_owned(),
        )
        .needing_host_rules(guard.clone());
        let record = agent_of_session("se_a");
        let argv = vec![OsString::from("agent")];

        assert!(matches!(
            contained.wrap(&record, &argv),
            Err(ContainRefused::HostRulesMissing { .. })
        ));

        guard.host_rules_arrived();
        assert_eq!(contained.wrap(&record, &argv).unwrap(), argv);
    }

    #[test]
    fn a_program_that_prints_no_version_is_unverified() {
        assert_eq!(installed_version(Path::new("/nonexistent/claude")), None);
    }

    #[test]
    fn other_agents_keep_their_own_state_and_get_no_variables() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let state = directory.path().join("state");

        let own = own_state(Agent::Codex, &home, &state, directory.path(), Login::Keychain).unwrap();
        assert!(own.agent_state.contains(&home.join(".codex")));
        assert!(own.env.is_empty());
        let own = own_state(Agent::OpenCode, &home, &state, directory.path(), Login::Keychain).unwrap();
        assert!(own.agent_state.contains(&state.join("opencode")));
    }

    #[test]
    fn agents_run_uncontained_until_settings_say_otherwise() {
        let (_directory, store) = store();
        assert!(!applies(&store, "acme").unwrap());
        put_global(&store, true, 1).unwrap();
        assert!(applies(&store, "acme").unwrap());
        assert_eq!(global(&store).unwrap(), (true, 1));
    }

    #[test]
    fn a_channels_choice_wins_over_settings_until_cleared() {
        let (_directory, store) = store();
        put_global(&store, true, 1).unwrap();
        put_channel(&store, "acme", Some(false), 1).unwrap();
        assert!(!applies(&store, "acme").unwrap());
        assert!(applies(&store, "other").unwrap());
        put_channel(&store, "acme", None, 2).unwrap();
        assert_eq!(channel(&store, "acme").unwrap(), None);
        assert!(applies(&store, "acme").unwrap());
    }
}
