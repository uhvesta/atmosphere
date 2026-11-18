//! The host's `config/stack.toml` on every machine (D411): `stack` on a
//! machine reads `~/.humata2/config/stack.toml`, so the same prefix and
//! settings apply wherever it runs. The file goes the way the tools do, as
//! bytes on stdin that the far end checks for size and SHA-256 before it
//! moves them in, and only when the machine's copy differs.
//!
//! Secrets never travel. Before anything is sent, every top-level
//! `key = value` line whose key names a token, password or the like, or
//! whose value looks like a GitHub token, is dropped ([`StackConfig`]);
//! what is left is sent, so a file that holds only such lines sends an
//! empty one. In `agents.toml`, whose settings all sit in tables, the lines
//! inside tables are judged the same way.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::error::{QuoteError, RunError};
use crate::install::{self, receive_script, InstallProblem, Receive};
use crate::link::Link;
use crate::manage::ManageError;
use crate::name::MachineName;
use crate::runner::{Purpose, Runner};

/// Where `stack` looks, under the humata home on the host.
const FOLDER: &str = "config";
const FILE: &str = "stack.toml";
/// The repo library's settings (RL-12), sent the same way.
const REPOS_FILE: &str = "repos.toml";
/// Which agents are on, and which Humata installs (AGENTSET), sent the same way.
const AGENTS_FILE: &str = "agents.toml";
/// The machine scripts rules (MACHINESCRIPTS), sent the same way.
const MACHINE_SCRIPTS_FILE: &str = "machine-scripts.toml";
/// The same folder as a machine has it, under `$HOME`.
const MACHINE_FOLDER: &str = ".humata2/config";

/// The most a send may take: the file is a few lines.
const PUSH_TIMEOUT: Duration = Duration::from_secs(60);
/// How many output lines a failed send keeps.
const TAIL_LINES: usize = 40;

/// A key naming one of these (in any case) holds a secret.
const SECRET_WORDS: [&str; 9] = [
    "token",
    "secret",
    "password",
    "passwd",
    "credential",
    "auth",
    "api_key",
    "apikey",
    "private_key",
];
/// A value starting with one of these is a GitHub token, whatever its key.
const SECRET_PREFIXES: [&str; 5] = ["ghp_", "gho_", "ghu_", "ghs_", "github_pat_"];

/// Why the host's stack settings couldn't be sent to a machine.
#[derive(Debug, Error)]
pub enum StackConfigError {
    #[error(
        "couldn't read the host's stack settings {}: {source}. Check that you own the file and can read it, or move it away so machines get none, then try again",
        .path.display()
    )]
    Unreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "the host's stack settings {} aren't UTF-8 text ({source}), so none were sent to any machine. Save the file as UTF-8 TOML, then try again",
        .path.display()
    )]
    NotText {
        path: PathBuf,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("couldn't find the link to {machine} to send it the stack settings: {source}")]
    Machine {
        machine: String,
        #[source]
        source: Box<ManageError>,
    },
    #[error(
        "couldn't build the command that sends the stack settings to {machine}: {source}. This is a fault in Humata; report it"
    )]
    Command {
        machine: MachineName,
        #[source]
        source: QuoteError,
    },
    #[error(
        "couldn't reach {machine} to send it the stack settings: {source}. Check that the machine is up and reachable; Humata sends them again when its link next comes up. The copy there was not changed"
    )]
    Reach {
        machine: MachineName,
        #[source]
        source: RunError,
    },
    #[error(
        "sending the stack settings to {machine} failed: {problem}. The copy there was not changed; fix that on the machine, and Humata sends them again when its link next comes up.\n{tail}"
    )]
    Push {
        machine: MachineName,
        problem: InstallProblem,
        tail: String,
    },
}

/// The host's `stack.toml` with its secrets taken out: the bytes to send and
/// their SHA-256.
#[derive(Clone, PartialEq, Eq)]
pub struct StackConfig {
    bytes: Arc<[u8]>,
    sha256: String,
    dropped: Vec<String>,
    /// `stack.toml` or `repos.toml`, on the host and on a machine.
    file: &'static str,
}

impl fmt::Debug for StackConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StackConfig")
            .field("len", &self.bytes.len())
            .field("sha256", &self.sha256)
            .field("dropped", &self.dropped)
            .finish()
    }
}

