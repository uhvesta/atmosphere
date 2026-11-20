//! Core data model for per-agent config layout, `[sync]` settings toggles,
//! per-machine overrides, rage-quit switch, and `SecretDenylist`.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

/// Supported CLI agents whose native per-session config homes are composed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    Copilot,
    Agy,
}

impl AgentKind {
    /// All supported agent kinds in canonical order.
    pub const ALL: [AgentKind; 4] = [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Copilot,
        AgentKind::Agy,
    ];

    /// Canonical lowercase identifier used in paths (`config/<agent>`) and env vars.
    pub const fn as_str(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Copilot => "copilot",
            AgentKind::Agy => "agy",
        }
    }

    /// Environment variable pointing the agent CLI to its per-session native config home.
    pub const fn config_env_var(self) -> &'static str {
        match self {
            AgentKind::Claude => "CLAUDE_CONFIG_DIR",
            AgentKind::Codex => "CODEX_HOME",
            AgentKind::Copilot => "COPILOT_HOME",
            AgentKind::Agy => "GEMINI_CLI_HOME",
        }
    }

    /// Relative path under the user's home directory where this agent's real native config lives.
    pub const fn default_home_rel_path(self) -> &'static str {
        match self {
            AgentKind::Claude => ".claude",
            AgentKind::Codex => ".codex",
            AgentKind::Copilot => ".copilot",
            AgentKind::Agy => ".gemini/antigravity-cli",
        }
    }

    /// Static specification of auth files, sync categories, and config formats for this agent.
    pub const fn spec(self) -> AgentConfigSpec {
        match self {
            AgentKind::Claude => AgentConfigSpec {
                agent: AgentKind::Claude,
                auth_passthrough_files: &[".credentials.json"],
                extra_home_auth_sources: &[],
                skills_entries: &["skills", "commands", "agents", "plugins"],
                instructions_files: &["CLAUDE.md"],
                mcp_config_file: ".mcp.json",
                mcp_format: ConfigFormat::Json,
                mcp_root_key: "mcpServers",
                settings_files: &["settings.json", "settings.local.json"],
            },
            AgentKind::Codex => AgentConfigSpec {
                agent: AgentKind::Codex,
                auth_passthrough_files: &["auth.json"],
                extra_home_auth_sources: &[],
                skills_entries: &["skills"],
                instructions_files: &["AGENTS.md"],
                mcp_config_file: "config.toml",
                mcp_format: ConfigFormat::Toml,
                mcp_root_key: "mcp_servers",
                settings_files: &["config.toml"],
            },
            AgentKind::Copilot => AgentConfigSpec {
                agent: AgentKind::Copilot,
                auth_passthrough_files: &["auth.json"],
                extra_home_auth_sources: &[(".config/gh/hosts.yml", "hosts.yml")],
                skills_entries: &["skills", "agents"],
                instructions_files: &["copilot-instructions.md"],
                mcp_config_file: "mcp-config.json",
                mcp_format: ConfigFormat::Json,
                mcp_root_key: "mcpServers",
                settings_files: &["config.json"],
            },
            AgentKind::Agy => AgentConfigSpec {
                agent: AgentKind::Agy,
                auth_passthrough_files: &["oauth_creds.json", "google_accounts.json"],
                extra_home_auth_sources: &[
                    (".gemini/oauth_creds.json", "oauth_creds.json"),
                    (".gemini/google_accounts.json", "google_accounts.json"),
                ],
                skills_entries: &["skills"],
                instructions_files: &["GEMINI.md"],
                mcp_config_file: "mcp_config.json",
                mcp_format: ConfigFormat::Json,
                mcp_root_key: "mcpServers",
                settings_files: &["settings.json"],
            },
        }
    }
}

impl fmt::Display for AgentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Serialization format for an agent's structured config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Json,
    Toml,
}

/// Static layout specification for one agent's native config directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentConfigSpec {
    pub agent: AgentKind,
    /// Auth files relative to `<home>/<default_home_rel_path>/` that are passed through locally.
    pub auth_passthrough_files: &'static [&'static str],
    /// Additional `(home_rel_source, session_rel_dest)` local auth pass-throughs (e.g. `~/.config/gh/hosts.yml`).
    pub extra_home_auth_sources: &'static [(&'static str, &'static str)],
    /// Directories/entries belonging to the `skills` sync category.
    pub skills_entries: &'static [&'static str],
    /// Instruction files belonging to the `instructions` sync category.
    pub instructions_files: &'static [&'static str],
    /// Filename holding MCP server configuration.
    pub mcp_config_file: &'static str,
    /// Format of `mcp_config_file`.
    pub mcp_format: ConfigFormat,
    /// Root table/object key holding MCP servers (`mcpServers` or `mcp_servers`).
    pub mcp_root_key: &'static str,
    /// Non-secret settings files belonging to the `settings` sync category.
    pub settings_files: &'static [&'static str],
}

