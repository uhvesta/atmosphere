//! What a host does for the daemon about the machine itself: takes its
//! variables and setup script, reports how it is, and clones repositories.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use humata_agent::Env;
use humata_wire::pb;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::error::HostError;
use crate::hex;

/// The most of a setup script's or git's output kept, from its end.
const OUTPUT_KEPT: usize = 8 * 1024;

/// How long a setup script may run.
const SETUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// How long `copilot --version` may take.
const VERSION_TIMEOUT: Duration = Duration::from_secs(20);

/// The variables that give agents a GitHub token.
const TOKENS: [&str; 3] = ["GH_TOKEN", "GITHUB_TOKEN", "COPILOT_GITHUB_TOKEN"];

/// Has git answer GitHub's request for a password with the host's token,
/// read from its variables each time, so the token is never written to the
/// clone's config or anywhere else on the host. The empty helper first
/// drops any the host's own git config names for GitHub.
const GITHUB_CREDENTIALS: [&str; 4] = [
    "-c",
    "credential.https://github.com.helper=",
    "-c",
    "credential.https://github.com.helper=!f() { test \"$1\" = get || exit 0; echo username=x-access-token; \
     echo \"password=${GH_TOKEN:-${GITHUB_TOKEN:-$COPILOT_GITHUB_TOKEN}}\"; }; f",
];

/// The variables that sign Claude Code in.
const ANTHROPIC_KEYS: [&str; 2] = ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"];

/// The last run of the setup script, kept beside the host's sessions so a
/// script that succeeded isn't run again on every connection.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SetupRun {
    /// SHA-256 of the script, in hex.
    script: String,
    succeeded: bool,
    output: String,
    ran_unix_ms: i64,
}

impl SetupRun {
    fn wire(&self) -> pb::SetupResult {
        pb::SetupResult {
            ran: true,
            succeeded: self.succeeded,
            output: self.output.clone(),
            ran_unix_ms: self.ran_unix_ms,
        }
    }
}

/// This machine as a host: its folder, the variables its agents get and the
/// Copilot program they run.
pub struct Machine {
    state: PathBuf,
    env: Env,
    copilot: PathBuf,
    claude: PathBuf,
    /// Held while the setup script runs, so it never runs twice at once.
    setup: tokio::sync::Mutex<()>,
}

impl Machine {
    pub fn new(state: PathBuf, env: Env, copilot: PathBuf, claude: PathBuf) -> Self {
        Self {
            state,
            env,
            copilot,
            claude,
            setup: tokio::sync::Mutex::new(()),
        }
    }

    /// Replaces the agents' variables; programs started from now on see
    /// the new ones.
    pub fn set_env(&self, env: BTreeMap<String, String>) {
        self.env.replace(env);
    }

    fn setup_file(&self) -> PathBuf {
        self.state.join("setup.json")
    }

