//! Every setting stack has: a default compiled in, or a key in
//! `<humata home>/config/stack.toml`. The humata home is `$HUMATA2_HOME`
//! when it is set, else `~/.humata2`. Humata copies the whole `config/`
//! folder, with the binary, to every machine, so the binary and that folder
//! are all stack needs: it reads no git config and no other environment.
//! Secrets never go in the file; stack uses the GitHub CLI's own login.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::cli::Target;
use crate::git::{Git, GitError};

/// The file name of the settings, in the config folder.
pub(crate) const FILE_NAME: &str = "stack.toml";

/// Every key stack.toml may set, in the order `stack config show` prints.
const KEYS: [&str; 10] = [
    "prefix",
    "trunk",
    "remote",
    "template",
    "draft",
    "overwrite_body",
    "native_stack",
    "history_comment",
    "pin_revisions",
    "keep_revisions",
];

/// Parts of a key name that mean a secret.
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

/// Starts of GitHub token values.
const TOKEN_STARTS: [&str; 5] = ["ghp_", "gho_", "ghu_", "ghs_", "github_pat_"];

/// Where a setting's value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum From {
    Default,
    File,
}

impl From {
    fn label(self) -> &'static str {
        match self {
            From::Default => "default",
            From::File => "stack.toml",
        }
    }
}

/// One setting's value and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Setting<T> {
    pub(crate) value: T,
    pub(crate) from: From,
}

impl<T> Setting<T> {
    fn default(value: T) -> Setting<T> {
        Setting {
            value,
            from: From::Default,
        }
    }
}

/// The effective settings, before command-line flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Config {
    /// The config folder: `<humata home>/config`.
    pub(crate) dir: PathBuf,
    /// Whether stack.toml exists. A missing file means every default.
    pub(crate) found: bool,
    /// The first part of new branch names. None: the gh login.
    pub(crate) prefix: Setting<Option<String>>,
    /// The branch stacks go into. None: the repository's default branch.
    pub(crate) trunk: Setting<Option<String>>,
    /// The git remote to fetch from and push to.
    pub(crate) remote: Setting<String>,
    /// A template file name in the config folder. None: the built-in one.
    pub(crate) template: Setting<Option<String>>,
    /// Open new pull requests as drafts.
    pub(crate) draft: Setting<bool>,
    /// Replace descriptions that were edited on GitHub.
    pub(crate) overwrite_body: Setting<bool>,
    /// Link chains as native GitHub stacks when gh can.
    pub(crate) native_stack: Setting<bool>,
    /// Keep a history comment on each pull request.
    pub(crate) history_comment: Setting<bool>,
    /// Push each revision's head and base to `refs/stack/...`, so old
    /// revisions stay fetchable after a force-push.
    pub(crate) pin_revisions: Setting<bool>,
    /// How many pinned revisions to keep per pull request; 0 keeps all.
    pub(crate) keep_revisions: Setting<u32>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigError {
    #[error(
        "neither HUMATA2_HOME nor HOME is set, so stack can't find its config folder. Set HOME, or \
         set HUMATA2_HOME to the folder that holds config/stack.toml"
    )]
    NoHome,

    #[error("could not read {path}. Fix its permissions, or remove it to use the defaults")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path}:{line} is not valid TOML. Fix that line; `stack config show` prints the keys")]
    Syntax {
        path: PathBuf,
        line: usize,
        #[source]
        source: toml::de::Error,
    },

    #[error(
        "{at} sets `{key}`, which looks like a secret. Secrets never go in stack.toml, because Humata \
         copies it to every machine. Remove the line: stack uses the GitHub CLI's login, so run \
         `gh auth login` on each machine instead"
    )]
    Secret { at: String, key: String },

    #[error("{at} sets `{key}`, which stack doesn't know. Remove it; the keys are: {known}")]
    Unknown { at: String, key: String, known: String },

    #[error("{at}: `{key}` must be {expected}")]
    Type {
        at: String,
        key: String,
        expected: &'static str,
    },

    #[error(
        "the branch prefix {prefix:?} (from {from}) {reason}. Pick a single word such as your \
         login, set it with `prefix = \"name\"` in stack.toml or `--prefix name`"
    )]
    BadPrefix {
        prefix: String,
        from: &'static str,
        reason: &'static str,
    },

    #[error("could not ask git whether {prefix:?} makes valid branch names")]
    PrefixCheck {
        prefix: String,
        #[source]
        source: GitError,
    },

    #[error(
        "could not read the PR template {path}, which stack.toml's `template` names. Create it, \
         or remove the `template` line to use the built-in template"
    )]
    Template {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{at}: `template = {name:?}` must be a plain file name in {dir}, such as \"pr_template.md\", \
         so it is copied with stack.toml"
    )]
    TemplateName { at: String, name: String, dir: PathBuf },
}

