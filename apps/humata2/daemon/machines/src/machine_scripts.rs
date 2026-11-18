//! Machine scripts (MACHINESCRIPTS, D680 to D684): the rules a person writes
//! in `~/.humata2/config/machine-scripts.toml`, checked so a bad rule is
//! found when the file is read, not when a machine links up. Nothing here
//! runs a command; the daemon's engine does.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

/// The most a rule's `timeout` may be.
const MAX_TIMEOUT_SECS: i64 = 3600;
/// A step's default and cap when the rule says nothing shorter.
const STEP_DEFAULT: Duration = Duration::from_secs(300);
/// A `session-leaving` step never holds a move longer than this.
const LEAVING_CAP: Duration = Duration::from_secs(60);
/// How long a `send` waits for the agent to be idle.
pub const WAIT_IDLE: Duration = Duration::from_secs(120);

/// When a rule fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Trigger {
    /// The first link-up after Add machine.
    Added,
    /// Every later link-up.
    Connected,
    /// A move ended on this machine.
    SessionArrived,
    /// A move is leaving this machine.
    SessionLeaving,
    /// A Migrate of this machine finished (D689): once per Migrate.
    Upgraded,
    /// Run on purpose by a person; every rule in scope applies.
    Manual,
}

impl Trigger {
    /// The name used in the rules file and the run log.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Connected => "connected",
            Self::SessionArrived => "session-arrived",
            Self::SessionLeaving => "session-leaving",
            Self::Upgraded => "upgraded",
            Self::Manual => "manual",
        }
    }

    /// The trigger named `text`, if it is one.
    pub fn parse(text: &str) -> Option<Trigger> {
        [
            Self::Added,
            Self::Connected,
            Self::SessionArrived,
            Self::SessionLeaving,
            Self::Upgraded,
            Self::Manual,
        ]
            .into_iter()
            .find(|trigger| trigger.as_str() == text)
    }

    /// Whether a session is part of the trigger.
    pub fn is_session(self) -> bool {
        matches!(self, Self::SessionArrived | Self::SessionLeaving)
    }
}

/// The name of a script file in `~/.humata2/scripts`: a file name, never a
/// path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptName(String);

impl ScriptName {
    fn new(text: &str) -> Option<Self> {
        let usable = !text.is_empty()
            && !text.contains(['/', '\\', '\0'])
            && text != "."
            && text != ".."
            && !text.starts_with('.');
        usable.then(|| Self(text.to_owned()))
    }

    /// The file name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether a `send` waits for the agent to be idle first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    Idle,
    None,
}

/// Whether a `send` presses Enter after the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keys {
    Enter,
    None,
}

/// One step of a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A script from the scripts folder, streamed to `bash -s -- args`.
    Run { script: ScriptName, args: Vec<String> },
    /// The machine's own `humata` with these arguments.
    Humata { args: Vec<String> },
    /// Start a session of an agent kind in a named channel.
    Start { agent: String, channel: String, title: Option<String> },
    /// Type into a session's agent.
    Send { text: String, keys: Keys, wait: Wait },
}

/// Which machines a rule is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Named(Vec<String>),
}

impl Scope {
    /// Whether `machine` is in scope.
    pub fn includes(&self, machine: &str) -> bool {
        match self {
            Self::All => true,
            Self::Named(names) => names.iter().any(|name| name == machine),
        }
    }
}

/// A checked rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRule {
    pub name: String,
    pub on: Vec<Trigger>,
    pub scope: Scope,
    pub once: bool,
    /// The cap on every step of the rule: at most an hour.
    pub timeout: Duration,
    pub steps: Vec<Step>,
}

impl ScriptRule {
    /// Whether the rule runs for `trigger` on `machine`. A manual run
    /// applies every rule in scope.
    pub fn applies(&self, trigger: Trigger, machine: &str) -> bool {
        self.scope.includes(machine) && (trigger == Trigger::Manual || self.on.contains(&trigger))
    }

    /// How long one step may take: five minutes, or the rule's own cap when
    /// that is shorter, and a minute at most while a move is leaving.
    pub fn step_timeout(&self, trigger: Trigger) -> Duration {
        let step = STEP_DEFAULT.min(self.timeout);
        if trigger == Trigger::SessionLeaving {
            step.min(LEAVING_CAP)
        } else {
            step
        }
    }
}

