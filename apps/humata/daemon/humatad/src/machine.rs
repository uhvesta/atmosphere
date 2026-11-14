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
use crate::fresh;
use crate::hex;

/// The most of a setup script's or git's output kept, from its end.
const OUTPUT_KEPT: usize = 8 * 1024;

/// How long a setup script may run.
const SETUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// How long a first clone may take: long enough for a large repository on
/// a slow link, but bounded, so a remote that stops answering doesn't hold
/// the session's start forever.
const CLONE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How long a push or a fetch may take before git is stopped.
const REMOTE_GIT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

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

    /// The last run as it was kept, which an older humatad may have
    /// written with the host's values in its output.
    fn kept_setup(&self) -> Option<SetupRun> {
        let bytes = std::fs::read(self.setup_file()).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// The last run, its output without the host's values.
    fn last_setup(&self) -> Option<SetupRun> {
        let mut run = self.kept_setup()?;
        run.output = redacted(&run.output, &self.secrets());
        Some(run)
    }

    /// Writes the last run where only this user can read it, replacing the
    /// file whole so a crash leaves the old one.
    fn save_setup(&self, run: &SetupRun) -> Result<(), HostError> {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let path = self.setup_file();
        let json = serde_json::to_vec(run).unwrap_or_default();
        let temporary = path.with_extension("json.new");
        let written = std::fs::create_dir_all(&self.state)
            .and_then(|()| {
                // One left by a crash may have other modes.
                let _ = std::fs::remove_file(&temporary);
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&temporary)
            })
            .and_then(|mut file| file.write_all(&json).and_then(|()| file.sync_all()))
            .and_then(|()| std::fs::rename(&temporary, &path));
        written.map_err(|source| HostError::SaveSetup { path, source })
    }

    /// Every value that must not be shown in what a program printed: the
    /// variables' values, and humatad's own values of the names that sign
    /// agents in. Humata can't tell which variables are secrets, so all are.
    fn secrets(&self) -> Vec<String> {
        let mut secrets: Vec<String> = self
            .env
            .vars()
            .into_iter()
            .map(|(_, value)| value)
            .collect();
        secrets.extend(
            TOKENS
                .iter()
                .chain(&ANTHROPIC_KEYS)
                .filter_map(|name| std::env::var(name).ok()),
        );
        secrets
    }

    /// What a program printed, both streams, without the host's values and
    /// cut to its last [`OUTPUT_KEPT`] bytes. The values are taken out
    /// before the cut, so the cut can't leave half of one.
    fn printed(&self, output: &std::process::Output) -> String {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        let text = redacted(&text, &self.secrets());
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
        let _running = self.setup.lock().await;
        if configure.setup_script.trim().is_empty() {
            // The host has no script now, so its status shows no run of one.
            let path = self.setup_file();
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(pb::SetupResult::default()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(pb::SetupResult::default())
                }
                Err(source) => Err(HostError::SaveSetup { path, source }),
            };
        }
        let digest = hex(&Sha256::digest(configure.setup_script.as_bytes()));
        if let Some(kept) = self.kept_setup()
            && kept.script == digest
            && kept.succeeded
        {
            let mut last = kept.clone();
            last.output = redacted(&kept.output, &self.secrets());
            if last.output != kept.output {
                // Kept by an older humatad with a value in it.
                self.save_setup(&last)?;
            }
            return Ok(last.wire());
        }
        let run = self.run_setup(&configure.setup_script, digest).await;
        self.save_setup(&run)?;
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
            Ok(Ok(output)) => (output.status.success(), self.printed(&output)),
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
            Ok(Ok(output)) => (String::new(), self.printed(&output)),
        }
    }

    /// Clones each repository under `repos/<folder>`, or fetches it if it is
    /// there already, and says where each is.
    pub async fn clone(
        &self,
        clone: pb::CloneRepositories,
    ) -> Result<pb::RepositoriesCloned, HostError> {
        self.clone_within(clone, CLONE_TIMEOUT).await
    }

    /// As [`Machine::clone`], giving each first clone `patience` to finish.
    async fn clone_within(
        &self,
        clone: pb::CloneRepositories,
        patience: Duration,
    ) -> Result<pb::RepositoriesCloned, HostError> {
        let folder = self.state.join("repos").join(plain(&clone.folder)?);
        let mut cloned = Vec::new();
        let mut notices = Vec::new();
        let vars = self.env.vars();
        let signed_in = vars.iter().any(|(name, _)| TOKENS.contains(&name.as_str()));
        let git = || {
            let mut git = Command::new("git");
            if signed_in {
                git.args(GITHUB_CREDENTIALS);
            }
            git.envs(vars.iter().cloned()).stdin(Stdio::null());
            git
        };
        for repository in clone.repositories {
            let path = folder.join(plain(&repository.name)?);
            // Held while it's cloned or updated, so a session starting from
            // it at the same time waits for this one rather than racing it.
            let _preparing = fresh::preparing(&path).await;
            if path.join(".git").exists() {
                let kind = fresh::Kind::Clone;
                let notice =
                    fresh::update(&git, &path, &repository.name, kind, "on this host").await;
                notices.extend(notice);
                cloned.push(pb::Repository {
                    name: repository.name,
                    path: path.display().to_string(),
                    url: String::new(),
                });
                continue;
            }
            std::fs::create_dir_all(&folder).map_err(|source| HostError::CreateFolder {
                path: folder.clone(),
                source,
            })?;
            let cloning = git()
                .arg("clone")
                .arg(&repository.url)
                .arg(&path)
                .env("GIT_TERMINAL_PROMPT", "0")
                // Dropping it when time is up stops git.
                .kill_on_drop(true)
                .output();
            let output = match tokio::time::timeout(patience, cloning).await {
                Ok(output) => output.map_err(|source| HostError::RunGit { source })?,
                Err(_) => {
                    // What it cloned so far would be taken for a clone.
                    let _ = std::fs::remove_dir_all(&path);
                    return Err(HostError::CloneTimedOut {
                        url: repository.url,
                        path,
                        minutes: minutes(patience),
                    });
                }
            };
            if !output.status.success() {
                return Err(HostError::Clone {
                    url: repository.url,
                    path,
                    output: self.printed(&output),
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
            notices,
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
        self.push_within(directory, repository, branch, REMOTE_GIT_TIMEOUT)
            .await
    }

    /// [`Self::push`], stopping git when it takes longer than `patience`.
    async fn push_within(
        &self,
        directory: &Path,
        repository: &str,
        branch: &str,
        patience: Duration,
    ) -> Result<(), HostError> {
        let arguments = ["push", "--quiet", "--set-upstream", "origin", branch];
        let Some(output) = self.remote_git(directory, &arguments, patience).await? else {
            return Err(HostError::PushTimedOut {
                repository: repository.to_owned(),
                branch: branch.to_owned(),
                minutes: minutes(patience),
            });
        };
        if output.status.success() {
            return Ok(());
        }
        Err(HostError::Push {
            repository: repository.to_owned(),
            branch: branch.to_owned(),
            output: self.printed(&output),
        })
    }

    /// Fetches the worktree at `directory`'s `origin`, if it has one.
    pub async fn fetch(&self, directory: &Path, repository: &str) -> Result<(), HostError> {
        self.fetch_within(directory, repository, REMOTE_GIT_TIMEOUT)
            .await
    }

    /// [`Self::fetch`], stopping git when it takes longer than `patience`.
    async fn fetch_within(
        &self,
        directory: &Path,
        repository: &str,
        patience: Duration,
    ) -> Result<(), HostError> {
        if origin(directory).await.is_empty() {
            return Ok(());
        }
        let arguments = ["fetch", "--quiet", "origin"];
        let Some(output) = self.remote_git(directory, &arguments, patience).await? else {
            return Err(HostError::FetchTimedOut {
                repository: repository.to_owned(),
                minutes: minutes(patience),
            });
        };
        if output.status.success() {
            return Ok(());
        }
        Err(HostError::Fetch {
            repository: repository.to_owned(),
            output: self.printed(&output),
        })
    }

    /// Runs git in `directory` with the host's variables, answering
    /// GitHub's request for a password with the host's token if it has one.
    /// None if it took longer than `patience` and was stopped: a remote
    /// that takes the connection and never answers would hold it forever.
    async fn remote_git(
        &self,
        directory: &Path,
        arguments: &[&str],
        patience: Duration,
    ) -> Result<Option<std::process::Output>, HostError> {
        let mut git = Command::new("git");
        let vars = self.env.vars();
        if vars.iter().any(|(name, _)| TOKENS.contains(&name.as_str())) {
            git.args(GITHUB_CREDENTIALS);
        }
        let running = git
            .envs(vars)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .arg("-C")
            .arg(directory)
            .args(arguments)
            // Dropping it when time is up stops git.
            .kill_on_drop(true)
            .output();
        match tokio::time::timeout(patience, running).await {
            Ok(output) => output
                .map(Some)
                .map_err(|source| HostError::RunGit { source }),
            Err(_) => Ok(None),
        }
    }
}

/// `patience` in whole minutes, for a message, rounded up.
fn minutes(patience: Duration) -> u64 {
    patience.as_secs().div_ceil(60)
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

/// The shortest value taken out of what a program printed. A shorter one,
/// such as `1` or `true`, is in too much ordinary text to be told apart.
const SECRET_LEAST: usize = 8;

/// `text` with each of `secrets` of [`SECRET_LEAST`] bytes or more replaced
/// by `***`, the longest first, so one that holds another leaves no tail.
fn redacted(text: &str, secrets: &[String]) -> String {
    let mut secrets: Vec<&str> = secrets
        .iter()
        .map(String::as_str)
        .filter(|secret| secret.len() >= SECRET_LEAST)
        .collect();
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    secrets.dedup();
    let mut text = text.to_owned();
    for secret in secrets {
        if text.contains(secret) {
            text = text.replace(secret, "***");
        }
    }
    text
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
    async fn a_setup_scripts_output_keeps_no_variables_value() {
        use std::os::unix::fs::PermissionsExt as _;
        let state = scratch();
        let machine = machine(state.path());
        let script = "echo \"token is $DEPLOY_TOKEN\"; exit 3";

        let result = machine
            .configure(configure(script, &[("DEPLOY_TOKEN", "hunter2-hunter2")]))
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let status = machine.status("test".to_owned()).await;

        let file = state.path().join("setup.json");
        let kept = std::fs::read_to_string(&file).unwrap_or_else(|error| panic!("{error}"));
        let mode = std::fs::metadata(&file)
            .unwrap_or_else(|error| panic!("{error}"))
            .permissions()
            .mode();
        assert!(result.ran && !result.succeeded);
        assert!(result.output.contains("token is"));
        assert!(!result.output.contains("hunter2-hunter2"));
        assert!(!kept.contains("hunter2-hunter2"));
        assert!(
            status
                .setup
                .is_some_and(|setup| !setup.output.contains("hunter2-hunter2"))
        );
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_value_is_left_out_wherever_it_is_printed_and_a_short_one_is_kept() {
        let secrets = [
            "abcdefgh".to_owned(),
            "abcdefgh-and-more".to_owned(),
            "short".to_owned(),
        ];

        let shown = redacted(
            "first abcdefgh-and-more then abcdefgh, short, and abcdefgh again",
            &secrets,
        );

        assert!(!shown.contains("abcdefgh"));
        assert!(!shown.contains("and-more"));
        assert!(shown.contains("short"));
        assert!(shown.starts_with("first ") && shown.ends_with(" again"));
    }

    #[tokio::test]
    async fn a_setup_result_kept_with_a_value_is_shown_without_it() {
        let state = scratch();
        let machine = machine(state.path());
        let script = "true";
        let old = SetupRun {
            script: hex(&Sha256::digest(script.as_bytes())),
            succeeded: true,
            output: "+ echo hunter2-hunter2".to_owned(),
            ran_unix_ms: 7,
        };
        let file = state.path().join("setup.json");
        std::fs::write(&file, serde_json::to_vec(&old).unwrap_or_default())
            .unwrap_or_else(|error| panic!("{error}"));
        machine.set_env(BTreeMap::from([(
            "DEPLOY_TOKEN".to_owned(),
            "hunter2-hunter2".to_owned(),
        )]));

        let status = machine.status("test".to_owned()).await;
        let reused = machine
            .configure(configure(script, &[("DEPLOY_TOKEN", "hunter2-hunter2")]))
            .await
            .unwrap_or_else(|error| panic!("{error}"));

        let kept = std::fs::read_to_string(&file).unwrap_or_else(|error| panic!("{error}"));
        assert!(
            status
                .setup
                .is_some_and(|setup| !setup.output.contains("hunter2-hunter2"))
        );
        // The kept run was reused, not run again.
        assert_eq!(reused.ran_unix_ms, 7);
        assert!(!reused.output.contains("hunter2-hunter2"));
        assert!(!kept.contains("hunter2-hunter2"));
    }

    #[tokio::test]
    async fn a_host_with_no_setup_script_shows_no_setup() {
        let state = scratch();
        let machine = machine(state.path());
        let _ = machine
            .configure(configure("echo ready", &[]))
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let with_script = machine.status("test".to_owned()).await;

        let _ = machine
            .configure(configure("", &[]))
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let without = machine.status("test".to_owned()).await;

        assert!(with_script.setup.is_some());
        assert_eq!(without.setup, None);
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

    #[tokio::test]
    async fn a_clone_that_takes_too_long_is_stopped_and_leaves_nothing_behind() {
        let state = scratch();
        // A git server that takes the connection and never answers.
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| panic!("couldn't listen: {error}"));
        let port = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("{error}"))
            .port();
        let silent = std::thread::spawn(move || listener.accept().map(|(stream, _)| stream));
        let machine = machine(&state.path().join("host"));
        let request = pb::CloneRepositories {
            folder: "project".to_owned(),
            repositories: vec![pb::RemoteRepository {
                name: "app".to_owned(),
                url: format!("git://127.0.0.1:{port}/app.git"),
            }],
        };

        let cloned = machine.clone_within(request, Duration::from_secs(1)).await;

        let Err(HostError::CloneTimedOut { path, .. }) = cloned else {
            panic!("the clone wasn't stopped: {cloned:?}");
        };
        assert!(!path.exists());
        drop(silent);
    }

    #[tokio::test]
    async fn a_push_and_a_fetch_that_take_too_long_are_stopped() {
        let state = scratch();
        // A git server that takes each connection and never answers.
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| panic!("couldn't listen: {error}"));
        let port = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("{error}"))
            .port();
        let silent = std::thread::spawn(move || {
            let first = listener.accept().map(|(stream, _)| stream);
            let second = listener.accept().map(|(stream, _)| stream);
            (first, second)
        });
        let copy = state.path().join("copy");
        let copy_path = copy.display().to_string();
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
        git(&["init", "-q", "-b", "work", &copy_path]);
        git(&[
            "-C",
            &copy_path,
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "first",
        ]);
        let url = format!("git://127.0.0.1:{port}/app.git");
        git(&["-C", &copy_path, "remote", "add", "origin", &url]);
        let machine = machine(&state.path().join("host"));
        let patience = Duration::from_secs(1);

        let pushed = machine.push_within(&copy, "app", "work", patience).await;
        let fetched = machine.fetch_within(&copy, "app", patience).await;

        assert!(
            matches!(&pushed, Err(HostError::PushTimedOut { branch, .. }) if branch == "work"),
            "the push wasn't stopped: {pushed:?}"
        );
        assert!(
            matches!(&fetched, Err(HostError::FetchTimedOut { .. })),
            "the fetch wasn't stopped: {fetched:?}"
        );
        drop(silent);
    }
}
