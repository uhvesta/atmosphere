//! PlanPush and PushAndOpenPullRequests (MV-12, GRPC §7 PUSH_AND_OPEN_PR),
//! and the commit, push and pull request steps that ArchiveSession's
//! COMMIT_PUSH_PR_THEN_ARCHIVE runs too (MV-10): `run_pushes` is the one
//! place those happen. The git work is `humata2_sessions`'; pull requests
//! are GitHub's, asked of the `gh` CLI on this Mac.

mod resume;

use std::path::{Path, PathBuf};
use std::process::Command;

use humata2_access::{AccessRequest, Capability, RepoBranch, RepoRef, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_review_diff::wire::spec_from_pb;
use humata2_review_diff::{BaseContext, Checkout, DiffError, DiffSpec, RepoSpec};
use humata2_sessions::{
    GitError, PushError, commit_all, default_branch, push_branch, repo_work, totals,
};
use humata2_store::{SessionDetail, SessionRepo};
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use super::archive::{PlanKind, failure_of, session_id};
use super::archive_remote::{follow_to_end, machine_failed};
use super::SessionSvc;
use crate::channel_service::{Gated, blocking, error_of, failed};
use crate::host_relay::assigned;
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::remote_work::RemoteWork;
use crate::status;
use resume::Kept;

/// One repository of a push: its clone, and what to do in it.
#[derive(Clone)]
pub(super) struct RepoPush {
    pub(super) owner: String,
    pub(super) name: String,
    pub(super) branch: String,
    pub(super) path: PathBuf,
    /// Committed first with this message, when set.
    pub(super) commit_message: Option<String>,
    /// The pull request to open for the branch, when set.
    pub(super) pull_request: Option<PullRequestText>,
}

/// The title and body of a pull request.
#[derive(Clone)]
pub(super) struct PullRequestText {
    pub(super) title: String,
    pub(super) body: String,
    /// The branch the pull request targets, when one was picked (a branch
    /// name, without `origin/`). Only a new pull request is opened against it.
    pub(super) base: Option<String>,
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
pub(super) fn step_rows(repos: &[RepoPush]) -> Vec<(String, String)> {
    planned_steps(repos)
        .into_iter()
        .map(|(repo, step)| (repo.step_key(step), step.title(&repo.slug())))
        .collect()
}

/// Where a run of pushes stopped: the error to show, the repositories whose
/// branch reached origin, and the ones whose steps did not run.
pub(super) struct Stopped {
    error: pb::Error,
    pushed: Vec<String>,
    skipped: Vec<String>,
}

impl Stopped {
    /// The error to answer with: what failed (its own message and fix), then
    /// `outcome`, then what was pushed and what was skipped.
    pub(super) fn into_error(self, outcome: &str) -> pb::Error {
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
pub(super) async fn run_pushes(
    progress: &Progress,
    repos: &[RepoPush],
    again: &str,
) -> Result<Vec<pb::PullRequestRef>, Stopped> {
    let steps = planned_steps(repos);
    let mut pushed: Vec<String> = Vec::new();
    let mut opened = Vec::new();
    for (at, &(repo, step)) in steps.iter().enumerate() {
        let key = repo.step_key(step);
        let outcome = match (step, &repo.pull_request) {
            (Step::Commit, _) => run_step(progress, &key, {
                let (repo, again) = (repo.clone(), again.to_owned());
                move || commit_step(&repo, &again)
            })
            .await
            .map(|()| None),
            (Step::Push, _) => run_step(progress, &key, {
                let (repo, again) = (repo.clone(), again.to_owned());
                move || push_step(&repo, &again)
            })
            .await
            .map(|()| None),
            (Step::OpenPr, Some(text)) => run_step(progress, &key, {
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
                for &(later, later_step) in &steps[at + 1..] {
                    progress.set_step(&later.step_key(later_step), pb::StepState::Skipped, None);
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
    progress: &Progress,
    key: &str,
    work: impl FnOnce() -> Result<T, pb::Error> + Send + 'static,
) -> Result<T, pb::Error> {
    progress.set_step(key, pb::StepState::Running, None);
    let outcome = match blocking(work).await {
        Ok(outcome) => outcome,
        Err(status) => Err(failure_of(&status)),
    };
    match &outcome {
        Ok(_) => progress.set_step(key, pb::StepState::Done, None),
        Err(error) => progress.set_step(key, pb::StepState::Failed, Some(error.message.clone())),
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

fn pr_step(repo: &RepoPush, text: &PullRequestText, again: &str) -> Result<pb::PullRequestRef, pb::Error> {
    open_or_reuse(&repo.path, &repo.slug(), &repo.branch, text)
        .map(|opened| pb::PullRequestRef {
            repo: Some(repo.repo()),
            number: opened.number,
            note: opened.note.unwrap_or_default(),
        })
        .map_err(|error| gh_failure(&error, again))
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
        PushError::Stage { .. } | PushError::Commit { .. } => error_of(
            error,
            &format!(
                "run `git status` in {path} to see what blocks the commit (git may need your name and email set with `git config`), fix that, then {again}"
            ),
            Some(git_failed(pb::git_failed::Step::Commit)),
        ),
        PushError::Push { branch, .. } if is_rejected(&stderr) => error_of(
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
        PushError::Push { branch, .. } if is_protected(&stderr) => error_of(
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
        PushError::Push { branch, .. } => error_of(
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
            format!("install the GitHub CLI (`brew install gh`) and run `gh auth login` on this Mac, then {again}; {landed}"),
        ),
        GhError::Failed { stderr, .. } if signed_out(stderr) => (
            pb::git_hub_cli_problem::Problem::NotSignedIn,
            format!("run `gh auth login` on this Mac, then {again}; {landed}"),
        ),
        GhError::Failed { stderr, .. } if stderr.contains("scope") => (
            pb::git_hub_cli_problem::Problem::ScopeMissing,
            format!("run `gh auth refresh` on this Mac with the scope the message names, then {again}; {landed}"),
        ),
        GhError::Failed { .. } | GhError::Listing { .. } | GhError::Link { .. } => (
            pb::git_hub_cli_problem::Problem::ApiError,
            format!("read the GitHub CLI's message above, fix what it names, then {again}; {landed}"),
        ),
    };
    error_of(
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

pub(crate) use crate::tool_path::gh_program;

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
            "pr", "list", "--repo", repo, "--head", branch, "--state", "open", "--json",
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
fn base_of(choice: &str) -> Option<String> {
    let trimmed = choice.trim();
    let name = trimmed.strip_prefix("origin/").unwrap_or(trimmed);
    (!name.is_empty()).then(|| name.to_owned())
}

/// A pull request's base that can't be used: the branch isn't on origin in
/// the clone as of its last fetch, or git couldn't be asked.
#[derive(Debug, thiserror::Error)]
enum BaseError {
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
fn check_bases(pushes: &[RepoPush]) -> Result<(), BaseError> {
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
fn base_failed(error: &BaseError) -> Status {
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
enum PlanError {
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
    #[error("couldn't compare the changes in {} against the base the spec gives", path.display())]
    Diff {
        path: PathBuf,
        #[source]
        source: DiffError,
    },
}

/// The push plan rows of `detail`'s repositories. A repo the spec names is
/// counted against its base there; any other repo against its default branch,
/// as before. Blocking: it runs git and gh.
fn plan_repos(
    detail: Option<&SessionDetail>,
    spec: Option<&DiffSpec>,
) -> Result<Vec<pb::push_plan::Repo>, PlanError> {
    let Some(detail) = detail else {
        return Ok(Vec::new());
    };
    let context = BaseContext {
        session: detail.session.clone(),
        fork_point: None,
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
        let against = spec.and_then(|spec| spec.repos.iter().find(|found| found.repo == slug));
        let (added, removed) = match against {
            Some(against) => spec_totals(&repo.path, against, &context)?,
            None => {
                let sums = totals(&work);
                (sums.added, sums.removed)
            }
        };
        plans.push(pb::push_plan::Repo {
            repo: Some(pb::RepoRef {
                owner: repo.owner.clone(),
                name: repo.name.clone(),
            }),
            branch: repo.branch.clone(),
            dirty_files: work.dirty_files,
            unpushed_commits: work.unpushed_commits,
            added,
            removed,
            existing,
        });
    }
    Ok(plans)
}

/// A repo's added and removed lines under its part of a plan's spec, counted
/// as the Diff panel counts them. Blocking: it runs git.
fn spec_totals(
    clone: &Path,
    against: &RepoSpec,
    context: &BaseContext,
) -> Result<(u32, u32), PlanError> {
    let diff = Checkout::open(clone)
        .and_then(|checkout| checkout.diff_blocking(against, context))
        .map_err(|source| PlanError::Diff {
            path: clone.to_owned(),
            source,
        })?;
    let (added, removed) = diff.totals();
    Ok((
        u32::try_from(added).unwrap_or(u32::MAX),
        u32::try_from(removed).unwrap_or(u32::MAX),
    ))
}

/// The access target for pushing `branch` of `owner/name`.
fn push_target(owner: &str, name: &str, branch: &str) -> RepoBranch {
    RepoBranch {
        repo: RepoRef::new(format!("{owner}/{name}")),
        branch: branch.to_owned(),
    }
}

/// The repositories a PushAndOpenPullRequests request names, each one a
/// repository of the session, with the pull request text it asks for.
fn chosen_repos(
    choices: &[pb::PushChoice],
    repos: &[SessionRepo],
) -> Result<Vec<RepoPush>, Status> {
    if choices.is_empty() {
        return Err(status::invalid_field(
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
                status::invalid_field(
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
                    status::invalid_field(
                        "repos",
                        Problem::UnknownValue,
                        format!("{}/{} isn't a repository of this session", repo.owner, repo.name),
                        "choose one of the session's repositories, and send it again",
                    )
                })?;
            let pull_request = if choice.open_pull_request {
                if choice.pr_title.trim().is_empty() {
                    return Err(status::invalid_field(
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
enum RemotePushError {
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

/// Ends the host's operation of a push on `machine` with how the machine's
/// ended: its pull requests, or its error.
fn end_push(progress: Progress, machine: &str, ended: Result<pb::Operation, Status>) {
    match ended {
        Ok(ended) if ended.state == pb::OperationState::Succeeded as i32 => {
            progress.succeed_with_pull_requests(ended.pull_requests);
        }
        Ok(ended) => progress.fail(ended.error.unwrap_or_else(|| {
            error_of(
                &RemotePushError::NoAnswer {
                    machine: machine.to_owned(),
                },
                "look at the session on the machine, then push again",
                None,
            )
        })),
        Err(refused) => progress.fail(status::error_of(&refused).unwrap_or_else(|| pb::Error {
            message: refused.message().to_owned(),
            fix: "push again".to_owned(),
            retry: None,
            kind: None,
        })),
    }
}

/// True when `branch` is a default branch (`main`, `master`, `HEAD`, or the
/// clone's `origin/HEAD` default branch when readable).
fn is_default_branch(path: &Path, branch: &str, check_clone: bool) -> bool {
    let trimmed = branch.trim();
    let name = trimmed.strip_prefix("refs/heads/").unwrap_or(trimmed);
    if name.is_empty()
        || name.eq_ignore_ascii_case("main")
        || name.eq_ignore_ascii_case("master")
        || name == "HEAD"
    {
        return true;
    }
    if check_clone && let Ok(default) = default_branch(path) {
        return default == name;
    }
    false
}

/// The access target for one push choice: `Target::PushBranch` for a
/// branch-only push to a non-default branch (D920), else `Target::Push`
/// (opening a PR or pushing a default branch).
fn push_access_target(push: &RepoPush, check_clone: bool) -> Target {
    let branch = push_target(&push.owner, &push.name, &push.branch);
    if push.pull_request.is_none() && !is_default_branch(&push.path, &push.branch, check_clone) {
        Target::PushBranch(branch)
    } else {
        Target::Push(branch)
    }
}

impl SessionSvc {
    /// Checks each target's `GitPush` access: the first one that must be
    /// asked is returned.
    pub(super) fn ask_for_pushes(
        &self,
        caller: &Caller,
        targets: Vec<RepoBranch>,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        let targets = targets.into_iter().map(Target::Push).collect();
        self.ask_for_push_targets(caller, targets, approved)
    }

    /// Checks each push target's `GitPush` access (`Target::Push` or
    /// `Target::PushBranch`): the first one that must be asked is returned.
    fn ask_for_push_targets(
        &self,
        caller: &Caller,
        targets: Vec<Target>,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        for target in targets {
            let (repo, branch) = match &target {
                Target::Push(rb) | Target::PushBranch(rb) => (rb.repo.clone(), rb.branch.clone()),
                _ => continue,
            };
            if let Some(asked) = self
                .gate(caller, Capability::GitPush, target, approved)
                .map_err(|refused| status::with_push_target(refused, &repo, &branch))?
            {
                return Ok(Some(asked));
            }
        }
        Ok(None)
    }

    /// What pushing each repository of the session would send, and the
    /// token PushAndOpenPullRequests takes. Changes nothing.
    pub(super) async fn plan_push_of(
        &self,
        caller: &Caller,
        request: pb::PlanPushRequest,
        approved: bool,
    ) -> Result<Gated<pb::PushPlan>, Status> {
        Self::refuse_peer(caller)?;
        let id = session_id(request.id.clone())?;
        let loaded = self.load(&id).await?;
        let targets: Vec<RepoBranch> = loaded
            .detail
            .iter()
            .flat_map(|detail| detail.repos.iter())
            .map(|repo| push_target(&repo.owner, &repo.name, &repo.branch))
            .collect();
        if let Some(asked) = self.ask_for_pushes(caller, targets, approved)? {
            return Ok(Gated::Ask(asked));
        }
        // A session on another machine is planned there: its clones, its git
        // and its gh are the machine's.
        if let Some((machine, remote)) = self.lives_elsewhere(&loaded.record) {
            return self
                .plan_push_on_machine(&machine, &remote, request)
                .await
                .map(Gated::Done);
        }
        let spec = request
            .spec
            .as_ref()
            .map(spec_from_pb)
            .transpose()
            .map_err(|error| {
                status::invalid_field(
                    "spec",
                    Problem::BadFormat,
                    format!("the spec doesn't parse: {error}"),
                    "send a base for each repository in the spec, or leave spec unset to compare each with its default branch",
                )
            })?;
        let detail = loaded.detail.clone();
        let repos = blocking(move || plan_repos(detail.as_ref(), spec.as_ref()))
            .await?
            .map_err(|error| {
                failed(
                    Code::FailedPrecondition,
                    &error,
                    "fix the cause the message names, then plan the push again",
                    None,
                )
            })?;
        Ok(Gated::Done(pb::PushPlan {
            id: Some(pb::SessionId { value: id.clone() }),
            repos,
            token: Some(pb::PlanToken {
                value: self.lifecycle.make_plan(PlanKind::Push, &id),
            }),
        }))
    }

    /// Commits, pushes and opens the pull requests the request chooses, as an
    /// operation. The plan token is used up here; the work runs after.
    pub(super) async fn push_and_open_pull_requests_of(
        &self,
        caller: &Caller,
        request: pb::PushAndOpenPullRequestsRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        Self::refuse_peer(caller)?;
        let id = session_id(request.id.clone())?;
        let loaded = self.load(&id).await?;
        let session_repos: Vec<SessionRepo> = loaded
            .detail
            .iter()
            .flat_map(|detail| detail.repos.iter())
            .cloned()
            .collect();
        let pushes = chosen_repos(&request.repos, &session_repos)?;
        let local_clone = self.lives_elsewhere(&loaded.record).is_none();
        let pushes_for_targets = pushes.clone();
        let targets = blocking(move || {
            pushes_for_targets
                .iter()
                .map(|push| push_access_target(push, local_clone))
                .collect::<Vec<_>>()
        })
        .await?;
        if let Some(asked) = self.ask_for_push_targets(caller, targets, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
        if let Some((machine, remote)) = self.lives_elsewhere(&loaded.record) {
            // The machine checks the bases in its own clones, spends its own
            // plan token and runs git and gh itself.
            return self
                .push_on_machine(caller, request, pushes, id, machine, remote)
                .await;
        }
        let checked = pushes.clone();
        blocking(move || check_bases(&checked))
            .await?
            .map_err(|error| base_failed(&error))?;
        let token = request
            .plan
            .as_ref()
            .map(|plan| plan.value.clone())
            .unwrap_or_default();
        if !self.lifecycle.use_plan(PlanKind::Push, &token, &id) {
            return Err(failed(
                Code::FailedPrecondition,
                &PlanError::StalePlan {
                    session: id.clone(),
                },
                "plan the push again, read what it would push, and send the new token",
                None,
            ));
        }
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::PushAndOpenPr,
            target,
            &step_rows(&pushes),
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        tokio::spawn(self.clone().run_push(
            progress,
            id,
            pushes,
            identity::actor(caller),
        ));
        Ok(Gated::Done(operation))
    }

    /// PlanPush for a session on `machine`: the machine reads its own clones
    /// with its own git and gh, and answers with its plan and the token
    /// PushAndOpenPullRequests takes.
    async fn plan_push_on_machine(
        &self,
        machine: &str,
        remote: &RemoteWork,
        request: pb::PlanPushRequest,
    ) -> Result<pb::PushPlan, Status> {
        let relayed = remote.relayed(machine).map_err(Status::from)?;
        relayed
            .session()
            .plan_push(assigned(request, None).map_err(Status::from)?)
            .await
            .map(tonic::Response::into_inner)
            .map_err(|source| machine_failed(machine, source))
    }

    /// PushAndOpenPullRequests for a session on `machine`, after the host
    /// checked the request and the caller's access: the host's operation
    /// shows the machine's steps and ends with its pull requests. An offline
    /// machine answers at once and nothing starts.
    async fn push_on_machine(
        &self,
        caller: &Caller,
        request: pb::PushAndOpenPullRequestsRequest,
        pushes: Vec<RepoPush>,
        id: String,
        machine: String,
        remote: RemoteWork,
    ) -> Result<Gated<pb::Operation>, Status> {
        remote.relayed(&machine).map_err(Status::from)?;
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::PushAndOpenPr,
            target,
            &step_rows(&pushes),
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        let asked = pb::PushAndOpenPullRequestsRequest {
            idempotency_key: None,
            ..request
        };
        tokio::spawn(self.clone().run_push_on_machine(
            progress,
            id,
            machine,
            remote,
            asked,
            identity::actor(caller),
        ));
        Ok(Gated::Done(operation))
    }

    /// Asks `machine` to push and follows its operation to the end, showing
    /// its steps on the host's. The machine goes on by itself if the link is
    /// lost; the branches it pushed stay pushed.
    async fn run_push_on_machine(
        self,
        progress: Progress,
        id: String,
        machine: String,
        remote: RemoteWork,
        asked: pb::PushAndOpenPullRequestsRequest,
        by: pb::Actor,
    ) {
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        // Kept on disk before the machine is asked, and again once it has
        // answered with its operation: a restart of the host goes on
        // following that operation to its end (resume.rs).
        let mut kept = Kept::new(&progress, &id, &machine);
        let ended = async {
            let relayed = remote.relayed(&machine).map_err(Status::from)?;
            let request = assigned(asked, None).map_err(Status::from)?;
            self.keep_push(&kept).await;
            // A cut here: the push is kept, the machine not asked yet.
            crate::fault::point("push_remote.kept");
            let first = relayed
                .session()
                .push_and_open_pull_requests(request)
                .await
                .map_err(|source| machine_failed(&machine, source))?
                .into_inner();
            kept.asked = first.id.as_ref().map(|id| id.value.clone());
            self.keep_push(&kept).await;
            // A cut here: the machine is pushing and the record names its
            // operation.
            crate::fault::point("push_remote.asked");
            follow_to_end(&relayed, &machine, first, &progress, || {
                failed(
                    Code::Unavailable,
                    &RemotePushError::LinkLost {
                        machine: machine.clone(),
                    },
                    "reconnect the machine and look at the session's changes: what it pushed stays pushed, so push again to finish",
                    None,
                )
            })
            .await
        }
        .await;
        drop(guard);
        end_push(progress, &machine, ended);
        // The host's operation ended; a restart from here has nothing to go on with.
        self.drop_kept(&kept).await;
        self.tell
            .session(&id, pb::Change::Updated, Some(by))
            .await;
    }

    /// The operation's work: the steps under the session's lock, then the
    /// session published, since its branches changed.
    async fn run_push(self, progress: Progress, id: String, pushes: Vec<RepoPush>, by: pb::Actor) {
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        let ran = run_pushes(&progress, &pushes, "push again").await;
        drop(guard);
        match ran {
            Ok(opened) => progress.succeed_with_pull_requests(opened),
            Err(stopped) => {
                progress.fail(stopped.into_error("The branches already pushed stay pushed."))
            }
        }
        self.tell
            .session(&id, pb::Change::Updated, Some(by))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_push(branch: &str, open_pr: bool) -> RepoPush {
        RepoPush {
            owner: "acme".to_owned(),
            name: "api".to_owned(),
            branch: branch.to_owned(),
            path: PathBuf::new(),
            commit_message: None,
            pull_request: open_pr.then(|| PullRequestText {
                title: "Fix bug".to_owned(),
                body: String::new(),
                base: None,
            }),
        }
    }

    #[test]
    fn non_default_branch_only_push_uses_push_branch_target_while_pr_and_default_use_push() {
        assert!(matches!(
            push_access_target(&repo_push("me/feat", false), false),
            Target::PushBranch(_)
        ));
        assert!(matches!(
            push_access_target(&repo_push("me/feat", true), false),
            Target::Push(_)
        ));
        for default_name in ["main", "master", "HEAD", "refs/heads/main"] {
            assert!(
                matches!(
                    push_access_target(&repo_push(default_name, false), false),
                    Target::Push(_)
                ),
                "{default_name} should stay guarded"
            );
        }
    }
}
