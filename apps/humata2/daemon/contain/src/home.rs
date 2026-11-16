//! What a contained agent may touch in the home folder (D217).
//!
//! The agent reaches its model's API, so it keeps its own config and state.
//! The rest of the home folder is hidden from it: credential folders, the
//! state of every other known agent, and each visible folder. Toolchain
//! dot-folders, git config and `~/Library` stay readable. The caller hides
//! Humata's own folders. This is plain path logic, so it builds on every
//! system.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use crate::ContainError;

/// Paths, relative to home, that hold credentials. A contained agent never
/// reads them, whether or not they exist.
const CREDENTIAL_PATHS: &[&str] = &[
    ".ssh",
    ".config/gh",
    ".gnupg",
    ".aws",
    ".kube",
    ".docker",
    ".netrc",
    ".npmrc",
    ".git-credentials",
    ".config/gcloud",
];

/// The agents whose home-folder state is hidden from every other agent.
const KNOWN_AGENTS: [Agent; 4] = [
    Agent::ClaudeCode,
    Agent::Codex,
    Agent::CopilotCli,
    Agent::OpenCode,
];

/// The agents Humata knows the home-folder state of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    /// Claude Code.
    ClaudeCode,
    /// The Codex CLI.
    Codex,
    /// GitHub Copilot CLI.
    CopilotCli,
    /// OpenCode.
    OpenCode,
    /// An agent Humata has no rules for. It keeps no home-folder state of its own.
    Other,
}

/// What a contained agent may and may not touch in the home folder (D217).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HomeRules {
    /// Folders the agent may not read.
    pub hidden: Vec<PathBuf>,
    /// The agent's own config and state. Readable and writable, even inside a
    /// hidden folder.
    pub agent_state: Vec<PathBuf>,
    /// Paths inside the agent's state that stay readable but never writable.
    pub read_only: Vec<PathBuf>,
}

/// The home-folder rules for `agent`, with every path joined onto `home`.
/// The paths are not resolved here; the containment resolves them when it
/// wraps the command.
///
/// Fails when `home` can't be listed, because the visible folders can't be
/// hidden without the listing.
pub fn home_rules(agent: Agent, home: &Path) -> Result<HomeRules, ContainError> {
    let mut hidden: Vec<PathBuf> = CREDENTIAL_PATHS
        .iter()
        .map(|rel| home.join(rel))
        .collect();
    for other in KNOWN_AGENTS.into_iter().filter(|other| *other != agent) {
        hidden.extend(
            state_paths(other)
                .iter()
                .chain(read_only_paths(other))
                .map(|rel| home.join(rel)),
        );
    }
    hidden.extend(visible_entries(home)?);
    hidden.sort();
    hidden.dedup();

    Ok(HomeRules {
        hidden,
        agent_state: state_paths(agent).iter().map(|rel| home.join(rel)).collect(),
        read_only: read_only_paths(agent)
            .iter()
            .map(|rel| home.join(rel))
            .collect(),
    })
}

/// Every entry in `home` except the dot-folders and `Library`. Names that
/// aren't UTF-8 are still listed, so they stay hidden.
fn visible_entries(home: &Path) -> Result<Vec<PathBuf>, ContainError> {
    let unreadable = |source: io::Error| ContainError::HomeUnreadable {
        path: home.to_path_buf(),
        source,
    };
    let mut visible = Vec::new();
    for entry in std::fs::read_dir(home).map_err(unreadable)? {
        let name: OsString = entry.map_err(unreadable)?.file_name();
        let dot_folder = name.as_encoded_bytes().first() == Some(&b'.');
        if !dot_folder && name != "Library" {
            visible.push(home.join(&name));
        }
    }
    Ok(visible)
}

/// The agent's own config and state, relative to home.
fn state_paths(agent: Agent) -> &'static [&'static str] {
    match agent {
        Agent::ClaudeCode => &[".claude", ".claude.json"],
        Agent::Codex => &[".codex"],
        Agent::CopilotCli => &[".copilot"],
        Agent::OpenCode => &[
            ".local/share/opencode",
            ".local/state/opencode",
            ".cache/opencode",
        ],
        Agent::Other => &[],
    }
}

