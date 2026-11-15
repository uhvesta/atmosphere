//! Providers: TOML entries of command templates the user writes in
//! Settings › Machine providers (MP-2), and the invocations made from them.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use toml::Spanned;

use crate::error::{ConfigError, Problem, ProblemKind, Problems, ValuesError};
use crate::link::Link;
use crate::name::{self, MachineName};
use crate::quote::{self, CMD};
use crate::runner::{EnvValue, Invocation, Purpose};

/// The Lima provider Humata ships (MP-22), as the user sees it in Settings.
pub(crate) const LIMA_TOML: &str = include_str!("../providers/lima.toml");

/// A provider command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Create,
    Identity,
    Start,
    Stop,
    Delete,
    Status,
}

impl Action {
    /// The command's field in the provider's TOML.
    pub fn field(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Identity => "identity",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Delete => "delete",
            Self::Status => "status",
        }
    }

    fn purpose(self) -> Purpose {
        match self {
            Self::Create => Purpose::Create,
            Self::Identity => Purpose::Identity,
            Self::Start => Purpose::Start,
            Self::Stop => Purpose::Stop,
            Self::Delete => Purpose::Delete,
            Self::Status => Purpose::Status,
        }
    }

    /// Long enough for a cloud VM to boot or a VM image to download.
    fn timeout(self) -> Duration {
        Duration::from_secs(match self {
            Self::Create => 30 * 60,
            Self::Identity | Self::Status => 2 * 60,
            Self::Start | Self::Delete => 10 * 60,
            Self::Stop => 5 * 60,
        })
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.field())
    }
}

/// How Humata reaches a provider's machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// `connect` runs `{cmd}` on the machine (MP-7).
    Connect,
    /// `identity` prints SSH details (MP-8).
    Identity,
}

/// A field in Add machine, exported as `HUMATA_<NAME>` (MP-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub prompt: String,
    pub default: Option<String>,
}

/// What a machine's `status` command reported (MP-21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineStatus {
    Running,
    /// Shown as offline with Start.
    Stopped,
    /// Anything else, as the provider printed it.
    Other(String),
}

/// One provider, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    id: String,
    label: String,
    create: String,
    reach: Reach,
    reach_template: String,
    start: Option<String>,
    stop: Option<String>,
    delete: Option<String>,
    status: Option<String>,
    params: Vec<Param>,
    env: Vec<String>,
    interactive: bool,
    run_on: Option<String>,
    built_in: bool,
}

impl Provider {
    /// Every provider in a providers file, or every problem in it.
    pub fn parse(text: &str) -> Result<Vec<Self>, ConfigError> {
        let raw: RawFile = toml::from_str(text).map_err(|source| ConfigError::Syntax {
            line: source.span().map_or(1, |span| line_of(text, span.start)),
            message: source.message().to_owned(),
            source,
        })?;
        let mut problems = Vec::new();
        let mut providers = Vec::new();
        for (id, raw) in raw.providers {
            if let Some(provider) = check(text, id, raw, &mut problems) {
                providers.push(provider);
            }
        }
        if problems.is_empty() {
            Ok(providers)
        } else {
            Err(ConfigError::Problems(Problems(problems)))
        }
    }

