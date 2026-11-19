//! A6, PR-15..PR-17: following a review's pull requests as they move, and
//! bringing a clone up to their new heads.
//!
//! One `gh api graphql` asks GitHub for every PR's head and state at once
//! (D286). A PR whose head moved is then fetched into its clone under a ref
//! of Humata's own (`refs/humata/pr-<n>`), which moves neither the clone's
//! branch nor its files, so checking changes nothing the agent sees. Only
//! UpdateClone moves the branch, and it tells the agent what came in.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use humata2_access::{Call, Capability, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_review_diff::bracketed_paste;
use humata2_store::{ReviewPr, ReviewSessionRecord, SessionRecord, Store, StoreError};
use serde_json::Value;
use tonic::{Code, Status};

use super::PullRequests;
use super::github::{self, GithubError};
use crate::channel_service::{blocking, failed};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::review::sink::Pasted;
use crate::status;

/// The most new commits and changed files the agent is told one by one.
const TELL_COMMITS_MAX: usize = 30;
const TELL_FILES_MAX: usize = 40;

/// What went wrong following a review's pull requests.
#[derive(Debug, thiserror::Error)]
pub(super) enum HeadsError {
    #[error("session {session} is not a pull request review")]
    NotReview { session: String },
    #[error("couldn't ask GitHub for the pull requests' heads")]
    Github {
        #[source]
        source: GithubError,
    },
    #[error("{what} can't be put in a GitHub query")]
    BadName { what: String },
    #[error("couldn't run git for the clone at {clone}")]
    GitSpawn {
        clone: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("`git {args}` failed in the clone at {clone} ({status}): {stderr}")]
    GitFailed {
        clone: PathBuf,
        args: String,
        status: String,
        stderr: String,
    },
    #[error("git printed something this version of Humata can't read for `git {args}` in {clone}")]
    GitOutput { clone: PathBuf, args: String },
    #[error("couldn't reach the review's clone on its machine")]
    Remote {
        #[source]
        source: super::clone_git::RemoteGitError,
    },
}

impl HeadsError {
    pub(super) fn status(&self) -> Status {
        match self {
            Self::NotReview { .. } => status::invalid_field(
                "session",
                pb::invalid_field::Problem::NotAllowedHere,
                status::chain(self),
                "pick a pull request review session; other sessions have no pull requests to follow",
            ),
            Self::Github { source } => github::status_of(source, None, None),
            Self::BadName { .. } => failed(
                Code::FailedPrecondition,
                self,
                "this review's repository name has characters GitHub's API doesn't accept; start the review again from the pull request's link",
                None,
            ),
            Self::GitSpawn { .. } => failed(
                Code::FailedPrecondition,
                self,
                "install git on this Mac (`xcode-select --install`) and try again",
                None,
            ),
            Self::Remote { source } => source.status(),
            Self::GitFailed { .. } | Self::GitOutput { .. } => failed(
                Code::FailedPrecondition,
                self,
                "check that this Mac can reach GitHub, then try again; the review's clone was not changed unless the message names it. If it names local changes in the clone, ask the agent to commit or discard them, or discard them yourself in that folder, then update again",
                None,
            ),
        }
    }
}

/// What GitHub says a pull request is now.
pub(super) struct Remote {
    head: String,
    state: i32,
    /// The branch it merges into; empty if GitHub didn't say.
    base: String,
}

/// `pr_<owner>_<name>_<n>`, every other character turned to `_`.
fn alias(pr: &ReviewPr) -> String {
    let flat: String = format!("{}_{}_{}", pr.owner, pr.name, pr.number)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("pr_{flat}")
}

fn state_of(text: &str) -> i32 {
    (match text {
        "OPEN" => pb::PullRequestState::Open,
        "MERGED" => pb::PullRequestState::Merged,
        "CLOSED" => pb::PullRequestState::Closed,
        _ => pb::PullRequestState::Unspecified,
    }) as i32
}

