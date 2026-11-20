//! Config sync (FUSEFSWIRE, D895): the `[sync]` settings kept in the humata
//! home's `config/config-sync.toml`, and the agent sessions whose composed
//! `config/<agent>/` homes follow them. Changing the settings re-composes
//! every registered session in place; no holder is ever restarted and the
//! real agent homes are only read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use humata2_config_compose::{
    AgentKind, ComposeError, ConfigComposer, MachineSyncOverride, MaterializeReport, ProtectedHumataMcp,
    SkillsMode, SyncConfig,
};
use humata2_grpc::humata::v1 as pb;
use tonic::{Code, Status};

use crate::status;

/// Where the sync settings are, inside the humata home.
fn file_of(state: &Path) -> PathBuf {
    state.parent().unwrap_or(state).join("config").join("config-sync.toml")
}

/// Where a session's composed config home for `agent` is, under the
/// machine's root. The same folder `ConfigComposer::session_config_dir`
/// names; a session launched before composing existed has none.
pub(crate) fn session_home(machine_root: &Path, session: &str, agent: AgentKind) -> PathBuf {
    machine_root.join(".sessions").join(session).join("config").join(agent.as_str())
}

/// The machine root session homes are under, set once at the daemon's
/// start (when composing is installed); None before, and in tests that
/// don't compose.
static MACHINE_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

pub(crate) fn use_machine_root(root: PathBuf) {
    *MACHINE_ROOT.lock().unwrap_or_else(PoisonError::into_inner) = Some(root);
}

/// The composed config home of `session`'s `agent` under `machine_root`,
/// and the conversation format it keeps there. None for an agent with no
/// config home, or a session launched before composing existed (no folder).
pub(crate) fn composed_home_of(
    machine_root: &Path,
    session: &str,
    agent: &str,
) -> Option<(humata2_sessions::ConversationFormat, PathBuf)> {
    let (kind, format) = match agent {
        "claude" => (AgentKind::Claude, humata2_sessions::ConversationFormat::ClaudeJsonl),
        "codex" => (AgentKind::Codex, humata2_sessions::ConversationFormat::CodexRollout),
        "copilot" => (AgentKind::Copilot, humata2_sessions::ConversationFormat::CopilotSessionState),
        // agy has no conversation format, so nothing of its is read or resumed.
        _ => return None,
    };
    let dir = session_home(machine_root, session, kind);
    dir.is_dir().then_some((format, dir))
}

/// `agents` as session `session` sees them: when its agent has a composed
/// config home under `machine_root`, that folder is where its conversations
/// are looked for (learned, read, resumed), not the daemon's own dir. Any
/// other session gets `agents` as they are.
pub(crate) fn agents_for_session_in(
    agents: &humata2_sessions::Agents,
    machine_root: &Path,
    session: &str,
    agent: &str,
) -> humata2_sessions::Agents {
    match composed_home_of(machine_root, session, agent) {
        Some((format, dir)) => agents.clone().with_cli_home(format, &dir),
        None => agents.clone(),
    }
}

/// `kind`, session `session`'s agent, with its conversations in the
/// session's composed home when it has one; `kind` as it is otherwise, or
/// when this daemon composes nothing.
pub(crate) fn kind_for_session(kind: &humata2_sessions::AgentKind, session: &str) -> humata2_sessions::AgentKind {
    let root = MACHINE_ROOT.lock().unwrap_or_else(PoisonError::into_inner).clone();
    match root.and_then(|root| composed_home_of(&root, session, &kind.name)) {
        Some((_, dir)) => kind.with_session_home(&dir),
        None => kind.clone(),
    }
}

/// The composed config home `session`'s `agent` already has under this
/// daemon's machine root; None when there is none. Makes nothing.
pub(crate) fn existing_home_of(session: &str, agent: &str) -> Option<PathBuf> {
    let root = MACHINE_ROOT.lock().unwrap_or_else(PoisonError::into_inner).clone()?;
    composed_home_of(&root, session, agent).map(|(_, dir)| dir)
}

