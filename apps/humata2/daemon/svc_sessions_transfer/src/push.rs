//! PlanPush and PushAndOpenPullRequests (MV-12, GRPC §7 PUSH_AND_OPEN_PR),
//! and the commit, push and pull request steps. The git work is
//! `humata2_sessions`'; pull requests are GitHub's, asked of the `gh` CLI.

pub mod resume;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use humata2_access::{Call, Capability, RepoBranch, RepoRef, Target};
use humata2_daemon_ports::Caller;
use humata2_sessions::{GitError, PushError, commit_all, push_branch, repo_work, totals};
use humata2_store::{SessionDetail, SessionRepo};
use humata2_svc_sessions_common::pb;
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use crate::SessionTransferSvc;
use crate::ports::{OpProgress, blocking, failed, invalid_field, missing, require_user};

/// One repository of a push: its clone, and what to do in it.
#[derive(Clone, Debug)]
pub struct RepoPush {
    pub owner: String,
    pub name: String,
    pub branch: String,
    pub path: PathBuf,
    /// Committed first with this message, when set.
    pub commit_message: Option<String>,
    /// The pull request to open for the branch, when set.
    pub pull_request: Option<PullRequestText>,
}

/// The title and body of a pull request.
#[derive(Clone, Debug)]
pub struct PullRequestText {
    pub title: String,
    pub body: String,
    /// The branch the pull request targets, when one was picked (a branch
    /// name, without `origin/`). Only a new pull request is opened against it.
    pub base: Option<String>,
}

/// One of a push's steps: `commit:<repo>`, `push:<repo>`, `open_pr:<repo>`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Commit,
    Push,
    OpenPr,
}

impl Step {
    fn key(self) -> &'static str {
        match self {
            Step::Commit => "commit",
            Step::Push => "push",
            Step::OpenPr => "open_pr",
        }
    }

    fn title(self, slug: &str) -> String {
        match self {
            Step::Commit => format!("Committing {slug}"),
            Step::Push => format!("Pushing {slug}"),
            Step::OpenPr => format!("Opening a pull request for {slug}"),
        }
    }
}

impl RepoPush {
    fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    fn repo(&self) -> pb::RepoRef {
        pb::RepoRef {
            owner: self.owner.clone(),
            name: self.name.clone(),
        }
    }

    fn step_key(&self, step: Step) -> String {
        format!("{}:{}", step.key(), self.slug())
    }
}

/// The steps `repos` need, in the order they run.
fn planned_steps(repos: &[RepoPush]) -> Vec<(&RepoPush, Step)> {
    let mut steps = Vec::new();
    for repo in repos {
        if repo.commit_message.is_some() {
            steps.push((repo, Step::Commit));
        }
        steps.push((repo, Step::Push));
        if repo.pull_request.is_some() {
            steps.push((repo, Step::OpenPr));
        }
    }
    steps
}

/// The `(key, title)` of each step of `repos`, for an operation to start with.
pub fn step_rows(repos: &[RepoPush]) -> Vec<(String, String)> {
    planned_steps(repos)
        .into_iter()
        .map(|(repo, step)| (repo.step_key(step), step.title(&repo.slug())))
        .collect()
}

/// Where a run of pushes stopped: the error to show, the repositories whose
/// branch reached origin, and the ones whose steps did not run.
pub struct Stopped {
    error: pb::Error,
    pushed: Vec<String>,
    skipped: Vec<String>,
}

impl Stopped {
    /// The error to answer with: what failed (its own message and fix), then
    /// `outcome`, then what was pushed and what was skipped.
    pub fn into_error(self, outcome: &str) -> pb::Error {
        let mut error = self.error;
        error.message = format!(
            "{} {outcome} Pushed: {}. Skipped: {}.",
            error.message,
            listed(&self.pushed),
            listed(&self.skipped)
        );
        error
    }
}

