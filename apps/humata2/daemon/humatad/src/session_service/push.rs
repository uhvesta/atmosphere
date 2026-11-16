//! PlanPush and PushAndOpenPullRequests (MV-12, GRPC §7 PUSH_AND_OPEN_PR),
//! and the commit, push and pull request steps that ArchiveSession's
//! COMMIT_PUSH_PR_THEN_ARCHIVE runs too (MV-10): `run_pushes` is the one
//! place those happen. The git work is `humata2_sessions`'; pull requests
//! are GitHub's, asked of the `gh` CLI on this Mac.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use humata2_access::{AccessRequest, Capability, RepoBranch, RepoRef, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{GitError, PushError, commit_all, push_branch, repo_work, totals};
use humata2_store::{SessionDetail, SessionRepo};
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use super::archive::{PlanKind, failure_of, session_id};
use super::fork::not_built_part;
use super::SessionSvc;
use crate::channel_service::{Gated, blocking, error_of, failed};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::status;

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
/// skipped; the pushes already done stay done. Returns the last pull request
/// opened. `again` is the verb the fixes use ("push again", "archive again").
pub(super) async fn run_pushes(
    progress: &Progress,
    repos: &[RepoPush],
    again: &str,
) -> Result<Option<pb::PullRequestRef>, Stopped> {
    let steps = planned_steps(repos);
    let mut pushed: Vec<String> = Vec::new();
    let mut last = None;
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
                if found.is_some() {
                    last = found;
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
    Ok(last)
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
        .map(|number| pb::PullRequestRef {
            repo: Some(repo.repo()),
            number,
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

/// The `gh` to run: `HUMATA2_GH` in an admin build (the tests' fake), else
/// `gh` on the PATH.
fn gh_program() -> OsString {
    #[cfg(feature = "admin")]
    if let Some(program) = std::env::var_os("HUMATA2_GH") {
        return program;
    }
    OsString::from("gh")
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

/// The number of the first open pull request from `branch`, if there is one.
fn open_pull_request(clone: &Path, repo: &str, branch: &str) -> Result<Option<u32>, GhError> {
    let out = gh(
        clone,
        repo,
        &[
            "pr", "list", "--repo", repo, "--head", branch, "--state", "open", "--json",
            "number,url",
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
        .and_then(|pr| pr.get("number"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|number| u32::try_from(number).ok()))
}

/// The number of the pull request for `branch`: the open one already there,
/// else a new one with `text`.
fn open_or_reuse(
    clone: &Path,
    repo: &str,
    branch: &str,
    text: &PullRequestText,
) -> Result<u32, GhError> {
    if let Some(number) = open_pull_request(clone, repo, branch)? {
        return Ok(number);
    }
    let out = gh(
        clone,
        repo,
        &[
            "pr", "create", "--repo", repo, "--head", branch, "--title", &text.title, "--body",
            &text.body,
        ],
    )?;
    let link = out
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with("https://"))
        .unwrap_or("");
    link.rsplit('/')
        .next()
        .and_then(|number| number.parse::<u32>().ok())
        .ok_or_else(|| GhError::Link {
            output: out.trim().to_owned(),
        })
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
}

/// The push plan rows of `detail`'s repositories. Blocking: it runs git and
/// gh.
fn plan_repos(detail: Option<&SessionDetail>) -> Result<Vec<pb::push_plan::Repo>, PlanError> {
    let mut plans = Vec::new();
    for repo in detail.iter().flat_map(|detail| detail.repos.iter()) {
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
            .map(|number| pb::PullRequestRef {
                repo: Some(pb::RepoRef {
                    owner: repo.owner.clone(),
                    name: repo.name.clone(),
                }),
                number,
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

impl SessionSvc {
    /// Checks each target's `GitPush` access: the first one that must be
    /// asked is returned.
    pub(super) fn ask_for_pushes(
        &self,
        caller: &Caller,
        targets: Vec<RepoBranch>,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        for target in targets {
            if let Some(asked) =
                self.gate(caller, Capability::GitPush, Target::Push(target), approved)?
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
        let id = session_id(request.id)?;
        let loaded = self.load(&id).await?;
        if self.lives_elsewhere(&loaded.record).is_some() {
            return Err(not_built_part(
                "humata.v1.SessionService.PlanPush",
                "a push on another machine",
            ));
        }
        let targets: Vec<RepoBranch> = loaded
            .detail
            .iter()
            .flat_map(|detail| detail.repos.iter())
            .map(|repo| push_target(&repo.owner, &repo.name, &repo.branch))
            .collect();
        if let Some(asked) = self.ask_for_pushes(caller, targets, approved)? {
            return Ok(Gated::Ask(asked));
        }
        let detail = loaded.detail.clone();
        let repos = blocking(move || plan_repos(detail.as_ref()))
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
        if self.lives_elsewhere(&loaded.record).is_some() {
            return Err(not_built_part(
                "humata.v1.SessionService.PushAndOpenPullRequests",
                "a push on another machine",
            ));
        }
        let session_repos: Vec<SessionRepo> = loaded
            .detail
            .iter()
            .flat_map(|detail| detail.repos.iter())
            .cloned()
            .collect();
        let pushes = chosen_repos(&request.repos, &session_repos)?;
        let targets = pushes
            .iter()
            .map(|push| push_target(&push.owner, &push.name, &push.branch))
            .collect();
        if let Some(asked) = self.ask_for_pushes(caller, targets, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
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

    /// The operation's work: the steps under the session's lock, then the
    /// session published, since its branches changed.
    async fn run_push(self, progress: Progress, id: String, pushes: Vec<RepoPush>, by: pb::Actor) {
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        let ran = run_pushes(&progress, &pushes, "push again").await;
        drop(guard);
        match ran {
            Ok(last) => progress.succeed(last.map(pb::operation::Result::PullRequest)),
            Err(stopped) => {
                progress.fail(stopped.into_error("The branches already pushed stay pushed."))
            }
        }
        self.tell
            .session(&id, pb::Change::Updated, Some(by))
            .await;
    }
}