impl ConfigError {
    /// 2 for a problem in the file the person can fix there; 1 otherwise.
    pub(crate) fn exit_code(&self) -> u8 {
        match self {
            ConfigError::NoHome
            | ConfigError::Read { .. }
            | ConfigError::PrefixCheck { .. }
            | ConfigError::Template { .. } => 1,
            _ => 2,
        }
    }
}

/// The config folder: `$HUMATA2_HOME/config`, else `~/.humata2/config`.
pub(crate) fn config_dir() -> Result<PathBuf, ConfigError> {
    let home = std::env::var_os("HUMATA2_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".humata2"))
        })
        .ok_or(ConfigError::NoHome)?;
    Ok(home.join("config"))
}

impl Config {
    /// Reads `<config dir>/stack.toml`. A missing file gives every default.
    pub(crate) fn load() -> Result<Config, ConfigError> {
        Config::load_from(&config_dir()?)
    }

    pub(crate) fn load_from(dir: &Path) -> Result<Config, ConfigError> {
        let path = dir.join(FILE_NAME);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => return Err(ConfigError::Read { path, source }),
        };
        let mut config = Config::defaults(dir.to_path_buf());
        if let Some(text) = text {
            config.found = true;
            config.apply(&path, &text)?;
        }
        Ok(config)
    }

    fn defaults(dir: PathBuf) -> Config {
        Config {
            dir,
            found: false,
            prefix: Setting::default(None),
            trunk: Setting::default(None),
            remote: Setting::default("origin".to_owned()),
            template: Setting::default(None),
            draft: Setting::default(false),
            overwrite_body: Setting::default(false),
            native_stack: Setting::default(true),
            history_comment: Setting::default(true),
            pin_revisions: Setting::default(true),
            keep_revisions: Setting::default(0),
        }
    }

    fn apply(&mut self, path: &Path, text: &str) -> Result<(), ConfigError> {
        let table: toml::Table = toml::from_str(text).map_err(|source| ConfigError::Syntax {
            path: path.to_path_buf(),
            line: source
                .span()
                .map_or(1, |span| line_of_offset(text, span.start)),
            source,
        })?;
        for (key, value) in &table {
            let at = location(path, text, key);
            let lowered = key.to_lowercase();
            let secret_value = value
                .as_str()
                .is_some_and(|value| TOKEN_STARTS.iter().any(|start| value.starts_with(start)));
            if SECRET_WORDS.iter().any(|word| lowered.contains(word)) || secret_value {
                return Err(ConfigError::Secret {
                    at,
                    key: key.clone(),
                });
            }
            let string = || {
                value
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| ConfigError::Type {
                        at: at.clone(),
                        key: key.clone(),
                        expected: "a non-empty string in quotes",
                    })
            };
            let boolean = || {
                value.as_bool().ok_or_else(|| ConfigError::Type {
                    at: at.clone(),
                    key: key.clone(),
                    expected: "true or false",
                })
            };
            match key.as_str() {
                "prefix" => self.prefix = from_file(Some(string()?)),
                "trunk" => self.trunk = from_file(Some(string()?)),
                "remote" => self.remote = from_file(string()?),
                "template" => {
                    let name = string()?;
                    let plain = !name.contains('/')
                        && !name.contains('\\')
                        && !name.starts_with('.')
                        && name != FILE_NAME;
                    if !plain {
                        return Err(ConfigError::TemplateName {
                            at,
                            name,
                            dir: self.dir.clone(),
                        });
                    }
                    self.template = from_file(Some(name));
                }
                "draft" => self.draft = from_file(boolean()?),
                "overwrite_body" => self.overwrite_body = from_file(boolean()?),
                "native_stack" => self.native_stack = from_file(boolean()?),
                "history_comment" => self.history_comment = from_file(boolean()?),
                "pin_revisions" => self.pin_revisions = from_file(boolean()?),
                "keep_revisions" => {
                    let count = value
                        .as_integer()
                        .and_then(|count| u32::try_from(count).ok())
                        .ok_or_else(|| ConfigError::Type {
                            at: at.clone(),
                            key: key.clone(),
                            expected: "a whole number, 0 or more (0 keeps every revision)",
                        })?;
                    self.keep_revisions = from_file(count);
                }
                _ => {
                    return Err(ConfigError::Unknown {
                        at,
                        key: key.clone(),
                        known: KEYS.join(", "),
                    });
                }
            }
        }
        Ok(())
    }

    /// The remote: the `--remote` flag, else stack.toml's, else `origin`.
    pub(crate) fn remote(&self, target: &Target) -> String {
        target.remote.clone().unwrap_or_else(|| self.remote.value.clone())
    }

    /// The trunk for a command that never calls GitHub: `--trunk`, else
    /// stack.toml's, else the remote's HEAD as last fetched, else `main`.
    pub(crate) fn local_trunk(&self, target: &Target, git: &Git, remote: &str) -> String {
        target
            .trunk
            .clone()
            .or_else(|| self.trunk.value.clone())
            .or_else(|| git.remote_default_branch(remote))
            .unwrap_or_else(|| "main".to_owned())
    }

    /// The trunk for a command that calls GitHub: `--trunk`, else
    /// stack.toml's, else `github_default` (the repository's default branch).
    pub(crate) fn trunk(&self, target: &Target, github_default: String) -> String {
        target
            .trunk
            .clone()
            .or_else(|| self.trunk.value.clone())
            .unwrap_or(github_default)
    }

    /// The prefix of new branch names: `flag`, else stack.toml's, else the
    /// gh login. Refused unless `<prefix>/stack/<topic>` is a valid branch.
    pub(crate) fn prefix(&self, git: &Git, flag: Option<&str>, login: &str) -> Result<String, ConfigError> {
        let (prefix, from) = match (flag, &self.prefix.value) {
            (Some(flag), _) => (flag.to_owned(), "--prefix"),
            (None, Some(file)) => (file.clone(), "stack.toml"),
            (None, None) => (login.to_owned(), "your GitHub login"),
        };
        let bad = |reason| ConfigError::BadPrefix {
            prefix: prefix.clone(),
            from,
            reason,
        };
        if prefix.is_empty() {
            return Err(bad("is empty"));
        }
        if prefix.contains('/') {
            return Err(bad("has a `/`; it must be one part of the branch name"));
        }
        let sample = format!("{prefix}/stack/topic");
        let valid = git
            .is_valid_branch(&sample)
            .map_err(|source| ConfigError::PrefixCheck {
                prefix: prefix.clone(),
                source,
            })?;
        if !valid {
            return Err(bad("doesn't make a valid git branch name (see `git check-ref-format`)"));
        }
        Ok(prefix)
    }

    /// The path of the named template file, when one is set.
    pub(crate) fn template_path(&self) -> Option<PathBuf> {
        self.template.value.as_ref().map(|name| self.dir.join(name))
    }

    /// The settings as `stack config show --json` prints them.
    pub(crate) fn to_json(&self) -> Value {
        let settings: Vec<Value> = self
            .rows()
            .into_iter()
            .map(|(key, value, from)| json!({"key": key, "value": value, "from": from.label()}))
            .collect();
        json!({
            "file": self.dir.join(FILE_NAME),
            "found": self.found,
            "settings": settings,
        })
    }

    /// The settings as `stack config show` prints them: one line per key.
    pub(crate) fn to_text(&self) -> String {
        let file = self.dir.join(FILE_NAME);
        let mut text = if self.found {
            format!("# Settings from {}\n", file.display())
        } else {
            format!("# {} doesn't exist: every setting is the default\n", file.display())
        };
        for (key, value, from) in self.rows() {
            let shown = match (&value, key) {
                (Value::Null, "prefix") => "(your GitHub login)".to_owned(),
                (Value::Null, "trunk") => "(the repository's default branch)".to_owned(),
                (Value::Null, "template") => "(built in; `stack guide --template` prints it)".to_owned(),
                (value, _) => value.to_string(),
            };
            text.push_str(&format!("{key} = {shown}  # {}\n", from.label()));
        }
        text
    }

    fn rows(&self) -> Vec<(&'static str, Value, From)> {
        let rows = vec![
            ("prefix", json!(self.prefix.value), self.prefix.from),
            ("trunk", json!(self.trunk.value), self.trunk.from),
            ("remote", json!(self.remote.value), self.remote.from),
            ("template", json!(self.template.value), self.template.from),
            ("draft", json!(self.draft.value), self.draft.from),
            ("overwrite_body", json!(self.overwrite_body.value), self.overwrite_body.from),
            ("native_stack", json!(self.native_stack.value), self.native_stack.from),
            ("history_comment", json!(self.history_comment.value), self.history_comment.from),
            ("pin_revisions", json!(self.pin_revisions.value), self.pin_revisions.from),
            ("keep_revisions", json!(self.keep_revisions.value), self.keep_revisions.from),
        ];
        debug_assert!(rows.iter().map(|row| row.0).eq(KEYS));
        rows
    }
}

