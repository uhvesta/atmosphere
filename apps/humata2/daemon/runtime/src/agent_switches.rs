//! AGENTSET (D720, D722): which agent kinds Humata offers, and whether it
//! installs a kind's CLI when the machine doesn't have it. D782 adds whether
//! a kind's permission and question asks go to the Inbox. The switches are
//! the host's `~/.humata2/config/agents.toml`, which ships to machines the
//! way `repos.toml` does. A missing file means every kind is enabled and
//! nothing is installed. A bad file is an error; the caller decides what to
//! do with it.
//!
//! The file is edited as text by [`set`], so the comments and the other
//! kinds' lines stay as they were; the edited text is parsed again before it
//! is written, so a file this module wrote always reads back.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;
use thiserror::Error;

/// The file's name, in the `config` folder of the humata home.
pub const FILE: &str = "agents.toml";
/// The folder under the humata home, as `stack.toml` has it.
const FOLDER: &str = "config";
/// The plain shell: not an agent, so always on and never a switch.
const ZSH: &str = "zsh";

/// `<humata home>/config/agents.toml`, where the humata home is `~/.humata2`.
pub fn host_path(home: &Path) -> PathBuf {
    home.join(FOLDER).join(FILE)
}

/// Why the agent switches couldn't be read, or changed.
#[derive(Debug, Error)]
pub enum SwitchesError {
    #[error(
        "couldn't read the agent switches file {}: {source}. Check that you can read it, or delete it to go back to every agent enabled, then try again",
        .path.display()
    )]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "{what} isn't valid TOML: {source}. Fix the line it points to (a switch is an [agents.<kind>] table with enabled, install_when_missing, install and asks_in_inbox), or delete the file to go back to every agent enabled. Humata leaves the file as it is"
    )]
    Parse {
        /// Names the file, or says it is the text of a switch change.
        what: String,
        #[source]
        source: toml::de::Error,
    },
    #[error(
        "the agent kind `{kind}` can't be used as a switch name: use only letters, digits, '-' and '_' (for example `my-agent`). Rename it to that, then try again"
    )]
    BadKind { kind: String },
    #[error(
        "couldn't save the agent switches to {}: {source}. Check that its folder is writable and has room, then try again. The file was not changed",
        .path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// One `[agents.<kind>]` table, as the file has it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct KindSwitch {
    enabled: Option<bool>,
    install_when_missing: Option<bool>,
    install: Option<String>,
    asks_in_inbox: Option<bool>,
}

/// The whole file: only the `agents` table.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileDoc {
    #[serde(default)]
    agents: BTreeMap<String, KindSwitch>,
}

/// The switches the file sets. A kind it doesn't mention has the defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentSwitches {
    kinds: BTreeMap<String, KindSwitch>,
}

impl AgentSwitches {
    /// The switches in the file at `path`. A missing file gives the defaults.
    pub fn read(path: &Path) -> Result<Self, SwitchesError> {
        match file_text(path)? {
            None => Ok(Self::default()),
            Some(text) => parse_named(&text, &file_label(path)),
        }
    }

    /// The switches in `text`, the contents of the file.
    pub fn parse(text: &str) -> Result<Self, SwitchesError> {
        parse_named(text, "the agent switches")
    }

    /// Whether the kind is offered. Every kind is, unless the file turns it
    /// off; the plain shell always is.
    pub fn enabled(&self, kind: &str) -> bool {
        if kind == ZSH {
            return true;
        }
        self.entry(kind)
            .and_then(|entry| entry.enabled)
            .unwrap_or(true)
    }

