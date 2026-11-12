//! Shipping a session as pull requests: landing pushes its branches from
//! its host and opens one pull request per repository on GitHub, from the
//! Mac; the Mac then follows them, brings their review comments into
//! threads, and when all have merged, finishes the task and cleans up.

use std::time::Duration;

use humata_wire::{Reply, pb};

use super::Daemon;
use crate::board::{Author, ItemFields, ItemPatch, Priority, Status};
use crate::error::DaemonError;
use crate::github::{self, GitHub, GitHubError, Pull, Repo, ReviewComment};
use crate::store::{Checks, NewThread, PullRequest, PullState, Standing, now_unix_ms};

use pb::request::Kind as Request;
use pb::response::Kind as Response;

/// How often open pull requests are read again.
const EVERY: Duration = Duration::from_secs(120);

/// Names the API to use instead of GitHub's, for GitHub Enterprise or a
/// test.
const API_VARIABLE: &str = "HUMATA_GITHUB_API";

impl Daemon {
    /// GitHub, with This Mac's token, and whether it is another server
    /// standing in for github.com.
    fn github(&self) -> Result<(GitHub, bool), GitHubError> {
        let env = self
            .coordinator_env
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let other = env
            .get(API_VARIABLE)
            .cloned()
            .or_else(|| std::env::var(API_VARIABLE).ok())
            .filter(|api| !api.is_empty());
        let given = github::TOKENS
            .iter()
            .find_map(|name| env.get(*name).filter(|token| !token.is_empty()).cloned());
        // Without one, the GitHub command line's, which only github.com takes.
        let token = match given {
            Some(token) => token,
            None if other.is_none() => github::cli_token().ok_or(GitHubError::NoToken)?,
            None => return Err(GitHubError::NoToken),
        };
        let api = other.as_deref().unwrap_or(github::API);
        Ok((GitHub::new(api, &token), other.is_some()))
    }

    /// Lands the session on its host, and with `open_pull_requests`, pushes
    /// it and opens or finds its pull requests.
    pub(super) async fn land(&self, mut land: pb::LandSession) -> Result<Reply, DaemonError> {
        let host = self.host_of(&land.session_id).await?;
        // Without a token, nothing is landed: the person fixes that first.
        let github = if land.open_pull_requests {
            land.push = true;
            Some(self.github()?)
        } else {
            None
        };
        let reply = self
            .forward(&host, Request::LandSession(land.clone()))
            .await?;
        let Reply::Response(Response::SessionLanded(mut landed)) = reply else {
            return Ok(reply);
        };
        if let Some((github, other)) = github {
            landed.pull_requests = self
                .open_pulls(&github, other, &land, &landed.repositories)
                .await?;
        }
        Ok(Reply::Response(Response::SessionLanded(landed)))
    }