/// Config sync category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncCategory {
    Skills,
    Instructions,
    Mcp,
    Settings,
}

/// How host-synced skills combine with machine-local skills when `skills` sync is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillsMode {
    /// Union local skills with host skills; host wins collisions.
    #[default]
    HostWins,
    /// Expose only host-synced skills.
    HostOnly,
}

fn default_true() -> bool {
    true
}

/// Per-category `[sync]` toggles.
///
/// Defaults match FUSEFS §8.2: `skills` and `instructions` ON; `mcp` and `settings` OFF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncToggles {
    #[serde(default = "default_true")]
    pub skills: bool,
    #[serde(default = "default_true")]
    pub instructions: bool,
    #[serde(default)]
    pub mcp: bool,
    #[serde(default)]
    pub settings: bool,
}

impl Default for SyncToggles {
    fn default() -> Self {
        Self {
            skills: true,
            instructions: true,
            mcp: false,
            settings: false,
        }
    }
}

impl SyncToggles {
    pub const fn all_off() -> Self {
        Self {
            skills: false,
            instructions: false,
            mcp: false,
            settings: false,
        }
    }

    pub const fn is_enabled(&self, category: SyncCategory) -> bool {
        match category {
            SyncCategory::Skills => self.skills,
            SyncCategory::Instructions => self.instructions,
            SyncCategory::Mcp => self.mcp,
            SyncCategory::Settings => self.settings,
        }
    }
}

/// Optional per-machine override under `[sync.machines."<name>"]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSyncOverride {
    #[serde(default, alias = "sync_enabled")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub skills: Option<bool>,
    #[serde(default)]
    pub instructions: Option<bool>,
    #[serde(default)]
    pub mcp: Option<bool>,
    #[serde(default)]
    pub settings: Option<bool>,
    #[serde(default)]
    pub skills_mode: Option<SkillsMode>,
}

/// Resolved sync policy for a specific machine after applying the global rage-quit
/// switch and any per-machine override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveSyncPolicy {
    pub sync_enabled: bool,
    pub toggles: SyncToggles,
    pub skills_mode: SkillsMode,
}

impl EffectiveSyncPolicy {
    pub const fn category_synced(&self, category: SyncCategory) -> bool {
        self.sync_enabled && self.toggles.is_enabled(category)
    }
}

/// Top-level `[sync]` configuration including the master **rage-quit switch**
/// (`sync_enabled`), default category toggles, and per-machine overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Master switch (`enabled` in TOML). Setting `false` triggers an instant
    /// non-destructive rage-quit across all categories.
    #[serde(default = "default_true", alias = "enabled")]
    pub sync_enabled: bool,
    #[serde(flatten)]
    pub toggles: SyncToggles,
    #[serde(default)]
    pub skills_mode: SkillsMode,
    #[serde(default)]
    pub machines: BTreeMap<String, MachineSyncOverride>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            sync_enabled: true,
            toggles: SyncToggles::default(),
            skills_mode: SkillsMode::default(),
            machines: BTreeMap::new(),
        }
    }
}

impl SyncConfig {
    /// Parses a `[sync]` table from a full `config-sync.toml` document.
    pub fn from_toml_str(input: &str) -> Result<Self, toml::de::Error> {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default)]
            sync: SyncConfig,
        }
        let wrapper: Wrapper = toml::from_str(input)?;
        Ok(wrapper.sync)
    }

    /// Instant non-destructive rage-quit: turns off host config sync globally.
    pub fn rage_quit(&mut self) {
        self.sync_enabled = false;
    }

    /// Re-enables host config sync globally.
    pub fn enable_sync(&mut self) {
        self.sync_enabled = true;
    }

    /// Resolves the effective sync policy for `machine_name`.
    pub fn effective_for_machine(&self, machine_name: &str) -> EffectiveSyncPolicy {
        if !self.sync_enabled {
            return EffectiveSyncPolicy {
                sync_enabled: false,
                toggles: SyncToggles::all_off(),
                skills_mode: self.skills_mode,
            };
        }

        let machine_override = self.machines.get(machine_name).or_else(|| {
            self.machines
                .iter()
                .find(|(pattern, _)| glob_match(pattern, machine_name))
                .map(|(_, v)| v)
        });

        let Some(ov) = machine_override else {
            return EffectiveSyncPolicy {
                sync_enabled: true,
                toggles: self.toggles,
                skills_mode: self.skills_mode,
            };
        };

        if ov.enabled == Some(false) {
            return EffectiveSyncPolicy {
                sync_enabled: false,
                toggles: SyncToggles::all_off(),
                skills_mode: ov.skills_mode.unwrap_or(self.skills_mode),
            };
        }

        EffectiveSyncPolicy {
            sync_enabled: true,
            toggles: SyncToggles {
                skills: ov.skills.unwrap_or(self.toggles.skills),
                instructions: ov.instructions.unwrap_or(self.toggles.instructions),
                mcp: ov.mcp.unwrap_or(self.toggles.mcp),
                settings: ov.settings.unwrap_or(self.toggles.settings),
            },
            skills_mode: ov.skills_mode.unwrap_or(self.skills_mode),
        }
    }
}

