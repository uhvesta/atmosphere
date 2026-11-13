//! The plugins Claude Code and Copilot have installed on a host, read from
//! their own records, with what each adds. Installing and removing stays in
//! each CLI; Humata only turns them on or off for a project's sessions.

use std::path::{Path, PathBuf};

use humata_wire::pb;
use serde_json::Value;

/// A manifest or record larger than this is left out, as it can't be one.
const LARGEST: u64 = 4 << 20;

/// Every plugin installed for the agents whose folders are `claude` (as
/// `~/.claude`) and `copilot` (as `~/.copilot`); a folder that isn't there
/// has none.
pub(crate) fn scan(claude: Option<&Path>, copilot: Option<&Path>) -> Vec<pb::FoundPlugin> {
    let mut found = Vec::new();
    if let Some(claude) = claude {
        found.extend(claude_plugins(claude));
    }
    if let Some(copilot) = copilot {
        found.extend(copilot_plugins(copilot));
    }
    found.sort_by(|a, b| (a.agent, &a.name, &a.id).cmp(&(b.agent, &b.name, &b.id)));
    found
}

/// Where each agent keeps its settings on this host: `CLAUDE_CONFIG_DIR`
/// and `COPILOT_HOME` if set, else under the home folder.
pub(crate) fn folders() -> (Option<PathBuf>, Option<PathBuf>) {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let folder = |variable: &str, under: &str| {
        std::env::var_os(variable)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(under)))
    };
    (
        folder("CLAUDE_CONFIG_DIR", ".claude"),
        folder("COPILOT_HOME", ".copilot"),
    )
}

/// `installed_plugins.json` lists them, by `name@marketplace`, and
/// `settings.json`'s `enabledPlugins` has those that are on.
fn claude_plugins(folder: &Path) -> Vec<pb::FoundPlugin> {
    let Some(installed) = read_json(&folder.join("plugins/installed_plugins.json")) else {
        return Vec::new();
    };
    let enabled = read_json(&folder.join("settings.json"))
        .and_then(|settings| settings.get("enabledPlugins").cloned())
        .unwrap_or(Value::Null);
    let Some(plugins) = installed.get("plugins").and_then(Value::as_object) else {
        return Vec::new();
    };
    plugins
        .iter()
        .filter_map(|(id, installs)| {
            // One install per scope; the person's own, else the first.
            let installs = installs.as_array()?;
            let install = installs
                .iter()
                .find(|install| install.get("scope").and_then(Value::as_str) == Some("user"))
                .or_else(|| installs.first())?;
            let root = PathBuf::from(install.get("installPath")?.as_str()?);
            let manifest =
                read_json(&root.join(".claude-plugin/plugin.json")).unwrap_or(Value::Null);
            let mut plugin = described(
                pb::PluginAgent::Claude,
                id,
                id.split('@').next().unwrap_or(id),
                &root,
                &manifest,
            );
            if plugin.version.is_empty() {
                plugin.version = text(install, "version");
            }
            plugin.enabled = enabled.get(id).and_then(Value::as_bool).unwrap_or(false);
            Some(plugin)
        })
        .collect()
}

/// `config.json`'s `installedPlugins` lists them, each with its folder and
/// whether it's on.
fn copilot_plugins(folder: &Path) -> Vec<pb::FoundPlugin> {
    let Some(config) = read_json(&folder.join("config.json")) else {
        return Vec::new();
    };
    let Some(installed) = config.get("installedPlugins").and_then(Value::as_array) else {
        return Vec::new();
    };
    installed
        .iter()
        .filter_map(|install| {
            let name = install.get("name")?.as_str()?;
            let marketplace = text(install, "marketplace");
            let id = if marketplace.is_empty() {
                name.to_owned()
            } else {
                format!("{name}@{marketplace}")
            };
            let root = PathBuf::from(install.get("cache_path")?.as_str()?);
            let manifest = [
                "plugin.json",
                ".plugin/plugin.json",
                ".github/plugin/plugin.json",
                ".claude-plugin/plugin.json",
            ]
            .iter()
            .find_map(|path| read_json(&root.join(path)))
            .unwrap_or(Value::Null);
            let mut plugin = described(pb::PluginAgent::Copilot, &id, name, &root, &manifest);
            if plugin.version.is_empty() {
                plugin.version = text(install, "version");
            }
            plugin.enabled = install
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            Some(plugin)
        })
        .collect()
}

