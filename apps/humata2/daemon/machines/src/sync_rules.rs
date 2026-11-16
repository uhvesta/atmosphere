//! Scripted sync rules (TS-3, TS-4): the config a user writes as
//! `[rules.<name>]` in TOML, checked so a rule can't write into Humata's
//! managed folders or outside the user's home. Nothing here runs a command.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::skills::{self, SkillSet, SkillsError, Source};

/// What a sync rule can't be used for.
#[derive(Debug, Error)]
pub enum SyncRulesError {
    /// The file isn't valid TOML of the expected shape.
    #[error("the sync rules aren't valid: {source}; fix the TOML in Settings and save again")]
    Syntax {
        #[source]
        source: toml::de::Error,
    },
    /// The file parsed, but rules in it can't be used; all problems at once.
    #[error("{}", .problems.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Invalid { problems: Vec<RuleProblem> },
    /// The rule's source is a command, not a folder.
    #[error("the rule {rule:?} takes its source from a command, which is run later, not scanned; run the rule instead")]
    NotAFolder { rule: String },
    /// Scanning the source folder failed.
    #[error("couldn't scan the source of the rule {rule:?}")]
    Scan {
        rule: String,
        #[source]
        source: SkillsError,
    },
}

/// One thing wrong with one rule.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuleProblem {
    /// The built-in rule's name.
    #[error("the rule name `skills` is reserved for the built-in skills sync (TS-4); rename `[rules.skills]` to something else")]
    ReservedName { rule: String },
    /// A name that isn't usable.
    #[error("the rule name {rule:?} must be lowercase letters, digits and `-`; rename the table")]
    BadName { rule: String },
    /// Neither `source` nor `command`.
    #[error("rule {rule:?}: set `source` (a folder on the host) or `command` (whose output is the source)")]
    NoSource { rule: String },
    /// Both `source` and `command`.
    #[error("rule {rule:?}: `source` and `command` are both set; keep exactly one")]
    BothSources { rule: String },
    /// A field that is empty.
    #[error("rule {rule:?}: `{field}` is empty; give it a value or remove it")]
    Empty { rule: String, field: &'static str },
    /// No destination.
    #[error("rule {rule:?}: `destination` is missing; set a folder under the machine's home such as `~/tools`")]
    NoDestination { rule: String },
    /// A destination that isn't allowed.
    #[error("rule {rule:?}: `destination` {destination:?} is not allowed ({reason}); {fix}")]
    BadDestination { rule: String, destination: String, reason: &'static str, fix: &'static str },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    rules: BTreeMap<String, Raw>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    source: Option<String>,
    command: Option<String>,
    destination: Option<String>,
    before: Option<String>,
    after: Option<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

/// Where a rule's files come from on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleSource {
    /// A folder on the host.
    Folder(PathBuf),
    /// A command template whose output is the source.
    Command(String),
}

/// A checked sync rule (TS-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRule {
    /// The rule's name.
    pub name: String,
    /// The source on the host.
    pub source: RuleSource,
    /// The destination on machines, relative to the machine's home.
    pub destination: PathBuf,
    /// Command template run on the machine before applying.
    pub before: Option<String>,
    /// Command template run on the machine after applying.
    pub after: Option<String>,
    /// Exclude globs (SK-6).
    pub exclude: Vec<String>,
}

/// Reads and checks the rules in `text`; every problem is reported together.
pub fn parse_rules(text: &str) -> Result<Vec<SyncRule>, SyncRulesError> {
    let file: File = toml::from_str(text).map_err(|source| SyncRulesError::Syntax { source })?;
    let mut rules = Vec::new();
    let mut problems = Vec::new();
    for (name, raw) in file.rules {
        match check(&name, raw) {
            Ok(rule) => rules.push(rule),
            Err(found) => problems.extend(found),
        }
    }
    if problems.is_empty() {
        Ok(rules)
    } else {
        Err(SyncRulesError::Invalid { problems })
    }
}