    async fn open_pulls(
        &self,
        github: &GitHub,
        other: bool,
        land: &pb::LandSession,
        landed: &[pb::LandedRepository],
    ) -> Result<Vec<pb::PullRequest>, DaemonError> {
        let session = &land.session_id;
        let item = self
            .store
            .placement(session)?
            .and_then(|placed| placed.item);
        let (title, details) = match land.message.trim().split_once('\n') {
            Some((title, details)) => (title.trim(), details.trim()),
            None => (land.message.trim(), ""),
        };
        let kept = self.store.pulls(session)?;
        let mut opened: Vec<(Repo, Pull, &pb::LandedRepository, bool)> = Vec::new();
        for repository in landed {
            let repo = Repo::of(&repository.origin_url, other).ok_or_else(|| {
                GitHubError::NotOnGitHub {
                    url: repository.origin_url.clone(),
                }
            })?;
            let before = kept.iter().find(|pull| {
                pull.repository == repository.name
                    && pull.branch == repository.branch
                    && pull.state == PullState::Open
            });
            // Only a pull request opened now gets Humata's description;
            // one there already keeps what people wrote in it since.
            let (pull, new) = match before {
                Some(before) => (github.pull(&repo, before.number).await?, false),
                None => match github.open_pull_from(&repo, &repository.branch).await? {
                    Some(pull) => (pull, false),
                    None => (
                        github
                            .open_pull(&repo, title, &repository.branch, &repository.base, details)
                            .await?,
                        true,
                    ),
                },
            };
            opened.push((repo, pull, repository, new));
        }
        let task = match &item {
            Some(item) => Some(self.store.item(item)?.fields.title),
            None => None,
        };
        let issue = match &item {
            Some(item) => self.store.issue_of(item)?,
            None => None,
        };
        let mut pulls = Vec::with_capacity(opened.len());
        for (repo, pull, repository, new) in &opened {
            if *new {
                let body = description(details, task.as_deref(), issue.as_deref(), repo, &opened);
                github.set_body(repo, pull.number, &body).await?;
            }
            let kept = PullRequest {
                session: session.clone(),
                repository: repository.name.clone(),
                github: repo.to_string(),
                number: pull.number,
                url: pull.html_url.clone(),
                title: title.to_owned(),
                branch: repository.branch.clone(),
                base: repository.base.clone(),
                state: state(pull),
                review: Standing::None,
                checks: Checks::None,
                read_unix_ms: now_unix_ms(),
                head: pull.head.sha.clone(),
            };
            self.store.keep_pull(&kept)?;
            pulls.push(wire(&kept, item.as_deref()));
        }
        Ok(pulls)
    }

    /// The pull requests of a session, or of a task's sessions, read from
    /// GitHub first with `refresh`.
    pub(super) async fn list_pulls(
        &self,
        list: &pb::ListPullRequests,
    ) -> Result<pb::PullRequests, DaemonError> {
        let sessions = if list.item_id.is_empty() {
            vec![list.session_id.clone()]
        } else {
            self.store.item(&list.item_id)?.session_ids
        };
        if list.refresh {
            let (github, _) = self.github()?;
            // Every pull request is read even when one fails, so the ones
            // that could be read are fresh; the first failure is shown.
            let mut failed = None;
            for session in &sessions {
                for pull in self.store.pulls(session)? {
                    if pull.state == PullState::Open
                        && let Err(error) = self.read_pull(&github, pull).await
                    {
                        if stops_reading(&error) {
                            return Err(error);
                        }
                        failed.get_or_insert(error);
                    }
                }
                // A clean-up that fails is tried again by the follower; it
                // doesn't hide the pull requests.
                let _ = self.finish_if_merged(session).await;
            }
            if let Some(error) = failed {
                return Err(error);
            }
        }
        let mut pull_requests = Vec::new();
        for session in &sessions {
            let item = self
                .store
                .placement(session)?
                .and_then(|placed| placed.item);
            for pull in self.store.pulls(session)? {
                pull_requests.push(wire(&pull, item.as_deref()));
            }
        }
        Ok(pb::PullRequests { pull_requests })
    }

    /// Reads every open pull request again, and finishes the sessions whose
    /// pull requests have all merged, trying again those whose host
    /// couldn't be reached before. One that fails doesn't stop the others;
    /// the first failure is returned at the end.
    async fn follow_pulls(&self) -> Result<(), DaemonError> {
        let open = self.store.open_pulls()?;
        let mut sessions = self.store.cleanups_pending()?;
        let mut failed = None;
        if !open.is_empty() {
            let (github, _) = self.github()?;
            for pull in open {
                if !sessions.contains(&pull.session) {
                    sessions.push(pull.session.clone());
                }
                if let Err(error) = self.read_pull(&github, pull).await {
                    if stops_reading(&error) {
                        return Err(error);
                    }
                    failed.get_or_insert(error);
                }
            }
        }
        for session in &sessions {
            if let Err(error) = self.finish_if_merged(session).await {
                failed.get_or_insert(error);
            }
        }
        failed.map_or(Ok(()), Err)
    }