fn listed(names: &[String]) -> String {
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

/// Runs each repository's steps in order: commit, push, open the pull
/// request. The first step that fails is shown failed and the steps after it
/// skipped; the pushes already done stay done. Returns the pull request of
/// each repository that has one (opened or found), in push order. `again` is
/// the verb the fixes use ("push again", "archive again").
pub async fn run_pushes(
    progress: &Arc<dyn OpProgress>,
    repos: &[RepoPush],
    again: &str,
) -> Result<Vec<pb::PullRequestRef>, Stopped> {
    let steps = planned_steps(repos);
    let mut pushed: Vec<String> = Vec::new();
    let mut opened = Vec::new();
    for (at, &(repo, step)) in steps.iter().enumerate() {
        let key = repo.step_key(step);
        let title = step.title(&repo.slug());
        let outcome = match (step, &repo.pull_request) {
            (Step::Commit, _) => run_step(progress, &key, &title, {
                let (repo, again) = (repo.clone(), again.to_owned());
                move || commit_step(&repo, &again)
            })
            .await
            .map(|()| None),
            (Step::Push, _) => run_step(progress, &key, &title, {
                let (repo, again) = (repo.clone(), again.to_owned());
                move || push_step(&repo, &again)
            })
            .await
            .map(|()| None),
            (Step::OpenPr, Some(text)) => run_step(progress, &key, &title, {
                let (repo, text, again) = (repo.clone(), text.clone(), again.to_owned());
                move || pr_step(&repo, &text, &again)
            })
            .await
            .map(Some),
            (Step::OpenPr, None) => continue,
        };
        match outcome {
            Ok(found) => {
                if step == Step::Push {
                    pushed.push(repo.slug());
                }
                if let Some(found) = found {
                    opened.push(found);
                }
            }
            Err(error) => {
                let mut skipped: Vec<String> = Vec::new();
                for &(later, _) in &steps[at..] {
                    let slug = later.slug();
                    if !pushed.contains(&slug) && !skipped.contains(&slug) {
                        skipped.push(slug);
                    }
                }
                return Err(Stopped {
                    error,
                    pushed,
                    skipped,
                });
            }
        }
    }
    Ok(opened)
}

/// Shows `key` running while `work` runs on a blocking thread, then done or
/// failed with the error it made.
async fn run_step<T: Send + 'static>(
    progress: &Arc<dyn OpProgress>,
    key: &str,
    title: &str,
    work: impl FnOnce() -> Result<T, pb::Error> + Send + 'static,
) -> Result<T, pb::Error> {
    progress.start_step(key, title).await;
    let outcome = match blocking(work).await {
        Ok(outcome) => outcome,
        Err(status) => Err(pb::Error {
            message: status.message().to_owned(),
            fix: String::new(),
            retry: None,
            kind: None,
        }),
    };
    match &outcome {
        Ok(_) => progress.finish_step(key).await,
        Err(err) => progress.fail_step(key, &err.message).await,
    }
    outcome
}

fn commit_step(repo: &RepoPush, again: &str) -> Result<(), pb::Error> {
    let message = repo.commit_message.as_deref().unwrap_or_default();
    commit_all(&repo.path, message)
        .map(|_| ())
        .map_err(|error| push_failure(&error, repo, again))
}

fn push_step(repo: &RepoPush, again: &str) -> Result<(), pb::Error> {
    push_branch(&repo.path, &repo.branch).map_err(|error| push_failure(&error, repo, again))
}

fn pr_step(
    repo: &RepoPush,
    text: &PullRequestText,
    again: &str,
) -> Result<pb::PullRequestRef, pb::Error> {
    open_or_reuse(&repo.path, &repo.slug(), &repo.branch, text)
        .map(|opened| pb::PullRequestRef {
            repo: Some(repo.repo()),
            number: opened.number,
            note: opened.note.unwrap_or_default(),
        })
        .map_err(|error| gh_failure(&error, again))
}

