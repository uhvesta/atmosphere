//! The agent switches at a start (AGENTSET, D720-D723): a kind the person
//! turned off is refused, and on the machine that runs the agent its CLI
//! must be found, installed first when the person said Humata may.
//!
//! The switches file (`agents.toml`) is read afresh for every check, so a
//! copy the host ships takes effect without a restart. Nothing here runs
//! for sessions that already exist: resume, restore, move and rebuild never
//! call it.
//!
//! "Found" is what launching does: the holder starts the program with the
//! daemon's own environment and no login shell, so a bare name is found on
//! the daemon's `PATH`, and a path is found when it is an executable file.
//! A CLI an installer puts somewhere the daemon's `PATH` doesn't reach is
//! therefore not found, and the install counts as failed, which says so.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Agents;
use tonic::{Code, Status};

use crate::agent_switches::{AgentSwitches, CliToFind, cli_to_find};
use crate::status;

/// How long an install may run.
const INSTALL_DEADLINE: Duration = Duration::from_secs(10 * 60);
/// How many of the last lines of an install's output a failure keeps.
const TAIL_LINES: usize = 20;

/// Why a start was refused by the agent switches.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GateError {
    #[error("{display} is turned off in Settings › Agents, so Humata won't start it")]
    Disabled { display: String },
    #[error("{display} isn't installed on this machine, or Humata can't find it: `{cli}` was not found")]
    Missing {
        display: String,
        cli: String,
        /// Whether Humata has a command to install it.
        can_install: bool,
        /// The install command, when there is one.
        command: Option<String>,
    },
    #[error("Every agent is turned off in Settings › Agents, so Humata has none to start")]
    NoneOn,
    #[error("{display} is turned off in Settings › Agents, so the fork can't run it")]
    ForkAgentOff { display: String },
    #[error("{display} is turned off in Settings › Agents, so this session can't continue in another channel")]
    ContinueAgentOff { display: String },
    #[error("installing {kind} with `{command}` failed")]
    InstallFailed {
        kind: String,
        command: String,
        /// The last lines of what the command printed.
        tail: String,
        #[source]
        source: Option<std::io::Error>,
    },
}

impl GateError {
    /// What the app and CLI show for this refusal.
    pub(crate) fn to_error(&self) -> pb::Error {
        let error = self;
        let (message, fix) = match self {
            GateError::Disabled { display } => (
                error.to_string(),
                format!("turn {display} on in Settings › Agents › {display}, then start the session again"),
            ),
            GateError::NoneOn => (
                error.to_string(),
                "turn an agent on in Settings › Agents, or name zsh for a plain shell".to_owned(),
            ),
            GateError::ForkAgentOff { display } => (
                error.to_string(),
                format!("name an agent that is on for the fork, or turn {display} on in Settings › Agents"),
            ),
            GateError::ContinueAgentOff { display } => (
                error.to_string(),
                format!("turn {display} on in Settings › Agents, then try again"),
            ),
            GateError::Missing { cli, command, can_install, display } => (
                error.to_string(),
                match (can_install, command) {
                    (true, Some(command)) => format!(
                        "install {cli} on this machine yourself (`{command}`), or turn on \"Install when missing\" under {display} in Settings › Agents and start again"
                    ),
                    _ => format!(
                        "install {cli} on this machine yourself so it is on the daemon's PATH, then start again; Humata has no install command for it, so \"Install when missing\" can't do it"
                    ),
                },
            ),
            GateError::InstallFailed { command, tail, source, .. } => (
                match source {
                    Some(source) => format!("{error}: {source}\n{tail}"),
                    None => format!("{error}\n{tail}"),
                },
                format!(
                    "run `{command}` yourself on this machine (npm needs Node.js installed first, and the CLI must end up on the daemon's PATH), or turn \"Install when missing\" off in Settings › Agents and install it your way"
                ),
            ),
        };
        pb::Error { message, fix, retry: None, kind: None }
    }
}