fn check(name: &str, raw: Raw) -> Result<SyncRule, Vec<RuleProblem>> {
    let rule = name.to_owned();
    let mut problems = Vec::new();
    if name == "skills" {
        problems.push(RuleProblem::ReservedName { rule: rule.clone() });
    } else if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        problems.push(RuleProblem::BadName { rule: rule.clone() });
    }
    let mut empty = |field: &'static str, value: &Option<String>| {
        if value.as_deref().is_some_and(|v| v.trim().is_empty()) {
            problems.push(RuleProblem::Empty { rule: rule.clone(), field });
            true
        } else {
            false
        }
    };
    let source_empty = empty("source", &raw.source);
    let command_empty = empty("command", &raw.command);
    empty("before", &raw.before);
    empty("after", &raw.after);
    let source = match (raw.source, raw.command) {
        (Some(_), Some(_)) => {
            problems.push(RuleProblem::BothSources { rule: rule.clone() });
            None
        }
        (None, None) => {
            problems.push(RuleProblem::NoSource { rule: rule.clone() });
            None
        }
        (Some(folder), None) if !source_empty => Some(RuleSource::Folder(PathBuf::from(folder))),
        (None, Some(command)) if !command_empty => Some(RuleSource::Command(command)),
        _ => None,
    };
    let destination = match raw.destination {
        None => {
            problems.push(RuleProblem::NoDestination { rule: rule.clone() });
            None
        }
        Some(text) => match destination_under_home(&text) {
            Ok(path) => Some(path),
            Err((reason, fix)) => {
                problems.push(RuleProblem::BadDestination { rule: rule.clone(), destination: text, reason, fix });
                None
            }
        },
    };
    match (source, destination, problems.is_empty()) {
        (Some(source), Some(destination), true) => {
            Ok(SyncRule { name: rule, source, destination, before: raw.before, after: raw.after, exclude: raw.exclude })
        }
        _ => Err(problems),
    }
}

/// Folders only Humata writes (TS-4), relative to home.
const MANAGED: &[&str] = &["Humata/.skills", "Humata/.skills-sets", "Humata/.skills-choices.json", ".humata2"];

/// Turns `~/x`, `x` into the path under home, or says why not.
fn destination_under_home(text: &str) -> Result<PathBuf, (&'static str, &'static str)> {
    const OUTSIDE: &str = "use `~/folder` or a path relative to home";
    let trimmed = text.trim();
    let relative = if let Some(rest) = trimmed.strip_prefix("~/") {
        rest
    } else if trimmed.starts_with('/') {
        return Err(("an absolute path outside the home folder", OUTSIDE));
    } else if trimmed == "~" || trimmed.starts_with('~') {
        return Err(("not a path under the home folder", OUTSIDE));
    } else {
        trimmed
    };
    let mut path = PathBuf::new();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err(("it climbs out with `..`", "remove the `..` parts")),
            _ => return Err(("an absolute path outside the home folder", OUTSIDE)),
        }
    }
    if path.as_os_str().is_empty() {
        return Err(("it is the home folder itself", "name a folder inside home such as `~/tools`"));
    }
    if MANAGED.iter().any(|managed| path.starts_with(managed)) {
        return Err((
            "it is inside a folder Humata manages",
            "pick another folder; skills sync has its own built-in rule",
        ));
    }
    Ok(path)
}

impl SyncRule {
    /// Scans a folder source into the same kind of set skills use, with the
    /// same secret filter and the rule's excludes (SK-6), so the same plan
    /// and apply carry it (TS-3). The set's top-level folder is the rule's
    /// name.
    pub fn scan_source(&self) -> Result<SkillSet, SyncRulesError> {
        let RuleSource::Folder(root) = &self.source else {
            return Err(SyncRulesError::NotAFolder { rule: self.name.clone() });
        };
        let source = Source { agent: self.name.clone(), root: root.clone() };
        skills::scan(&[source], &self.exclude)
            .map_err(|source| SyncRulesError::Scan { rule: self.name.clone(), source })
    }
}
