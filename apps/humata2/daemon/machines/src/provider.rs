//! Providers: TOML entries of command templates the user writes in
//! Settings › Machines › Providers (MP-2), and the invocations made from them.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use toml::Spanned;

use crate::error::{ConfigError, FieldProblem, IdentityError, Problem, ProblemKind, Problems, ValuesError};
use crate::identity::{self, SshTarget};
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
    List,
    Start,
    Stop,
    Wake,
    Refresh,
    Ssh,
    Delete,
    Status,
}

impl Action {
    /// The command's field in the provider's TOML.
    pub fn field(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Identity => "identity",
            Self::List => "list",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Wake => "wake",
            Self::Refresh => "refresh",
            Self::Ssh => "ssh",
            Self::Delete => "delete",
            Self::Status => "status",
        }
    }

    fn purpose(self) -> Purpose {
        match self {
            Self::Create => Purpose::Create,
            Self::Identity => Purpose::Identity,
            Self::List => Purpose::List,
            Self::Start => Purpose::Start,
            Self::Stop => Purpose::Stop,
            Self::Wake => Purpose::Wake,
            Self::Refresh => Purpose::Refresh,
            Self::Ssh => Purpose::Ssh,
            Self::Delete => Purpose::Delete,
            Self::Status => Purpose::Status,
        }
    }

    /// Long enough for a cloud VM to boot or a VM image to download.
    fn timeout(self) -> Duration {
        Duration::from_secs(match self {
            Self::Create => 30 * 60,
            Self::Identity | Self::List | Self::Refresh | Self::Status | Self::Ssh => 2 * 60,
            Self::Start | Self::Wake | Self::Delete => 10 * 60,
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
    /// Sleeping, suspended, or paused; can be woken on demand.
    Sleeping,
    /// Shown as offline with Start.
    Stopped,
    /// Booting, provisioning, or waking up.
    Starting,
    /// No longer exists on the provider side.
    Missing,
    /// Anything else, as the provider printed it.
    Other(String),
}

/// Structured output from a provider's `status` command: JSON
/// `{ state, instance_id?, ssh? }` or plain-text state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub state: MachineStatus,
    pub instance_id: Option<String>,
    pub ssh: Option<SshTarget>,
}

/// One existing instance discovered by a provider's `list` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredInstance {
    pub name: String,
    pub state: MachineStatus,
    pub params: BTreeMap<String, String>,
    pub ssh: Option<SshTarget>,
    pub instance_id: Option<String>,
}

/// One provider, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    id: String,
    label: String,
    create: Option<String>,
    connect: Option<String>,
    identity: Option<String>,
    list: Option<String>,
    start: Option<String>,
    stop: Option<String>,
    wake: Option<String>,
    refresh: Option<String>,
    ssh: Option<String>,
    delete: Option<String>,
    status: Option<String>,
    wake_on_demand: bool,
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
                field: "connect".to_owned(),
                line: None,
                kind: ProblemKind::MissingLink,
            }]))),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Primary transport kind: `Connect` when `connect` is declared (even if
    /// `identity` is also declared to refresh SSH metadata/state), else
    /// `Identity`.
    pub fn reach(&self) -> Reach {
        if self.connect.is_some() {
            Reach::Connect
        } else {
            Reach::Identity
        }
    }

    pub fn has_connect(&self) -> bool {
        self.connect.is_some()
    }

    pub fn has_identity(&self) -> bool {
        self.identity.is_some()
    }

    /// Whether a sleeping or stopped machine can be woken on demand. Defaults
    /// to `true` when `wake` or `start` is declared.
    pub fn wake_on_demand(&self) -> bool {
        self.wake_on_demand
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
        let mut commands = Vec::new();
        for (field, command) in [
            ("create", &self.create),
            ("connect", &self.connect),
            ("identity", &self.identity),
            ("list", &self.list),
            ("start", &self.start),
            ("stop", &self.stop),
            ("wake", &self.wake),
            ("refresh", &self.refresh),
            ("ssh", &self.ssh),
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

    /// The provider-level `list` command for discovering existing instances;
    /// none when the provider doesn't declare `list`.
    pub fn list_invocation(&self) -> Option<Invocation> {
        let template = self.list.as_deref()?;
        let env = self
            .env
            .iter()
            .map(|name| (name.clone(), EnvValue::HostVariable(name.clone())))
            .collect();
        Some(Invocation {
            program: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), template.to_owned()],
            env,
            stdin: None,
            timeout: Some(Action::List.timeout()),
            purpose: Purpose::List,
        })
    }

    /// The link to `machine` through `connect`; none when `connect` is not
    /// declared (an identity-only provider's link comes from `identity`).
    pub fn link(
        &self,
        machine: &MachineName,
        values: &BTreeMap<String, String>,
    ) -> Result<Option<Link>, ValuesError> {
        let Some(template) = &self.connect else {
            return Ok(None);
        };
        Ok(Some(Link::connect(
            machine.clone(),
            template.clone(),
            self.environment(machine, values)?,
        )))
    }

    fn template(&self, action: Action) -> Option<&str> {
        match action {
            Action::Create => self.create.as_deref(),
            Action::Identity => self.identity.as_deref(),
            Action::List => self.list.as_deref(),
            Action::Start => self.start.as_deref(),
            Action::Stop => self.stop.as_deref(),
            Action::Wake => self
                .wake
                .as_deref()
                .or_else(|| self.wake_on_demand.then_some(self.start.as_deref()).flatten()),
            Action::Refresh => self.refresh.as_deref(),
            Action::Ssh => self.ssh.as_deref(),
            Action::Delete => self.delete.as_deref(),
            Action::Status => self.status.as_deref(),
        }
    }
}