impl From<GateError> for Status {
    fn from(error: GateError) -> Self {
        status::failure(Code::FailedPrecondition, error.to_error())
    }
}

static LAST_BAD_FILE: Mutex<Option<String>> = Mutex::new(None);

/// The switches in the agents' file, or the defaults when there is no file
/// path or the file is bad (logged once per distinct problem).
pub(crate) fn switches(agents: &Agents) -> Option<AgentSwitches> {
    let path = agents.settings_file()?;
    match AgentSwitches::read(path) {
        Ok(switches) => {
            *LAST_BAD_FILE.lock().unwrap_or_else(PoisonError::into_inner) = None;
            Some(switches)
        }
        Err(error) => {
            let text = status::chain(&error);
            let mut last = LAST_BAD_FILE.lock().unwrap_or_else(PoisonError::into_inner);
            if last.as_deref() != Some(text.as_str()) {
                crate::log::say!("humatad2: warning: {text}; every agent is treated as on, with no installs");
                *last = Some(text);
            }
            AgentSwitches::parse("").ok()
        }
    }
}

/// Refuses `kind` when the person turned it off.
pub(crate) fn enabled(agents: &Agents, kind: &str) -> Result<(), GateError> {
    match switches(agents) {
        Some(switches) if !switches.enabled(kind) => Err(GateError::Disabled {
            display: agents.person_name(kind),
        }),
        _ => Ok(()),
    }
}

/// Like [`enabled`], but a refusal is the error `make` builds from the
/// agent's display name, for starts that say why in their own words.
pub(crate) fn enabled_or(
    agents: &Agents,
    kind: &str,
    make: fn(String) -> GateError,
) -> Result<(), GateError> {
    match enabled(agents, kind) {
        Err(GateError::Disabled { display }) => Err(make(display)),
        other => other,
    }
}

/// The agent a start that names none runs: the first of `preferred` that is
/// on, else the first agent that is on, in the agents' order (never the
/// plain shell unless it is preferred). Without a switches file the first
/// non-empty preferred wins.
pub(crate) fn pick(agents: &Agents, preferred: &[String]) -> Result<String, GateError> {
    let preferred: Vec<&str> = preferred.iter().map(String::as_str).filter(|name| !name.is_empty()).collect();
    let Some(switches) = switches(agents) else {
        return preferred.first().map(|name| (*name).to_owned()).ok_or(GateError::NoneOn);
    };
    let order: Vec<&str> = agents.kinds().iter().map(|kind| kind.name.as_str()).collect();
    switches
        .pick(&preferred, &order)
        .map(str::to_owned)
        .ok_or(GateError::NoneOn)
}

/// What a start on this machine needs before it can run the agent.
/// Without a switches file (a daemon built without a home) nothing is
/// checked, so an agent the daemon was told to run runs as before.
/// `Ok(true)` means the CLI is missing and Humata may install it, which
/// [`install_now`] does; `Ok(false)` means it is found.
pub(crate) fn check(agents: &Agents, kind: &str) -> Result<bool, GateError> {
    let Some(switches) = switches(agents) else {
        return Ok(false);
    };
    enabled(agents, kind)?;
    let Ok(found) = agents.get(kind) else {
        return Ok(false);
    };
    let target = cli_to_find(kind, &found.program, &found.start_args);
    if is_found(&target) {
        return Ok(false);
    }
    let command = switches.install_command(kind);
    if command.is_some() && switches.install_when_missing(kind) {
        return Ok(true);
    }
    Err(GateError::Missing {
        display: agents.person_name(kind),
        cli: describe(&target),
        can_install: command.is_some(),
        command,
    })
}