fn pb_error_of(
    error: &(dyn std::error::Error + 'static),
    fix: &str,
    kind: Option<pb::error::Kind>,
) -> pb::Error {
    pb::Error {
        message: humata2_rpc_kit::chain(error),
        fix: fix.to_owned(),
        retry: None,
        kind,
    }
}

/// What a git step that failed says, and how to fix it.
fn push_failure(error: &PushError, repo: &RepoPush, again: &str) -> pb::Error {
    let path = repo.path.display();
    let stderr = stderr_of(error);
    let git_failed = |step: pb::git_failed::Step| {
        pb::error::Kind::GitFailed(pb::GitFailed {
            step: step as i32,
            repo: Some(repo.repo()),
            stderr_tail: stderr.clone(),
            ..pb::GitFailed::default()
        })
    };
    match error {
        PushError::Stage { .. } | PushError::Commit { .. } => pb_error_of(
            error,
            &format!(
                "run `git status` in {path} to see what blocks the commit (git may need your name and email set with `git config`), fix that, then {again}"
            ),
            Some(git_failed(pb::git_failed::Step::Commit)),
        ),
        PushError::Push { branch, .. } if is_rejected(&stderr) => pb_error_of(
            error,
            &format!(
                "GitHub has commits on {branch} that this session doesn't have; in {path} run `git pull --rebase origin {branch}`, fix any conflicts, then {again}"
            ),
            Some(pb::error::Kind::PushRefused(pb::PushRefused {
                reason: pb::push_refused::Reason::NonFastForward as i32,
                repo: Some(repo.repo()),
                branch: branch.clone(),
            })),
        ),
        PushError::Push { branch, .. } if is_protected(&stderr) => pb_error_of(
            error,
            &format!(
                "GitHub protects {branch}, so it can't take this push; push another branch, or ask the repository's owner to allow it, then {again}"
            ),
            Some(pb::error::Kind::PushRefused(pb::PushRefused {
                reason: pb::push_refused::Reason::ProtectedBranch as i32,
                repo: Some(repo.repo()),
                branch: branch.clone(),
            })),
        ),
        PushError::Push { branch, .. } => pb_error_of(
            error,
            &format!(
                "run `git push --set-upstream origin {branch}` in {path} to see git's whole message, fix what it names, then {again}"
            ),
            Some(git_failed(pb::git_failed::Step::Push)),
        ),
    }
}

fn stderr_of(error: &PushError) -> String {
    let source = match error {
        PushError::Stage { source, .. }
        | PushError::Commit { source, .. }
        | PushError::Push { source, .. } => source,
    };
    match source {
        GitError::Failed { stderr, .. } => stderr.clone(),
        other => other.to_string(),
    }
}

fn is_rejected(stderr: &str) -> bool {
    stderr.contains("non-fast-forward") || stderr.contains("fetch first")
}

fn is_protected(stderr: &str) -> bool {
    stderr.contains("protected branch") || stderr.contains("GH006")
}

/// What the `gh` CLI couldn't do for a pull request. Each message says what
/// failed, for the person to read.
#[derive(Debug, thiserror::Error)]
enum GhError {
    #[error("could not run the GitHub CLI `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the GitHub CLI failed for {repo} ({status}): {stderr}")]
    Failed {
        repo: String,
        status: String,
        stderr: String,
    },
    #[error("couldn't read the GitHub CLI's list of pull requests for {repo}: {source}")]
    Listing {
        repo: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the GitHub CLI printed {output:?} for the new pull request, not a link to one")]
    Link { output: String },
}

/// The pull request's problem as the API shows it, with the fix. The branch
/// is already on GitHub by the time a pull request is asked for.
fn gh_failure(error: &GhError, again: &str) -> pb::Error {
    let landed = "the branch is already pushed";
    let (problem, fix) = match error {
        GhError::Spawn { .. } => (
            pb::git_hub_cli_problem::Problem::NotInstalled,
            format!(
                "install the GitHub CLI (`brew install gh`) and run `gh auth login` on this Mac, then {again}; {landed}"
            ),
        ),
        GhError::Failed { stderr, .. } if signed_out(stderr) => (
            pb::git_hub_cli_problem::Problem::NotSignedIn,
            format!("run `gh auth login` on this Mac, then {again}; {landed}"),
        ),
        GhError::Failed { stderr, .. } if stderr.contains("scope") => (
            pb::git_hub_cli_problem::Problem::ScopeMissing,
            format!(
                "run `gh auth refresh` on this Mac with the scope the message names, then {again}; {landed}"
            ),
        ),
        GhError::Failed { .. } | GhError::Listing { .. } | GhError::Link { .. } => (
            pb::git_hub_cli_problem::Problem::ApiError,
            format!("read the GitHub CLI's message above, fix what it names, then {again}; {landed}"),
        ),
    };
    pb_error_of(
        error,
        &fix,
        Some(pb::error::Kind::GithubCli(pb::GitHubCliProblem {
            problem: problem as i32,
            ..pb::GitHubCliProblem::default()
        })),
    )
}

fn signed_out(stderr: &str) -> bool {
    stderr.contains("gh auth login") || stderr.contains("not logged in")
}

/// Resolves the `gh` CLI binary path.
pub fn gh_program() -> PathBuf {
    std::env::var_os("HUMATA_GH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("gh"))
}

/// Runs `gh` with `args` in `clone`, for the GitHub repository `repo`
/// (`owner/name`), and returns what it printed.
fn gh(clone: &Path, repo: &str, args: &[&str]) -> Result<String, GhError> {
    let program = gh_program();
    let output = Command::new(&program)
        .args(args)
        .current_dir(clone)
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .output()
        .map_err(|source| GhError::Spawn {
            program: program.to_string_lossy().into_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(GhError::Failed {
            repo: repo.to_owned(),
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// An open pull request for a branch: its number, and the branch it targets
/// when `gh` says.
struct OpenPr {
    number: u32,
    base: Option<String>,
}

/// The first open pull request from `branch`, if there is one.
fn open_pull_request(clone: &Path, repo: &str, branch: &str) -> Result<Option<OpenPr>, GhError> {
    let out = gh(
        clone,
        repo,
        &[
            "pr",
            "list",
            "--repo",
            repo,
            "--head",
            branch,
            "--state",
            "open",
            "--json",
            "number,url,baseRefName",
        ],
    )?;
    let listed: serde_json::Value =
        serde_json::from_str(&out).map_err(|source| GhError::Listing {
            repo: repo.to_owned(),
            source,
        })?;
    Ok(listed
        .as_array()
        .and_then(|list| list.first())
        .and_then(|pr| {
            let number = pr
                .get("number")
                .and_then(serde_json::Value::as_u64)
                .and_then(|number| u32::try_from(number).ok())?;
            let base = pr
                .get("baseRefName")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            Some(OpenPr { number, base })
        }))
}

/// The pull request a push step settled on, and a note when the open one it
/// found targets another branch than the one picked.
struct Opened {
    number: u32,
    note: Option<String>,
}

/// The pull request for `branch`: the open one already there, else a new one
/// with `text`, against its picked base when it has one. An open pull request
/// is never retargeted; when its base differs from the picked one, the note
/// says where it targets.
fn open_or_reuse(
    clone: &Path,
    repo: &str,
    branch: &str,
    text: &PullRequestText,
) -> Result<Opened, GhError> {
    if let Some(open) = open_pull_request(clone, repo, branch)? {
        let note = match (text.base.as_deref(), open.base.as_deref()) {
            (Some(wanted), Some(targets)) if wanted != targets => Some(format!(
                "PR #{} targets {targets}, so it was not retargeted to {wanted}; change its base on GitHub if {wanted} is the right one",
                open.number
            )),
            _ => None,
        };
        return Ok(Opened {
            number: open.number,
            note,
        });
    }
    let mut args = vec![
        "pr",
        "create",
        "--repo",
        repo,
        "--head",
        branch,
        "--title",
        text.title.as_str(),
        "--body",
        text.body.as_str(),
    ];
    if let Some(base) = text.base.as_deref() {
        args.extend(["--base", base]);
    }
    let out = gh(clone, repo, &args)?;
    let link = out
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with("https://"))
        .unwrap_or("");
    link.rsplit('/')
        .next()
        .and_then(|number| number.parse::<u32>().ok())
        .map(|number| Opened { number, note: None })
        .ok_or_else(|| GhError::Link {
            output: out.trim().to_owned(),
        })
}

/// The base branch a choice asks for, without `origin/`; empty means none.
pub fn base_of(choice: &str) -> Option<String> {
    let trimmed = choice.trim();
    let name = trimmed.strip_prefix("origin/").unwrap_or(trimmed);
    (!name.is_empty()).then(|| name.to_owned())
}

/// A pull request's base that can't be used: the branch isn't on origin in
/// the clone as of its last fetch, or git couldn't be asked.
#[derive(Debug, thiserror::Error)]
pub enum BaseError {
    #[error("couldn't ask git whether {base} is on origin in {}", path.display())]
    Check {
        path: PathBuf,
        base: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{base} isn't on origin in {} as of its last fetch, so a pull request can't target it", path.display())]
    NotOnOrigin { path: PathBuf, base: String },
}

/// Each pull request's base must be a branch origin has as of the clone's
/// last fetch. Checked before anything is pushed, so a missing base refuses
/// the whole request.
pub fn check_bases(pushes: &[RepoPush]) -> Result<(), BaseError> {
    for repo in pushes {
        let Some(base) = repo
            .pull_request
            .as_ref()
            .and_then(|text| text.base.as_deref())
        else {
            continue;
        };
        let remote = format!("refs/remotes/origin/{base}");
        let output = Command::new("git")
            .arg("rev-parse")
            .arg("--verify")
            .arg("--quiet")
            .arg(&remote)
            .current_dir(&repo.path)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(|source| BaseError::Check {
                path: repo.path.clone(),
                base: base.to_owned(),
                source,
            })?;
        if !output.status.success() {
            return Err(BaseError::NotOnOrigin {
                path: repo.path.clone(),
                base: base.to_owned(),
            });
        }
    }
    Ok(())
}

/// The refusal for a base that can't be used, with the fix.
pub fn base_failed(error: &BaseError) -> Status {
    let fix = match error {
        BaseError::NotOnOrigin { path, base } => format!(
            "run `git fetch origin` in {} so origin's branches are current; if {base} still isn't on origin, pick a branch that is, then push again",
            path.display()
        ),
        BaseError::Check { path, .. } => format!(
            "check that git is installed and can read {}, then push again",
            path.display()
        ),
    };
    failed(Code::FailedPrecondition, error, &fix, None)
}

/// What a plan couldn't read, or a token that isn't a live plan for the
/// session.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("couldn't read the changes in {owner}/{name}")]
    Work {
        owner: String,
        name: String,
        #[source]
        source: GitError,
    },
    #[error(
        "the plan given is not one made to push session {session}, or it was already used or has expired"
    )]
    StalePlan { session: String },
}

/// The push plan rows of `detail`'s repositories. Blocking: it runs git and gh.
pub fn plan_repos(detail: Option<&SessionDetail>) -> Result<Vec<pb::push_plan::Repo>, PlanError> {
    let Some(detail) = detail else {
        return Ok(Vec::new());
    };
    let mut plans = Vec::new();
    for repo in &detail.repos {
        let work = repo_work(&repo.path).map_err(|source| PlanError::Work {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            source,
        })?;
        let slug = format!("{}/{}", repo.owner, repo.name);
        // A gh that can't say leaves `existing` unset; the plan shows the rest.
        let existing = open_pull_request(&repo.path, &slug, &repo.branch)
            .ok()
            .flatten()
            .map(|open| pb::PullRequestRef {
                repo: Some(pb::RepoRef {
                    owner: repo.owner.clone(),
                    name: repo.name.clone(),
                }),
                number: open.number,
                note: String::new(),
            });
        let sums = totals(&work);
        plans.push(pb::push_plan::Repo {
            repo: Some(pb::RepoRef {
                owner: repo.owner.clone(),
                name: repo.name.clone(),
            }),
            branch: repo.branch.clone(),
            dirty_files: work.dirty_files,
            unpushed_commits: work.unpushed_commits,
            added: sums.added,
            removed: sums.removed,
            existing,
        });
    }
    Ok(plans)
}

/// The access target for pushing `branch` of `owner/name`.
pub fn push_target(owner: &str, name: &str, branch: &str) -> RepoBranch {
    RepoBranch {
        repo: RepoRef::new(format!("{owner}/{name}")),
        branch: branch.to_owned(),
    }
}

/// The repositories a PushAndOpenPullRequests request names, each one a
/// repository of the session, with the pull request text it asks for.
pub fn chosen_repos(
    choices: &[pb::PushChoice],
    repos: &[SessionRepo],
) -> Result<Vec<RepoPush>, Status> {
    if choices.is_empty() {
        return Err(invalid_field(
            "repos",
            Problem::Missing,
            "no repository is chosen to push",
            "choose at least one repository of the session, and send it again",
        ));
    }
    choices
        .iter()
        .map(|choice| -> Result<RepoPush, Status> {
            let repo = choice.repo.as_ref().ok_or_else(|| {
                invalid_field(
                    "repos",
                    Problem::Missing,
                    "a choice has no repository",
                    "set repo on each choice, and send it again",
                )
            })?;
            let found = repos
                .iter()
                .find(|found| found.owner == repo.owner && found.name == repo.name)
                .ok_or_else(|| {
                    invalid_field(
                        "repos",
                        Problem::UnknownValue,
                        format!(
                            "{}/{} isn't a repository of this session",
                            repo.owner, repo.name
                        ),
                        "choose one of the session's repositories, and send it again",
                    )
                })?;
            let pull_request = if choice.open_pull_request {
                if choice.pr_title.trim().is_empty() {
                    return Err(invalid_field(
                        "pr_title",
                        Problem::Missing,
                        "the pull request has no title",
                        "set pr_title, and send it again",
                    ));
                }
                Some(PullRequestText {
                    title: choice.pr_title.clone(),
                    body: choice.pr_body.clone(),
                    base: base_of(&choice.base_branch),
                })
            } else {
                None
            };
            Ok(RepoPush {
                owner: found.owner.clone(),
                name: found.name.clone(),
                branch: found.branch.clone(),
                path: found.path.clone(),
                commit_message: (!choice.commit_message.is_empty())
                    .then(|| choice.commit_message.clone()),
                pull_request,
            })
        })
        .collect()
}

/// Why a push on another machine couldn't be followed to its end.
#[derive(Debug, thiserror::Error)]
pub enum RemotePushError {
    #[error("the link to {machine} was lost while it was pushing, so it isn't known how far the push got")]
    LinkLost { machine: String },
    #[error("{machine} ended the push without saying how it went")]
    NoAnswer { machine: String },
    #[error("Humata restarted before {machine} confirmed it had started the push, so it isn't known whether anything was pushed")]
    NotConfirmed { machine: String },
    #[error("{machine} no longer knows the push (its operation {operation}), which ended when {machine} restarted")]
    Unknown { machine: String, operation: String },
    #[error("{machine} was forgotten while its push was being followed")]
    Forgotten { machine: String },
    #[error("this daemon has no way to reach {machine}, so its push can't be followed")]
    NoLinks { machine: String },
}

fn require_session_id(id: Option<pb::SessionId>) -> Result<String, Status> {
    let value = id.map(|id| id.value).unwrap_or_default();
    if value.trim().is_empty() {
        return Err(missing("id"));
    }
    Ok(value)
}

impl SessionTransferSvc {
    /// Checks each target's `GitPush` access.
    pub fn require_pushes(
        &self,
        caller: &Caller,
        targets: Vec<RepoBranch>,
    ) -> Result<(), Status> {
        for target in targets {
            self.env
                .require_capability(caller, Capability::GitPush, Target::Push(target))?;
        }
        Ok(())
    }

    /// What pushing each repository of the session would send, and the
    /// token PushAndOpenPullRequests takes. Changes nothing.
    pub async fn handle_plan_push(
        &self,
        caller: &Caller,
        request: pb::PlanPushRequest,
    ) -> Result<pb::PushPlan, Status> {
        require_user(caller)?;
        let id = require_session_id(request.id)?;
        let loaded = self.env.load_session(&self.core, &id)?;
        let targets: Vec<RepoBranch> = loaded
            .detail
            .repos
            .iter()
            .map(|repo| push_target(&repo.owner, &repo.name, &repo.branch))
            .collect();
        self.require_pushes(caller, targets)?;
        if !self.env.is_host(&loaded.record.machine) {
            return Err(self.env.not_wired("remote plan_push"));
        }
        let detail = loaded.detail.clone();
        let repos = blocking(move || plan_repos(Some(&detail)))
            .await?
            .map_err(|error| {
                failed(
                    Code::FailedPrecondition,
                    &error,
                    "fix the cause the message names, then plan the push again",
                    None,
                )
            })?;
        Ok(pb::PushPlan {
            id: Some(pb::SessionId { value: id }),
            repos,
            token: Some(pb::PlanToken {
                value: format!("plan_{}", uuid::Uuid::new_v4().simple()),
            }),
        })
    }

    /// Commits, pushes and opens the pull requests the request chooses, as an
    /// operation.
    pub async fn handle_push_and_open_pull_requests(
        &self,
        caller: &Caller,
        request: pb::PushAndOpenPullRequestsRequest,
    ) -> Result<pb::Operation, Status> {
        require_user(caller)?;
        let id = require_session_id(request.id)?;
        let loaded = self.env.load_session(&self.core, &id)?;
        let pushes = chosen_repos(&request.repos, &loaded.detail.repos)?;
        let targets = pushes
            .iter()
            .map(|push| push_target(&push.owner, &push.name, &push.branch))
            .collect();
        self.require_pushes(caller, targets)?;
        if !self.env.is_host(&loaded.record.machine) {
            return Err(self.env.not_wired("remote push_and_open_pull_requests"));
        }
        let checked = pushes.clone();
        blocking(move || check_bases(&checked))
            .await?
            .map_err(|error| base_failed(&error))?;
        let (operation, progress) = self
            .env
            .start_operation(pb::OperationKind::PushAndOpenPr, step_rows(&pushes))
            .await?;
        let svc = self.clone();
        let by = self.env.actor_of(caller);
        tokio::spawn(async move {
            match run_pushes(&progress, &pushes, "push again").await {
                Ok(_opened) => progress.succeed().await,
                Err(stopped) => {
                    let err = stopped.into_error("The branches already pushed stay pushed.");
                    progress.fail(&err.message, &err.fix).await;
                }
            }
            svc.env
                .publish_session(&id, pb::Change::Updated, by)
                .await;
        });
        Ok(operation)
    }
}