impl StackConfig {
    /// Where the host keeps its file, in the humata home (`~/.humata2`).
    pub fn host_path(humata_home: &Path) -> PathBuf {
        humata_home.join(FOLDER).join(FILE)
    }

    /// Where the host keeps the repo library's `config/repos.toml` (RL-12).
    pub fn repos_host_path(humata_home: &Path) -> PathBuf {
        humata_home.join(FOLDER).join(REPOS_FILE)
    }

    /// Where the host keeps the agent switches, `config/agents.toml` (AGENTSET).
    pub fn agents_host_path(humata_home: &Path) -> PathBuf {
        humata_home.join(FOLDER).join(AGENTS_FILE)
    }

    /// Where the host keeps the machine scripts rules, `config/machine-scripts.toml`.
    pub fn machine_scripts_host_path(humata_home: &Path) -> PathBuf {
        humata_home.join(FOLDER).join(MACHINE_SCRIPTS_FILE)
    }

    /// The settings in `text`, secrets taken out.
    pub fn from_text(text: &str) -> Self {
        Self::of_file(text, FILE)
    }

    fn of_file(text: &str, file: &'static str) -> Self {
        let Filtered { text, dropped } = filter(text, file == AGENTS_FILE);
        let bytes: Arc<[u8]> = Arc::from(text.into_bytes());
        let sha256 = install::hex(&Sha256::digest(&bytes));
        Self {
            bytes,
            sha256,
            dropped,
            file,
        }
    }

    /// The file at `path`, secrets taken out; none when there is no file,
    /// which leaves a machine's own copy alone.
    pub fn read(path: &Path) -> Result<Option<Self>, StackConfigError> {
        Self::read_as(path, FILE)
    }

    /// As [`read`](Self::read), for the repo library's `repos.toml` (RL-12).
    pub fn read_repos(path: &Path) -> Result<Option<Self>, StackConfigError> {
        Self::read_as(path, REPOS_FILE)
    }

    /// As [`read`](Self::read), for `agents.toml` (AGENTSET).
    pub fn read_agents(path: &Path) -> Result<Option<Self>, StackConfigError> {
        Self::read_as(path, AGENTS_FILE)
    }

    /// As [`read`](Self::read), for `machine-scripts.toml` (MACHINESCRIPTS).
    pub fn read_machine_scripts(path: &Path) -> Result<Option<Self>, StackConfigError> {
        Self::read_as(path, MACHINE_SCRIPTS_FILE)
    }

    fn read_as(path: &Path, file: &'static str) -> Result<Option<Self>, StackConfigError> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(StackConfigError::Unreadable {
                    path: path.to_owned(),
                    source,
                })
            }
        };
        let text = String::from_utf8(bytes).map_err(|source| StackConfigError::NotText {
            path: path.to_owned(),
            source,
        })?;
        Ok(Some(Self::of_file(&text, file)))
    }

    /// The keys that were left out because they look like secrets, never
    /// their values.
    pub fn dropped(&self) -> &[String] {
        &self.dropped
    }
}

/// What the filter kept and what it left out.
struct Filtered {
    text: String,
    dropped: Vec<String>,
}

/// How a value that goes on past its line ends.
#[derive(Clone, Copy)]
enum Open {
    /// A multi-line string, closed by this delimiter.
    Triple(&'static str),
    /// An array, closed when its brackets balance (this many are open).
    Array(i32),
}

impl Open {
    /// What `value` (the text after `=`) leaves open at the end of its line.
    fn start(value: &str) -> Option<Self> {
        for delimiter in ["\"\"\"", "'''"] {
            if let Some(rest) = value.strip_prefix(delimiter) {
                return (!rest.contains(delimiter)).then_some(Self::Triple(delimiter));
            }
        }
        if value.starts_with('[') {
            let depth = bracket_depth(value);
            return (depth > 0).then_some(Self::Array(depth));
        }
        None
    }

    /// What is still open after `line`.
    fn after(self, line: &str) -> Option<Self> {
        match self {
            Self::Triple(delimiter) => (!line.contains(delimiter)).then_some(self),
            Self::Array(open) => {
                let depth = open + bracket_depth(line);
                (depth > 0).then_some(Self::Array(depth))
            }
        }
    }
}

/// Opening minus closing brackets in `text`, ignoring quoted ones and what
/// follows a `#`.
fn bracket_depth(text: &str) -> i32 {
    let mut depth = 0;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '#') => break,
            (None, '[') => depth += 1,
            (None, ']') => depth -= 1,
            (None, _) => {}
        }
    }
    depth
}