/// The agent's settings and hooks, relative to home. They stay readable, so
/// the agent can load them, but the agent can't change them.
fn read_only_paths(agent: Agent) -> &'static [&'static str] {
    match agent {
        Agent::ClaudeCode => &[
            ".claude/settings.json",
            ".claude/settings.local.json",
            ".claude/hooks",
            ".claude/CLAUDE.md",
            ".claude/agents",
            ".claude/commands",
            ".claude/skills",
            ".claude/plugins",
        ],
        Agent::Codex => &[".codex/config.toml", ".codex/AGENTS.md"],
        Agent::CopilotCli => &[".copilot/mcp-config.json"],
        Agent::OpenCode => &[".config/opencode"],
        Agent::Other => &[],
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{Agent, home_rules};
    use crate::ContainError;

    /// A home folder with the folders the rules care about. The returned
    /// `TempDir` must live as long as the path.
    fn sample_home() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp folder");
        let home = dir.path().join("home");
        for folder in [
            "Documents",
            "Library",
            "code",
            ".ssh",
            ".cargo",
            ".claude",
            ".codex",
        ] {
            std::fs::create_dir_all(home.join(folder)).expect("a folder");
        }
        std::fs::write(home.join(".claude.json"), "{}\n").expect("a file");
        std::fs::write(home.join(".claude/settings.json"), "{}\n").expect("a file");
        (dir, home)
    }

    #[test]
    fn claude_code_keeps_its_own_state_and_hides_the_rest_of_the_home() {
        let (_dir, home) = sample_home();
        let rules = home_rules(Agent::ClaudeCode, &home).expect("rules");

        for hidden in ["Documents", "code", ".ssh", ".codex", ".aws"] {
            assert!(rules.hidden.contains(&home.join(hidden)), "{hidden} is not hidden");
        }
        for kept in ["Library", ".cargo", ".claude"] {
            assert!(!rules.hidden.contains(&home.join(kept)), "{kept} is hidden");
        }
        assert!(rules.agent_state.contains(&home.join(".claude")));
        assert!(rules.agent_state.contains(&home.join(".claude.json")));
        assert!(rules.read_only.contains(&home.join(".claude/settings.json")));
    }

    #[test]
    fn codex_hides_claude_codes_state_and_keeps_its_own() {
        let (_dir, home) = sample_home();
        let rules = home_rules(Agent::Codex, &home).expect("rules");

        assert!(rules.hidden.contains(&home.join(".claude")));
        assert!(rules.hidden.contains(&home.join(".claude.json")));
        assert!(rules.agent_state.contains(&home.join(".codex")));
        assert!(!rules.hidden.contains(&home.join(".codex")));
    }

    #[test]
    fn an_agent_without_rules_keeps_no_state_and_hides_every_known_agent() {
        let (_dir, home) = sample_home();
        let rules = home_rules(Agent::Other, &home).expect("rules");

        assert!(rules.agent_state.is_empty());
        assert!(rules.read_only.is_empty());
        for hidden in [".claude", ".codex", ".copilot", ".local/share/opencode"] {
            assert!(rules.hidden.contains(&home.join(hidden)), "{hidden} is not hidden");
        }
    }

    #[test]
    fn hidden_folders_come_in_a_stable_order_without_repeats() {
        let (_dir, home) = sample_home();
        let rules = home_rules(Agent::Codex, &home).expect("rules");

        assert!(rules.hidden.windows(2).all(|pair| pair[0] < pair[1]));
    }

    // APFS refuses names that aren't UTF-8, so only Linux file systems can hold one.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_visible_entry_whose_name_is_not_utf8_is_still_hidden() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let (_dir, home) = sample_home();
        let odd = home.join(OsStr::from_bytes(b"notes-\xff"));
        std::fs::create_dir(&odd).expect("a folder");
        let rules = home_rules(Agent::ClaudeCode, &home).expect("rules");

        assert!(rules.hidden.contains(&odd));
    }

    #[test]
    fn a_home_folder_that_cannot_be_listed_is_an_error() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let missing = dir.path().join("nobody");

        let result = home_rules(Agent::ClaudeCode, &missing);
        assert!(
            matches!(result, Err(ContainError::HomeUnreadable { .. })),
            "got {result:?}"
        );
    }
}