fn from_file<T>(value: T) -> Setting<T> {
    Setting {
        value,
        from: From::File,
    }
}

/// The 1-based line of a byte offset.
fn line_of_offset(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .map_or(1, |before| before.matches('\n').count() + 1)
}

/// `<path>:<line>` of the line that sets `key`, or the path alone.
fn location(path: &Path, text: &str, key: &str) -> String {
    let quoted = format!("\"{key}\"");
    let line = text.lines().position(|line| {
        let line = line.trim_start();
        [key, quoted.as_str()].iter().any(|name| {
            line.strip_prefix(name)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        })
    });
    match line {
        Some(index) => format!("{}:{}", path.display(), index + 1),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigError, FILE_NAME, From};

    fn load(text: &str) -> Result<Config, ConfigError> {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), text).unwrap();
        Config::load_from(dir.path())
    }

    #[test]
    fn a_missing_file_gives_every_default() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load_from(dir.path()).unwrap();
        assert!(!config.found);
        assert_eq!(config.remote.value, "origin");
        assert_eq!(config.prefix.value, None);
        assert!(config.native_stack.value && config.history_comment.value);
        assert!(!config.draft.value && !config.overwrite_body.value);
    }

    #[test]
    fn keys_in_the_file_override_the_defaults_and_say_so() {
        let config = load("prefix = \"foo\"\nnative_stack = false\ntemplate = \"pr.md\"\n").unwrap();
        assert_eq!(config.prefix.value.as_deref(), Some("foo"));
        assert_eq!(config.prefix.from, From::File);
        assert!(!config.native_stack.value);
        assert_eq!(config.remote.from, From::Default);
        assert_eq!(config.template_path(), Some(config.dir.join("pr.md")));
    }

    #[test]
    fn an_unknown_key_is_refused_with_its_line() {
        match load("prefix = \"foo\"\ncolour = \"red\"\n") {
            Err(ConfigError::Unknown { at, key, .. }) => {
                assert_eq!(key, "colour");
                assert!(at.ends_with(":2"), "{at}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_secret_key_or_token_value_is_refused() {
        assert!(matches!(load("gh_token = \"x\"\n"), Err(ConfigError::Secret { .. })));
        assert!(matches!(load("Password = \"x\"\n"), Err(ConfigError::Secret { .. })));
        assert!(matches!(load("prefix = \"ghp_abc\"\n"), Err(ConfigError::Secret { .. })));
    }

    #[test]
    fn bad_toml_names_the_line() {
        match load("prefix = \"foo\"\n\nnative_stack = \n") {
            Err(ConfigError::Syntax { line, .. }) => assert_eq!(line, 3),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_wrong_type_or_a_template_outside_the_folder_is_refused() {
        assert!(matches!(load("draft = \"yes\"\n"), Err(ConfigError::Type { .. })));
        assert!(matches!(load("prefix = \"\"\n"), Err(ConfigError::Type { .. })));
        assert!(matches!(load("template = \"../x.md\"\n"), Err(ConfigError::TemplateName { .. })));
    }
}