    /// Reads `pull`'s state, reviews, checks and review comments, and keeps
    /// them.
    async fn read_pull(&self, github: &GitHub, pull: PullRequest) -> Result<(), DaemonError> {
        let Some(repo) = Repo::of(&pull.github, true) else {
            return Ok(());
        };
        let read = github.pull(&repo, pull.number).await?;
        let reviews = github.reviews(&repo, pull.number).await?;
        let checks = github.checks(&repo, &read.head.sha).await?;
        let comments = github.review_comments(&repo, pull.number).await?;
        self.bring_comments(&pull, comments)?;
        self.store.keep_pull(&PullRequest {
            state: state(&read),
            review: review(&github::standing(&reviews)),
            checks: checks_state(&checks),
            url: read.html_url,
            read_unix_ms: now_unix_ms(),
            head: read.head.sha,
            ..pull
        })?;
        Ok(())
    }

    /// Starts or joins a thread for each review comment not brought in yet;
    /// they wait, unsent, for the next review.
    fn bring_comments(
        &self,
        pull: &PullRequest,
        mut comments: Vec<ReviewComment>,
    ) -> Result<(), DaemonError> {
        comments.sort_by_key(|comment| comment.id);
        for comment in comments {
            if self.store.github_thread(comment.id)?.is_some() {
                continue;
            }
            let login = comment
                .user
                .as_ref()
                .map(|user| user.login.clone())
                .unwrap_or_default();
            let replied = match comment.in_reply_to_id {
                Some(parent) => self.store.github_thread(parent)?,
                None => None,
            };
            if let Some(thread) = replied {
                self.store
                    .add_github_comment(thread, comment.id, &login, &comment.body)?;
                continue;
            }
            let end = comment.line.or(comment.original_line).unwrap_or(1);
            let start = comment.start_line.unwrap_or(end).min(end);
            let new = NewThread {
                session: pull.session.clone(),
                repository: pull.repository.clone(),
                path: comment.path.clone(),
                start_line: start,
                end_line: end,
                code: hunk_lines(&comment.diff_hunk, end - start + 1),
                before: Vec::new(),
                quote: String::new(),
            };
            self.store
                .add_github_thread(&new, comment.id, &login, &comment.body)?;
        }
        Ok(())
    }

    /// When every pull request of `session` has merged: its task to Done
    /// (once, so a task the person reopens stays open), then the session
    /// archived, which removes its worktrees, and its branches deleted on
    /// its host, tried again until the host has done it. A session with
    /// work since, committed or not, is left as it is, for the person.
    async fn finish_if_merged(&self, session: &str) -> Result<(), DaemonError> {
        let pulls = self.store.pulls(session)?;
        if pulls.is_empty() || pulls.iter().any(|pull| pull.state != PullState::Merged) {
            return Ok(());
        }
        if self.store.mark_merged(session)? {
            self.task_done(session)?;
        } else if !self
            .store
            .cleanups_pending()?
            .iter()
            .any(|pending| pending == session)
        {
            return Ok(());
        }
        self.clean_up(session, &pulls).await?;
        self.store.mark_cleaned(session)?;
        Ok(())
    }

    fn task_done(&self, session: &str) -> Result<(), DaemonError> {
        if let Some(item) = self
            .store
            .placement(session)?
            .and_then(|placed| placed.item)
        {
            self.store.update_item(
                &item,
                ItemPatch {
                    status: Some(Status::Done),
                    ..ItemPatch::default()
                },
                &Author::on(pb::ChangeSurface::Humata),
            )?;
        }
        Ok(())
    }

    async fn clean_up(&self, session: &str, pulls: &[PullRequest]) -> Result<(), DaemonError> {
        let host = self.host_of(session).await?;
        let delete = Request::DeleteBranches(pb::DeleteBranches {
            archive: true,
            session_id: session.to_owned(),
            branches: pulls
                .iter()
                .map(|pull| pb::LandedBranch {
                    repository: pull.repository.clone(),
                    branch: pull.branch.clone(),
                    merged_tip: pull.head.clone(),
                })
                .collect(),
        });
        refused(self.forward(&host, delete).await?)
    }