/// The rules, and what is worth saying about them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScriptRules {
    pub rules: Vec<ScriptRule>,
    pub warnings: Vec<ScriptWarning>,
}

/// Something that isn't wrong yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptWarning {
    /// A machine the rule names isn't known; it may be added later.
    UnknownMachine { rule: String, machine: String },
}

/// One thing wrong with one rule.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ScriptProblem {
    #[error("a rule name {rule:?} must be lowercase letters, digits and `-`; rename it")]
    BadName { rule: String },
    #[error("the rule name {rule:?} is used twice; give each rule its own name")]
    DuplicateName { rule: String },
    #[error("rule {rule:?}: `on` is missing or empty; add one or more of added, connected, session-arrived, session-leaving")]
    NoTrigger { rule: String },
    #[error("rule {rule:?}: `{trigger}` isn't a trigger; use added, connected, session-arrived or session-leaving (`humata machine script run` is how a rule runs on demand)")]
    UnknownTrigger { rule: String, trigger: String },
    #[error("rule {rule:?}: `machines` is empty; list machine names, use \"*\" for all, or remove the line")]
    NoMachines { rule: String },
    #[error("rule {rule:?}: it has no steps; add a `[[rule.step]]` with run, humata, start or send")]
    NoSteps { rule: String },
    #[error("rule {rule:?}, step {step}: it has {found} of run, humata, start and send; keep exactly one")]
    StepKind { rule: String, step: usize, found: usize },
    #[error("rule {rule:?}, step {step}: `{field}` goes with {with}; add it or remove `{field}`")]
    StrayField { rule: String, step: usize, field: &'static str, with: &'static str },
    #[error("rule {rule:?}, step {step}: {script:?} isn't a script file name; use a plain name such as `setup.sh`, not a path, and not starting with `.`")]
    BadScriptName { rule: String, step: usize, script: String },
    #[error("rule {rule:?}, step {step}: the script {script:?} isn't in the scripts folder; put it there (Settings › Machines › Scripts › Reveal scripts) or fix the name")]
    MissingScript { rule: String, step: usize, script: String },
    #[error("rule {rule:?}, step {step}: `humata` is empty; give its arguments, such as [\"session\", \"list\"]")]
    EmptyHumata { rule: String, step: usize },
    #[error("rule {rule:?}, step {step}: `{field}` is empty; give it a value")]
    EmptyField { rule: String, step: usize, field: &'static str },
    #[error("rule {rule:?}, step {step}: `start` only works on added and connected, because on a session trigger the session is already there; remove the step or the session trigger")]
    StartOnSessionTrigger { rule: String, step: usize },
    #[error("rule {rule:?}, step {step}: `{field}` can't be {value:?}; use {allowed}")]
    BadChoice { rule: String, step: usize, field: &'static str, value: String, allowed: &'static str },
    #[error("rule {rule:?}: `timeout` {seconds} isn't allowed; give whole seconds from 1 to 3600")]
    BadTimeout { rule: String, seconds: i64 },
}

/// Why the rules couldn't be used.
#[derive(Debug, Error)]
pub enum MachineScriptsError {
    #[error("couldn't read the machine scripts file {}: {source}; check that you own it and can read it, then try again", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the machine scripts file isn't valid TOML of the expected shape: {source}; fix it and run `humata machine script check`")]
    Syntax {
        #[source]
        source: toml::de::Error,
    },
    #[error("{}", .problems.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Invalid { problems: Vec<ScriptProblem> },
}

/// Where the scripts are, in the humata home (`~/.humata2`).
pub fn scripts_dir(humata_home: &Path) -> PathBuf {
    humata_home.join("scripts")
}

/// Where the rules file is.
pub fn rules_path(humata_home: &Path) -> PathBuf {
    humata_home.join("config").join("machine-scripts.toml")
}