/// The plugin in `root`, as its manifest describes it, with what it adds:
/// in the folders and files the manifest names, else the usual ones.
fn described(
    agent: pb::PluginAgent,
    id: &str,
    name: &str,
    root: &Path,
    manifest: &Value,
) -> pb::FoundPlugin {
    let named = text(manifest, "name");
    let folders = |key: &str, usual: &[&str]| -> Vec<PathBuf> {
        match manifest.get(key) {
            Some(Value::String(path)) => vec![root.join(path)],
            Some(Value::Array(paths)) => paths
                .iter()
                .filter_map(Value::as_str)
                .map(|path| root.join(path))
                .collect(),
            _ => usual.iter().map(|path| root.join(path)).collect(),
        }
    };
    let skills = names(&folders("skills", &["skills"]), |entry| {
        entry.is_dir() && entry.join("SKILL.md").is_file()
    });
    let commands = names(
        &folders("commands", &["commands", "com.github.copilot/commands"]),
        |entry| entry.extension().is_some_and(|extension| extension == "md"),
    );
    let agents = names(
        &folders("agents", &["agents", "com.github.copilot/agents"]),
        |entry| entry.extension().is_some_and(|extension| extension == "md"),
    );
    let mcp_servers = match manifest.get("mcpServers") {
        Some(Value::Object(servers)) => servers.keys().cloned().collect(),
        Some(Value::String(path)) => servers(&root.join(path)),
        _ => [".mcp.json", "mcp.json", ".github/mcp.json"]
            .iter()
            .map(|path| servers(&root.join(path)))
            .find(|found| !found.is_empty())
            .unwrap_or_default(),
    };
    let hooks = manifest.get("hooks").is_some()
        || [
            "hooks/hooks.json",
            "hooks.json",
            "com.github.copilot/hooks/hooks.json",
        ]
        .iter()
        .any(|path| root.join(path).is_file());
    let lsp_servers = manifest.get("lspServers").is_some()
        || [".lsp.json", "lsp.json", ".github/lsp.json"]
            .iter()
            .any(|path| root.join(path).is_file());
    pb::FoundPlugin {
        agent: agent as i32,
        id: id.to_owned(),
        name: if named.is_empty() {
            name.to_owned()
        } else {
            named
        },
        description: text(manifest, "description"),
        version: text(manifest, "version"),
        host: String::new(),
        enabled: false,
        skills,
        commands,
        agents,
        mcp_servers,
        hooks,
        lsp_servers,
    }
}

/// The names of what's in `folders` that `keep` keeps, without `.md` or
/// `.agent.md`, sorted.
fn names(folders: &[PathBuf], keep: impl Fn(&Path) -> bool) -> Vec<String> {
    let mut found: Vec<String> = folders
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flat_map(|entries| entries.flatten().map(|entry| entry.path()))
        .filter(|entry| keep(entry))
        .filter_map(|entry| {
            let name = entry.file_name()?.to_str()?;
            let name = name
                .strip_suffix(".agent.md")
                .or_else(|| name.strip_suffix(".md"))
                .unwrap_or(name);
            Some(name.to_owned())
        })
        .collect();
    found.sort();
    found.dedup();
    found
}