fn quotable(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Asks GitHub, in one query through the host's own `gh`, for each pull
/// request's head and state, by alias. A pull request GitHub doesn't
/// answer for is left out. Blocking.
pub(super) fn remotes_blocking(prs: &[&ReviewPr]) -> Result<BTreeMap<String, Remote>, HeadsError> {
    if prs.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut query = String::from("query{");
    let mut seen = Vec::new();
    for pr in prs {
        let alias = alias(pr);
        if seen.contains(&alias) {
            continue;
        }
        for what in [&pr.owner, &pr.name] {
            if !quotable(what) {
                return Err(HeadsError::BadName { what: what.clone() });
            }
        }
        query.push_str(&format!(
            "{alias}:repository(owner:\"{}\",name:\"{}\"){{pullRequest(number:{}){{number url baseRefName headRefName headRefOid state commits{{totalCount}}}}}} ",
            pr.owner, pr.name, pr.number
        ));
        seen.push(alias);
    }
    query.push('}');
    let argument = format!("query={query}");
    let answer = github::gh_json(&["api", "graphql", "-f", &argument])
        .map_err(|source| HeadsError::Github { source })?;
    let mut remotes = BTreeMap::new();
    for alias in seen {
        let Some(pull) = answer.pointer(&format!("/data/{alias}/pullRequest")) else {
            continue;
        };
        let Some(head) = pull.get("headRefOid").and_then(Value::as_str) else {
            continue;
        };
        let state = pull.get("state").and_then(Value::as_str).map_or(0, state_of);
        let base = pull.get("baseRefName").and_then(Value::as_str).unwrap_or_default();
        remotes.insert(
            alias,
            Remote {
                head: head.to_owned(),
                state,
                base: base.to_owned(),
            },
        );
    }
    Ok(remotes)
}

fn run(clone: &Path, args: &[&str]) -> Result<Output, HeadsError> {
    // A review on another machine has its clones there (PR-4, PR-21).
    if let Some(done) = super::clone_git::remote(clone, args, None) {
        return done.map_err(|source| HeadsError::Remote { source });
    }
    Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|source| HeadsError::GitSpawn {
            clone: clone.to_path_buf(),
            source,
        })
}