    /// Makes a task of each open issue of the project's repositories on
    /// GitHub not imported before.
    pub(super) async fn import_issues(
        &self,
        project_id: &str,
    ) -> Result<pb::IssuesImported, DaemonError> {
        let (github, other) = self.github()?;
        let project = self.store.project(project_id)?;
        let mut items = Vec::new();
        let mut skipped = 0;
        for repository in &project.repositories {
            let url = match project.urls.get(&repository.name) {
                Some(url) => url.clone(),
                None => super::origin("", &repository.path)
                    .await
                    .unwrap_or_default(),
            };
            let Some(repo) = Repo::of(&url, other) else {
                continue;
            };
            for issue in github.issues(&repo).await? {
                let key = format!("{repo}#{}", issue.number);
                if self.store.imported_issue(project_id, &key)?.is_some() {
                    skipped += 1;
                    continue;
                }
                let item = self.store.add_item(
                    project_id,
                    ItemFields {
                        parent_id: None,
                        status: Status::Todo,
                        priority: Priority::default(),
                        title: issue.title,
                        spec: format!("{}\n\nFrom {key}.", issue.body.unwrap_or_default().trim())
                            .trim_start()
                            .to_owned(),
                    },
                    &Author::on(pb::ChangeSurface::Humata),
                )?;
                self.store.record_issue(project_id, &key, &item.id)?;
                items.push(self.shown_item(&item).await?);
            }
        }
        Ok(pb::IssuesImported { items, skipped })
    }
}

/// Reads open pull requests every few minutes while the daemon runs.
pub(super) async fn follow(daemon: std::sync::Weak<Daemon>) {
    let mut every = tokio::time::interval(EVERY);
    every.tick().await;
    loop {
        every.tick().await;
        let Some(daemon) = daemon.upgrade() else {
            return;
        };
        // Missing a read is fine: the next one catches up, and the person
        // sees why when they open the task.
        let _ = daemon.follow_pulls().await;
    }
}

/// Whether reading more pull requests would fail the same way: no token, a
/// token GitHub refuses, or the rate limit.
fn stops_reading(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::GitHub(
            GitHubError::NoToken
                | GitHubError::Unauthorized { .. }
                | GitHubError::RateLimited { .. }
        )
    )
}

/// A forwarded request's failure, as the error it is.
fn refused(reply: Reply) -> Result<(), DaemonError> {
    match reply {
        Reply::Response(Response::Error(error)) => Err(DaemonError::Refused {
            code: error.code(),
            message: error.message,
        }),
        _ => Ok(()),
    }
}

/// The description of the pull request to `repo`: what the change is, the
/// task, the issue it closes, and the others it goes with.
fn description(
    details: &str,
    task: Option<&str>,
    issue: Option<&str>,
    repo: &Repo,
    all: &[(Repo, Pull, &pb::LandedRepository, bool)],
) -> String {
    let mut body = String::new();
    if !details.is_empty() {
        body.push_str(details);
        body.push_str("\n\n");
    }
    if let Some(task) = task {
        body.push_str(&format!("Task: {task}\n"));
    }
    if let Some(issue) = issue
        && let Some((issued, number)) = issue.rsplit_once('#')
    {
        if issued == repo.to_string() {
            body.push_str(&format!("Closes #{number}\n"));
        } else {
            body.push_str(&format!("For {issue}\n"));
        }
    }
    let others: Vec<String> = all
        .iter()
        .filter(|(other, ..)| other != repo)
        .map(|(other, pull, ..)| format!("- {other}#{}", pull.number))
        .collect();
    if !others.is_empty() {
        body.push_str("\nGoes with:\n");
        body.push_str(&others.join("\n"));
        body.push('\n');
    }
    body.push_str("\nLanded from Humata.");
    body
}