    fn last_setup(&self) -> Option<SetupRun> {
        let bytes = std::fs::read(self.setup_file()).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Sets the agents' variables, and runs `script` if it changed since it
    /// last ran or failed then.
    pub async fn configure(
        &self,
        configure: pb::ConfigureHost,
    ) -> Result<pb::SetupResult, HostError> {
        let env: BTreeMap<String, String> = configure
            .env
            .into_iter()
            .map(|var| (var.name, var.value))
            .collect();
        self.env.replace(env);
        if configure.setup_script.trim().is_empty() {
            return Ok(pb::SetupResult::default());
        }
        let digest = hex(&Sha256::digest(configure.setup_script.as_bytes()));
        let _running = self.setup.lock().await;
        if let Some(last) = self.last_setup()
            && last.script == digest
            && last.succeeded
        {
            return Ok(last.wire());
        }
        let run = self.run_setup(&configure.setup_script, digest).await;
        let path = self.setup_file();
        let json = serde_json::to_vec(&run).unwrap_or_default();
        std::fs::create_dir_all(&self.state)
            .and_then(|()| std::fs::write(&path, json))
            .map_err(|source| HostError::SaveSetup { path, source })?;
        Ok(run.wire())
    }

    async fn run_setup(&self, script: &str, digest: String) -> SetupRun {
        let home = std::env::var_os("HOME").map_or_else(|| self.state.clone(), PathBuf::from);
        let child = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .current_dir(home)
            .envs(self.env.vars())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output();
        let (succeeded, output) = match tokio::time::timeout(SETUP_TIMEOUT, child).await {
            Err(_) => (
                false,
                format!(
                    "Stopped after {} minutes without finishing.",
                    SETUP_TIMEOUT.as_secs() / 60
                ),
            ),
            Ok(Err(error)) => (false, format!("Couldn't run /bin/sh: {error}")),
            Ok(Ok(output)) => (output.status.success(), printed(&output)),
        };
        SetupRun {
            script: digest,
            succeeded,
            output,
            ran_unix_ms: now(),
        }
    }

    pub async fn status(&self, program_version: String) -> pb::HostStatus {
        let platform = run(Command::new("uname").arg("-sm"))
            .await
            .map(|out| out.trim().to_owned())
            .unwrap_or_default();
        let free_disk_bytes = run(Command::new("df").arg("-Pk").arg(existing(&self.state)))
            .await
            .and_then(|out| free_kilobytes(&out))
            .map_or(0, |kilobytes| kilobytes * 1024);
        let (copilot_version, copilot_problem) = self
            .version(&self.copilot, "the Copilot CLI", "HUMATA_COPILOT")
            .await;
        let (claude_version, claude_problem) = self
            .version(&self.claude, "Claude Code", "HUMATA_CLAUDE")
            .await;
        let has = |name: &&str| {
            self.env.has(name) || std::env::var_os(name).is_some_and(|value| !value.is_empty())
        };
        let github_token = TOKENS.iter().any(has);
        let anthropic_key = ANTHROPIC_KEYS.iter().any(has);
        pb::HostStatus {
            program_version,
            platform,
            free_disk_bytes,
            copilot_version,
            copilot_problem,
            github_token,
            setup: self.last_setup().map(|run| run.wire()),
            claude_version,
            claude_problem,
            anthropic_key,
            environment_problem: crate::login_env::problem(),
        }
    }

    /// `program --version`'s first line, or why it couldn't be run.
    async fn version(&self, program: &Path, what: &str, variable: &str) -> (String, String) {
        let mut version = Command::new(program);
        version
            .arg("--version")
            .envs(self.env.vars())
            .stdin(Stdio::null())
            .kill_on_drop(true);
        match tokio::time::timeout(VERSION_TIMEOUT, version.output()).await {
            Err(_) => (
                String::new(),
                format!("`{} --version` didn't finish", program.display()),
            ),
            Ok(Err(error)) => (
                String::new(),
                format!(
                    "couldn't run {}: {error}. Install {what} on this host, or set {variable} to it",
                    program.display()
                ),
            ),
            Ok(Ok(output)) if output.status.success() => (
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
                String::new(),
            ),
            Ok(Ok(output)) => (String::new(), printed(&output)),
        }
    }

    /// Clones each repository under `repos/<folder>`, or fetches it if it is
    /// there already, and says where each is.
    pub async fn clone(
        &self,
        clone: pb::CloneRepositories,
    ) -> Result<pb::RepositoriesCloned, HostError> {
        let folder = self.state.join("repos").join(plain(&clone.folder)?);
        let mut cloned = Vec::new();
        for repository in clone.repositories {
            let path = folder.join(plain(&repository.name)?);
            let mut git = Command::new("git");
            let vars = self.env.vars();
            if vars.iter().any(|(name, _)| TOKENS.contains(&name.as_str())) {
                git.args(GITHUB_CREDENTIALS);
            }
            git.envs(vars).stdin(Stdio::null());
            if path.join(".git").exists() {
                git.arg("-C")
                    .arg(&path)
                    .args(["pull", "--ff-only", "--quiet"]);
            } else {
                std::fs::create_dir_all(&folder).map_err(|source| HostError::CreateFolder {
                    path: folder.clone(),
                    source,
                })?;
                git.arg("clone").arg(&repository.url).arg(&path);
            }
            let output = git
                .output()
                .await
                .map_err(|source| HostError::RunGit { source })?;
            if !output.status.success() {
                return Err(HostError::Clone {
                    url: repository.url,
                    path,
                    output: printed(&output),
                });
            }
            cloned.push(pb::Repository {
                name: repository.name,
                path: path.display().to_string(),
                url: String::new(),
            });
        }
        let mut tips = Vec::with_capacity(cloned.len());
        for repository in &cloned {
            let commits = humata_agent::bundle::tips(Path::new(&repository.path)).await;
            tips.push(pb::RepositoryCommits {
                name: repository.name.clone(),
                // Without them, what is sent is only larger.
                commits: commits.unwrap_or_default(),
            });
        }
        Ok(pb::RepositoriesCloned {
            repositories: cloned,
            tips,
        })
    }

    /// The skills in the agents' folders in the home folder and in each
    /// repository, read on a background thread; a repository with no path
    /// is this host's clone of it, if there is one.
    pub async fn scan_skills(&self, scan: pb::ScanSkills) -> Result<pb::SkillsScanned, HostError> {
        let mut repositories = Vec::new();
        for folder in scan.repositories {
            let path = if folder.path.is_empty() {
                self.state
                    .join("repos")
                    .join(plain(&folder.project_id)?)
                    .join(plain(&folder.name)?)
            } else {
                PathBuf::from(&folder.path)
            };
            repositories.push((folder, path));
        }
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let skills = tokio::task::spawn_blocking(move || {
            crate::skills::scan(home.as_deref(), &repositories)
        })
        .await
        .map_err(|source| HostError::ScanSkills { source })?;
        Ok(pb::SkillsScanned { skills })
    }

    /// The plugins Claude Code and Copilot have installed here, read on a
    /// background thread.
    pub async fn scan_plugins(&self) -> Result<pb::PluginsScanned, HostError> {
        let plugins = tokio::task::spawn_blocking(|| {
            let (claude, copilot) = crate::plugins::folders();
            crate::plugins::scan(claude.as_deref(), copilot.as_deref())
        })
        .await
        .map_err(|source| HostError::ScanPlugins { source })?;
        Ok(pb::PluginsScanned { plugins })
    }

    /// Pushes `branch` from the worktree at `directory` to its `origin`,
    /// setting it as the branch's upstream.
    pub async fn push(
        &self,
        directory: &Path,
        repository: &str,
        branch: &str,
    ) -> Result<(), HostError> {
        let output = self
            .remote_git(
                directory,
                &["push", "--quiet", "--set-upstream", "origin", branch],
            )
            .await?;
        if output.status.success() {
            return Ok(());
        }
        Err(HostError::Push {
            repository: repository.to_owned(),
            branch: branch.to_owned(),
            output: printed(&output),
        })
    }

    /// Fetches the worktree at `directory`'s `origin`, if it has one.
    pub async fn fetch(&self, directory: &Path, repository: &str) -> Result<(), HostError> {
        if origin(directory).await.is_empty() {
            return Ok(());
        }
        let output = self
            .remote_git(directory, &["fetch", "--quiet", "origin"])
            .await?;
        if output.status.success() {
            return Ok(());
        }
        Err(HostError::Fetch {
            repository: repository.to_owned(),
            output: printed(&output),
        })
    }

    /// Runs git in `directory` with the host's variables, answering
    /// GitHub's request for a password with the host's token if it has one.
    async fn remote_git(
        &self,
        directory: &Path,
        arguments: &[&str],
    ) -> Result<std::process::Output, HostError> {
        let mut git = Command::new("git");
        let vars = self.env.vars();
        if vars.iter().any(|(name, _)| TOKENS.contains(&name.as_str())) {
            git.args(GITHUB_CREDENTIALS);
        }
        git.envs(vars)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .output()
            .await
            .map_err(|source| HostError::RunGit { source })
    }
}

/// The URL of the `origin` remote of the repository at `directory`; empty
/// if it has none.
pub async fn origin(directory: &Path) -> String {
    Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(["remote", "get-url", "origin"])
        .stdin(Stdio::null())
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// `name` if it is one plain folder name, not a path.
fn plain(name: &str) -> Result<&str, HostError> {
    let mut parts = Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(name),
        _ => Err(HostError::NotAName {
            name: name.to_owned(),
        }),
    }
}

/// The nearest folder at or above `path` that exists, for `df`.
fn existing(path: &Path) -> &Path {
    path.ancestors()
        .find(|path| path.exists())
        .unwrap_or(Path::new("/"))
}

/// The available kilobytes in `df -Pk`'s output.
fn free_kilobytes(output: &str) -> Option<u64> {
    output
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()
}

async fn run(command: &mut Command) -> Option<String> {
    let output = command.stdin(Stdio::null()).output().await.ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// What a program printed, both streams, cut to its last [`OUTPUT_KEPT`]
/// bytes.
fn printed(output: &std::process::Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let text = text.trim();
    if text.len() <= OUTPUT_KEPT {
        return text.to_owned();
    }
    let mut start = text.len() - OUTPUT_KEPT;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(state: &Path) -> Machine {
        Machine::new(
            state.to_owned(),
            Env::default(),
            PathBuf::from("/nonexistent/copilot"),
            PathBuf::from("/nonexistent/claude"),
        )
    }

    fn configure(script: &str, env: &[(&str, &str)]) -> pb::ConfigureHost {
        pb::ConfigureHost {
            env: env
                .iter()
                .map(|(name, value)| pb::EnvVar {
                    name: (*name).to_owned(),
                    value: (*value).to_owned(),
                })
                .collect(),
            setup_script: script.to_owned(),
            model_server_port: 0,
        }
    }

    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"))
    }

    #[tokio::test]
    async fn the_setup_script_runs_with_the_variables_and_not_again_once_it_succeeded() {
        let state = scratch();
        let counter = state.path().join("runs");
        let script = format!("echo \"$GREETING\" >> {}", counter.display());
        let machine = machine(state.path());

        let first = machine
            .configure(configure(&script, &[("GREETING", "hello")]))
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let again = machine
            .configure(configure(&script, &[("GREETING", "hello")]))
            .await
            .unwrap_or_else(|error| panic!("{error}"));

        assert!(first.ran && first.succeeded);
        assert_eq!(again.ran_unix_ms, first.ran_unix_ms);
        assert_eq!(
            std::fs::read_to_string(&counter).ok().as_deref(),
            Some("hello\n")
        );
    }

    #[tokio::test]
    async fn a_failed_setup_script_runs_again_and_says_what_it_printed() {
        let state = scratch();
        let machine = machine(state.path());
        let script = "echo missing tool; exit 3";

        let first = machine
            .configure(configure(script, &[]))
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let status = machine.status("test".to_owned()).await;

        assert!(first.ran && !first.succeeded);
        assert!(first.output.contains("missing tool"));
        assert_eq!(status.setup, Some(first));
    }

    #[tokio::test]
    async fn the_status_has_the_disk_and_a_token_given_by_the_daemon_and_says_why_copilot_is_missing()
     {
        let state = scratch();
        let machine = machine(state.path());
        let _ = machine
            .configure(configure("", &[("GH_TOKEN", "secret")]))
            .await;

        let status = machine.status("test".to_owned()).await;

        assert!(status.free_disk_bytes > 0);
        assert!(!status.platform.is_empty());
        assert!(status.github_token);
        assert!(status.copilot_version.is_empty());
        assert!(!status.copilot_problem.is_empty());
        assert!(status.claude_version.is_empty());
        assert!(!status.claude_problem.is_empty());
        assert!(!status.anthropic_key);
    }

    #[tokio::test]
    async fn cloning_twice_fetches_and_a_path_for_a_name_is_refused() {
        let state = scratch();
        let origin = state.path().join("origin");
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@localhost")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@localhost")
                .status()
                .unwrap_or_else(|error| panic!("{error}"));
            assert!(status.success());
        };
        let origin_path = origin.display().to_string();
        git(&["init", "-q", &origin_path]);
        git(&[
            "-C",
            &origin_path,
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "first",
        ]);
        let machine = machine(&state.path().join("host"));
        let request = |name: &str| pb::CloneRepositories {
            folder: "project".to_owned(),
            repositories: vec![pb::RemoteRepository {
                name: name.to_owned(),
                url: origin_path.clone(),
            }],
        };

        let first = machine
            .clone(request("app"))
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let second = machine
            .clone(request("app"))
            .await
            .unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(first, second);
        assert!(Path::new(&first.repositories[0].path).join(".git").exists());
        assert!(matches!(
            machine.clone(request("../escape")).await,
            Err(HostError::NotAName { .. })
        ));
    }
}