/// Installs the kind's CLI, once even for starts at the same time: the
/// second waits for the first and then finds the CLI. Never installs over
/// a CLI that is found.
pub(crate) async fn install_now(agents: &Agents, kind: &str) -> Result<(), GateError> {
    let (Some(switches), Ok(found)) = (switches(agents), agents.get(kind)) else {
        return Ok(());
    };
    let target = cli_to_find(kind, &found.program, &found.start_args);
    let Some(command) = switches.install_command(kind) else {
        return Ok(());
    };
    let lock = install_lock(kind);
    let _held = lock.lock().await;
    if is_found(&target) {
        return Ok(());
    }
    install(kind, &command).await?;
    if is_found(&target) {
        Ok(())
    } else {
        Err(GateError::InstallFailed {
            kind: kind.to_owned(),
            command,
            tail: format!(
                "the command finished, but `{}` is still not found on the daemon's PATH",
                describe(&target)
            ),
            source: None,
        })
    }
}

/// Enabled, then found, installing first when it is missing and allowed.
pub(crate) async fn ready(agents: &Agents, kind: &str) -> Result<(), GateError> {
    if check(agents, kind)? {
        install_now(agents, kind).await?;
    }
    Ok(())
}

fn describe(target: &CliToFind) -> String {
    match target {
        CliToFind::Name(name) => name.clone(),
        CliToFind::Path(path) => path.display().to_string(),
    }
}

fn is_found(target: &CliToFind) -> bool {
    match target {
        CliToFind::Path(path) => is_executable(path),
        CliToFind::Name(name) => std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).any(|dir| is_executable(&dir.join(name))))
            .unwrap_or(false),
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

type Locks = Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

fn install_lock(kind: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Locks> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Locks::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    locks.entry(kind.to_owned()).or_default().clone()
}