    /// The agent a start that named none lands on: the first of `preferred`
    /// that is on, else the first of `order` that is on and isn't the plain
    /// shell. The shell only counts when it is in `preferred`. `None` when
    /// every candidate is off.
    pub fn pick<'a>(&self, preferred: &[&'a str], order: &[&'a str]) -> Option<&'a str> {
        preferred
            .iter()
            .copied()
            .find(|kind| self.enabled(kind))
            .or_else(|| order.iter().copied().find(|kind| *kind != ZSH && self.enabled(kind)))
    }

    /// Whether Humata installs the kind's CLI on a machine that doesn't have
    /// it. Off unless the file turns it on, and off for a kind without an
    /// install command.
    pub fn install_when_missing(&self, kind: &str) -> bool {
        self.install_command(kind).is_some()
            && self
                .entry(kind)
                .and_then(|entry| entry.install_when_missing)
                .unwrap_or(false)
    }

    /// The command that installs the kind's CLI. A built-in kind has its own
    /// command, which the file can't change; any other kind uses the file's
    /// `install`. The plain shell has none.
    pub fn install_command(&self, kind: &str) -> Option<String> {
        if kind == ZSH {
            return None;
        }
        if let Some(command) = builtin_install_command(kind) {
            return Some(command.to_owned());
        }
        self.entry(kind)
            .and_then(|entry| entry.install.clone())
            .filter(|command| !command.trim().is_empty())
    }

    /// Whether the kind's permission and question asks wait in the Inbox
    /// (D782). On unless the file turns it off; the plain shell asks nothing,
    /// so it reads as on.
    pub fn asks_in_inbox(&self, kind: &str) -> bool {
        if kind == ZSH {
            return true;
        }
        self.entry(kind)
            .and_then(|entry| entry.asks_in_inbox)
            .unwrap_or(true)
    }

    fn entry(&self, kind: &str) -> Option<&KindSwitch> {
        self.kinds.get(kind)
    }
}

/// The install command Humata has for a built-in kind (D722).
pub fn builtin_install_command(kind: &str) -> Option<&'static str> {
    match kind {
        "claude" => Some("curl -fsSL https://claude.ai/install.sh | bash"),
        "codex" => Some("npm install -g @openai/codex"),
        "copilot" => Some("npm install -g @github/copilot"),
        "opencode" => Some("curl -fsSL https://opencode.ai/install | bash"),
        "agy" => Some("curl -fsSL https://antigravity.google/cli/install.sh | bash"),
        _ => None,
    }
}

/// What to look for to know a kind's CLI is on the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliToFind {
    /// A command name, looked up on the login PATH.
    Name(String),
    /// A program path, which must be an executable file.
    Path(PathBuf),
}

/// What to look for to know a kind's CLI is on the machine. opencode runs
/// through `/usr/bin/env humata opencode`, so it looks for `opencode`. Any
/// other `env` wrapper looks for the first word of its arguments that is
/// neither an assignment nor an option.
pub fn cli_to_find(kind: &str, program: &Path, start_args: &[String]) -> CliToFind {
    if kind == "opencode" {
        return CliToFind::Name(kind.to_owned());
    }
    if is_env_wrapper(program) {
        if kind == "codex" {
            return CliToFind::Name(kind.to_owned());
        }
        if let Some(name) = start_args
            .iter()
            .find(|arg| !arg.contains('=') && !arg.starts_with('-'))
        {
            return CliToFind::Name(name.clone());
        }
    }
    let text = program.to_string_lossy();
    if text.contains('/') {
        CliToFind::Path(program.to_path_buf())
    } else {
        CliToFind::Name(text.into_owned())
    }
}

fn is_env_wrapper(program: &Path) -> bool {
    let text = program.to_string_lossy();
    text.contains('/') && text.ends_with("/env")
}

/// Something to run after the switches file changes, such as pushing it to
/// the linked machines (D782). Set once, at start; telling it before then
/// does nothing.
#[derive(Clone, Default)]
pub struct OnChange(std::sync::Arc<std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>>);

impl OnChange {
    /// What to run on each change. A second call is ignored.
    pub fn set(&self, run: impl Fn() + Send + Sync + 'static) {
        let _ = self.0.set(Box::new(run));
    }

    /// Runs what was set, if anything.
    pub fn tell(&self) {
        if let Some(run) = self.0.get() {
            run();
        }
    }
}

/// A change to one kind's switches. A field left `None` is not touched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Change {
    pub enabled: Option<bool>,
    pub install_when_missing: Option<bool>,
    pub asks_in_inbox: Option<bool>,
}