/// [`agents_for_session_in`] for the machine root this daemon composes
/// under, naming the agent from the store; `agents` as they are when
/// there is no root or the store has no such agent.
pub(crate) fn agents_for_session(
    agents: &humata2_sessions::Agents,
    store: &humata2_store::Store,
    session: &str,
) -> humata2_sessions::Agents {
    let root = MACHINE_ROOT.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let Some(root) = root else {
        return agents.clone();
    };
    match store.agent(session) {
        Ok(Some(record)) => agents_for_session_in(agents, &root, session, &record.launch.agent),
        _ => agents.clone(),
    }
}

/// The agents whose config homes a session may have composed.
const COMPOSED_AGENTS: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codex, AgentKind::Copilot];

/// `agents` as a session arriving on this machine sees them: the config home
/// of its agent is made under `machine_root` first, so the conversation that
/// lands is where the session's launch looks for it. `agent` is the session's
/// agent when this machine already knows it; a machine that doesn't (a box
/// taking in a move or a fork) learns it only from the staged copy, so each
/// agent's home is made and the one the session launches with is the one used.
fn agents_for_incoming_in(
    agents: &humata2_sessions::Agents,
    machine_root: &Path,
    session: &str,
    agent: Option<&str>,
) -> humata2_sessions::Agents {
    let mut seen = agents.clone();
    // A launch composes only an agent that is named for one of these (the
    // lookups and `ComposeEnv` match by name), so an agent of any other name
    // that keeps the same format of conversation (a stand-in, a renamed
    // copy) stays in its own dir: landing it in a composed home would put
    // it where no launch or later lookup of its session looks.
    let named = |kind: &AgentKind| agents.kinds().iter().any(|have| have.name == kind.as_str());
    for kind in COMPOSED_AGENTS
        .iter()
        .filter(|kind| agent.is_none_or(|name| name == kind.as_str()))
        .filter(|kind| named(kind))
    {
        // Best effort: if the folder can't be made, the conversation lands
        // in the agent's own dir as before, and the launch's compose says why.
        if std::fs::create_dir_all(session_home(machine_root, session, *kind)).is_ok() {
            seen = agents_for_session_in(&seen, machine_root, session, kind.as_str());
        }
    }
    seen
}

/// Carry's view of sessions' conversations on a machine that composes
/// config homes: a move, fork or bring reads a session from, and lands it
/// into, the home its agent looks in, not the agent's global dir.
pub(crate) struct ComposedHomes;

impl crate::carry_service::SessionHomes for ComposedHomes {
    fn for_session(&self, starter: &humata2_sessions::Starter, session: &str) -> humata2_sessions::Starter {
        humata2_sessions::Starter {
            agents: agents_for_session(&starter.agents, &starter.store, session),
            ..starter.clone()
        }
    }