/// Maps a single status word (from plain text or JSON `state`) to [`MachineStatus`].
pub(crate) fn parse_status_word(word: &str) -> MachineStatus {
    let trimmed = word.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "running" | "active" | "started" | "up" | "ready" => MachineStatus::Running,
        "sleeping" | "suspended" | "paused" | "hibernated" | "idle" | "asleep" => {
            MachineStatus::Sleeping
        }
        "stopped" | "off" | "terminated" | "shutoff" | "shut off" | "halted" | "exited"
        | "down" | "poweroff" | "deallocated" => MachineStatus::Stopped,
        "starting" | "booting" | "pending" | "provisioning" | "waking" | "resuming" => {
            MachineStatus::Starting
        }
        "missing" | "not_found" | "not found" | "deleted" | "destroyed" | "gone" => {
            MachineStatus::Missing
        }
        _ => MachineStatus::Other(trimmed.chars().take(64).collect()),
    }
}

/// Reads what a `status` command printed: one word, or JSON with a `state`
/// field (`jq -c '{state: .status}'`).
pub fn parse_status(stdout: &[u8]) -> MachineStatus {
    let text = String::from_utf8_lossy(stdout);
    let text = text.trim();
    let word = if text.starts_with('{') {
        serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| value.get("state")?.as_str().map(str::to_owned))
            .unwrap_or_default()
    } else {
        text.lines().last().unwrap_or_default().trim().to_owned()
    };
    parse_status_word(&word)
}

/// Reads structured JSON (`{ state, instance_id?, ssh? }`) or plain-text
/// output from a provider's `status` command. Refuses output containing key
/// material.
pub fn parse_status_report(stdout: &[u8]) -> Result<StatusReport, IdentityError> {
    if identity::has_key_material(stdout) {
        return Err(IdentityError::KeyMaterial);
    }
    let text = String::from_utf8_lossy(stdout);
    let trimmed = text.trim();
    if !trimmed.starts_with('{') {
        let word = trimmed.lines().last().unwrap_or_default().trim();
        return Ok(StatusReport {
            state: parse_status_word(word),
            instance_id: None,
            ssh: None,
        });
    }
    let value: Value =
        serde_json::from_str(trimmed).map_err(|source| IdentityError::NotJson { source })?;
    let Value::Object(map) = value else {
        return Err(IdentityError::Field {
            field: "output".to_owned(),
            problem: FieldProblem::NotObject,
        });
    };
    let state_word = match map.get("state") {
        Some(Value::String(s)) => s.as_str(),
        Some(_) => {
            return Err(IdentityError::Field {
                field: "state".to_owned(),
                problem: FieldProblem::NotString,
            });
        }
        None => {
            return Err(IdentityError::Field {
                field: "state".to_owned(),
                problem: FieldProblem::Missing,
            });
        }
    };
    let instance_id = match map.get("instance_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => (!s.trim().is_empty()).then(|| s.clone()),
        Some(_) => {
            return Err(IdentityError::Field {
                field: "instance_id".to_owned(),
                problem: FieldProblem::NotString,
            });
        }
    };
    let ssh = match map.get("ssh") {
        None | Some(Value::Null) => None,
        Some(Value::Object(obj)) => Some(identity::parse_ssh_object(obj, "ssh.")?),
        Some(_) => {
            return Err(IdentityError::Field {
                field: "ssh".to_owned(),
                problem: FieldProblem::NotObject,
            });
        }
    };
    Ok(StatusReport {
        state: parse_status_word(state_word),
        instance_id,
        ssh,
    })
}