/// Applies `change` to the `[agents.<kind>]` table in the file at `path`, and
/// gives the switches after it. Creates the file and its folder when missing.
/// Every other line and comment stays as it is. A bad file is an error and is
/// left alone.
pub fn set(path: &Path, kind: &str, change: Change) -> Result<AgentSwitches, SwitchesError> {
    if !valid_kind(kind) {
        return Err(SwitchesError::BadKind {
            kind: kind.to_owned(),
        });
    }
    let text = file_text(path)?.unwrap_or_default();
    let label = file_label(path);
    let current = parse_named(&text, &label)?;
    let updated = edit_text(&text, kind, change);
    if updated == text {
        return Ok(current);
    }
    let switches = parse_named(&updated, &label)?;
    write_atomic(path, &updated)?;
    Ok(switches)
}

fn valid_kind(kind: &str) -> bool {
    !kind.is_empty()
        && kind
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn file_label(path: &Path) -> String {
    format!("the agent switches file {}", path.display())
}

fn file_text(path: &Path) -> Result<Option<String>, SwitchesError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(SwitchesError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn parse_named(text: &str, what: &str) -> Result<AgentSwitches, SwitchesError> {
    let doc: FileDoc = toml::from_str(text).map_err(|source| SwitchesError::Parse {
        what: what.to_owned(),
        source,
    })?;
    if let Some(kind) = doc.agents.keys().find(|kind| !valid_kind(kind)) {
        return Err(SwitchesError::BadKind { kind: kind.clone() });
    }
    Ok(AgentSwitches { kinds: doc.agents })
}

fn edit_text(text: &str, kind: &str, change: Change) -> String {
    let mut wanted: Vec<(&str, bool)> = Vec::new();
    if let Some(on) = change.enabled {
        wanted.push(("enabled", on));
    }
    if let Some(on) = change.install_when_missing {
        wanted.push(("install_when_missing", on));
    }
    if let Some(on) = change.asks_in_inbox {
        wanted.push(("asks_in_inbox", on));
    }
    if wanted.is_empty() {
        return text.to_owned();
    }

    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    match lines.iter().position(|line| is_header_of(line, kind)) {
        Some(header) => {
            let end = lines[header + 1..]
                .iter()
                .position(|line| is_table_header(line))
                .map_or(lines.len(), |offset| header + 1 + offset);
            let mut missing = Vec::new();
            for (key, on) in wanted {
                match (header + 1..end).find(|&i| key_of(&lines[i]) == Some(key)) {
                    Some(i) => lines[i] = with_value(&lines[i], key, on),
                    None => missing.push(format!("{key} = {on}")),
                }
            }
            for (offset, line) in missing.into_iter().enumerate() {
                lines.insert(header + 1 + offset, line);
            }
        }
        None => {
            if lines.last().is_some_and(|line| !line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("[agents.{kind}]"));
            lines.extend(wanted.iter().map(|(key, on)| format!("{key} = {on}")));
        }
    }
    let mut out = lines.join(eol);
    out.push_str(eol);
    out
}

fn is_header_of(line: &str, kind: &str) -> bool {
    let code = match line.find('#') {
        Some(at) => &line[..at],
        None => line,
    };
    let compact: String = code.split_whitespace().collect();
    compact == format!("[agents.{kind}]")
}

fn is_table_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

fn key_of(line: &str) -> Option<&str> {
    let code = match line.find('#') {
        Some(at) => &line[..at],
        None => line,
    };
    if is_table_header(code) {
        return None;
    }
    let (key, _) = code.split_once('=')?;
    Some(key.trim())
}

fn with_value(line: &str, key: &str, on: bool) -> String {
    let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    match line.find('#') {
        Some(at) => format!("{indent}{key} = {on} {}", &line[at..]),
        None => format!("{indent}{key} = {on}"),
    }
}

fn write_atomic(path: &Path, text: &str) -> Result<(), SwitchesError> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let folder = match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    fs::create_dir_all(folder).map_err(|source| SwitchesError::Write {
        path: path.to_path_buf(),
        source,
    })?;
    let tmp = folder.join(format!(
        ".{FILE}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    result.map_err(|source| {
        let _ = fs::remove_file(&tmp);
        SwitchesError::Write {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_file(dir: &Path, text: &str) -> PathBuf {
        let path = dir.join(FILE);
        fs::write(&path, text).expect("write the test file");
        path
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn host_path_is_the_config_file_under_the_home() {
        assert_eq!(
            host_path(Path::new("/home/me/.humata2")),
            PathBuf::from("/home/me/.humata2/config/agents.toml")
        );
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().expect("temp dir");
        let switches = AgentSwitches::read(&dir.path().join(FILE)).expect("a missing file is fine");
        assert_eq!(switches, AgentSwitches::default());
        assert!(switches.enabled("copilot"));
        assert!(!switches.install_when_missing("claude"));
        assert_eq!(
            switches.install_command("claude").as_deref(),
            builtin_install_command("claude")
        );
        assert!(switches.enabled("agy"));
        assert!(!switches.install_when_missing("agy"));
        assert_eq!(
            switches.install_command("agy").as_deref(),
            Some("curl -fsSL https://antigravity.google/cli/install.sh | bash")
        );
    }

    #[test]
    fn a_disabled_kind_is_off_and_the_others_stay_on() {
        let switches =
            AgentSwitches::parse("[agents.copilot]\nenabled = false\n").expect("valid file");
        assert!(!switches.enabled("copilot"));
        assert!(switches.enabled("codex"));
    }

    #[test]
    fn zsh_is_always_on() {
        let switches = AgentSwitches::parse("[agents.zsh]\nenabled = false\n").expect("valid file");
        assert!(switches.enabled(ZSH));
        assert!(!switches.install_when_missing(ZSH));
        assert_eq!(switches.install_command(ZSH), None);
    }

    #[test]
    fn install_is_off_unless_turned_on_and_the_kind_has_a_command() {
        let switches = AgentSwitches::parse(
            "[agents.codex]\ninstall_when_missing = true\n\n[agents.my-agent]\ninstall_when_missing = true\n",
        )
        .expect("valid file");
        assert!(switches.install_when_missing("codex"));
        assert!(!switches.install_when_missing("claude"));
        assert!(!switches.install_when_missing("my-agent"));
    }

    #[test]
    fn the_file_cannot_override_a_builtin_install_command() {
        let switches = AgentSwitches::parse("[agents.claude]\ninstall = \"echo replaced\"\n")
            .expect("valid file");
        assert_eq!(
            switches.install_command("claude").as_deref(),
            builtin_install_command("claude")
        );
    }

    #[test]
    fn a_custom_kind_uses_the_install_command_in_the_file() {
        let switches = AgentSwitches::parse(
            "[agents.my-agent]\ninstall = \"npm install -g my-agent\"\ninstall_when_missing = true\n",
        )
        .expect("valid file");
        assert_eq!(
            switches.install_command("my-agent").as_deref(),
            Some("npm install -g my-agent")
        );
        assert!(switches.install_when_missing("my-agent"));
    }

    #[test]
    fn cli_to_find_names_the_cli_behind_each_program_shape() {
        assert_eq!(
            cli_to_find(
                "opencode",
                Path::new("/usr/bin/env"),
                &args(&["humata", "opencode"])
            ),
            CliToFind::Name("opencode".to_owned())
        );
        assert_eq!(
            cli_to_find(
                "codex",
                Path::new("/usr/bin/env"),
                &args(&["FOO=1", "-i", "codex", "--x"])
            ),
            CliToFind::Name("codex".to_owned())
        );
        assert_eq!(
            cli_to_find(
                "codex",
                Path::new("/usr/bin/env"),
                &args(&[
                    "HUMATA_SOCKET=/run/h.sock",
                    "/bin/humata",
                    "codex",
                    "--hooks",
                    "{ Stop = [] }",
                    "--"
                ])
            ),
            CliToFind::Name("codex".to_owned())
        );
        assert_eq!(
            cli_to_find(
                "codex",
                Path::new("/bin/sh"),
                &args(&["/rig/cli-agent.sh", "--session-id", "x"])
            ),
            CliToFind::Path(PathBuf::from("/bin/sh"))
        );
        assert_eq!(
            cli_to_find("copilot", Path::new("/opt/bin/copilot"), &args(&[])),
            CliToFind::Path(PathBuf::from("/opt/bin/copilot"))
        );
        assert_eq!(
            cli_to_find("codex", Path::new("codex"), &args(&[])),
            CliToFind::Name("codex".to_owned())
        );
        assert_eq!(
            cli_to_find("agy", Path::new("agy"), &args(&[])),
            CliToFind::Name("agy".to_owned())
        );
    }

    #[test]
    fn set_creates_the_file_and_its_folder() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = host_path(dir.path());
        let change = Change {
            enabled: Some(false),
            ..Change::default()
        };
        let switches = set(&path, "copilot", change).expect("set on a missing file");
        assert!(!switches.enabled("copilot"));
        let reread = AgentSwitches::read(&path).expect("read back");
        assert!(!reread.enabled("copilot"));
    }

    #[test]
    fn set_keeps_comments_and_other_kinds() {
        let dir = tempfile::tempdir().expect("temp dir");
        let text = "# Agents Humata offers.\n[agents.codex]\n# keep me\ninstall_when_missing = true\n\n[agents.copilot]\nenabled = true # on\n";
        let path = with_file(dir.path(), text);
        let change = Change {
            enabled: Some(false),
            ..Change::default()
        };
        let switches = set(&path, "copilot", change).expect("set");
        assert!(!switches.enabled("copilot"));
        assert!(switches.install_when_missing("codex"));

        let written = fs::read_to_string(&path).expect("read the file");
        assert!(written.contains("# Agents Humata offers."));
        assert!(written.contains("# keep me"));
        assert!(written.contains("install_when_missing = true"));
        assert!(written.contains("enabled = false # on"));
    }

    #[test]
    fn set_appends_a_missing_table_and_keeps_the_rest() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = with_file(dir.path(), "# mine\n[agents.codex]\nenabled = false\n");
        let change = Change {
            install_when_missing: Some(true),
            ..Change::default()
        };
        set(&path, "claude", change).expect("set");

        let switches = AgentSwitches::read(&path).expect("read back");
        assert!(switches.install_when_missing("claude"));
        assert!(!switches.enabled("codex"));
        let written = fs::read_to_string(&path).expect("read the file");
        assert!(written.starts_with("# mine\n[agents.codex]\nenabled = false\n"));
    }

    #[test]
    fn toggling_a_kind_twice_does_not_duplicate_its_table() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = host_path(dir.path());
        let off = Change {
            enabled: Some(false),
            ..Change::default()
        };
        let on = Change {
            enabled: Some(true),
            ..Change::default()
        };
        set(&path, "copilot", off).expect("turn off");
        let switches = set(&path, "copilot", on).expect("turn on");
        assert!(switches.enabled("copilot"));
        let written = fs::read_to_string(&path).expect("read the file");
        assert_eq!(written.matches("[agents.copilot]").count(), 1);
    }

    #[test]
    fn a_bad_file_is_an_error_and_set_leaves_it_alone() {
        let dir = tempfile::tempdir().expect("temp dir");
        let text = "[agents.copilot\nenabled = false\n";
        let path = with_file(dir.path(), text);
        assert!(matches!(
            AgentSwitches::read(&path),
            Err(SwitchesError::Parse { .. })
        ));
        let change = Change {
            install_when_missing: Some(true),
            ..Change::default()
        };
        assert!(matches!(
            set(&path, "codex", change),
            Err(SwitchesError::Parse { .. })
        ));
        assert_eq!(fs::read_to_string(&path).expect("read the file"), text);
    }

    #[test]
    fn asks_go_to_the_inbox_unless_the_kind_turns_them_off() {
        let switches = AgentSwitches::parse("[agents.copilot]\nasks_in_inbox = false\n")
            .expect("valid file");
        assert!(!switches.asks_in_inbox("copilot"));
        assert!(switches.asks_in_inbox("claude"));
        assert!(AgentSwitches::default().asks_in_inbox("copilot"));
    }

    #[test]
    fn zsh_always_reads_as_asking_in_the_inbox() {
        let switches =
            AgentSwitches::parse("[agents.zsh]\nasks_in_inbox = false\n").expect("valid file");
        assert!(switches.asks_in_inbox(ZSH));
    }

    #[test]
    fn setting_asks_keeps_the_kinds_other_switches_and_comments() {
        let dir = tempfile::tempdir().expect("temp dir");
        let text = "# mine\n[agents.copilot]\nenabled = true # on\ninstall_when_missing = true\n";
        let path = with_file(dir.path(), text);
        let change = Change {
            asks_in_inbox: Some(false),
            ..Change::default()
        };
        let switches = set(&path, "copilot", change).expect("set");
        assert!(!switches.asks_in_inbox("copilot"));
        assert!(switches.enabled("copilot"));
        assert!(switches.install_when_missing("copilot"));
        let written = fs::read_to_string(&path).expect("read the file");
        assert!(written.contains("# mine"));
        assert!(written.contains("enabled = true # on"));
        let back_on = Change {
            asks_in_inbox: Some(true),
            ..Change::default()
        };
        let switches = set(&path, "copilot", back_on).expect("set back");
        assert!(switches.asks_in_inbox("copilot"));
        let written = fs::read_to_string(&path).expect("read the file");
        assert_eq!(written.matches("asks_in_inbox").count(), 1);
    }

    #[test]
    fn an_unknown_field_in_a_kind_is_an_error() {
        assert!(matches!(
            AgentSwitches::parse("[agents.copilot]\nenabeld = false\n"),
            Err(SwitchesError::Parse { .. })
        ));
    }

    #[test]
    fn a_bad_kind_name_is_an_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = host_path(dir.path());
        let change = Change {
            enabled: Some(false),
            ..Change::default()
        };
        assert!(matches!(
            set(&path, "bad kind", change),
            Err(SwitchesError::BadKind { .. })
        ));
        assert!(matches!(
            AgentSwitches::parse("[agents.\"my kind\"]\nenabled = false\n"),
            Err(SwitchesError::BadKind { .. })
        ));
    }

    /// The built-in kinds in Settings › Agents order.
    const ORDER: [&str; 6] = ["claude", "codex", "copilot", "opencode", "agy", "zsh"];

    #[test]
    fn a_preferred_kind_that_is_on_is_picked() {
        let switches = AgentSwitches::parse("").expect("valid file");
        assert_eq!(switches.pick(&["codex", "claude"], &ORDER), Some("codex"));
    }

    #[test]
    fn with_claude_and_codex_off_a_start_naming_none_gets_copilot() {
        let switches = AgentSwitches::parse(
            "[agents.claude]\nenabled = false\n[agents.codex]\nenabled = false\n",
        )
        .expect("valid file");
        assert_eq!(switches.pick(&["claude"], &ORDER), Some("copilot"));
    }

    #[test]
    fn the_next_preferred_kind_that_is_on_wins_over_list_order() {
        let switches = AgentSwitches::parse("[agents.claude]\nenabled = false\n").expect("valid file");
        // The channel's Claude is off; Settings' OpenCode is on, so Codex,
        // first on in the list, doesn't take it.
        assert_eq!(switches.pick(&["claude", "opencode"], &ORDER), Some("opencode"));
    }

    #[test]
    fn with_every_agent_off_nothing_is_picked_not_even_the_shell() {
        let off: String = ORDER
            .iter()
            .filter(|kind| **kind != ZSH)
            .map(|kind| format!("[agents.{kind}]\nenabled = false\n"))
            .collect();
        let switches = AgentSwitches::parse(&off).expect("valid file");
        assert_eq!(switches.pick(&["claude"], &ORDER), None);
    }

    #[test]
    fn a_shell_asked_for_by_name_is_kept() {
        let switches = AgentSwitches::parse("[agents.claude]\nenabled = false\n").expect("valid file");
        assert_eq!(switches.pick(&[ZSH], &ORDER), Some(ZSH));
    }
}