    fn for_incoming(&self, starter: &humata2_sessions::Starter, session: &str) -> humata2_sessions::Starter {
        let root = MACHINE_ROOT.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let Some(root) = root else {
            return starter.clone();
        };
        let known = starter.store.agent(session).ok().flatten().map(|record| record.launch.agent);
        humata2_sessions::Starter {
            agents: agents_for_incoming_in(&starter.agents, &root, session, known.as_deref()),
            ..starter.clone()
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigSyncError {
    #[error("couldn't read the sync settings file {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the sync settings file {path} isn't valid sync TOML")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("couldn't write the sync settings")]
    Serialize {
        #[source]
        source: toml::ser::Error,
    },
    #[error("couldn't save the sync settings file {path}")]
    Save {
        path: PathBuf,
        #[source]
        source: ComposeError,
    },
    #[error("couldn't compose the {agent} config home of session {session}")]
    Compose {
        session: String,
        agent: AgentKind,
        #[source]
        source: ComposeError,
    },
}

impl ConfigSyncError {
    pub(crate) fn status(&self) -> Status {
        let message = status::chain(self);
        match self {
            Self::Parse { path, .. } => status::failure(
                Code::FailedPrecondition,
                status::error(
                    message,
                    format!(
                        "fix {} by hand, or delete it to go back to the sync defaults (that loses what it holds); nothing was saved",
                        path.display()
                    ),
                    pb::error::Kind::InvalidField(pb::InvalidField {
                        field: "sync".to_owned(),
                        problem: pb::invalid_field::Problem::BadFormat as i32,
                    }),
                ),
            ),
            Self::Read { .. } | Self::Serialize { .. } | Self::Save { .. } => status::internal(
                format!("{message}. Fix the file's permissions or free some disk space, then save again"),
                "",
            ),
            Self::Compose { .. } => status::internal(
                format!(
                    "{message}. The sync settings were saved; fix the folder named above, then change them again to re-compose"
                ),
                "",
            ),
        }
    }
}

/// A session whose config home is composed, and what it was composed with.
struct Composed {
    agent: AgentKind,
    /// None until the session's first compose: it is not re-composed before.
    protected: Option<ProtectedHumataMcp>,
}

struct Inner {
    config: SyncConfig,
    sessions: BTreeMap<String, Composed>,
}

pub(crate) struct ConfigSync {
    path: PathBuf,
    machine_root: PathBuf,
    local_home: PathBuf,
    machine_name: String,
    fuse_root: PathBuf,
    /// Where a host's synced config is kept, once one is.
    bundle_root: Option<PathBuf>,
    inner: Mutex<Inner>,
}

#[allow(dead_code, reason = "composing at launch and end is wired by the session-env part")]
impl ConfigSync {
    /// `state` is the daemon's state folder, inside the humata home;
    /// `machine_root` holds `.sessions/<id>/config/<agent>/`; `local_home`
    /// is the real HOME, which is only read; `fuse_root` is where FUSE is
    /// mounted. A sync file that can't be read shows the defaults and is
    /// logged; saving names it and leaves it alone.
    pub(crate) fn new(
        state: &Path,
        machine_root: PathBuf,
        local_home: PathBuf,
        machine_name: String,
        fuse_root: PathBuf,
    ) -> Self {
        let path = file_of(state);
        let config = load(&path).unwrap_or_else(|error| {
            crate::log::say!("humatad2: using the default sync settings: {}", status::chain(&error));
            SyncConfig::default()
        });
        Self {
            path,
            machine_root,
            local_home,
            machine_name,
            fuse_root,
            bundle_root: None,
            inner: Mutex::new(Inner { config, sessions: BTreeMap::new() }),
        }
    }

    /// Composes from the synced host config kept under `root/<agent>/`.
    pub(crate) fn with_bundle_root(mut self, root: PathBuf) -> Self {
        self.bundle_root = Some(root);
        self
    }

    pub(crate) fn fuse_root(&self) -> &Path {
        &self.fuse_root
    }

    pub(crate) fn machine_name(&self) -> &str {
        &self.machine_name
    }

    pub(crate) fn machine_root(&self) -> &Path {
        &self.machine_root
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The sync settings in force.
    pub(crate) fn config(&self) -> SyncConfig {
        self.lock().config.clone()
    }

    /// Notes that `session` runs `agent`, to be re-composed when the sync
    /// settings change. It is composed from `compose`.
    pub(crate) fn register(&self, session: &str, agent: AgentKind) {
        self.lock()
            .sessions
            .entry(session.to_owned())
            .and_modify(|known| known.agent = agent)
            .or_insert(Composed { agent, protected: None });
    }

    /// Forgets `session`; its folder is left as it is.
    pub(crate) fn unregister(&self, session: &str) {
        self.lock().sessions.remove(session);
    }

    fn composer(&self, config: &SyncConfig, protected: ProtectedHumataMcp) -> ConfigComposer {
        let mut composer = ConfigComposer::new(
            self.machine_root.clone(),
            self.local_home.clone(),
            self.machine_name.clone(),
            protected,
        );
        composer.sync_config = config.clone();
        composer.synced_bundle_root = self.bundle_root.clone();
        composer
    }

    /// Composes `session`'s `config/<agent>/` home from the settings in
    /// force, and registers it to follow them.
    pub(crate) fn compose(
        &self,
        session: &str,
        agent: AgentKind,
        protected: ProtectedHumataMcp,
    ) -> Result<MaterializeReport, ConfigSyncError> {
        let mut inner = self.lock();
        let composer = self.composer(&inner.config, protected.clone());
        let report = composer
            .materialize_session(session, agent)
            .map_err(|source| ConfigSyncError::Compose { session: session.to_owned(), agent, source })?;
        inner.sessions.insert(session.to_owned(), Composed { agent, protected: Some(protected) });
        Ok(report)
    }

    /// Saves `new`, then re-composes every composed session in place. A
    /// session that fails does not stop the others; the first failure is
    /// returned after all were tried, and the settings stay saved. A file
    /// the user broke by hand is named and left as it is.
    pub(crate) fn apply(&self, new: SyncConfig) -> Result<Vec<MaterializeReport>, ConfigSyncError> {
        let mut inner = self.lock();
        // Read first: a broken file is never overwritten.
        let before = load(&self.path)?;
        save(&self.path, &new)?;
        let flipped = before.sync_enabled != new.sync_enabled;
        inner.config = new.clone();
        // A deleted session's home is gone with its folder: forget it rather
        // than compose a fresh one back into place.
        let root = &self.machine_root;
        inner.sessions.retain(|session, composed| session_home(root, session, composed.agent).is_dir());
        let mut reports = Vec::new();
        let mut first_failure = None;
        for (session, composed) in &inner.sessions {
            let Some(protected) = composed.protected.clone() else { continue };
            let mut composer = self.composer(&new, protected);
            let done = match (flipped, new.sync_enabled) {
                (true, false) => composer.disable_sync(session, composed.agent),
                (true, true) => composer.enable_sync(session, composed.agent),
                (false, _) => composer.materialize_session(session, composed.agent),
            };
            match done {
                Ok(report) => reports.push(report),
                Err(source) => {
                    if first_failure.is_none() {
                        first_failure =
                            Some(ConfigSyncError::Compose { session: session.clone(), agent: composed.agent, source });
                    }
                }
            }
        }
        match first_failure {
            Some(error) => Err(error),
            None => Ok(reports),
        }
    }
}

/// The sync settings, or the defaults when no file is saved.
fn load(path: &Path) -> Result<SyncConfig, ConfigSyncError> {
    match std::fs::read_to_string(path) {
        Ok(text) => SyncConfig::from_toml_str(&text)
            .map_err(|source| ConfigSyncError::Parse { path: path.to_owned(), source }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SyncConfig::default()),
        Err(source) => Err(ConfigSyncError::Read { path: path.to_owned(), source }),
    }
}

fn save(path: &Path, config: &SyncConfig) -> Result<(), ConfigSyncError> {
    #[derive(serde::Serialize)]
    struct Document<'a> {
        sync: &'a SyncConfig,
    }
    let text = toml::to_string(&Document { sync: config })
        .map_err(|source| ConfigSyncError::Serialize { source })?;
    humata2_config_compose::compose::atomic_write(path, text.as_bytes())
        .map_err(|source| ConfigSyncError::Save { path: path.to_owned(), source })
}

/// Whether Settings has turned the FUSE mount on. False until it is set
/// (D895), and when the settings file can't be read. `state` is the folder
/// the Settings service is given.
pub(crate) fn saved_fusefs_enabled(state: &Path) -> bool {
    crate::settings_service::fusefs_enabled_in(state)
}

/// The sync settings as the Settings page shows them.
pub(crate) fn shown(config: &SyncConfig) -> pb::SyncSettings {
    pb::SyncSettings {
        sync_enabled: config.sync_enabled,
        skills: config.toggles.skills,
        instructions: config.toggles.instructions,
        mcp: config.toggles.mcp,
        settings: config.toggles.settings,
        skills_mode: skills_mode_shown(config.skills_mode) as i32,
        machines: config
            .machines
            .iter()
            .map(|(machine, over)| {
                (
                    machine.clone(),
                    pb::MachineSyncOverride {
                        enabled: over.enabled,
                        skills: over.skills,
                        instructions: over.instructions,
                        mcp: over.mcp,
                        settings: over.settings,
                        skills_mode: over.skills_mode.map(|mode| skills_mode_shown(mode) as i32),
                    },
                )
            })
            .collect(),
    }
}

fn skills_mode_shown(mode: SkillsMode) -> pb::SyncSkillsMode {
    match mode {
        SkillsMode::HostWins => pb::SyncSkillsMode::HostWins,
        SkillsMode::HostOnly => pb::SyncSkillsMode::HostOnly,
    }
}

fn skills_mode_of(field: &str, value: i32) -> Result<SkillsMode, Status> {
    match pb::SyncSkillsMode::try_from(value) {
        Ok(pb::SyncSkillsMode::Unspecified | pb::SyncSkillsMode::HostWins) => Ok(SkillsMode::HostWins),
        Ok(pb::SyncSkillsMode::HostOnly) => Ok(SkillsMode::HostOnly),
        Err(_) => Err(status::invalid_field(
            field,
            pb::invalid_field::Problem::UnknownValue,
            format!("{value} isn't a way to combine host skills with a session's own"),
            "pick Host wins or Host only",
        )),
    }
}

/// The settings a request asks for, or the first bad field.
pub(crate) fn wanted(sync: &pb::SyncSettings) -> Result<SyncConfig, Status> {
    let mut machines = BTreeMap::new();
    for (machine, over) in &sync.machines {
        if machine.trim().is_empty() {
            return Err(status::invalid_field(
                "sync.machines",
                pb::invalid_field::Problem::Missing,
                "a machine's sync override has no machine name",
                "pick the machines from the list",
            ));
        }
        let skills_mode = match over.skills_mode {
            Some(value) => Some(skills_mode_of("sync.machines.skills_mode", value)?),
            None => None,
        };
        machines.insert(
            machine.clone(),
            MachineSyncOverride {
                enabled: over.enabled,
                skills: over.skills,
                instructions: over.instructions,
                mcp: over.mcp,
                settings: over.settings,
                skills_mode,
            },
        );
    }
    Ok(SyncConfig {
        sync_enabled: sync.sync_enabled,
        toggles: humata2_config_compose::SyncToggles {
            skills: sync.skills,
            instructions: sync.instructions,
            mcp: sync.mcp,
            settings: sync.settings,
        },
        skills_mode: skills_mode_of("sync.skills_mode", sync.skills_mode)?,
        machines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST_NOTES: &str = "host notes";

    struct Rig {
        _dir: tempfile::TempDir,
        state: PathBuf,
        local_home: PathBuf,
        sync: ConfigSync,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let state = root.join("humata").join("state");
        std::fs::create_dir_all(&state).unwrap();
        let local_home = root.join("home");
        let claude = local_home.join(".claude");
        std::fs::create_dir_all(claude.join("skills").join("mine")).unwrap();
        std::fs::write(claude.join("skills").join("mine").join("SKILL.md"), "local skill").unwrap();
        std::fs::write(claude.join("CLAUDE.md"), "local notes").unwrap();
        let bundle = root.join("bundle");
        std::fs::create_dir_all(bundle.join("claude")).unwrap();
        std::fs::write(bundle.join("claude").join("CLAUDE.md"), HOST_NOTES).unwrap();
        let sync = ConfigSync::new(
            &state,
            root.join("machine"),
            local_home.clone(),
            "box".to_owned(),
            root.join("mnt"),
        )
        .with_bundle_root(bundle);
        Rig { _dir: dir, state, local_home, sync }
    }

    fn protected() -> ProtectedHumataMcp {
        ProtectedHumataMcp::new("humata", vec!["mcp".to_owned()])
    }

    #[test]
    fn a_session_with_a_composed_home_has_its_conversation_looked_for_there() {
        let dir = tempfile::tempdir().unwrap();
        let (home, folder) = (Path::new("/home/me"), Path::new("/work/s"));
        let agents = humata2_sessions::Agents::builtin();
        let composed = session_home(dir.path(), "se_a", AgentKind::Codex);
        std::fs::create_dir_all(&composed).unwrap();

        let own = agents_for_session_in(&agents, dir.path(), "se_a", "codex");
        let placed = own.get("codex").unwrap().placement(home, folder).unwrap();
        assert_eq!(placed.cli_home.as_deref(), Some(composed.as_path()));

        // An older session (no composed home), another agent: unchanged.
        let older = agents_for_session_in(&agents, dir.path(), "se_old", "codex");
        assert_eq!(older.get("codex").unwrap().placement(home, folder).unwrap().cli_home, None);
        let other = agents_for_session_in(&agents, dir.path(), "se_a", "opencode");
        assert!(other.get("opencode").unwrap().placement(home, folder).is_none());
    }

    #[test]
    fn an_incoming_session_lands_where_its_composed_launch_looks_and_compose_keeps_it() {
        let rig = rig();
        let machine_root = rig._dir.path().join("machine");
        let agents = humata2_sessions::Agents::builtin();
        let (home, folder) = (Path::new("/home/me"), Path::new("/work/a"));

        let seen = agents_for_incoming_in(&agents, &machine_root, "se_in", Some("claude"));

        let landing = seen.get("claude").unwrap().placement(home, folder).unwrap();
        assert_eq!(landing.cli_home, Some(session_home(&machine_root, "se_in", AgentKind::Claude)));
        // Only the session's own agent got a home.
        assert!(composed_home_of(&machine_root, "se_in", "codex").is_none());

        // What landed there survives the compose the launch does later.
        let kept = session_home(&machine_root, "se_in", AgentKind::Claude).join("projects").join("-work-a").join("c1.jsonl");
        std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
        std::fs::write(&kept, "turns").unwrap();
        rig.sync.compose("se_in", AgentKind::Claude, protected()).unwrap();
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "turns");
    }

    #[test]
    fn an_incoming_session_of_an_unknown_agent_gets_every_agents_home() {
        let dir = tempfile::tempdir().unwrap();
        let agents = humata2_sessions::Agents::builtin();

        let seen = agents_for_incoming_in(&agents, dir.path(), "se_new", None);

        for agent in ["claude", "codex", "copilot"] {
            assert!(composed_home_of(dir.path(), "se_new", agent).is_some(), "{agent}");
        }
        let placed = seen.get("codex").unwrap().placement(Path::new("/h"), Path::new("/w")).unwrap();
        assert_eq!(placed.cli_home, Some(session_home(dir.path(), "se_new", AgentKind::Codex)));
    }

    #[test]
    fn an_incoming_session_of_an_agent_no_launch_composes_stays_in_the_agents_own_dir() {
        let dir = tempfile::tempdir().unwrap();
        let standin = br#"[{"name":"standin","program":"p","start_args":[],"resume_args":[],
            "conversation_file":"{home}/.claude/projects/{folder_dashed}/{conversation}.jsonl",
            "conversation_format":"claude-jsonl"}]"#;
        let agents = humata2_sessions::Agents::from_json(standin).unwrap();

        let seen = agents_for_incoming_in(&agents, dir.path(), "se_new", None);

        let placed = seen.get("standin").unwrap().placement(Path::new("/h"), Path::new("/w")).unwrap();
        assert_eq!(placed.cli_home, None);
        assert!(!dir.path().join(".sessions").exists());
    }

    #[test]
    fn the_session_home_is_the_folder_the_composer_makes() {
        let rig = rig();
        let machine_root = rig._dir.path().join("machine");
        for agent in AgentKind::ALL {
            let report = rig.sync.compose("se_home", agent, protected()).unwrap();
            assert_eq!(report.session_config_dir, session_home(&machine_root, "se_home", agent));
        }
    }

    /// Every file under `root` with its bytes, for a before and after.
    fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(dir: &Path, found: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, found);
                } else {
                    found.insert(path.clone(), std::fs::read(&path).unwrap());
                }
            }
        }
        let mut found = BTreeMap::new();
        walk(root, &mut found);
        found
    }

    fn notes(report: &MaterializeReport) -> PathBuf {
        report.session_config_dir.join("CLAUDE.md")
    }

    fn with_sync(on: bool) -> SyncConfig {
        SyncConfig { sync_enabled: on, ..SyncConfig::default() }
    }

    #[test]
    fn sync_off_recomposes_a_session_in_place_and_on_restores_it() {
        let rig = rig();
        let report = rig.sync.compose("s1", AgentKind::Claude, protected()).unwrap();
        assert_eq!(std::fs::read_to_string(notes(&report)).unwrap(), HOST_NOTES);
        let before = tree(&rig.local_home);

        let off = rig.sync.apply(with_sync(false)).unwrap();

        assert_eq!(off.len(), 1);
        assert!(!off[0].holder_restart_required);
        // The host's copy is gone; what shows is the machine's own file.
        assert_eq!(std::fs::read_to_string(notes(&off[0])).unwrap(), "local notes");
        assert!(!off[0].effective_policy.sync_enabled);
        assert_eq!(tree(&rig.local_home), before);

        let on = rig.sync.apply(with_sync(true)).unwrap();

        assert!(!on[0].holder_restart_required);
        assert_eq!(std::fs::read_to_string(notes(&on[0])).unwrap(), HOST_NOTES);
        assert_eq!(tree(&rig.local_home), before);
    }

    #[test]
    fn saved_settings_are_read_back_by_a_new_start() {
        let rig = rig();
        let mut wanted = with_sync(false);
        wanted.toggles.mcp = true;
        wanted.machines.insert(
            "box".to_owned(),
            MachineSyncOverride { skills: Some(false), skills_mode: Some(SkillsMode::HostOnly), ..Default::default() },
        );
        rig.sync.apply(wanted.clone()).unwrap();

        let again = ConfigSync::new(
            &rig.state,
            PathBuf::from("/unused"),
            PathBuf::from("/unused"),
            "box".to_owned(),
            PathBuf::from("/unused"),
        );

        assert_eq!(again.config(), wanted);
    }

    #[test]
    fn a_broken_file_is_reported_and_left_untouched() {
        let rig = rig();
        let path = file_of(&rig.state);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[sync\nthis is not toml").unwrap();
        rig.sync.compose("s1", AgentKind::Claude, protected()).unwrap();

        let refused = rig.sync.apply(with_sync(false));

        assert!(matches!(refused, Err(ConfigSyncError::Parse { .. })));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[sync\nthis is not toml");
        // Nothing in force changed either.
        assert!(rig.sync.config().sync_enabled);
    }

    #[test]
    fn an_unregistered_session_is_not_recomposed() {
        let rig = rig();
        rig.sync.compose("s1", AgentKind::Claude, protected()).unwrap();
        rig.sync.unregister("s1");

        assert!(rig.sync.apply(with_sync(false)).unwrap().is_empty());
    }

    #[test]
    fn the_wire_settings_round_trip_and_an_unknown_mode_is_refused() {
        let mut config = with_sync(true);
        config.skills_mode = SkillsMode::HostOnly;
        config.machines.insert("box".to_owned(), MachineSyncOverride { mcp: Some(true), ..Default::default() });
        assert_eq!(wanted(&shown(&config)).unwrap(), config);

        let unspecified = pb::SyncSettings { skills_mode: pb::SyncSkillsMode::Unspecified as i32, ..shown(&config) };
        assert_eq!(wanted(&unspecified).unwrap().skills_mode, SkillsMode::HostWins);

        let unknown = pb::SyncSettings { skills_mode: 99, ..shown(&config) };
        assert_eq!(wanted(&unknown).unwrap_err().code(), Code::InvalidArgument);
    }
}