    /// The built-in Lima provider (MP-22).
    pub fn lima() -> Result<Self, ConfigError> {
        let mut providers = Self::parse(LIMA_TOML)?;
        match providers.pop() {
            Some(mut provider) => {
                provider.built_in = true;
                Ok(provider)
            }
            None => Err(ConfigError::Problems(Problems(vec![Problem {
                provider: "lima".to_owned(),
                field: "create".to_owned(),
                line: None,
                kind: ProblemKind::MissingCreate,
            }]))),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn reach(&self) -> Reach {
        self.reach
    }

    pub fn interactive(&self) -> bool {
        self.interactive
    }

    /// The machine provider commands run on; none means the Mac (MP-5).
    pub fn run_on(&self) -> Option<&str> {
        self.run_on.as_deref()
    }

    pub fn built_in(&self) -> bool {
        self.built_in
    }

    pub fn params(&self) -> &[Param] {
        &self.params
    }

    /// The host variables the provider's commands receive, by name.
    pub fn env(&self) -> &[String] {
        &self.env
    }

    /// Whether the machine page shows this action (MP-2: only when defined).
    pub fn has(&self, action: Action) -> bool {
        self.template(action).is_some()
    }

    /// Every command the provider will run, by field, for the import
    /// confirmation (MP-23).
    pub fn commands(&self) -> Vec<(&'static str, &str)> {
        let mut commands = vec![("create", self.create.as_str())];
        commands.push(match self.reach {
            Reach::Connect => ("connect", self.reach_template.as_str()),
            Reach::Identity => ("identity", self.reach_template.as_str()),
        });
        for (field, command) in [
            ("start", &self.start),
            ("stop", &self.stop),
            ("delete", &self.delete),
            ("status", &self.status),
        ] {
            if let Some(command) = command {
                commands.push((field, command.as_str()));
            }
        }
        commands
    }

    /// The variables every command for `machine` gets: `HUMATA_NAME`,
    /// `HUMATA_<PARAM>` for each declared param, and the listed host
    /// variables by name (MP-3, MP-4).
    pub fn environment(
        &self,
        machine: &MachineName,
        values: &BTreeMap<String, String>,
    ) -> Result<Vec<(String, EnvValue)>, ValuesError> {
        if let Some(param) = values
            .keys()
            .find(|key| !self.params.iter().any(|param| &param.name == *key))
        {
            return Err(ValuesError::Undeclared {
                provider: self.id.clone(),
                param: param.clone(),
            });
        }
        let mut env = vec![(
            "HUMATA_NAME".to_owned(),
            EnvValue::Value(machine.as_str().to_owned()),
        )];
        for param in &self.params {
            let Some(value) = values.get(&param.name).or(param.default.as_ref()) else {
                return Err(ValuesError::Missing {
                    provider: self.id.clone(),
                    param: param.name.clone(),
                    prompt: param.prompt.clone(),
                });
            };
            if value.contains('\0') {
                return Err(ValuesError::Nul {
                    param: param.name.clone(),
                });
            }
            env.push((
                format!("HUMATA_{}", param.name.to_ascii_uppercase()),
                EnvValue::Value(value.clone()),
            ));
        }
        for name in &self.env {
            env.push((name.clone(), EnvValue::HostVariable(name.clone())));
        }
        Ok(env)
    }

    /// The command for `action` on `machine`, run with `sh -c`; none when
    /// the provider doesn't define it.
    pub fn invocation(
        &self,
        action: Action,
        machine: &MachineName,
        values: &BTreeMap<String, String>,
    ) -> Result<Option<Invocation>, ValuesError> {
        let Some(template) = self.template(action) else {
            return Ok(None);
        };
        let env = self.environment(machine, values)?;
        // A machine name is always one safe shell word.
        let script = quote::render(template, machine.as_str(), None);
        Ok(Some(Invocation {
            program: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script],
            env,
            stdin: None,
            timeout: Some(action.timeout()),
            purpose: action.purpose(),
        }))
    }

    /// The link to `machine` through `connect`; none for an identity
    /// provider, whose link comes from the identity output.
    pub fn link(
        &self,
        machine: &MachineName,
        values: &BTreeMap<String, String>,
    ) -> Result<Option<Link>, ValuesError> {
        if self.reach != Reach::Connect {
            return Ok(None);
        }
        Ok(Some(Link::connect(
            machine.clone(),
            self.reach_template.clone(),
            self.environment(machine, values)?,
        )))
    }

    fn template(&self, action: Action) -> Option<&str> {
        match action {
            Action::Create => Some(&self.create),
            Action::Identity => (self.reach == Reach::Identity).then_some(&self.reach_template),
            Action::Start => self.start.as_ref(),
            Action::Stop => self.stop.as_ref(),
            Action::Delete => self.delete.as_ref(),
            Action::Status => self.status.as_ref(),
        }
        .map(String::as_str)
    }
}

/// Reads what a `status` command printed: one word, or JSON with a `state`
/// field (`jq -c '{state: .status}'`).
pub fn parse_status(stdout: &[u8]) -> MachineStatus {
    let text = String::from_utf8_lossy(stdout);
    let text = text.trim();
    let word = if text.starts_with('{') {
        serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|value| value.get("state")?.as_str().map(str::to_owned))
            .unwrap_or_default()
    } else {
        text.lines().last().unwrap_or_default().trim().to_owned()
    };
    match word.to_ascii_lowercase().as_str() {
        "running" | "active" | "started" | "up" | "ready" => MachineStatus::Running,
        "stopped" | "off" | "terminated" | "shutoff" | "shut off" | "halted" | "exited"
        | "down" | "poweroff" | "deallocated" => MachineStatus::Stopped,
        _ => MachineStatus::Other(word.chars().take(64).collect()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    providers: BTreeMap<String, RawProvider>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProvider {
    label: Option<String>,
    create: Option<Spanned<String>>,
    connect: Option<Spanned<String>>,
    identity: Option<Spanned<String>>,
    start: Option<Spanned<String>>,
    stop: Option<Spanned<String>>,
    delete: Option<Spanned<String>>,
    status: Option<Spanned<String>>,
    #[serde(default)]
    params: Vec<RawParam>,
    #[serde(default)]
    env: Vec<Spanned<String>>,
    #[serde(default)]
    interactive: bool,
    run_on: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawParam {
    name: Spanned<String>,
    prompt: Option<String>,
    default: Option<String>,
}

fn line_of(text: &str, offset: usize) -> usize {
    let end = offset.min(text.len());
    text.as_bytes()[..end].iter().filter(|b| **b == b'\n').count() + 1
}

/// Checks one raw provider, adding what is wrong to `problems`.
fn check(text: &str, id: String, raw: RawProvider, problems: &mut Vec<Problem>) -> Option<Provider> {
    let before = problems.len();
    let line = |value: &Spanned<String>| Some(line_of(text, value.span().start));
    let mut problem = |field: &str, line: Option<usize>, kind: ProblemKind| {
        problems.push(Problem {
            provider: id.clone(),
            field: field.to_owned(),
            line,
            kind,
        });
    };

    if !name::is_provider_name(&id) {
        problem("", None, ProblemKind::BadProviderName);
    }
    for (field, command) in [
        ("create", &raw.create),
        ("connect", &raw.connect),
        ("identity", &raw.identity),
        ("start", &raw.start),
        ("stop", &raw.stop),
        ("delete", &raw.delete),
        ("status", &raw.status),
    ] {
        let Some(command) = command else { continue };
        let value = command.get_ref();
        if value.trim().is_empty() {
            problem(field, line(command), ProblemKind::EmptyCommand);
        } else if value.contains('\0') {
            problem(field, line(command), ProblemKind::Nul);
        } else if field != "connect" && value.contains(CMD) {
            problem(field, line(command), ProblemKind::CmdOutsideConnect);
        }
    }
    if raw.create.is_none() {
        problem("create", None, ProblemKind::MissingCreate);
    }
    match (&raw.connect, &raw.identity) {
        (None, None) => problem("connect", None, ProblemKind::MissingLink),
        (Some(connect), Some(_)) => problem("connect", line(connect), ProblemKind::BothLinks),
        (Some(connect), None) if !connect.get_ref().contains(CMD) => {
            problem("connect", line(connect), ProblemKind::ConnectWithoutCmd);
        }
        _ => {}
    }
    let mut seen = BTreeSet::new();
    for (index, param) in raw.params.iter().enumerate() {
        let field = format!("params[{index}].name");
        let name = param.name.get_ref();
        if name == "name" {
            problem(&field, line(&param.name), ProblemKind::ReservedParam);
        } else if !name::is_param_name(name) {
            problem(&field, line(&param.name), ProblemKind::BadParamName { name: name.clone() });
        } else if !seen.insert(name.clone()) {
            problem(&field, line(&param.name), ProblemKind::DuplicateParam { name: name.clone() });
        }
    }
    let mut seen = BTreeSet::new();
    for (index, variable) in raw.env.iter().enumerate() {
        let field = format!("env[{index}]");
        let name = variable.get_ref();
        if !name::is_env_name(name) {
            problem(&field, line(variable), ProblemKind::BadEnvName { name: name.clone() });
        } else if name::is_reserved_env_name(name) {
            problem(&field, line(variable), ProblemKind::ReservedEnvName { name: name.clone() });
        } else if !seen.insert(name.clone()) {
            problem(&field, line(variable), ProblemKind::DuplicateEnv { name: name.clone() });
        }
    }
    if problems.len() > before {
        return None;
    }

    let create = raw.create?.into_inner();
    let (reach, reach_template) = match (raw.connect, raw.identity) {
        (Some(connect), _) => (Reach::Connect, connect.into_inner()),
        (None, Some(identity)) => (Reach::Identity, identity.into_inner()),
        (None, None) => return None,
    };
    Some(Provider {
        label: raw
            .label
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| id.clone()),
        id,
        create,
        reach,
        reach_template,
        start: raw.start.map(Spanned::into_inner),
        stop: raw.stop.map(Spanned::into_inner),
        delete: raw.delete.map(Spanned::into_inner),
        status: raw.status.map(Spanned::into_inner),
        params: raw
            .params
            .into_iter()
            .map(|param| {
                let name = param.name.into_inner();
                Param {
                    prompt: param.prompt.unwrap_or_else(|| name.clone()),
                    name,
                    default: param.default,
                }
            })
            .collect(),
        env: raw.env.into_iter().map(Spanned::into_inner).collect(),
        interactive: raw.interactive,
        run_on: raw.run_on,
        built_in: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_lima_provider_is_valid() {
        let lima = Provider::lima().unwrap();
        assert_eq!(lima.id(), "lima");
        assert_eq!(lima.reach(), Reach::Connect);
        assert!(lima.built_in());
        for action in [Action::Start, Action::Stop, Action::Delete, Action::Status] {
            assert!(lima.has(action), "{action}");
        }
    }

    #[test]
    fn lines_count_from_one() {
        assert_eq!(line_of("a\nb\nc", 0), 1);
        assert_eq!(line_of("a\nb\nc", 2), 2);
        assert_eq!(line_of("a\nb\nc", 99), 3);
    }
}