/// The rules in the humata home; no file means no rules.
pub fn read_machine_scripts(humata_home: &Path, machines: &[String]) -> Result<ScriptRules, MachineScriptsError> {
    let path = rules_path(humata_home);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(ScriptRules::default()),
        Err(source) => return Err(MachineScriptsError::Read { path, source }),
    };
    let scripts = scripts_dir(humata_home);
    parse_machine_scripts(&text, &|name| scripts.join(name.as_str()).is_file(), machines)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    rule: Vec<RawRule>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_list(self) -> Vec<String> {
        match self {
            Self::One(one) => vec![one],
            Self::Many(many) => many,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    #[serde(default)]
    name: String,
    on: Option<OneOrMany>,
    machines: Option<OneOrMany>,
    #[serde(default)]
    once: bool,
    timeout: Option<i64>,
    #[serde(default)]
    step: Vec<RawStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    run: Option<String>,
    args: Option<Vec<String>>,
    humata: Option<Vec<String>>,
    start: Option<RawStart>,
    send: Option<String>,
    keys: Option<String>,
    wait: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStart {
    #[serde(default)]
    agent: String,
    #[serde(default)]
    channel: String,
    title: Option<String>,
}

/// Reads and checks the rules in `text`; every problem is reported together.
/// `exists` says whether a script is in the scripts folder; `machines` are
/// the machines Humata knows.
pub fn parse_machine_scripts(
    text: &str,
    exists: &dyn Fn(&ScriptName) -> bool,
    machines: &[String],
) -> Result<ScriptRules, MachineScriptsError> {
    let file: File = toml::from_str(text).map_err(|source| MachineScriptsError::Syntax { source })?;
    let mut rules = Vec::new();
    let mut warnings = Vec::new();
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in file.rule {
        let mut found = Vec::new();
        if !seen.insert(raw.name.clone()) && is_rule_name(&raw.name) {
            found.push(ScriptProblem::DuplicateName { rule: raw.name.clone() });
        }
        let rule = check_rule(raw, exists, machines, &mut found, &mut warnings);
        if found.is_empty() {
            rules.push(rule);
        } else {
            problems.extend(found);
        }
    }
    if problems.is_empty() {
        Ok(ScriptRules { rules, warnings })
    } else {
        Err(MachineScriptsError::Invalid { problems })
    }
}

fn is_rule_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn check_rule(
    raw: RawRule,
    exists: &dyn Fn(&ScriptName) -> bool,
    machines: &[String],
    found: &mut Vec<ScriptProblem>,
    warnings: &mut Vec<ScriptWarning>,
) -> ScriptRule {
    let rule = raw.name;
    if !is_rule_name(&rule) {
        found.push(ScriptProblem::BadName { rule: rule.clone() });
    }

    let mut on = Vec::new();
    let texts = raw.on.map(OneOrMany::into_list).unwrap_or_default();
    if texts.is_empty() {
        found.push(ScriptProblem::NoTrigger { rule: rule.clone() });
    }
    for text in texts {
        match Trigger::parse(&text) {
            Some(trigger) if trigger != Trigger::Manual => {
                if !on.contains(&trigger) {
                    on.push(trigger);
                }
            }
            _ => found.push(ScriptProblem::UnknownTrigger { rule: rule.clone(), trigger: text }),
        }
    }

    let scope = match raw.machines.map(OneOrMany::into_list) {
        None => Scope::All,
        Some(names) if names.is_empty() => {
            found.push(ScriptProblem::NoMachines { rule: rule.clone() });
            Scope::All
        }
        Some(names) if names.iter().any(|name| name == "*") => Scope::All,
        Some(names) => {
            for name in &names {
                if !machines.contains(name) {
                    warnings.push(ScriptWarning::UnknownMachine { rule: rule.clone(), machine: name.clone() });
                }
            }
            Scope::Named(names)
        }
    };

    let timeout = match raw.timeout {
        None => Duration::from_secs(MAX_TIMEOUT_SECS as u64),
        Some(seconds) if (1..=MAX_TIMEOUT_SECS).contains(&seconds) => Duration::from_secs(seconds as u64),
        Some(seconds) => {
            found.push(ScriptProblem::BadTimeout { rule: rule.clone(), seconds });
            Duration::from_secs(MAX_TIMEOUT_SECS as u64)
        }
    };

    if raw.step.is_empty() {
        found.push(ScriptProblem::NoSteps { rule: rule.clone() });
    }
    let on_session = on.iter().any(|trigger| trigger.is_session());
    let mut steps = Vec::new();
    for (at, step) in raw.step.into_iter().enumerate() {
        if let Some(step) = check_step(&rule, at + 1, step, on_session, exists, found) {
            steps.push(step);
        }
    }

    ScriptRule { name: rule, on, scope, once: raw.once, timeout, steps }
}

fn check_step(
    rule: &str,
    step: usize,
    raw: RawStep,
    on_session: bool,
    exists: &dyn Fn(&ScriptName) -> bool,
    found: &mut Vec<ScriptProblem>,
) -> Option<Step> {
    let rule = rule.to_owned();
    let kinds = [raw.run.is_some(), raw.humata.is_some(), raw.start.is_some(), raw.send.is_some()]
        .into_iter()
        .filter(|present| *present)
        .count();
    if kinds != 1 {
        found.push(ScriptProblem::StepKind { rule, step, found: kinds });
        return None;
    }
    let mut stray = |field: &'static str, with: &'static str, present: bool| {
        if present {
            found.push(ScriptProblem::StrayField { rule: rule.clone(), step, field, with });
        }
    };
    stray("args", "`run`", raw.args.is_some() && raw.run.is_none());
    stray("keys", "`send`", raw.keys.is_some() && raw.send.is_none());
    stray("wait", "`send`", raw.wait.is_some() && raw.send.is_none());

    if let Some(script) = raw.run {
        let Some(name) = ScriptName::new(&script) else {
            found.push(ScriptProblem::BadScriptName { rule, step, script });
            return None;
        };
        if !exists(&name) {
            found.push(ScriptProblem::MissingScript { rule, step, script });
            return None;
        }
        return Some(Step::Run { script: name, args: raw.args.unwrap_or_default() });
    }
    if let Some(args) = raw.humata {
        if args.is_empty() {
            found.push(ScriptProblem::EmptyHumata { rule, step });
            return None;
        }
        return Some(Step::Humata { args });
    }
    if let Some(start) = raw.start {
        let mut good = true;
        if on_session {
            found.push(ScriptProblem::StartOnSessionTrigger { rule: rule.clone(), step });
            good = false;
        }
        for (field, value) in [("agent", &start.agent), ("channel", &start.channel)] {
            if value.trim().is_empty() {
                found.push(ScriptProblem::EmptyField { rule: rule.clone(), step, field });
                good = false;
            }
        }
        return good.then_some(Step::Start { agent: start.agent, channel: start.channel, title: start.title });
    }
    let text = raw.send.unwrap_or_default();
    let mut good = true;
    if text.trim().is_empty() {
        found.push(ScriptProblem::EmptyField { rule: rule.clone(), step, field: "send" });
        good = false;
    }
    let keys = match raw.keys.as_deref() {
        None | Some("enter") => Keys::Enter,
        Some("none") => Keys::None,
        Some(value) => {
            found.push(bad_choice(&rule, step, "keys", value, "\"enter\" or \"none\""));
            good = false;
            Keys::Enter
        }
    };
    let wait = match raw.wait.as_deref() {
        None | Some("idle") => Wait::Idle,
        Some("none") => Wait::None,
        Some(value) => {
            found.push(bad_choice(&rule, step, "wait", value, "\"idle\" or \"none\""));
            good = false;
            Wait::Idle
        }
    };
    good.then_some(Step::Send { text, keys, wait })
}

fn bad_choice(rule: &str, step: usize, field: &'static str, value: &str, allowed: &'static str) -> ScriptProblem {
    ScriptProblem::BadChoice { rule: rule.to_owned(), step, field, value: value.to_owned(), allowed }
}

/// A commented sample, written the first time so the file is there to edit.
/// One text for the daemon and the app's Edit rules (humata2_script_sample).
pub const SAMPLE_RULES: &str = humata2_script_sample::SAMPLE_RULES;


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_names_are_file_names() {
        for good in ["setup.sh", "a-b_c", "x"] {
            assert!(ScriptName::new(good).is_some(), "{good}");
        }
        for bad in ["", ".", "..", ".hidden", "a/b", "../x", "a\\b", "a\0b"] {
            assert!(ScriptName::new(bad).is_none(), "{bad:?}");
        }
    }
}