pub(super) fn git(clone: &Path, args: &[&str]) -> Result<String, HeadsError> {
    let output = run(clone, args)?;
    if !output.status.success() {
        return Err(HeadsError::GitFailed {
            clone: clone.to_path_buf(),
            args: args.join(" "),
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// PR-10: `origin/<branch>` in `clone`, the ref a review's Diff compares
/// with for a pull request into `branch`. A review's clone has only the
/// branches its channel's main clone had, so a base other than main is
/// fetched from the remote the first time. Blocking.
pub(super) fn base_ref_blocking(clone: &Path, branch: &str) -> Result<String, HeadsError> {
    let tracking = format!("origin/{branch}");
    let commit = format!("{tracking}^{{commit}}");
    let lock = fetches(clone);
    let _fetching = lock.lock().unwrap_or_else(PoisonError::into_inner);
    let known = run(clone, &["rev-parse", "--verify", "--quiet", &commit])?;
    if !known.status.success() {
        git(clone, &["fetch", "--quiet", "--no-tags", "origin", &base_refspec(branch)])?;
    }
    Ok(tracking)
}

/// Fetches `origin/<branch>` into `clone` again, so the base a Diff
/// compares with is no older than the head that was just fetched, or is the
/// pull request's new base. Blocking.
fn refresh_base(clone: &Path, branch: &str) -> Result<(), HeadsError> {
    if branch.is_empty() {
        return Ok(());
    }
    fetch(clone, &base_refspec(branch))
}

fn base_refspec(branch: &str) -> String {
    format!("+refs/heads/{branch}:refs/remotes/origin/{branch}")
}

/// The lock every fetch into `clone` holds. Two fetches of one ref at once
/// fail on git's ref lock, and a Diff's check for its base would race a
/// fetch of it. Fetches run on blocking threads, so the lock is a plain
/// mutex; one is kept per clone path for the daemon's life.
fn fetches(clone: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    Arc::clone(locks.entry(clone.to_path_buf()).or_default())
}

/// `git fetch origin <refspec>` in `clone`, holding the clone's lock.
fn fetch(clone: &Path, refspec: &str) -> Result<(), HeadsError> {
    let lock = fetches(clone);
    let _fetching = lock.lock().unwrap_or_else(PoisonError::into_inner);
    git(clone, &["fetch", "--quiet", "--no-tags", "origin", refspec]).map(drop)
}

/// What a pull request's new head holds that the reviewed one doesn't.
struct Compared {
    /// The head GitHub has now, as fetched.
    tip: String,
    new_commits: u32,
    force_pushed: bool,
    /// `(sha, subject)`, newest first.
    commits: Vec<(String, String)>,
    /// Files changed between the reviewed head and the new one.
    files: Vec<String>,
    /// Per file, the lines of the reviewed head that the new one changes
    /// or removes, 1-based and inclusive.
    lines: BTreeMap<String, Vec<(u32, u32)>>,
}

/// The old lines `-a,b` of every hunk of a zero-context diff, by file.
fn changed_lines(diff: &str) -> BTreeMap<String, Vec<(u32, u32)>> {
    let mut lines: BTreeMap<String, Vec<(u32, u32)>> = BTreeMap::new();
    let mut file: Option<String> = None;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("--- ") {
            file = path
                .strip_prefix("a/")
                .map(|path| path.trim_end_matches('\t').to_owned())
                .filter(|path| !path.starts_with('"'));
        } else if let Some(hunk) = line.strip_prefix("@@ -") {
            let Some(path) = &file else { continue };
            let old = hunk.split(' ').next().unwrap_or_default();
            let (start, count) = old.split_once(',').unwrap_or((old, "1"));
            let (Ok(start), Ok(count)) = (start.parse::<u32>(), count.parse::<u32>()) else {
                continue;
            };
            // A pure insertion changes no line the reviewer saw.
            if count > 0 {
                lines
                    .entry(path.clone())
                    .or_default()
                    .push((start, start.saturating_add(count - 1)));
            }
        }
    }
    lines
}

/// Fetches pull request `number`'s head into the clone, under a ref of
/// Humata's own, and compares it with `reviewed`. Moves nothing of the
/// clone's: not its branch, not its files. Blocking.
fn compare(clone: &Path, number: u32, reviewed: &str) -> Result<Compared, HeadsError> {
    let held = format!("refs/humata/pr-{number}");
    fetch(clone, &format!("+refs/pull/{number}/head:{held}"))?;
    let tip = git(clone, &["rev-parse", "--verify", &format!("{held}^{{commit}}")])?;
    if tip == reviewed {
        return Ok(Compared {
            tip,
            new_commits: 0,
            force_pushed: false,
            commits: Vec::new(),
            files: Vec::new(),
            lines: BTreeMap::new(),
        });
    }
    let range = format!("{reviewed}..{tip}");
    let counted = git(clone, &["rev-list", "--count", &range])?;
    let new_commits = counted.parse().map_err(|_| HeadsError::GitOutput {
        clone: clone.to_path_buf(),
        args: format!("rev-list --count {range}"),
    })?;
    let ancestor = run(clone, &["merge-base", "--is-ancestor", reviewed, &tip])?;
    let force_pushed = match ancestor.status.code() {
        Some(0) => false,
        Some(1) => true,
        _ => {
            return Err(HeadsError::GitFailed {
                clone: clone.to_path_buf(),
                args: format!("merge-base --is-ancestor {reviewed} {tip}"),
                status: ancestor.status.to_string(),
                stderr: String::from_utf8_lossy(&ancestor.stderr).trim().to_owned(),
            });
        }
    };
    let listed = git(
        clone,
        &["log", "--no-color", "-n", "200", "--format=%H%x09%s", &range],
    )?;
    let commits = listed
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(sha, subject)| (sha.to_owned(), subject.to_owned()))
        .collect();
    let names = git(clone, &["diff", "--no-color", "--no-ext-diff", "--no-renames", "--name-only", reviewed, &tip])?;
    let files = names.lines().map(str::to_owned).collect();
    let diff = git(
        clone,
        &["diff", "--no-color", "--no-ext-diff", "--no-renames", "-U0", reviewed, &tip],
    )?;
    Ok(Compared {
        tip,
        new_commits,
        force_pushed,
        commits,
        files,
        lines: changed_lines(&diff),
    })
}

/// One pull request as a check found it.
pub(super) struct Found {
    pub(super) pr: ReviewPr,
    tip: String,
    state: i32,
    compared: Option<Compared>,
    /// The branch it merges into now, when that isn't the recorded one.
    retargeted: Option<String>,
}

impl Found {
    /// The pull request as it is now: its base is the retargeted one.
    fn current(&self) -> ReviewPr {
        let mut pr = self.pr.clone();
        if let Some(base) = &self.retargeted {
            pr.base_branch = base.clone();
        }
        pr
    }

    /// The head change for this pull request, with its stack among `all`,
    /// the review's pull requests as they are now.
    fn change(&self, all: &[ReviewPr]) -> pb::HeadChange {
        pb::HeadChange {
            pr: Some(pr_ref(&self.pr)),
            new_commits: self.compared.as_ref().map_or(0, |compared| compared.new_commits),
            force_pushed: self.compared.as_ref().is_some_and(|compared| compared.force_pushed),
            state: self.state,
            base_branch: self.retargeted.clone().unwrap_or_default(),
            stack: stack_of(&self.current(), all),
        }
    }

    /// Whether a check lists it (A8, D298): the head isn't the clone's, the
    /// state isn't the recorded one, or GitHub retargeted it.
    fn changed(&self) -> bool {
        self.tip != self.pr.head_sha || self.state != self.pr.state || self.retargeted.is_some()
    }

    /// Whether this is news compared with what the last check saw: a head
    /// the clone and the last check don't know, a new state, or a new base.
    pub(super) fn news(&self) -> bool {
        (self.tip != self.pr.checked_sha && self.tip != self.pr.head_sha)
            || self.state != self.pr.state
            || self.retargeted.is_some()
    }
}

pub(super) fn pr_ref(pr: &ReviewPr) -> pb::PullRequestRef {
    pb::PullRequestRef {
        repo: Some(pb::RepoRef {
            owner: pr.owner.clone(),
            name: pr.name.clone(),
        }),
        number: pr.number,
        ..pb::PullRequestRef::default()
    }
}

/// D300: where `pr` sits among the review's own pull requests `all`. Its
/// parent is the one whose head branch is the base `pr` merges into; its
/// children are the ones based on `pr`'s head branch. Only pull requests of
/// the same repository count. `None` when it has neither.
fn stack_of(pr: &ReviewPr, all: &[ReviewPr]) -> Option<pb::StackPosition> {
    let neighbours = || {
        all.iter()
            .filter(|other| other.owner == pr.owner && other.name == pr.name && other.number != pr.number)
    };
    let parent = neighbours().find(|other| other.head_branch == pr.base_branch);
    let children: Vec<pb::PullRequestRef> = neighbours()
        .filter(|other| other.base_branch == pr.head_branch)
        .map(pr_ref)
        .collect();
    if parent.is_none() && children.is_empty() {
        return None;
    }
    Some(pb::StackPosition {
        parent: parent.map(pr_ref),
        children,
    })
}

/// The head changes of `found` that `keep` picks, each with its stack among
/// the review's pull requests as they now are.
pub(super) fn changes_where(found: &[Found], keep: impl Fn(&Found) -> bool) -> Vec<pb::HeadChange> {
    let current: Vec<ReviewPr> = found.iter().map(Found::current).collect();
    found
        .iter()
        .filter(|found| keep(found))
        .map(|found| found.change(&current))
        .collect()
}

/// Compares each of `prs` with what GitHub says (`remotes`), fetching the
/// ones whose head moved into their clones under `folder`, with their base
/// (PR-10), and the new base of one GitHub retargeted. With `only_news`, a
/// head the last check already saw isn't fetched again. Blocking.
pub(super) fn found_blocking(
    folder: &Path,
    prs: &[ReviewPr],
    remotes: &BTreeMap<String, Remote>,
    only_news: bool,
) -> Result<Vec<Found>, HeadsError> {
    let mut found = Vec::new();
    for pr in prs {
        let Some(remote) = remotes.get(&alias(pr)) else {
            continue;
        };
        let quiet = remote.head == pr.head_sha
            || (only_news && remote.head == pr.checked_sha && remote.state == pr.state);
        let clone = folder.join(&pr.name);
        let compared = if quiet {
            None
        } else {
            Some(compare(&clone, pr.number, &pr.head_sha)?)
        };
        let retargeted =
            (!remote.base.is_empty() && remote.base != pr.base_branch).then(|| remote.base.clone());
        if compared.is_some() || retargeted.is_some() {
            refresh_base(&clone, retargeted.as_deref().unwrap_or(&pr.base_branch))?;
        }
        let tip = compared
            .as_ref()
            .map_or_else(|| remote.head.clone(), |compared| compared.tip.clone());
        found.push(Found {
            pr: pr.clone(),
            tip,
            state: remote.state,
            compared,
            retargeted,
        });
    }
    Ok(found)
}

/// Saves what a check saw: each pull request's newest head and state. The
/// record is read again just before it is written, so a change made
/// meanwhile (the post attempt's start) isn't lost. Blocking.
pub(super) fn record_checked(store: &Store, session: &str, found: &[Found]) -> Result<(), StoreError> {
    for found in found {
        if let Some(base) = &found.retargeted {
            let pr = &found.pr;
            store.set_review_pr_base(session, &pr.owner, &pr.name, pr.number, base)?;
        }
    }
    record(store, session, found.iter().map(|found| (&found.pr, None, found.tip.as_str(), found.state)))
}

fn record<'a>(
    store: &Store,
    session: &str,
    seen: impl Iterator<Item = (&'a ReviewPr, Option<&'a str>, &'a str, i32)>,
) -> Result<(), StoreError> {
    for (pr, head, checked, state) in seen {
        store.set_review_pr_heads(session, &pr.owner, &pr.name, pr.number, head, checked, state)?;
    }
    Ok(())
}

pub(super) fn now_stamp() -> humata2_grpc::Timestamp {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    humata2_grpc::Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    }
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// What the agent is told when its clones move: the new commits and the
/// files changed since the head it was reviewing (PR-16).
fn message_for(updates: &[Update]) -> String {
    let mut text = String::from("The pull request you are reviewing has changed on GitHub, and your clone has been brought up to date.\n");
    for update in updates {
        let compared = &update.compared;
        text.push_str(&format!(
            "\n{}/{}#{}: now at {} (was {}) on branch {}.\n",
            update.pr.owner,
            update.pr.name,
            update.pr.number,
            short(&compared.tip),
            short(&update.pr.head_sha),
            update.pr.head_branch,
        ));
        if compared.force_pushed {
            text.push_str("The branch was force-pushed: the head you reviewed is no longer part of it, so earlier commits may have been rewritten.\n");
        }
        text.push_str(&format!("New commits ({}):\n", compared.new_commits));
        for (sha, subject) in compared.commits.iter().take(TELL_COMMITS_MAX) {
            text.push_str(&format!("- {} {subject}\n", short(sha)));
        }
        if compared.commits.len() > TELL_COMMITS_MAX {
            text.push_str(&format!("- and {} more\n", compared.commits.len() - TELL_COMMITS_MAX));
        }
        text.push_str(&format!("Files changed since {}:\n", short(&update.pr.head_sha)));
        for file in compared.files.iter().take(TELL_FILES_MAX) {
            text.push_str(&format!("- {file}\n"));
        }
        if compared.files.len() > TELL_FILES_MAX {
            text.push_str(&format!("- and {} more\n", compared.files.len() - TELL_FILES_MAX));
        }
    }
    text.push_str("\nDrafts on lines that changed are marked outdated; read those files again before you answer about them.");
    text
}

/// A pull request whose clone is being brought to a new head.
struct Update {
    pr: ReviewPr,
    compared: Compared,
}

/// How long a finished check's answer stands for the next check of the
/// same session (D298). An admin build takes `HUMATA2_PR_CHECK_REUSE_MS`.
fn reuse_window() -> Duration {
    if humata2_rpc_kit::admin::is_admin_mode()
        && let Some(ms) = std::env::var("HUMATA2_PR_CHECK_REUSE_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
    {
        return Duration::from_millis(ms);
    }
    Duration::from_secs(15)
}

/// A check's answer and when it finished.
struct Checked {
    at: Instant,
    answer: pb::CheckHeadsResponse,
}

/// Each session's last check (D298). A check holds its session's slot while
/// it runs, so one that starts meanwhile waits and takes the same answer.
#[derive(Default)]
pub(super) struct Checks {
    slots: Mutex<HashMap<String, Arc<tokio::sync::Mutex<Option<Checked>>>>>,
}

impl Checks {
    fn slot(&self, session: &str) -> Arc<tokio::sync::Mutex<Option<Checked>>> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(slots.entry(session.to_owned()).or_default())
    }

    /// Drops the session's last answer, once its clone moved or the session
    /// was archived or freed.
    pub(super) fn forget(&self, session: &str) {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner).remove(session);
    }
}

impl PullRequests {
    /// The session's records, and whether the caller may read it.
    pub(super) async fn review_records(
        &self,
        id: Option<&pb::SessionId>,
    ) -> Result<(SessionRecord, ReviewSessionRecord), Status> {
        let id = id
            .map(|session| session.value.clone())
            .filter(|session| !session.is_empty())
            .ok_or_else(|| status::missing("session"))?;
        let machine = self.inner.machine.clone();
        let (store, key) = (self.inner.store.clone(), id.clone());
        let (session, review) = blocking(move || {
            let session = store.session(&key)?;
            let review = store.review_session(&key)?;
            Ok::<_, StoreError>((session, review))
        })
        .await?
        .map_err(|error| status::store(error, &machine, "look up the review"))?;
        let session = session.ok_or_else(|| {
            status::not_found(
                pb::EntityKind::Session,
                &id,
                format!("no session {id} is on this machine"),
                "list the sessions to see their ids; a review's session is on the machine it was started on",
            )
        })?;
        let review = review.ok_or_else(|| HeadsError::NotReview { session: id }.status())?;
        Ok((session, review))
    }

    // A6, PR-15, A8 (D298): one check of each PR's head against the
    // reviewed one (new commits, force push, state, base), moving nothing of
    // the clones. Only what changed is listed; news goes on the feed as a
    // poll's does. A check while one runs, or soon after one, takes its
    // answer.
    pub(super) async fn check_heads_of(
        &self,
        caller: &Caller,
        request: pb::CheckHeadsRequest,
    ) -> Result<pb::CheckHeadsResponse, Status> {
        let (session, _) = self.review_records(request.session.as_ref()).await?;
        self.inner.guard.require(
            caller,
            &Call::ReadSession(identity::session_ref(&session, &self.inner.machine)),
        )?;
        let slot = self.inner.checks.slot(&session.id);
        let mut last = slot.lock().await;
        if let Some(checked) = last.as_ref()
            && checked.at.elapsed() < reuse_window()
        {
            return Ok(checked.answer.clone());
        }
        // Read again: a check this one waited for has recorded what it saw.
        let (session, review) = self.review_records(request.session.as_ref()).await?;
        let checked_at = Some(now_stamp());
        let found = self.look_at(&session, &review, false).await?;
        let changes = changes_where(&found, Found::changed);
        let news = changes_where(&found, Found::news);
        let (store, id) = (self.inner.store.clone(), session.id.clone());
        let machine = self.inner.machine.clone();
        blocking(move || record_checked(&store, &id, &found))
            .await?
            .map_err(|error| status::store(error, &machine, "keep what the check saw"))?;
        self.publish_heads(&session.id, &session.channel, news);
        let answer = pb::CheckHeadsResponse { changes, checked_at };
        *last = Some(Checked {
            at: Instant::now(),
            answer: answer.clone(),
        });
        Ok(answer)
    }

    /// Asks GitHub about the review's pull requests and fetches the moved
    /// ones into their clones.
    async fn look_at(
        &self,
        session: &SessionRecord,
        review: &ReviewSessionRecord,
        only_news: bool,
    ) -> Result<Vec<Found>, Status> {
        let (folder, prs) = (session.folder.clone(), review.prs.clone());
        blocking(move || {
            let asked: Vec<&ReviewPr> = prs.iter().collect();
            let remotes = remotes_blocking(&asked)?;
            found_blocking(&folder, &prs, &remotes, only_news)
        })
        .await?
        .map_err(|error| error.status())
    }

    // A6, PR-16: fetch the new heads into the clones, move each clone's
    // branch to its head, mark changed drafts outdated and tell the agent
    // the new commits and the files changed.
    pub(super) async fn update_clone_of(
        &self,
        caller: &Caller,
        request: pb::UpdateCloneRequest,
    ) -> Result<pb::Operation, Status> {
        let (session, review) = self.review_records(request.session.as_ref()).await?;
        self.inner.guard.require(
            caller,
            &Call::Use {
                capability: Capability::UserReviewState,
                target: Target::Session(identity::session_ref(&session, &self.inner.machine)),
            },
        )?;
        let steps = [
            ("fetch".to_owned(), "Fetching the new commits".to_owned()),
            ("clone".to_owned(), "Moving the clone to the new head".to_owned()),
            ("drafts".to_owned(), "Marking drafts on changed lines outdated".to_owned()),
            ("tell".to_owned(), "Telling the agent what changed".to_owned()),
        ];
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::PrReview(pb::SessionId {
                value: session.id.clone(),
            })),
        };
        let progress = match self.inner.operations.start_steps(
            caller,
            pb::OperationKind::UpdatePrClone,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(operation),
        };
        let operation = progress.operation();
        let me = self.clone();
        tokio::spawn(async move { me.run_update(progress, session, review).await });
        Ok(operation)
    }

    async fn run_update(self, progress: Progress, session: SessionRecord, review: ReviewSessionRecord) {
        match self.update(&progress, &session, &review).await {
            Ok(()) => progress.succeed(None),
            Err((step, status)) => {
                progress.set_step(step, pb::StepState::Failed, Some(status.message().to_owned()));
                let error = status::error_of(&status).unwrap_or_else(|| pb::Error {
                    message: status.message().to_owned(),
                    fix: "try updating the clone again".to_owned(),
                    retry: None,
                    kind: None,
                });
                progress.fail(error);
            }
        }
    }

    /// The work of UpdateClone. Ordered so that running it again after a
    /// failure finishes the job: the head is recorded last, and what was
    /// done before is done again harmlessly.
    async fn update(
        &self,
        progress: &Progress,
        session: &SessionRecord,
        review: &ReviewSessionRecord,
    ) -> Result<(), (&'static str, Status)> {
        progress.set_step("fetch", pb::StepState::Running, None);
        let (folder, prs) = (session.folder.clone(), review.prs.clone());
        let updates = blocking(move || -> Result<Vec<Update>, HeadsError> {
            let mut updates = Vec::new();
            for pr in prs {
                let clone = folder.join(&pr.name);
                let compared = compare(&clone, pr.number, &pr.head_sha)?;
                if compared.tip != pr.head_sha {
                    // PR-10: a rebased head needs the base it was rebased on.
                    refresh_base(&clone, &pr.base_branch)?;
                    updates.push(Update { pr, compared });
                }
            }
            Ok(updates)
        })
        .await
        .map_err(|status| ("fetch", status))?
        .map_err(|error| ("fetch", error.status()))?;
        progress.set_step("fetch", pb::StepState::Done, None);

        progress.set_step("clone", pb::StepState::Running, None);
        let (folder, moves): (PathBuf, Vec<(String, String, String)>) = (
            session.folder.clone(),
            updates
                .iter()
                .map(|update| {
                    (
                        update.pr.name.clone(),
                        update.pr.head_branch.clone(),
                        update.compared.tip.clone(),
                    )
                })
                .collect(),
        );
        blocking(move || -> Result<(), HeadsError> {
            for (name, branch, tip) in moves {
                // Keeps files the new head doesn't touch; refuses, saying
                // so, if the clone has changes it would overwrite.
                git(&folder.join(name), &["checkout", "--quiet", "-B", &branch, &tip])?;
            }
            Ok(())
        })
        .await
        .map_err(|status| ("clone", status))?
        .map_err(|error| ("clone", error.status()))?;
        // A9: a mark follows its patch to the new head now, dated now. One
        // that can't is carried by the next read of the commits instead.
        let store = self.inner.store.clone();
        let (id, folder, moved): (String, PathBuf, Vec<ReviewPr>) = (
            session.id.clone(),
            session.folder.clone(),
            updates.iter().map(|update| update.pr.clone()).collect(),
        );
        let carried = blocking(move || {
            for pr in &moved {
                if let Err(error) = super::commits::carry_moved_blocking(&store, &folder, pr) {
                    crate::log::say!(
                        "humatad2: session {id}: {}/{}#{}'s reviewed commits wait for a list: {}",
                        pr.owner,
                        pr.name,
                        pr.number,
                        status::chain(&error)
                    );
                }
            }
        })
        .await;
        if let Err(status) = carried {
            crate::log::say!("humatad2: carrying reviewed commits: {}", status.message());
        }
        progress.set_step("clone", pb::StepState::Done, None);

        progress.set_step("drafts", pb::StepState::Running, None);
        let changed = |repo: &str, path: &str, first: u32, last: u32| -> bool {
            updates.iter().any(|update| {
                let slug = format!("{}/{}", update.pr.owner, update.pr.name);
                (repo == slug || repo == update.pr.name)
                    && update.compared.lines.get(path).is_some_and(|ranges| {
                        (first == 0 && last == 0) || ranges.iter().any(|&(a, b)| a <= last && first <= b)
                    })
            })
        };
        self.inner
            .review
            .outdate_drafts(&session.id, &changed)
            .await
            .map_err(|status| ("drafts", status))?;
        progress.set_step("drafts", pb::StepState::Done, None);

        progress.set_step("tell", pb::StepState::Running, None);
        if !updates.is_empty() {
            self.tell_agent(&session.id, &message_for(&updates))
                .await
                .map_err(|status| ("tell", status))?;
        }
        progress.set_step("tell", pb::StepState::Done, None);

        let (store, id) = (self.inner.store.clone(), session.id.clone());
        let machine = self.inner.machine.clone();
        blocking(move || {
            record(
                &store,
                &id,
                updates
                    .iter()
                    .map(|update| (&update.pr, Some(update.compared.tip.as_str()), update.compared.tip.as_str(), update.pr.state)),
            )
        })
        .await
        .map_err(|status| ("tell", status))?
        .map_err(|error| ("tell", status::store(error, &machine, "record the new head")))?;
        // The clone moved: the next check asks GitHub again.
        self.inner.checks.forget(&session.id);
        Ok(())
    }

    /// Types `text` to the review's agent as one message.
    async fn tell_agent(&self, session: &str, text: &str) -> Result<(), Status> {
        if let Some(told) = super::clone_git::tell_remote(session, text, &format!("{session}/pr-update/{:x}", fingerprint(text))).await {
            return told;
        }
        let sink = &self.inner.review.inner.sink;
        let target = sink.target(session).await?;
        let key = format!("{session}/pr-update/{:x}", fingerprint(text));
        match sink.paste(&target, &key, &bracketed_paste(text)).await {
            Pasted::Typed => Ok(()),
            Pasted::NotTyped(status) | Pasted::Unknown(status) => Err(status),
        }
    }
}