async fn install(kind: &str, command: &str) -> Result<(), GateError> {
    let failed = |tail: String, source: Option<std::io::Error>| GateError::InstallFailed {
        kind: kind.to_owned(),
        command: command.to_owned(),
        tail,
        source,
    };
    crate::log::say!("humatad2: installing {kind}: {command}");
    let child = tokio::process::Command::new("bash")
        .arg("-lc")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| failed(String::new(), Some(source)))?;
    let output = match tokio::time::timeout(INSTALL_DEADLINE, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(source)) => return Err(failed(String::new(), Some(source))),
        Err(_) => {
            return Err(failed(
                format!("it did not finish within {} minutes", INSTALL_DEADLINE.as_secs() / 60),
                None,
            ));
        }
    };
    if output.status.success() {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n");
    Err(failed(tail, None))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::*;

    /// Agents with one custom kind `fake` whose program is `bin`, switches
    /// in `dir/agents.toml`.
    fn rig(dir: &Path, toml: &str) -> (Agents, PathBuf) {
        let bin = dir.join("fake-cli");
        let json = serde_json::json!([{
            "name": "fake",
            "program": bin,
            "start_args": [],
            "resume_args": [],
            "conversation_file": "",
        }]);
        let settings = dir.join("agents.toml");
        std::fs::write(&settings, toml).unwrap();
        let agents = Agents::from_json(json.to_string().as_bytes())
            .unwrap()
            .with_settings(settings);
        (agents, bin)
    }

    /// Agents with kinds claude, codex and copilot, switches in `dir/agents.toml`.
    fn trio(dir: &Path, toml: Option<&str>) -> Agents {
        let kinds: Vec<_> = ["claude", "codex", "copilot"]
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "program": dir.join(name),
                    "start_args": [],
                    "resume_args": [],
                    "conversation_file": "",
                })
            })
            .collect();
        let agents = Agents::from_json(serde_json::Value::Array(kinds).to_string().as_bytes()).unwrap();
        match toml {
            Some(toml) => {
                let settings = dir.join("agents.toml");
                std::fs::write(&settings, toml).unwrap();
                agents.with_settings(settings)
            }
            None => agents,
        }
    }

    #[test]
    fn a_start_naming_none_skips_agents_that_are_off() {
        let dir = tempfile::tempdir().unwrap();
        let agents = trio(
            dir.path(),
            Some("[agents.claude]\nenabled = false\n[agents.codex]\nenabled = false\n"),
        );
        assert_eq!(pick(&agents, &["claude".to_owned()]).unwrap(), "copilot");
    }

    #[test]
    fn a_start_naming_none_with_every_agent_off_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let off = "[agents.claude]\nenabled = false\n[agents.codex]\nenabled = false\n[agents.copilot]\nenabled = false\n";
        let agents = trio(dir.path(), Some(off));
        assert!(matches!(pick(&agents, &["claude".to_owned()]), Err(GateError::NoneOn)));
    }

    #[test]
    fn without_a_settings_file_the_preferred_agent_is_picked() {
        let dir = tempfile::tempdir().unwrap();
        let agents = trio(dir.path(), None);
        assert_eq!(pick(&agents, &["codex".to_owned()]).unwrap(), "codex");
    }

    fn installer(dir: &Path, bin: &Path, ok: bool) -> String {
        let count = dir.join("count");
        let make = if ok {
            format!("printf '#!/bin/sh\\n' > '{0}'; chmod +x '{0}'", bin.display())
        } else {
            "echo boom >&2; exit 3".to_owned()
        };
        format!("sleep 0.3; echo x >> '{}'; {make}", count.display())
    }

    #[test]
    fn a_disabled_kind_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (agents, _) = rig(dir.path(), "[agents.fake]\nenabled = false\n");
        assert!(matches!(enabled(&agents, "fake"), Err(GateError::Disabled { .. })));
    }

    #[test]
    fn without_a_settings_path_everything_is_allowed() {
        let agents = Agents::builtin();
        assert!(enabled(&agents, "claude").is_ok());
    }

    #[tokio::test]
    async fn without_a_settings_path_nothing_is_looked_for() {
        let agents = Agents::builtin();
        assert!(ready(&agents, "claude").await.is_ok());
    }

    #[tokio::test]
    async fn a_missing_cli_with_install_off_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let (agents, _) = rig(dir.path(), "");
        assert!(matches!(ready(&agents, "fake").await, Err(GateError::Missing { .. })));
    }

    #[tokio::test]
    async fn a_found_cli_is_ready_and_never_installed() {
        let dir = tempfile::tempdir().unwrap();
        let (agents, bin) = rig(dir.path(), "");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = installer(dir.path(), &dir.path().join("elsewhere"), true);
        let toml = format!("[agents.fake]\ninstall_when_missing = true\ninstall = {command:?}\n");
        std::fs::write(dir.path().join("agents.toml"), toml).unwrap();
        assert!(ready(&agents, "fake").await.is_ok());
        assert!(!dir.path().join("count").exists());
    }

    #[tokio::test]
    async fn two_starts_at_once_install_once() {
        let dir = tempfile::tempdir().unwrap();
        let (agents, bin) = rig(dir.path(), "");
        let command = installer(dir.path(), &bin, true);
        let toml = format!("[agents.fake]\ninstall_when_missing = true\ninstall = {command:?}\n");
        std::fs::write(dir.path().join("agents.toml"), toml).unwrap();
        let (a, b) = tokio::join!(ready(&agents, "fake"), ready(&agents, "fake"));
        assert!(a.is_ok() && b.is_ok());
        let runs = std::fs::read_to_string(dir.path().join("count")).unwrap();
        assert_eq!(runs.lines().count(), 1);
    }

    #[tokio::test]
    async fn a_failing_install_is_install_failed() {
        let dir = tempfile::tempdir().unwrap();
        let (agents, bin) = rig(dir.path(), "");
        let command = installer(dir.path(), &bin, false);
        let toml = format!("[agents.fake]\ninstall_when_missing = true\ninstall = {command:?}\n");
        std::fs::write(dir.path().join("agents.toml"), toml).unwrap();
        match ready(&agents, "fake").await {
            Err(GateError::InstallFailed { tail, .. }) => assert!(tail.contains("boom")),
            other => panic!("expected InstallFailed, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_cli_with_install_on_asks_for_an_install_and_a_found_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let (agents, bin) = rig(dir.path(), "[agents.fake]\ninstall_when_missing = true\ninstall = \"true\"\n");
        assert!(check(&agents, "fake").unwrap());
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!check(&agents, "fake").unwrap());
    }
}
