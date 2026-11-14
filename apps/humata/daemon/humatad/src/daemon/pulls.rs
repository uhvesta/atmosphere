//! Shipping a session as pull requests: landing pushes its branches from
//! its host and opens one pull request per repository on GitHub, from the
//! Mac; the Mac then follows them, brings their review comments into
//! threads, and when all have merged, finishes the task and cleans up.

use std::collections::BTreeMap;
use std::time::Duration;

use humata_wire::{Reply, pb};

use super::Daemon;
use crate::board::{Author, ItemFields, ItemPatch, Priority, Status};
use crate::error::DaemonError;
use crate::github::{self, Fetched, GitHub, GitHubError, Pull, Repo, ReviewComment};
use crate::store::{Checks, NewThread, PullRequest, PullState, Standing, now_unix_ms};

use pb::request::Kind as Request;
use pb::response::Kind as Response;

/// Minutes between reads of open pull requests, unless the person picks
/// others in Settings › GitHub.
const DEFAULT_MINUTES: u32 = 2;
/// The longest wait between reads: a day.
const MOST_MINUTES: u32 = 24 * 60;

/// Whether pull requests are read only when the person asks.
const MANUAL: &str = "pulls.manual";
/// Minutes between reads while they are read on their own.
const EVERY_MINUTES: &str = "pulls.every_minutes";

/// Names the API to use instead of GitHub's, for GitHub Enterprise or a
/// test.
const API_VARIABLE: &str = "HUMATA_GITHUB_API";

/// The GitHub token to use and where it comes from: the first of
/// [`github::TOKENS`] that `env` holds, or else what `command_line` gives,
/// the GitHub command line's sign-in. That one only github.com takes, so
/// with `other_api` it isn't asked for.
async fn token_from<F>(
    env: &BTreeMap<String, String>,
    other_api: bool,
    command_line: impl FnOnce() -> F,
) -> Option<(pb::GithubTokenSource, String)>
where
    F: Future<Output = Option<String>>,
{
    let sources = [
        pb::GithubTokenSource::GhToken,
        pb::GithubTokenSource::GithubToken,
    ];
    let given = github::TOKENS
        .iter()
        .zip(sources)
        .find_map(|(name, source)| {
            let token = env.get(*name).filter(|token| !token.is_empty())?;
            Some((source, token.clone()))
        });
    if given.is_some() || other_api {
        return given;
    }
    let token = command_line().await?;
    Some((pb::GithubTokenSource::CommandLine, token))
}

impl Daemon {
    /// This Mac's GitHub token and where it comes from, if there is one,
    /// and the API named instead of GitHub's, if one is.
    async fn github_token(&self) -> (Option<(pb::GithubTokenSource, String)>, Option<String>) {
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
        let token = token_from(&env, other.is_some(), github::cli_token).await;
        (token, other)
    }

    /// GitHub, with This Mac's token, and whether it is another server
    /// standing in for github.com.
    pub(super) async fn github(&self) -> Result<(GitHub, bool), GitHubError> {
        let (token, other) = self.github_token().await;
        let (_, token) = token.ok_or(GitHubError::NoToken)?;
        let api = other.as_deref().unwrap_or(github::API);
        Ok((GitHub::new(api, &token), other.is_some()))
    }

    /// Where This Mac's GitHub token comes from, never the token itself.
    pub(super) async fn github_token_source(&self) -> pb::GithubTokenSource {
        match self.github_token().await.0 {
            Some((source, _)) => source,
            None => pb::GithubTokenSource::NoToken,
        }
    }

