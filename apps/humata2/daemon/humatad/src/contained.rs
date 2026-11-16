//! Contain agents (SEC-6, D217): whether a session's agent runs contained,
//! from its channel's override or else Settings, and the sandbox it runs in.
//! Every launch and resume asks here, through revive's `Contain` hook, so a
//! change applies when a session next launches or resumes. A session that
//! must be contained and can't be stays stopped: it never runs uncontained
//! instead.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use humata2_contain::{Agent, Containment, home_rules};
use humata2_revive::{Contain, ContainRefused};
use humata2_store::{AgentRecord, Store, StoreError};

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
    channels: PathBuf,
    /// The daemon's state folder: hidden, but for the hook files agents run.
    state: PathBuf,
    /// The daemon's socket: the one local connection allowed.
    socket: PathBuf,
    home: PathBuf,
}

impl Contained {
    pub(crate) fn new(store: Arc<Store>, channels: PathBuf, state: PathBuf, socket: PathBuf, home: PathBuf) -> Self {
        Self {
            store,
            channels,
            state,
            socket,
            home,
        }
    }

    fn containment(&self, session: &humata2_store::SessionRecord, shared: PathBuf) -> Result<Containment, ContainRefused> {
        let rules = home_rules(agent_of(&session.agent), &self.home)?;
        let mut hidden = rules.hidden;
        hidden.push(self.channels.clone());
        hidden.push(self.state.clone());
        let mut agent_state = rules.agent_state;
        if agent_of(&session.agent) == Agent::OpenCode {
            // The `humata opencode` wrapper keeps its server's state here.
            agent_state.push(self.state.join("opencode"));
        }
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
            agent_state,
            read_only: rules.read_only,
            temp: session.folder.join(".humata").join("tmp"),
        })
    }
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
        if !applies(&self.store, &session.channel).map_err(setting)? {
            return Ok(argv.to_vec());
        }
        let channel = self
            .store
            .channel(&session.channel)
            .map_err(setting)?
            .ok_or_else(|| ContainRefused::NoSession {
                session: record.session.clone(),
            })?;
        let shared = self.channels.join(&channel.folder).join("shared");
        Ok(self.containment(&session, shared)?.wrap(argv)?)
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