/// Denylist of auth/token/credential filenames and structured config keys.
///
/// Used to guarantee that sanitized `s/<id>/config/<agent>` FUSE views and
/// exported `ConfigBundle`s NEVER expose secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretDenylist {
    denied_filenames: BTreeSet<String>,
    denied_extensions: BTreeSet<String>,
    denied_exact_keys: BTreeSet<String>,
    denied_key_suffixes: Vec<String>,
}

impl Default for SecretDenylist {
    fn default() -> Self {
        let denied_filenames = [
            ".credentials.json",
            "credentials.json",
            "auth.json",
            "hosts.yml",
            "hosts.yaml",
            "oauth_creds.json",
            "google_accounts.json",
            "application_default_credentials.json",
            ".netrc",
            "id_rsa",
            "id_ed25519",
            "id_ecdsa",
            "credentials",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();

        let denied_extensions = ["pem", "key", "p12", "pfx"]
            .into_iter()
            .map(str::to_string)
            .collect();

        let denied_exact_keys = [
            "access_token",
            "refresh_token",
            "id_token",
            "oauth_token",
            "auth_token",
            "bearer_token",
            "session_token",
            "token",
            "api_key",
            "apikey",
            "client_secret",
            "secret",
            "password",
            "passwd",
            "private_key",
            "authorization",
            "cookie",
            "x-api-key",
            "x_api_key",
            "anthropic_api_key",
            "openai_api_key",
            "github_token",
            "gh_token",
            "gemini_api_key",
            "claude_code_oauth_token",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();

        let denied_key_suffixes = [
            "_token",
            "_secret",
            "_password",
            "_passwd",
            "_api_key",
            "_apikey",
            "_access_key",
            "_private_key",
            "_auth_key",
            "_credential",
            "_credentials",
            "-token",
            "-secret",
            "-api-key",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();

        Self {
            denied_filenames,
            denied_extensions,
            denied_exact_keys,
            denied_key_suffixes,
        }
    }
}

impl SecretDenylist {
    /// Adds extra denied filenames (matched case-insensitively against file basename).
    pub fn with_extra_filenames<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for name in names {
            self.denied_filenames
                .insert(name.into().to_ascii_lowercase());
        }
        self
    }

    /// Adds extra denied JSON/TOML key names (matched case-insensitively).
    pub fn with_extra_keys<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for key in keys {
            self.denied_exact_keys
                .insert(key.into().to_ascii_lowercase());
        }
        self
    }

    /// Returns `true` if `path` refers to a `Secret`-classified file that must
    /// never be exposed in FUSE or synced across machines.
    pub fn is_secret_path(&self, path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        let lower = name.to_ascii_lowercase();
        if self.denied_filenames.contains(&lower) {
            return true;
        }
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if self.denied_extensions.contains(&ext.to_ascii_lowercase()) {
                return true;
            }
        }
        false
    }

    /// Returns `true` if `key` is a secret JSON/TOML key name (or env var / HTTP header).
    pub fn is_secret_key(&self, key: &str) -> bool {
        let lower = key.to_ascii_lowercase();
        if self.denied_exact_keys.contains(&lower) {
            return true;
        }
        self.denied_key_suffixes
            .iter()
            .any(|suffix| lower.ends_with(suffix))
    }

    /// Recursively strips all denied keys from a JSON value.
    pub fn scrub_json(&self, value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut out = serde_json::Map::new();
                for (k, v) in map {
                    if self.is_secret_key(k) {
                        continue;
                    }
                    out.insert(k.clone(), self.scrub_json(v));
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(|v| self.scrub_json(v)).collect())
            }
            other => other.clone(),
        }
    }

    /// Recursively strips all denied keys from a TOML value.
    pub fn scrub_toml(&self, value: &toml::Value) -> toml::Value {
        match value {
            toml::Value::Table(table) => {
                let mut out = toml::map::Map::new();
                for (k, v) in table {
                    if self.is_secret_key(k) {
                        continue;
                    }
                    out.insert(k.clone(), self.scrub_toml(v));
                }
                toml::Value::Table(out)
            }
            toml::Value::Array(items) => {
                toml::Value::Array(items.iter().map(|v| self.scrub_toml(v)).collect())
            }
            other => other.clone(),
        }
    }
}

/// Simple `*` and `?` wildcard matcher for machine name patterns (e.g. `"gpu-*"`).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_idx: Option<usize> = None;
    let mut match_idx: usize = 0;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_idx = Some(pi);
            match_idx = ti;
            pi += 1;
        } else if let Some(si) = star_idx {
            pi = si + 1;
            match_idx += 1;
            ti = match_idx;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }

    pi == p.len()
}