/// Whether a top-level `key = value` holds a secret.
fn is_secret(key: &str, value: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let value = value.trim_start_matches(['"', '\'']);
    SECRET_WORDS.iter().any(|word| key.contains(word))
        || SECRET_PREFIXES.iter().any(|prefix| value.starts_with(prefix))
}

/// `text` without its secrets: each top-level `key = value` line (those
/// before the first `[table]`) that [`is_secret`] says holds one, with the
/// lines a multi-line value goes on over. With `in_tables`, the lines inside
/// tables are judged too: `agents.toml` keeps each agent's settings in its
/// own `[agents.<name>]` table, so a token there would otherwise travel.
/// Everything else stays as written.
fn filter(text: &str, in_tables: bool) -> Filtered {
    let mut kept = String::with_capacity(text.len());
    let mut dropped = Vec::new();
    let mut in_table = false;
    // The rest of a multi-line value, and whether it is being dropped.
    let mut open: Option<(Open, bool)> = None;
    for line in text.split_inclusive('\n') {
        if let Some((state, dropping)) = open.take() {
            open = state.after(line).map(|state| (state, dropping));
            if !dropping {
                kept.push_str(line);
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_table = true;
        }
        let pair = if (in_table && !in_tables) || trimmed.starts_with('#') || trimmed.starts_with('[') {
            None
        } else {
            trimmed.split_once('=')
        };
        let Some((key, value)) = pair else {
            kept.push_str(line);
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let secret = is_secret(key, value);
        open = Open::start(value).map(|state| (state, secret));
        if secret {
            dropped.push(key.to_owned());
        } else {
            kept.push_str(line);
        }
    }
    Filtered { text: kept, dropped }
}

/// The script that puts `config` in `~/.humata2/config/stack.toml` (stdin
/// has its bytes): when the file there already has this SHA-256 it reads the
/// bytes and stops; otherwise it checks what arrived against size and
/// SHA-256 and only then moves it in, so a kill or a damaged stream leaves
/// the old copy.
fn script(config: &StackConfig, nonce: &str) -> String {
    let (len, sha) = (config.bytes.len(), &config.sha256);
    let file = config.file;
    let same = format!(
        r#"set -u
f="$HOME/{MACHINE_FOLDER}/{file}"
if [ -f "$f" ]; then
  if command -v sha256sum >/dev/null 2>&1; then s=$(sha256sum "$f" | cut -d ' ' -f 1)
  elif command -v shasum >/dev/null 2>&1; then s=$(shasum -a 256 "$f" | cut -d ' ' -f 1)
  else s=; fi
  if [ "$s" = {sha} ]; then head -c {len} >/dev/null; exit 0; fi
fi
"#
    );
    let receive = receive_script(&Receive {
        dir: MACHINE_FOLDER,
        stem: file,
        target: file,
        mode: "600",
        len,
        sha,
        nonce,
        after: "",
    });
    format!("{same}{receive}")
}

/// Puts `config` on the machine `link` reaches unless its copy is already
/// that, over the same SSH path that installs the tools.
pub(crate) fn push(link: &Link, runner: &dyn Runner, config: &StackConfig) -> Result<(), StackConfigError> {
    let machine = link.machine().clone();
    let invocation = link
        .script(
            &script(config, &install::nonce()),
            Purpose::Install,
            Some(PUSH_TIMEOUT),
            Some(Arc::clone(&config.bytes)),
        )
        .map_err(|source| StackConfigError::Command {
            machine: machine.clone(),
            source,
        })?;
    let output = runner.run(&invocation).map_err(|source| StackConfigError::Reach {
        machine: machine.clone(),
        source,
    })?;
    if output.exit.success() {
        return Ok(());
    }
    let tail = output.tail(TAIL_LINES);
    Err(StackConfigError::Push {
        machine,
        problem: InstallProblem::from_exit(output.exit, !tail.trim().is_empty()),
        tail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{EnvValue, Exit, Invocation};
    use crate::system::SystemRunner;

    fn kept(text: &str) -> String {
        filter(text, false).text
    }

    #[test]
    fn plain_settings_stay_as_written() {
        let text = "# how stacks are named\nprefix = \"alice\"\n\npin_revisions = false\n[remote]\nname = \"origin\"\n";

        assert_eq!(kept(text), text);
        assert!(filter(text, false).dropped.is_empty());
    }

    #[test]
    fn a_top_level_line_whose_key_names_a_secret_is_dropped_in_any_case() {
        let text = "prefix = \"alice\"\ngh_token = \"abc\"\nGITHUB_TOKEN='abc'\nMy_Password = 1\nclient_secret = \"x\"\ndb_passwd = \"x\"\naws_credentials = \"x\"\nauthor = \"alice\"\napi_key = \"x\"\napikey = \"x\"\nprivate_key = \"x\"\npin_revisions = false\n";

        // `author` holds "auth", which is the rule's own word.
        assert_eq!(kept(text), "prefix = \"alice\"\npin_revisions = false\n");
        assert_eq!(
            filter(text, false).dropped,
            [
                "gh_token",
                "GITHUB_TOKEN",
                "My_Password",
                "client_secret",
                "db_passwd",
                "aws_credentials",
                "author",
                "api_key",
                "apikey",
                "private_key"
            ]
        );
    }

    #[test]
    fn a_value_that_looks_like_a_github_token_is_dropped_whatever_its_key() {
        let text = "a = \"ghp_abc\"\nb = 'gho_abc'\nc = ghu_abc\nd = \"ghs_abc\"\ne = \"github_pat_abc\"\nf = \"alice\"\ng = \"my_ghp_abc\"\n";

        assert_eq!(kept(text), "f = \"alice\"\ng = \"my_ghp_abc\"\n");
    }

    #[test]
    fn lines_in_a_table_and_comments_are_never_taken_for_top_level_pairs() {
        let text = "# token = \"x\"\n[section]\ntoken = \"kept\"\n";

        assert_eq!(kept(text), text);
    }

    #[test]
    fn the_lines_a_secret_value_goes_on_over_go_with_it() {
        let text = "prefix = \"alice\"\ntoken = \"\"\"\nline one\nprefix = \"inside\"\n\"\"\"\ntokens = [\n  \"a\",\n  [\"b\"],\n]\npin_revisions = false\n";

        assert_eq!(kept(text), "prefix = \"alice\"\npin_revisions = false\n");
    }

    #[test]
    fn the_lines_a_kept_value_goes_on_over_are_not_read_as_pairs() {
        let text = "note = \"\"\"\ntoken = \"inside the note\"\n\"\"\"\nitems = [\n  \"token = x\",\n]\n";

        assert_eq!(kept(text), text);
    }

    #[test]
    fn the_last_line_may_have_no_newline() {
        assert_eq!(kept("prefix = \"a\"\ntoken = \"x\""), "prefix = \"a\"\n");
        assert_eq!(kept("prefix = \"a\""), "prefix = \"a\"");
    }

    #[test]
    fn what_is_sent_is_summed_after_the_secrets_are_out() {
        let config = StackConfig::from_text("prefix = \"alice\"\ngh_token = \"ghp_abc\"\n");

        assert_eq!(config.sha256, install::hex(&Sha256::digest(b"prefix = \"alice\"\n")));
        assert_eq!(config.dropped(), ["gh_token"]);
        assert!(!format!("{config:?}").contains("ghp_abc"));
    }

    #[test]
    fn a_missing_file_sends_nothing_and_a_present_one_is_read_filtered() {
        let dir = tempfile::tempdir().unwrap();
        let path = StackConfig::host_path(dir.path());
        assert_eq!(StackConfig::read(&path).unwrap(), None);

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "prefix = \"alice\"\ntoken = \"x\"\n").unwrap();

        let config = StackConfig::read(&path).unwrap().expect("the file is there");
        assert_eq!(config, StackConfig::from_text("prefix = \"alice\"\ntoken = \"x\"\n"));
        assert_eq!(config.dropped(), ["token"]);
    }

    #[test]
    fn the_repo_libraries_settings_go_to_a_file_of_their_own_beside_the_stack_ones() {
        let home = tempfile::tempdir().unwrap();
        let stack = StackConfig::from_text("prefix = \"alice\"\n");
        let repos = StackConfig::of_file("owners = [\"acme\"]\ngh_token = \"x\"\n", REPOS_FILE);

        assert_eq!(run_here(home.path(), &stack, &stack.bytes), Exit::Code(0));
        assert_eq!(run_here(home.path(), &repos, &repos.bytes), Exit::Code(0));

        let folder = home.path().join(".humata2/config");
        assert_eq!(std::fs::read(folder.join("stack.toml")).unwrap(), b"prefix = \"alice\"\n");
        assert_eq!(std::fs::read(folder.join("repos.toml")).unwrap(), b"owners = [\"acme\"]\n");
        assert_eq!(repos.dropped(), ["gh_token"]);
    }

    #[test]
    fn the_agent_switches_go_to_a_file_of_their_own_with_secrets_taken_out() {
        let home = tempfile::tempdir().unwrap();
        let agents = StackConfig::of_file("[agents.copilot]\nenabled = false\ntoken = \"x\"\n", AGENTS_FILE);

        assert_eq!(run_here(home.path(), &agents, &agents.bytes), Exit::Code(0));

        let folder = home.path().join(".humata2/config");
        assert_eq!(
            std::fs::read(folder.join("agents.toml")).unwrap(),
            b"[agents.copilot]\nenabled = false\n"
        );
        assert_eq!(agents.dropped(), ["token"]);
    }

    #[test]
    fn a_file_that_is_not_text_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stack.toml");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();

        assert!(matches!(StackConfig::read(&path), Err(StackConfigError::NotText { .. })));
    }

    /// Runs the send script here with `home` as `$HOME` and `stdin` as what
    /// arrives.
    fn run_here(home: &Path, config: &StackConfig, stdin: &[u8]) -> Exit {
        let invocation = Invocation {
            program: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script(config, &install::nonce())],
            env: vec![
                ("HOME".to_owned(), EnvValue::Value(home.display().to_string())),
                ("PATH".to_owned(), EnvValue::Value("/usr/bin:/bin".to_owned())),
            ],
            stdin: Some(Arc::from(stdin)),
            timeout: Some(Duration::from_secs(20)),
            purpose: Purpose::Install,
        };
        SystemRunner::new(&()).run(&invocation).unwrap().exit
    }

    fn on_machine(home: &Path) -> PathBuf {
        home.join(".humata2/config/stack.toml")
    }

    #[test]
    fn a_file_that_arrives_whole_is_put_in_place_and_replaced_when_it_changes() {
        let home = tempfile::tempdir().unwrap();
        let first = StackConfig::from_text("prefix = \"alice\"\n");
        let second = StackConfig::from_text("prefix = \"bob\"\npin_revisions = false\n");

        assert_eq!(run_here(home.path(), &first, &first.bytes), Exit::Code(0));
        assert_eq!(std::fs::read(on_machine(home.path())).unwrap(), b"prefix = \"alice\"\n");
        assert_eq!(run_here(home.path(), &second, &second.bytes), Exit::Code(0));

        assert_eq!(
            std::fs::read(on_machine(home.path())).unwrap(),
            b"prefix = \"bob\"\npin_revisions = false\n"
        );
        let left: Vec<String> = std::fs::read_dir(on_machine(home.path()).parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["stack.toml"]);
    }

    #[test]
    fn a_copy_that_is_already_there_is_left_as_it_is() {
        let home = tempfile::tempdir().unwrap();
        let config = StackConfig::from_text("prefix = \"alice\"\n");
        assert_eq!(run_here(home.path(), &config, &config.bytes), Exit::Code(0));
        let before = std::fs::metadata(on_machine(home.path())).unwrap().modified().unwrap();

        assert_eq!(run_here(home.path(), &config, &config.bytes), Exit::Code(0));

        let after = std::fs::metadata(on_machine(home.path())).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn bytes_that_arrive_changed_or_short_are_refused_and_the_old_copy_stays() {
        let home = tempfile::tempdir().unwrap();
        let old = StackConfig::from_text("prefix = \"alice\"\n");
        let new = StackConfig::from_text("prefix = \"bob\"\n");
        assert_eq!(run_here(home.path(), &old, &old.bytes), Exit::Code(0));

        let changed = run_here(home.path(), &new, b"prefix = \"eve\"\n");
        let short = run_here(home.path(), &new, b"prefix");

        assert_eq!(InstallProblem::from_exit(changed, false), InstallProblem::ChecksumMismatch);
        assert_eq!(InstallProblem::from_exit(short, false), InstallProblem::Truncated);
        assert_eq!(std::fs::read(on_machine(home.path())).unwrap(), b"prefix = \"alice\"\n");
    }
}