/// A short number for `text`, so one message is typed once however often
/// it is sent.
fn fingerprint(text: &str) -> u64 {
    text.bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pull request of `acme/<name>`, open, at head `sha<number>`.
    fn pr(name: &str, number: u32, head_branch: &str, base_branch: &str) -> ReviewPr {
        ReviewPr {
            owner: "acme".to_owned(),
            name: name.to_owned(),
            number,
            base_branch: base_branch.to_owned(),
            head_branch: head_branch.to_owned(),
            head_sha: format!("sha{number}"),
            checked_sha: format!("sha{number}"),
            state: pb::PullRequestState::Open as i32,
            submit_started_unix_ms: None,
        }
    }

    fn numbers(refs: &[pb::PullRequestRef]) -> Vec<u32> {
        refs.iter().map(|pr| pr.number).collect()
    }

    #[test]
    fn a_two_pr_stack_gives_each_its_neighbour() {
        let bottom = pr("api", 1, "feat-a", "main");
        let top = pr("api", 2, "feat-b", "feat-a");
        let all = [bottom.clone(), top.clone()];

        let bottom_stack = stack_of(&bottom, &all).expect("the bottom is in a stack");
        assert_eq!(bottom_stack.parent, None);
        assert_eq!(numbers(&bottom_stack.children), vec![2]);

        let top_stack = stack_of(&top, &all).expect("the top is in a stack");
        assert_eq!(top_stack.parent.as_ref().map(|parent| parent.number), Some(1));
        assert!(top_stack.children.is_empty());
    }

    #[test]
    fn a_tree_gives_a_pr_every_child_stacked_on_its_head() {
        let root = pr("api", 1, "feat-root", "main");
        let all = [
            root.clone(),
            pr("api", 2, "feat-x", "feat-root"),
            pr("api", 3, "feat-y", "feat-root"),
        ];

        let stack = stack_of(&root, &all).expect("the root is in a stack");
        assert_eq!(stack.parent, None);
        assert_eq!(numbers(&stack.children), vec![2, 3]);
    }

    #[test]
    fn a_pr_with_no_stacked_neighbour_is_in_no_stack() {
        let all = [pr("api", 1, "feat-a", "main"), pr("api", 2, "feat-b", "feat-a")];
        let unrelated = pr("api", 4, "other", "main");

        assert!(stack_of(&unrelated, &all).is_none());
    }

    #[test]
    fn a_pr_in_another_repo_with_the_same_branches_is_neither_parent_nor_child() {
        let web_parent_candidate = pr("web", 9, "feat-a", "main");
        let api_bottom = pr("api", 1, "feat-a", "main");
        let web_child_candidate = pr("web", 10, "feat-c", "feat-a");
        let api_top = pr("api", 2, "feat-b", "feat-a");
        let all = [
            web_parent_candidate,
            api_bottom.clone(),
            api_top.clone(),
            web_child_candidate,
        ];

        let top = stack_of(&api_top, &all).expect("the top is in a stack");
        assert_eq!(top.parent.as_ref().map(|parent| parent.number), Some(1));

        let bottom = stack_of(&api_bottom, &all).expect("the bottom is in a stack");
        assert_eq!(numbers(&bottom.children), vec![2]);
    }
}