    /// Lands the session on its host, and with `open_pull_requests`, pushes
    /// it and opens or finds its pull requests.
    pub(super) async fn land(&self, mut land: pb::LandSession) -> Result<Reply, DaemonError> {
        let host = self.host_of(&land.session_id).await?;
        // Without a token, nothing is landed: the person fixes that first.
        let github = if land.open_pull_requests {
            land.push = true;
            Some(self.github().await?)
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
            // one there already keeps what people wrote in it since. One
            // Humata doesn't know yet that says no more than it was opened
            // with was opened by a land that failed before describing it.
            let (pull, new) = match before {
                Some(before) => (github.pull(&repo, before.number).await?, false),
                None => match github.open_pull_from(&repo, &repository.branch).await? {
                    Some(pull) => {
                        let undescribed = pull
                            .body
                            .as_deref()
                            .map(str::trim)
                            .is_none_or(|body| body.is_empty() || body == details);
                        (pull, undescribed)
                    }
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
                problem: String::new(),
                pull_etag: String::new(),
                checks_etag: String::new(),
            };
            self.store.keep_pull(&kept)?;
            pulls.push(wire(&kept, item.as_deref()));
        }
        // A session that merged before and has a pull request open again
        // is finished again when that one merges.
        if pulls
            .iter()
            .any(|pull| pull.state() == pb::PullRequestState::Open)
        {
            self.store.forget_merged(session)?;
        }
        Ok(pulls)
    }

    /// How open pull requests are followed, with the default's minutes
    /// filled in.
    pub(super) fn pull_refresh(&self) -> Result<pb::PullRefresh, DaemonError> {
        let manual = self.store.setting(MANUAL)?.as_deref() == Some("1");
        let every_minutes = self
            .store
            .setting(EVERY_MINUTES)?
            .and_then(|minutes| minutes.parse().ok())
            .filter(|minutes| (1..=MOST_MINUTES).contains(minutes))
            .unwrap_or(DEFAULT_MINUTES);
        // Where the token comes from is said only to the person's read.
        Ok(pb::PullRefresh {
            manual,
            every_minutes,
            token_source: pb::GithubTokenSource::Unspecified.into(),
        })
    }

    /// Keeps how open pull requests are followed, and has the follower
    /// wait by it from now on, not by what it was waiting out.
    pub(super) fn set_pull_refresh(&self, refresh: &pb::PullRefresh) -> Result<(), DaemonError> {
        let minutes = match refresh.every_minutes {
            0 => DEFAULT_MINUTES,
            minutes if minutes <= MOST_MINUTES => minutes,
            minutes => return Err(DaemonError::PullRefreshMinutes { minutes }),
        };
        self.store
            .set_setting(MANUAL, if refresh.manual { "1" } else { "0" })?;
        self.store
            .set_setting(EVERY_MINUTES, &minutes.to_string())?;
        self.pull_refresh_changed.send_replace(());
        Ok(())
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
            let mut open = Vec::new();
            for session in &sessions {
                open.extend(
                    self.store
                        .pulls(session)?
                        .into_iter()
                        .filter(|pull| pull.state == PullState::Open),
                );
            }
            // One that can't be read shows why on itself; the rest are
            // shown as read.
            self.read_pulls(open).await?;
            for session in &sessions {
                // A clean-up that fails is tried again by the follower; it
                // doesn't hide the pull requests.
                let _ = self.finish_if_merged(session).await;
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

    /// One round of the follower, at `now`: reads the open pull requests
    /// again if `reads` and GitHub's rate limit isn't on, and otherwise
    /// only tries the clean-ups again. `limited_until` is when a rate
    /// limit a read ran into ends, kept between rounds: GitHub asks not to
    /// be asked meanwhile, and may lengthen the limit for those who do.
    pub(crate) async fn follow_round(
        &self,
        reads: bool,
        limited_until: &mut Option<tokio::time::Instant>,
        now: tokio::time::Instant,
    ) {
        if limited_until.is_some_and(|until| now >= until) {
            *limited_until = None;
        }
        // Missing a read is fine: the next one catches up, and the person
        // sees why when they open the task.
        if reads && limited_until.is_none() {
            let (limited_for, _) = self.follow_pulls().await;
            *limited_until = limited_for.map(|wait| now + wait);
        } else {
            let _ = self.retry_cleanups().await;
        }
    }

    /// Reads every open pull request again, and finishes the sessions whose
    /// pull requests have all merged, trying again those whose host
    /// couldn't be reached before. A pull request that can't be read keeps
    /// why; a clean-up that fails doesn't stop the others. Returns how
    /// long GitHub asked to be left alone, if a read ran into its rate
    /// limit, and the first failure.
    async fn follow_pulls(&self) -> (Option<Duration>, Result<(), DaemonError>) {
        let listed = self
            .store
            .open_pulls()
            .and_then(|open| Ok((open, self.store.cleanups_pending()?)));
        let (open, mut sessions) = match listed {
            Ok(listed) => listed,
            Err(error) => return (None, Err(error.into())),
        };
        for pull in &open {
            if !sessions.contains(&pull.session) {
                sessions.push(pull.session.clone());
            }
        }
        let mut failed = None;
        let mut limited_for = None;
        match self.read_pulls(open).await {
            Ok(limited) => limited_for = limited,
            Err(error) => {
                failed.get_or_insert(error);
            }
        }
        if let Err(error) = self.finish_all(&sessions).await {
            failed.get_or_insert(error);
        }
        (limited_for, failed.map_or(Ok(()), Err))
    }

    /// Tries again the clean-ups whose host couldn't be reached before,
    /// without reading GitHub, for when pull requests are read only when
    /// asked.
    async fn retry_cleanups(&self) -> Result<(), DaemonError> {
        let sessions = self.store.cleanups_pending()?;
        self.finish_all(&sessions).await
    }

    /// Finishes each of `sessions` whose pull requests have all merged; one
    /// that fails doesn't stop the others, and the first failure is returned.
    async fn finish_all(&self, sessions: &[String]) -> Result<(), DaemonError> {
        let mut failed = None;
        for session in sessions {
            if let Err(error) = self.finish_if_merged(session).await {
                failed.get_or_insert(error);
            }
        }
        failed.map_or(Ok(()), Err)
    }

    /// Reads each of `open` again. One that can't be read keeps why, shown
    /// on it until it is; a failure that would fail the rest the same way,
    /// such as no token or the rate limit, is kept on each of them unread.
    /// Fails only when what went wrong couldn't be kept. Returns how long
    /// GitHub asked to be left alone, if a read ran into its rate limit.
    async fn read_pulls(&self, open: Vec<PullRequest>) -> Result<Option<Duration>, DaemonError> {
        let mut limited_for = None;
        if open.is_empty() {
            return Ok(limited_for);
        }
        // GitHub, or why none of them can be read.
        let mut reading = self
            .github()
            .await
            .map(|(github, _)| github)
            .map_err(|error| error.to_string());
        for pull in open {
            let github = match &reading {
                Ok(github) => github.clone(),
                Err(problem) => {
                    self.store
                        .pull_problem(&pull.session, &pull.repository, problem)?;
                    continue;
                }
            };
            let (session, repository) = (pull.session.clone(), pull.repository.clone());
            if let Err(error) = self.read_pull(&github, pull).await {
                let problem = error.to_string();
                self.store.pull_problem(&session, &repository, &problem)?;
                if let DaemonError::GitHub(GitHubError::RateLimited { wait_minutes }) = &error {
                    limited_for = Some(Duration::from_secs(wait_minutes.saturating_mul(60)));
                }
                if stops_reading(&error) {
                    reading = Err(problem);
                }
            }
        }
        Ok(limited_for)
    }

    /// Reads `pull`'s state, reviews, checks and review comments, and keeps
    /// them. Only its checks are read again when GitHub says the pull
    /// request is as it was, since a review or comment would change it.
    async fn read_pull(&self, github: &GitHub, pull: PullRequest) -> Result<(), DaemonError> {
        let Some(repo) = Repo::of(&pull.github, true) else {
            return Ok(());
        };
        let read = match github
            .pull_since(&repo, pull.number, &pull.pull_etag)
            .await?
        {
            Fetched::Changed { value, etag } => Some((value, etag.unwrap_or_default())),
            Fetched::Unchanged => None,
        };
        let head = read
            .as_ref()
            .map_or(pull.head.as_str(), |(read, _)| read.head.sha.as_str())
            .to_owned();
        // A new head's checks are new.
        let checks_etag = if head == pull.head {
            pull.checks_etag.as_str()
        } else {
            ""
        };
        let checks = github.checks_since(&repo, &head, checks_etag).await?;
        let mut kept = PullRequest {
            read_unix_ms: now_unix_ms(),
            problem: String::new(),
            ..pull
        };
        if let Fetched::Changed { value, etag } = checks {
            kept.checks = checks_state(&value);
            kept.checks_etag = etag.unwrap_or_default();
        }
        if let Some((read, etag)) = read {
            let reviews = github.reviews(&repo, kept.number).await?;
            let comments = github.review_comments(&repo, kept.number).await?;
            self.bring_comments(&kept, comments)?;
            kept.state = state(&read);
            kept.review = review(&github::standing(&reviews));
            kept.url = read.html_url;
            kept.head = read.head.sha;
            kept.pull_etag = etag;
        }
        self.store.keep_pull(&kept)?;
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
    /// its host, tried again until the host has done it or is no longer
    /// one of the hosts. A session with work since, committed or not, is
    /// left as it is, for the person.
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
        match self.clean_up(session, &pulls).await {
            // A host that was removed has nothing left to clean up.
            Ok(()) | Err(DaemonError::UnknownHost { .. }) => {}
            Err(error) => return Err(error),
        }
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
        let (github, other) = self.github().await?;
        let project = self.store.project(project_id)?;
        let mut made = Vec::new();
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
                made.push(item.id);
            }
        }
        // Shown together, as listing the project's sessions for each would
        // take as long again per issue.
        let items = if made.is_empty() {
            Vec::new()
        } else {
            let mut shown = self.shown_items(project_id).await?;
            shown.retain(|item| made.contains(&item.id));
            shown.sort_by_key(|item| made.iter().position(|id| *id == item.id));
            shown
        };
        Ok(pb::IssuesImported { items, skipped })
    }
}

/// Reads open pull requests while the daemon runs, as often as Settings ›
/// GitHub says, or never on its own when they are read only when asked.
/// `changed` is told when that setting changes, which is waited by at once.
pub(super) async fn follow(
    daemon: std::sync::Weak<Daemon>,
    mut changed: tokio::sync::watch::Receiver<()>,
) {
    // The first read waits a while, as the daemon has only just started.
    let mut last = tokio::time::Instant::now();
    let mut limited_until = None;
    loop {
        let refresh = match daemon.upgrade() {
            // A setting that can't be read is followed as the default.
            Some(daemon) => daemon.pull_refresh().unwrap_or_default(),
            None => return,
        };
        let wait = next_read(&refresh, last.elapsed());
        // Clean-ups that failed are still tried again at the default pace
        // when pull requests are read only when asked; that reads no GitHub.
        let reads = wait.is_some();
        let wait = wait.unwrap_or_else(|| {
            Duration::from_secs(u64::from(DEFAULT_MINUTES) * 60).saturating_sub(last.elapsed())
        });
        let read = tokio::time::sleep(wait);
        tokio::select! {
            () = read => {}
            told = changed.changed() => {
                // The daemon is gone once nothing can tell it any more.
                if told.is_err() {
                    return;
                }
                continue;
            }
        }
        let Some(daemon) = daemon.upgrade() else {
            return;
        };
        last = tokio::time::Instant::now();
        daemon.follow_round(reads, &mut limited_until, last).await;
    }
}

/// How long after `since_last`, the time since the last read, the next read
/// is, by `refresh`; none while pull requests are read only when asked.
/// One overdue, as when the minutes were just made fewer, is now.
fn next_read(refresh: &pb::PullRefresh, since_last: Duration) -> Option<Duration> {
    if refresh.manual {
        return None;
    }
    let minutes = match refresh.every_minutes {
        0 => DEFAULT_MINUTES,
        minutes => minutes.min(MOST_MINUTES),
    };
    let every = Duration::from_secs(u64::from(minutes) * 60);
    Some(every.saturating_sub(since_last))
}

/// Whether reading more pull requests would fail the same way: no token, a
/// token GitHub refuses, the rate limit, or GitHub out of reach.
fn stops_reading(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::GitHub(
            GitHubError::NoToken
                | GitHubError::Unauthorized { .. }
                | GitHubError::RateLimited { .. }
                | GitHubError::Unreachable { .. }
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
        problem: pull.problem.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
        variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    async fn signed_in() -> Option<String> {
        Some("from-gh".to_owned())
    }

    async fn signed_out() -> Option<String> {
        None
    }

    #[tokio::test]
    async fn a_token_in_a_variable_goes_before_the_command_lines() {
        let both = env(&[("GITHUB_TOKEN", "second"), ("GH_TOKEN", "first")]);
        assert_eq!(
            token_from(&both, false, signed_in).await,
            Some((pb::GithubTokenSource::GhToken, "first".to_owned()))
        );
        // An empty one is none.
        let other = env(&[("GITHUB_TOKEN", "second"), ("GH_TOKEN", "")]);
        assert_eq!(
            token_from(&other, false, signed_in).await,
            Some((pb::GithubTokenSource::GithubToken, "second".to_owned()))
        );
    }

    #[tokio::test]
    async fn without_a_variable_the_command_lines_sign_in_is_the_token() {
        let unrelated = env(&[("CARGO_HOME", "/tmp/cargo")]);
        assert_eq!(
            token_from(&unrelated, false, signed_in).await,
            Some((pb::GithubTokenSource::CommandLine, "from-gh".to_owned()))
        );
        assert_eq!(token_from(&unrelated, false, signed_out).await, None);
    }

    #[tokio::test]
    async fn another_api_takes_only_a_variables_token() {
        assert_eq!(token_from(&env(&[]), true, signed_in).await, None);
        assert_eq!(
            token_from(&env(&[("GH_TOKEN", "first")]), true, signed_in).await,
            Some((pb::GithubTokenSource::GhToken, "first".to_owned()))
        );
    }

    #[test]
    fn manual_pull_requests_are_never_read_on_their_own() {
        let manual = pb::PullRefresh {
            manual: true,
            every_minutes: 5,
            ..Default::default()
        };
        assert_eq!(next_read(&manual, Duration::ZERO), None);
        assert_eq!(next_read(&manual, Duration::from_secs(86_400)), None);
    }

    #[test]
    fn automatic_pull_requests_are_read_once_their_minutes_pass() {
        let every = |minutes| pb::PullRefresh {
            manual: false,
            every_minutes: minutes,
            ..Default::default()
        };
        assert_eq!(
            next_read(&every(5), Duration::from_secs(60)),
            Some(Duration::from_secs(240))
        );
        assert_eq!(
            next_read(&every(5), Duration::ZERO),
            Some(Duration::from_secs(300))
        );
        // No minutes is the default.
        assert_eq!(
            next_read(&every(0), Duration::ZERO),
            Some(Duration::from_secs(u64::from(DEFAULT_MINUTES) * 60))
        );
    }

    #[test]
    fn a_read_overdue_after_fewer_minutes_is_now() {
        let one = pb::PullRefresh {
            manual: false,
            every_minutes: 1,
            ..Default::default()
        };
        // Twenty minutes into waiting out an hour, one minute is past.
        assert_eq!(
            next_read(&one, Duration::from_secs(20 * 60)),
            Some(Duration::ZERO)
        );
    }

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