/// Parses the JSON array printed by a provider's `list` command:
/// `[{ name, state, params?, ssh?, instance_id? }]`.
pub fn parse_list(stdout: &[u8]) -> Result<Vec<DiscoveredInstance>, IdentityError> {
    if identity::has_key_material(stdout) {
        return Err(IdentityError::KeyMaterial);
    }
    let value: Value =
        serde_json::from_slice(stdout).map_err(|source| IdentityError::NotJson { source })?;
    let Value::Array(items) = value else {
        return Err(IdentityError::Field {
            field: "output".to_owned(),
            problem: FieldProblem::NotArray,
        });
    };
    let mut instances = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        let prefix = format!("[{index}].");
        let Value::Object(map) = item else {
            return Err(IdentityError::Field {
                field: format!("[{index}]"),
                problem: FieldProblem::NotObject,
            });
        };
        let name = match map.get("name") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
            Some(Value::String(_)) => {
                return Err(IdentityError::Field {
                    field: format!("{prefix}name"),
                    problem: FieldProblem::Empty,
                });
            }
            Some(_) => {
                return Err(IdentityError::Field {
                    field: format!("{prefix}name"),
                    problem: FieldProblem::NotString,
                });
            }
            None => {
                return Err(IdentityError::Field {
                    field: format!("{prefix}name"),
                    problem: FieldProblem::Missing,
                });
            }
        };
        let state = match map.get("state") {
            Some(Value::String(s)) => parse_status_word(s),
            Some(_) => {
                return Err(IdentityError::Field {
                    field: format!("{prefix}state"),
                    problem: FieldProblem::NotString,
                });
            }
            None => {
                return Err(IdentityError::Field {
                    field: format!("{prefix}state"),
                    problem: FieldProblem::Missing,
                });
            }
        };
        let mut params = BTreeMap::new();
        match map.get("params") {
            None | Some(Value::Null) => {}
            Some(Value::Object(obj)) => {
                for (k, v) in obj {
                    match v {
                        Value::String(s) => {
                            params.insert(k.clone(), s.clone());
                        }
                        Value::Number(n) => {
                            params.insert(k.clone(), n.to_string());
                        }
                        Value::Bool(b) => {
                            params.insert(k.clone(), b.to_string());
                        }
                        _ => {
                            return Err(IdentityError::Field {
                                field: format!("{prefix}params.{k}"),
                                problem: FieldProblem::NotString,
                            });
                        }
                    }
                }
            }
            Some(_) => {
                return Err(IdentityError::Field {
                    field: format!("{prefix}params"),
                    problem: FieldProblem::NotObject,
                });
            }
        }
        let ssh = match map.get("ssh") {
            None | Some(Value::Null) => None,
            Some(Value::Object(obj)) => Some(identity::parse_ssh_object(obj, &format!("{prefix}ssh."))?),
            Some(_) => {
                return Err(IdentityError::Field {
                    field: format!("{prefix}ssh"),
                    problem: FieldProblem::NotObject,
                });
            }
        };
        let instance_id = match map.get("instance_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => (!s.trim().is_empty()).then(|| s.clone()),
            Some(_) => {
                return Err(IdentityError::Field {
                    field: format!("{prefix}instance_id"),
                    problem: FieldProblem::NotString,
                });
            }
        };
        instances.push(DiscoveredInstance {
            name,
            state,
            params,
            ssh,
            instance_id,
        });
    }
    Ok(instances)
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
    list: Option<Spanned<String>>,
    start: Option<Spanned<String>>,
    stop: Option<Spanned<String>>,
    wake: Option<Spanned<String>>,
    refresh: Option<Spanned<String>>,
    ssh: Option<Spanned<String>>,
    delete: Option<Spanned<String>>,
    status: Option<Spanned<String>>,
    wake_on_demand: Option<bool>,
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
        ("list", &raw.list),
        ("start", &raw.start),
        ("stop", &raw.stop),
        ("wake", &raw.wake),
        ("refresh", &raw.refresh),
        ("ssh", &raw.ssh),
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
    if raw.connect.is_none() && raw.identity.is_none() && raw.list.is_none() {
        problem("connect", None, ProblemKind::MissingLink);
    }
    if let Some(connect) = &raw.connect {
        if !connect.get_ref().contains(CMD) {
            problem("connect", line(connect), ProblemKind::ConnectWithoutCmd);
        }
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

    let wake_on_demand = raw
        .wake_on_demand
        .unwrap_or_else(|| raw.wake.is_some() || raw.start.is_some());
    Some(Provider {
        label: raw
            .label
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| id.clone()),
        id,
        create: raw.create.map(Spanned::into_inner),
        connect: raw.connect.map(Spanned::into_inner),
        identity: raw.identity.map(Spanned::into_inner),
        list: raw.list.map(Spanned::into_inner),
        start: raw.start.map(Spanned::into_inner),
        stop: raw.stop.map(Spanned::into_inner),
        wake: raw.wake.map(Spanned::into_inner),
        refresh: raw.refresh.map(Spanned::into_inner),
        ssh: raw.ssh.map(Spanned::into_inner),
        delete: raw.delete.map(Spanned::into_inner),
        status: raw.status.map(Spanned::into_inner),
        wake_on_demand,
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