/// The MCP servers a config file names.
fn servers(path: &Path) -> Vec<String> {
    read_json(path)
        .and_then(|config| {
            config
                .get("mcpServers")
                .and_then(Value::as_object)
                .map(|servers| servers.keys().cloned().collect())
        })
        .unwrap_or_default()
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The JSON in `path`, past any lines of `//` comments, as Copilot writes
/// at the top of its config; None if it isn't there or isn't JSON.
fn read_json(path: &Path) -> Option<Value> {
    if std::fs::metadata(path).ok()?.len() > LARGEST {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let body: String = text
        .lines()
        .skip_while(|line| line.trim_start().starts_with("//") || line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("folder");
        std::fs::write(path, text).expect("written");
    }

    #[test]
    fn claude_plugins_are_read_with_what_they_add_and_whether_they_are_on() {
        let home = tempfile::tempdir().expect("temp");
        let claude = home.path().join(".claude");
        let root = home.path().join("cache/review/1.2.0");
        write(
            &claude.join("plugins/installed_plugins.json"),
            &serde_json::json!({"version": 2, "plugins": {
                "review@market": [{"scope": "user", "installPath": root, "version": "1.2.0"}],
                "gone@market": [{"scope": "user", "installPath": home.path().join("nowhere")}],
            }})
            .to_string(),
        );
        write(
            &claude.join("settings.json"),
            r#"{"enabledPlugins": {"review@market": true, "gone@market": false}}"#,
        );
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name": "review", "description": "Reviews diffs"}"#,
        );
        write(
            &root.join("skills/triage/SKILL.md"),
            "---\nname: triage\n---\n",
        );
        write(&root.join("commands/review.md"), "Review it");
        write(&root.join("agents/critic.md"), "Be critical");
        write(&root.join("hooks/hooks.json"), "{}");
        write(&root.join(".mcp.json"), r#"{"mcpServers": {"github": {}}}"#);

        let found = scan(Some(&claude), None);

        let review = found
            .iter()
            .find(|plugin| plugin.id == "review@market")
            .expect("review");
        assert_eq!(review.agent(), pb::PluginAgent::Claude);
        assert_eq!(review.name, "review");
        assert_eq!(review.description, "Reviews diffs");
        assert_eq!(review.version, "1.2.0");
        assert!(review.enabled);
        assert_eq!(review.skills, ["triage"]);
        assert_eq!(review.commands, ["review"]);
        assert_eq!(review.agents, ["critic"]);
        assert_eq!(review.mcp_servers, ["github"]);
        assert!(review.hooks);
        let gone = found
            .iter()
            .find(|plugin| plugin.id == "gone@market")
            .expect("gone");
        assert!(!gone.enabled);
        assert!(gone.skills.is_empty());
    }

    #[test]
    fn copilot_plugins_are_read_past_the_config_comments() {
        let home = tempfile::tempdir().expect("temp");
        let copilot = home.path().join(".copilot");
        let root = copilot.join("installed-plugins/_direct/notes-1");
        write(
            &copilot.join("config.json"),
            &format!(
                "// User settings belong in settings.json.\n// This file is managed automatically.\n{}",
                serde_json::json!({"installedPlugins": [
                    {"name": "notes", "marketplace": "", "version": "0.1.0", "cache_path": root, "enabled": false},
                    {"name": "lint", "marketplace": "awesome", "cache_path": home.path().join("lint")},
                ]})
            ),
        );
        write(
            &root.join("plugin.json"),
            r#"{"name": "notes", "mcpServers": {"notes": {}}}"#,
        );
        write(&root.join("skills/keep-notes/SKILL.md"), "Keep notes");
        write(&root.join("agents/scribe.agent.md"), "Write it down");

        let found = scan(None, Some(&copilot));

        let notes = found
            .iter()
            .find(|plugin| plugin.id == "notes")
            .expect("notes");
        assert_eq!(notes.agent(), pb::PluginAgent::Copilot);
        assert!(!notes.enabled);
        assert_eq!(notes.version, "0.1.0");
        assert_eq!(notes.skills, ["keep-notes"]);
        assert_eq!(notes.agents, ["scribe"]);
        assert_eq!(notes.mcp_servers, ["notes"]);
        let lint = found
            .iter()
            .find(|plugin| plugin.id == "lint@awesome")
            .expect("lint");
        assert!(lint.enabled);
    }

    #[test]
    fn no_folders_have_no_plugins() {
        let home = tempfile::tempdir().expect("temp");
        assert!(
            scan(
                Some(&home.path().join(".claude")),
                Some(&home.path().join(".copilot"))
            )
            .is_empty()
        );
    }
}