fn state(pull: &Pull) -> PullState {
    if pull.merged || pull.merged_at.is_some() {
        PullState::Merged
    } else if pull.state == "closed" {
        PullState::Closed
    } else {
        PullState::Open
    }
}

/// Where the reviewers stand together: one asking for changes holds it
/// back, else an approval lets it through.
fn review(standing: &[(String, String)]) -> Standing {
    let any = |wanted: &str| standing.iter().any(|(_, state)| state == wanted);
    if any("CHANGES_REQUESTED") {
        Standing::ChangesRequested
    } else if any("APPROVED") {
        Standing::Approved
    } else if any("COMMENTED") {
        Standing::Commented
    } else {
        Standing::None
    }
}

fn checks_state(runs: &[github::CheckRun]) -> Checks {
    if runs.is_empty() {
        return Checks::None;
    }
    let failed = |conclusion: &str| {
        matches!(
            conclusion,
            "failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure"
        )
    };
    if runs
        .iter()
        .any(|run| run.conclusion.as_deref().is_some_and(failed))
    {
        Checks::Failing
    } else if runs.iter().any(|run| run.status != "completed") {
        Checks::Pending
    } else {
        Checks::Passing
    }
}

/// The last `count` lines of a review comment's diff hunk, as the file has
/// them.
fn hunk_lines(hunk: &str, count: u32) -> Vec<String> {
    let lines: Vec<String> = hunk
        .lines()
        .filter(|line| !line.starts_with("@@") && !line.starts_with('-'))
        .map(|line| line.get(1..).unwrap_or_default().to_owned())
        .collect();
    let count = usize::try_from(count)
        .unwrap_or(usize::MAX)
        .min(lines.len());
    lines[lines.len() - count..].to_vec()
}

fn wire(pull: &PullRequest, item: Option<&str>) -> pb::PullRequest {
    pb::PullRequest {
        session_id: pull.session.clone(),
        item_id: item.unwrap_or_default().to_owned(),
        repository: pull.repository.clone(),
        github_repository: pull.github.clone(),
        number: pull.number,
        url: pull.url.clone(),
        title: pull.title.clone(),
        branch: pull.branch.clone(),
        base: pull.base.clone(),
        state: match pull.state {
            PullState::Open => pb::PullRequestState::Open,
            PullState::Merged => pb::PullRequestState::Merged,
            PullState::Closed => pb::PullRequestState::Closed,
        }
        .into(),
        review: match pull.review {
            Standing::None => pb::ReviewStanding::None,
            Standing::ChangesRequested => pb::ReviewStanding::ChangesRequested,
            Standing::Commented => pb::ReviewStanding::Commented,
            Standing::Approved => pb::ReviewStanding::Approved,
        }
        .into(),
        checks: match pull.checks {
            Checks::None => pb::ChecksState::None,
            Checks::Pending => pb::ChecksState::Pending,
            Checks::Passing => pb::ChecksState::Passing,
            Checks::Failing => pb::ChecksState::Failing,
        }
        .into(),
        read_unix_ms: pull.read_unix_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comment_keeps_the_lines_it_is_on_from_its_hunk() {
        let hunk = "@@ -1,3 +1,4 @@\n fn main() {\n-    old();\n+    new();\n+    more();";
        assert_eq!(hunk_lines(hunk, 2), vec!["    new();", "    more();"]);
        assert_eq!(hunk_lines(hunk, 9).len(), 3);
    }

    #[test]
    fn changes_requested_hold_a_pull_request_back_over_approvals() {
        let standing = |states: &[&str]| {
            states
                .iter()
                .map(|state| (String::new(), (*state).to_owned()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            review(&standing(&["APPROVED", "CHANGES_REQUESTED"])),
            Standing::ChangesRequested
        );
        assert_eq!(
            review(&standing(&["COMMENTED", "APPROVED"])),
            Standing::Approved
        );
        assert_eq!(review(&[]), Standing::None);
    }
}
