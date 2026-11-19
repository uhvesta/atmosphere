//! A5, PR-12..PR-14, D287: posting a review, one per pull request, only on
//! Submit.
//!
//! PlanSubmit sets each PR's drafts against the PR's own diff in the
//! session's clone (no network): a draft whose lines changed, or that GitHub
//! would refuse with 422 because they aren't in the diff, is left out and
//! stays a draft. The plan token is single-use and binds the clone's head,
//! which is the `commit_id` the review is posted at.
//!
//! SubmitReview writes down what it is about to post (the attempt's start
//! in the store, its body and drafts in a file beside the store) before the
//! POST, so a restart can look for the review on GitHub (`reconcile_submits`)
//! and never posts the same review twice blindly: a review found is the
//! posted one, one not found leaves the drafts as drafts, marked unconfirmed.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use humata2_access::{Call, Capability, Target};
use humata2_daemon_ports::Caller;
use humata2_daemon_runtime::fault;
use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::{
    blocking, chain, error_of, failed, invalid_field, log::say, missing, not_found,
};
use humata2_store::{ReviewPr, ReviewSessionRecord, StoreError};
use serde_json::{Value, json};
use tonic::{Code, Status};

use super::PullRequests;
use super::github;
use crate::ports::{session_ref, store_error};

/// How long a plan's token stays good.
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// A review GitHub lists as submitted this long before the attempt began
/// still counts, so a few seconds of difference between the clocks don't
/// hide the review that was posted.
const CLOCK_SLACK_SECONDS: i64 = 60;

/// What planning or posting a review went wrong with, before it is said as
/// a status.
#[derive(Debug, thiserror::Error)]
enum SubmitError {
    #[error("couldn't run git in {path}")]
    GitSpawn {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("`git {command}` failed in {path}: {stderr}")]
    GitFailed {
        command: String,
        path: PathBuf,
        stderr: String,
    },
    #[error("the clone of {pr} isn't in this review's session folder")]
    NoClone { pr: String },
    #[error("this plan was already used, or has expired, or is of another review")]
    StalePlan,
    #[error("the clone of {pr} moved from {planned} to {now} since the plan was made")]
    HeadMoved { pr: String, planned: String, now: String },
    #[error("{pr} is not one of this review's pull requests")]
    NoSuchPr { pr: String },
    #[error("{pr} was named twice in the submission")]
    NamedTwice { pr: String },
    #[error("draft {id} can't be posted to {pr}: it isn't a draft for it, or its lines are left out of the plan")]
    NotPostable { id: String, pr: String },
    #[error("couldn't keep a note of the review of {pr} before posting it")]
    Journal {
        pr: String,
        #[source]
        source: io::Error,
    },
    #[error("couldn't reach the review's clone on its machine")]
    Remote {
        #[source]
        source: super::clone_git::RemoteGitError,
    },
    #[error("couldn't keep a note of the review of {pr} before posting it")]
    JournalStore {
        pr: String,
        #[source]
        source: StoreError,
    },
}

impl SubmitError {
    fn status(&self) -> Status {
        match self {
            Self::StalePlan | Self::HeadMoved { .. } => failed(
                Code::FailedPrecondition,
                self,
                "plan the submit again, read what it would post, and send the new token; nothing was posted",
                None,
            ),
            Self::NoSuchPr { .. } | Self::NamedTwice { .. } => invalid_field(
                "reviews",
                pb::invalid_field::Problem::NotAllowedHere,
                chain(self),
                "name each pull request of the review once, as the plan lists them",
            ),
            Self::NotPostable { .. } => invalid_field(
                "reviews.drafts",
                pb::invalid_field::Problem::NotAllowedHere,
                chain(self),
                "plan the submit again and send only the drafts it lists to post; a draft left out stays a draft until its lines are fixed or it is dropped",
            ),
            Self::NoClone { .. } => failed(
                Code::FailedPrecondition,
                self,
                "the clone was moved or deleted: restore the session's folder, or start the review again",
                None,
            ),
            Self::Remote { source } => source.status(),
            Self::GitSpawn { .. } | Self::GitFailed { .. } => failed(
                Code::FailedPrecondition,
                self,
                "check that git is installed on this machine and the session's clone is a healthy repository (`git status` in it), then try again",
                None,
            ),
            Self::Journal { .. } | Self::JournalStore { .. } => failed(
                Code::Internal,
                self,
                "free some space or fix the permissions of Humata's state folder, then submit again; nothing was posted",
                None,
            ),
        }
    }
}

/// Plans made and not yet taken, and the lock one submit holds, so two
/// submits of a session don't interleave.
#[derive(Default)]
pub(super) struct Plans {
    made: Mutex<HashMap<String, Planned>>,
    running: tokio::sync::Mutex<()>,
}

struct Planned {
    session: String,
    made: Instant,
    /// Each pull request's `owner/name#number` and the head it was planned at.
    heads: BTreeMap<String, String>,
}

impl Plans {
    fn make(&self, session: &str, heads: BTreeMap<String, String>) -> String {
        let token = format!("pl_{}", uuid::Uuid::new_v4().simple());
        let mut made = self.made.lock().unwrap_or_else(PoisonError::into_inner);
        made.retain(|_, planned| planned.made.elapsed() < PLAN_LIFETIME);
        made.insert(
            token.clone(),
            Planned {
                session: session.to_owned(),
                made: Instant::now(),
                heads,
            },
        );
        token
    }

    /// Uses `token` up: the heads it was planned at, when it was a live plan
    /// of `session`.
    fn take(&self, token: &str, session: &str) -> Option<BTreeMap<String, String>> {
        let mut made = self.made.lock().unwrap_or_else(PoisonError::into_inner);
        let live = made
            .get(token)
            .is_some_and(|planned| planned.session == session && planned.made.elapsed() < PLAN_LIFETIME);
        if live { made.remove(token).map(|planned| planned.heads) } else { None }
    }
}

/// One pull request as the plan sees it.
struct PerPr {
    pr: ReviewPr,
    reference: pb::PullRequestRef,
    /// The clone's head: the `commit_id` the review is posted at.
    commit: String,
    /// What will be posted.
    drafts: Vec<pb::Comment>,
    left_out: Vec<(pb::Comment, pb::submit_plan::left_out::Reason)>,
}

impl PerPr {
    fn key(&self) -> String {
        key_of(&self.pr)
    }
}

fn key_of(pr: &ReviewPr) -> String {
    format!("{}/{}#{}", pr.owner, pr.name, pr.number)
}

struct KeyOf<'a>(&'a pb::PullRequestRef);

impl fmt::Display for KeyOf<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let repo = self.0.repo.clone().unwrap_or_default();
        write!(formatter, "{}/{}#{}", repo.owner, repo.name, self.0.number)
    }
}

/// Two comment ids name the same comment (with or without the session).
fn same_id(left: &str, right: &str) -> bool {
    left == right || left.rsplit('/').next() == right.rsplit('/').next()
}

fn is_draft_for(comment: &pb::Comment, pr: &ReviewPr) -> bool {
    let for_this = matches!(
        comment.destination.as_ref().and_then(|destination| destination.destination.as_ref()),
        Some(pb::comment_destination::Destination::PullRequest(target))
            if target.number == pr.number
                && target.repo.as_ref().is_some_and(|repo| repo.owner == pr.owner && repo.name == pr.name)
    );
    for_this && comment.status == pb::CommentStatus::Draft as i32 && comment.archived_at.is_none()
}

fn id_of(comment: &pb::Comment) -> &str {
    comment.id.as_ref().map_or("", |id| id.value.as_str())
}

// ---------------------------------------------------------------------------
// The diff a pull request's lines are checked against.

/// Inclusive line ranges a diff shows of one file, per side.
#[derive(Default)]
struct Shown {
    old: Vec<(u32, u32)>,
    new: Vec<(u32, u32)>,
}

impl Shown {
    fn holds(&self, side: Side, first: u32, last: u32) -> bool {
        let ranges = match side {
            Side::Old => &self.old,
            Side::New => &self.new,
        };
        ranges.iter().any(|(from, to)| *from <= first && last <= *to)
    }
}

#[derive(Clone, Copy)]
enum Side {
    Old,
    New,
}

fn span(text: &str) -> Option<(u32, u32)> {
    match text.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

/// `@@ -a,b +c,d @@` as the old and new `(start, count)`.
fn hunk(line: &str) -> Option<((u32, u32), (u32, u32))> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    Some((span(old)?, span(new)?))
}

fn range_of((start, count): (u32, u32)) -> Option<(u32, u32)> {
    (count > 0).then(|| (start, start.saturating_add(count - 1)))
}

/// The lines each file's hunks show, from `git diff` output with context.
fn parse_diff(text: &str) -> HashMap<String, Shown> {
    let mut files: HashMap<String, Shown> = HashMap::new();
    let mut old_path: Option<String> = None;
    let mut current: Option<String> = None;
    let (mut old_left, mut new_left) = (0_u32, 0_u32);
    for line in text.lines() {
        if old_left > 0 || new_left > 0 {
            match line.chars().next() {
                Some('-') => old_left = old_left.saturating_sub(1),
                Some('+') => new_left = new_left.saturating_sub(1),
                Some('\\') => {}
                _ => {
                    old_left = old_left.saturating_sub(1);
                    new_left = new_left.saturating_sub(1);
                }
            }
            continue;
        }
        if line.starts_with("diff --git ") {
            old_path = None;
            current = None;
        } else if let Some(path) = line.strip_prefix("--- ") {
            old_path = path.strip_prefix("a/").map(str::to_owned);
        } else if let Some(path) = line.strip_prefix("+++ ") {
            current = path.strip_prefix("b/").map(str::to_owned).or_else(|| old_path.take());
            if let Some(path) = &current {
                files.entry(path.clone()).or_default();
            }
        } else if let Some((old, new)) = hunk(line) {
            if let Some(shown) = current.as_ref().and_then(|path| files.get_mut(path)) {
                shown.old.extend(range_of(old));
                shown.new.extend(range_of(new));
            }
            (old_left, new_left) = (old.1, new.1);
        }
    }
    files
}

/// Runs git in `path` with an environment the caller's can't steer.
fn git(path: &Path, args: &[&str]) -> Result<String, SubmitError> {
    // A review on another machine has its clones there (PR-4, PR-21).
    let output = match super::clone_git::remote(path, args, None) {
        Some(done) => done.map_err(|source| SubmitError::Remote { source })?,
        None => Command::new("git")
            .args(args)
            .current_dir(path)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .map_err(|source| SubmitError::GitSpawn {
                path: path.to_owned(),
                source,
            })?,
    };
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(SubmitError::GitFailed {
            command: args.join(" "),
            path: path.to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// The clone's head and, when the PR's base branch is there to compare with,
/// the lines of `base...head`. A base the clone doesn't have is not checked:
/// GitHub is left to say.
fn clone_facts(path: &Path, base_branch: &str) -> Result<(String, Option<HashMap<String, Shown>>), SubmitError> {
    let head = git(path, &["rev-parse", "--verify", "HEAD"])?.trim().to_owned();
    for base in [format!("origin/{base_branch}"), base_branch.to_owned()] {
        let spec = format!("{base}^{{commit}}");
        if git(path, &["rev-parse", "--verify", "--quiet", &spec]).is_err() {
            continue;
        }
        let range = format!("{base}...{head}");
        let diff = git(
            path,
            &[
                "-c",
                "core.quotepath=off",
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-renames",
                "--unified=3",
                &range,
            ],
        )?;
        return Ok((head, Some(parse_diff(&diff))));
    }
    Ok((head, None))
}

fn side_of(comment: &pb::Comment) -> Side {
    match comment.anchor.as_ref().map(|anchor| anchor.side) {
        Some(side) if side == pb::DiffSide::Old as i32 => Side::Old,
        _ => Side::New,
    }
}

fn lines_of(comment: &pb::Comment) -> (u32, u32) {
    let lines = comment
        .anchor
        .as_ref()
        .and_then(|anchor| anchor.lines.clone())
        .unwrap_or_default();
    (lines.first, lines.last.max(lines.first))
}

fn path_of(comment: &pb::Comment) -> String {
    comment
        .anchor
        .as_ref()
        .and_then(|anchor| anchor.file.as_ref())
        .map(|file| file.path.clone())
        .unwrap_or_default()
}

/// Whether `comment`'s lines are in the diff `shown` (all of it when none).
fn in_diff(comment: &pb::Comment, shown: Option<&HashMap<String, Shown>>) -> bool {
    let Some(shown) = shown else { return true };
    let (first, last) = lines_of(comment);
    shown
        .get(&path_of(comment))
        .is_some_and(|file| file.holds(side_of(comment), first, last))
}

// ---------------------------------------------------------------------------
// What is written down before a post.

/// What a post was going to say, kept beside the store before the post.
struct Attempt {
    commit_id: String,
    body: String,
    started_unix_ms: i64,
    /// The drafts in it: id, path, last line and text.
    drafts: Vec<(String, String, u32, String)>,
}

impl Attempt {
    fn to_json(&self) -> Value {
        json!({
            "commit_id": self.commit_id,
            "body": self.body,
            "started_unix_ms": self.started_unix_ms,
            "drafts": self.drafts.iter().map(|(id, path, line, text)| {
                json!({ "id": id, "path": path, "line": line, "text": text })
            }).collect::<Vec<_>>(),
        })
    }

    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            commit_id: value.get("commit_id")?.as_str()?.to_owned(),
            body: value.get("body")?.as_str()?.to_owned(),
            started_unix_ms: value.get("started_unix_ms")?.as_i64()?,
            drafts: value
                .get("drafts")?
                .as_array()?
                .iter()
                .filter_map(|draft| {
                    Some((
                        draft.get("id")?.as_str()?.to_owned(),
                        draft.get("path")?.as_str()?.to_owned(),
                        u32::try_from(draft.get("line")?.as_u64()?).ok()?,
                        draft.get("text")?.as_str()?.to_owned(),
                    ))
                })
                .collect(),
        })
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

/// Unix seconds of an RFC 3339 time such as `2026-11-16T10:20:30Z` (or with
/// a `+hh:mm` offset); none when it isn't one.
fn unix_seconds(text: &str) -> Option<i64> {
    let (date, time) = text.split_once(['T', 't', ' '])?;
    let mut ymd = date.split('-');
    let (year, month, day): (i64, i64, i64) = (
        ymd.next()?.parse().ok()?,
        ymd.next()?.parse().ok()?,
        ymd.next()?.parse().ok()?,
    );
    let split = time.find(['Z', 'z', '+', '-']).unwrap_or(time.len());
    let (clock, zone) = time.split_at(split);
    let mut hms = clock.split(':');
    let (hour, minute): (i64, i64) = (hms.next()?.parse().ok()?, hms.next()?.parse().ok()?);
    let second: i64 = hms.next().map_or(Some(0), |s| s.split('.').next()?.parse().ok())?;
    let offset = match zone.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let mut parts = zone[1..].split(':');
            let hours: i64 = parts.next()?.parse().ok()?;
            let minutes: i64 = parts.next().map_or(Some(0), |m| m.parse().ok())?;
            let seconds = hours * 3600 + minutes * 60;
            if sign == '-' { -seconds } else { seconds }
        }
        _ => 0,
    };
    // Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

impl PullRequests {
    /// The review session of `session` (any caller that may submit), its
    /// binding and its record.
    async fn submittable(
        &self,
        caller: &Caller,
        session: Option<&pb::SessionId>,
    ) -> Result<(humata2_store::SessionRecord, ReviewSessionRecord), Status> {
        let id = session
            .map(|session| session.value.clone())
            .filter(|session| !session.is_empty())
            .ok_or_else(|| missing("session"))?;
        let machine = self.inner.machine.clone();
        let (store, key) = (self.inner.store.clone(), id.clone());
        let (record, binding) = blocking(move || -> Result<_, StoreError> {
            Ok((store.session(&key)?, store.review_session(&key)?))
        })
        .await?
        .map_err(|error| store_error(error, &machine, "look up the review"))?;
        let record = record.ok_or_else(|| {
            not_found(
                pb::EntityKind::Session,
                &id,
                format!("no session {id} is on this machine"),
                "list the sessions to see their ids; a review's session is on the machine it was started on",
            )
        })?;
        let target = session_ref(&record, &self.inner.machine);
        self.inner.guard.require(
            caller,
            &Call::Use {
                capability: Capability::PrSubmit,
                target: Target::Session(target),
            },
        )?;
        let binding = binding.ok_or_else(|| {
            invalid_field(
                "session",
                pb::invalid_field::Problem::NotAllowedHere,
                format!("session {id} is not a pull request review, so there is no review to submit"),
                "submit from a review session; a session that isn't a pull request review posts nothing to GitHub",
            )
        })?;
        Ok((record, binding))
    }

    /// Each pull request's drafts, set against its diff in the clone.
    async fn prepare(&self, session: &str, binding: &ReviewSessionRecord) -> Result<Vec<PerPr>, Status> {
        let store = self.inner.store.clone();
        let key = session.to_owned();
        let machine = self.inner.machine.clone();
        let detail = blocking(move || store.session_detail(&key))
            .await?
            .map_err(|error| store_error(error, &machine, "look up the review's clones"))?;
        let comments = self.inner.review.pr_comments(session).await?;
        let mut planned = Vec::new();
        for pr in &binding.prs {
            let clone = detail
                .as_ref()
                .and_then(|detail| {
                    detail
                        .repos
                        .iter()
                        .find(|repo| repo.owner.eq_ignore_ascii_case(&pr.owner) && repo.name.eq_ignore_ascii_case(&pr.name))
                })
                .map(|repo| repo.path.clone())
                .ok_or_else(|| SubmitError::NoClone { pr: key_of(pr) }.status())?;
            let base = pr.base_branch.clone();
            let (commit, shown) = blocking(move || clone_facts(&clone, &base))
                .await?
                .map_err(|error| error.status())?;
            let mut drafts = Vec::new();
            let mut left_out = Vec::new();
            for comment in comments.iter().filter(|comment| is_draft_for(comment, pr)) {
                if comment.anchor.as_ref().is_some_and(|anchor| anchor.outdated) {
                    left_out.push((comment.clone(), pb::submit_plan::left_out::Reason::Outdated));
                } else if !in_diff(comment, shown.as_ref()) {
                    left_out.push((comment.clone(), pb::submit_plan::left_out::Reason::OutOfDiff));
                } else {
                    drafts.push(comment.clone());
                }
            }
            planned.push(PerPr {
                pr: pr.clone(),
                reference: pb::PullRequestRef {
                    repo: Some(pb::RepoRef {
                        owner: pr.owner.clone(),
                        name: pr.name.clone(),
                    }),
                    number: pr.number,
                    note: String::new(),
                },
                commit,
                drafts,
                left_out,
            });
        }
        Ok(planned)
    }

    /// PR-12: the drafts per pull request that will be posted, those left out
    /// and why, the drafted summary, and a single-use plan token.
    pub(super) async fn plan_submit_of(
        &self,
        caller: &Caller,
        request: pb::PlanSubmitRequest,
    ) -> Result<pb::SubmitPlan, Status> {
        let (record, binding) = self.submittable(caller, request.session.as_ref()).await?;
        let planned = self.prepare(&record.id, &binding).await?;
        let mut heads = BTreeMap::new();
        let mut per_pr = Vec::new();
        for planned in planned {
            heads.insert(planned.key(), planned.commit.clone());
            per_pr.push(pb::submit_plan::PerPullRequest {
                drafted_summary: self.drafted_summary_for(&record.id, &planned.reference).await,
                left_out: planned
                    .left_out
                    .into_iter()
                    .map(|(draft, reason)| pb::submit_plan::LeftOut {
                        draft: Some(draft),
                        reason: reason as i32,
                    })
                    .collect(),
                drafts: planned.drafts,
                pr: Some(planned.reference),
            });
        }
        Ok(pb::SubmitPlan {
            session: request.session,
            pull_requests: per_pr,
            token: Some(pb::PlanToken {
                value: self.inner.submits.make(&record.id, heads),
            }),
        })
    }

    /// PR-13, PR-14: one `POST pulls/<n>/reviews` per pull request, the
    /// attempt kept before the post, the drafts that were taken marked
    /// posted. A pull request GitHub refuses keeps its drafts and says why;
    /// the others still post.
    pub(super) async fn submit_review_of(
        &self,
        caller: &Caller,
        request: pb::SubmitReviewRequest,
    ) -> Result<pb::SubmitReviewResponse, Status> {
        let (record, binding) = self.submittable(caller, request.session.as_ref()).await?;
        let _running = self.inner.submits.running.lock().await;
        let token = request.plan.as_ref().map(|plan| plan.value.as_str()).unwrap_or_default();
        let planned_heads = self
            .inner
            .submits
            .take(token, &record.id)
            .ok_or_else(|| SubmitError::StalePlan.status())?;

        // Everything is checked before anything is posted.
        let planned = self.prepare(&record.id, &binding).await?;
        let mut chosen: Vec<(&PerPr, &pb::ReviewSubmission, Vec<pb::Comment>)> = Vec::new();
        for submission in &request.reviews {
            let reference = submission.pr.clone().unwrap_or_default();
            let shown = KeyOf(&reference).to_string();
            let Some(per) = planned.iter().find(|per| per.key() == shown) else {
                return Err(SubmitError::NoSuchPr { pr: shown }.status());
            };
            if chosen.iter().any(|(taken, _, _)| taken.key() == shown) {
                return Err(SubmitError::NamedTwice { pr: shown }.status());
            }
            if planned_heads.get(&shown) != Some(&per.commit) {
                return Err(SubmitError::HeadMoved {
                    pr: shown.clone(),
                    planned: planned_heads.get(&shown).cloned().unwrap_or_default(),
                    now: per.commit.clone(),
                }
                .status());
            }
            let mut taking = Vec::new();
            for wanted in &submission.drafts {
                let Some(draft) = per.drafts.iter().find(|draft| same_id(id_of(draft), &wanted.value)) else {
                    return Err(SubmitError::NotPostable {
                        id: wanted.value.clone(),
                        pr: shown,
                    }
                    .status());
                };
                if !taking.iter().any(|taken: &pb::Comment| id_of(taken) == id_of(draft)) {
                    taking.push(draft.clone());
                }
            }
            chosen.push((per, submission, taking));
        }

        let mut results = Vec::new();
        for (per, submission, drafts) in chosen {
            results.push(self.post_one(&record.id, per, submission, &drafts).await?);
        }
        Ok(pb::SubmitReviewResponse { results })
    }

    /// Posts one pull request's review. A failure of GitHub is the result's
    /// `error`; only a failure to keep the attempt's note, which stops the
    /// post before it is made, is the call's.
    async fn post_one(
        &self,
        session: &str,
        per: &PerPr,
        submission: &pb::ReviewSubmission,
        drafts: &[pb::Comment],
    ) -> Result<pb::submit_review_response::Result, Status> {
        let event = match pb::Verdict::try_from(submission.verdict) {
            Ok(pb::Verdict::Approve) => "APPROVE",
            Ok(pb::Verdict::RequestChanges) => "REQUEST_CHANGES",
            _ => "COMMENT",
        };
        let comments: Vec<Value> = drafts.iter().map(comment_json).collect();
        let body = json!({
            "commit_id": per.commit,
            "event": event,
            "body": submission.summary,
            "comments": comments,
        });
        let bytes = serde_json::to_vec(&body).unwrap_or_default();

        // Written down before the post, so a restart can look for it.
        let attempt = Attempt {
            commit_id: per.commit.clone(),
            body: submission.summary.clone(),
            started_unix_ms: now_ms(),
            drafts: drafts
                .iter()
                .map(|draft| (id_of(draft).to_owned(), path_of(draft), lines_of(draft).1, draft.text.clone()))
                .collect(),
        };
        let (store, owner, name, number) = (
            self.inner.store.clone(),
            per.pr.owner.clone(),
            per.pr.name.clone(),
            per.pr.number,
        );
        let (note, session_key, started) = (self.attempt_path(session, &per.pr), session.to_owned(), attempt.started_unix_ms);
        let pr_key = per.key();
        let noted = attempt.to_json().to_string().into_bytes();
        blocking(move || -> Result<(), SubmitError> {
            if let Some(folder) = note.parent() {
                std::fs::create_dir_all(folder).map_err(|source| SubmitError::Journal {
                    pr: pr_key.clone(),
                    source,
                })?;
            }
            humata2_store::write_atomic(&note, &noted).map_err(|source| SubmitError::JournalStore {
                pr: pr_key.clone(),
                source,
            })?;
            store
                .set_review_submit_started(&session_key, &owner, &name, number, Some(started))
                .map_err(|source| SubmitError::JournalStore { pr: pr_key, source })?;
            Ok(())
        })
        .await?
        .map_err(|error| error.status())?;

        let (owner, name) = (per.pr.owner.clone(), per.pr.name.clone());
        let posted = blocking(move || github::post_review(&owner, &name, number, &bytes)).await?;
        // A crash here is what reconcile_submits is for.
        fault::point("pr.posted");
        match posted {
            Ok(answer) => {
                let url = answer
                    .get("html_url")
                    .and_then(Value::as_str)
                    .map_or_else(|| format!("https://github.com/{}/{}/pull/{}", per.pr.owner, per.pr.name, number), str::to_owned);
                let ids: Vec<String> = drafts.iter().map(|draft| id_of(draft).to_owned()).collect();
                match self.inner.review.mark_pr_posted(session, &ids, &url).await {
                    Ok(()) => self.settle(session, &per.pr).await,
                    // GitHub has the review; the start stays so a restart
                    // finds it and marks the drafts.
                    Err(error) => say!(
                        "humatad2: posted the review of {} but couldn't mark its drafts posted: {}",
                        per.key(),
                        error.message()
                    ),
                }
                Ok(pb::submit_review_response::Result {
                    pr: Some(per.reference.clone()),
                    review_url: url,
                    error: None,
                })
            }
            Err(error) => {
                // GitHub answered with a refusal: nothing was posted. Any
                // other failure may have been after GitHub took it, so the
                // start stays for reconcile_submits.
                if matches!(error, github::GithubError::Failed { .. }) {
                    self.settle(session, &per.pr).await;
                }
                let said = github::status_of(&error, per.reference.repo.as_ref(), Some(number));
                let said = crate::ports::status_error_of(&said).unwrap_or_else(|| pb::Error {
                    message: chain(&error),
                    fix: "read the GitHub CLI's message, fix what it names, then submit again; the drafts are kept".to_owned(),
                    retry: None,
                    kind: None,
                });
                Ok(pb::submit_review_response::Result {
                    pr: Some(per.reference.clone()),
                    review_url: String::new(),
                    error: Some(said),
                })
            }
        }
    }

    fn attempt_path(&self, session: &str, pr: &ReviewPr) -> PathBuf {
        self.inner
            .store
            .root()
            .join("pr-submits")
            .join(format!("{session}-{}-{}-{}.json", pr.owner, pr.name, pr.number))
    }

    /// The attempt is over: its start and its note are gone.
    async fn settle(&self, session: &str, pr: &ReviewPr) {
        let (store, key, owner, name, number) = (
            self.inner.store.clone(),
            session.to_owned(),
            pr.owner.clone(),
            pr.name.clone(),
            pr.number,
        );
        let note = self.attempt_path(session, pr);
        let cleared = blocking(move || {
            let _ = std::fs::remove_file(&note);
            store.set_review_submit_started(&key, &owner, &name, number, None)
        })
        .await;
        match cleared {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => say!(
                "humatad2: couldn't clear the start of the review of {}/{}#{}: {}",
                pr.owner,
                pr.name,
                pr.number,
                chain(&error)
            ),
            Err(error) => say!("humatad2: couldn't clear a review's start: {}", error.message()),
        }
    }
}

fn comment_json(draft: &pb::Comment) -> Value {
    let (first, last) = lines_of(draft);
    let side = match side_of(draft) {
        Side::Old => "LEFT",
        Side::New => "RIGHT",
    };
    let mut comment = json!({
        "path": path_of(draft),
        "line": last,
        "side": side,
        "body": draft.text,
    });
    if first < last {
        comment["start_line"] = json!(first);
        comment["start_side"] = json!(side);
    }
    comment
}

// ---------------------------------------------------------------------------
// The agent's draft of the summary.

impl PullRequests {
    fn summary_path(&self, session: &str, pr: &pb::PullRequestRef) -> PathBuf {
        let repo = pr.repo.clone().unwrap_or_default();
        self.inner
            .store
            .root()
            .join("pr-submits")
            .join(format!("summary-{session}-{}-{}-{}.txt", repo.owner, repo.name, pr.number))
    }

    /// The agent's drafted summary for one pull request; empty when it has
    /// drafted none.
    async fn drafted_summary_for(&self, session: &str, pr: &pb::PullRequestRef) -> String {
        let path = self.summary_path(session, pr);
        blocking(move || std::fs::read_to_string(path).unwrap_or_default())
            .await
            .unwrap_or_default()
    }

    /// PR-12: the agent's draft of the review summary for one pull request.
    /// Drafting again replaces the earlier draft; nothing is posted.
    pub(super) async fn draft_summary_of(
        &self,
        caller: &Caller,
        request: pb::DraftSummaryRequest,
    ) -> Result<pb::DraftSummaryResponse, Status> {
        let asked = request.session.clone().or_else(|| match caller {
            Caller::Agent(agent) => Some(pb::SessionId {
                value: agent.session.id.to_string(),
            }),
            _ => None,
        });
        let id = asked
            .map(|session| session.value)
            .filter(|session| !session.is_empty())
            .ok_or_else(|| missing("session"))?;
        let machine = self.inner.machine.clone();
        let (store, key) = (self.inner.store.clone(), id.clone());
        let (record, binding) = blocking(move || -> Result<_, StoreError> {
            Ok((store.session(&key)?, store.review_session(&key)?))
        })
        .await?
        .map_err(|error| store_error(error, &machine, "look up the review"))?;
        let record = record.ok_or_else(|| {
            not_found(
                pb::EntityKind::Session,
                &id,
                format!("no session {id} is on this machine"),
                "list the sessions to see their ids",
            )
        })?;
        self.inner.guard.require(
            caller,
            &Call::Use {
                capability: Capability::ReviewDraft,
                target: Target::Session(session_ref(&record, &self.inner.machine)),
            },
        )?;
        let reference = request.pr.clone().unwrap_or_default();
        let shown = KeyOf(&reference).to_string();
        let named = binding
            .as_ref()
            .is_some_and(|binding| binding.prs.iter().any(|pr| key_of(pr) == shown));
        if !named {
            return Err(SubmitError::NoSuchPr { pr: shown }.status());
        }
        let path = self.summary_path(&record.id, &reference);
        let summary = request.summary.into_bytes();
        blocking(move || -> Result<(), SubmitError> {
            if let Some(folder) = path.parent() {
                std::fs::create_dir_all(folder).map_err(|source| SubmitError::Journal {
                    pr: shown.clone(),
                    source,
                })?;
            }
            humata2_store::write_atomic(&path, &summary).map_err(|source| SubmitError::JournalStore { pr: shown, source })
        })
        .await?
        .map_err(|error| error.status())?;
        Ok(pb::DraftSummaryResponse {
            delivery: pb::Delivery::Applied as i32,
            journal: None,
        })
    }
}

// ---------------------------------------------------------------------------
// After a crash between GitHub's answer and the drafts being marked.

/// The listed reviews that are the one the attempt posted: by the signed-in
/// user, at the attempt's head, with its body, submitted at or after it began.
fn matching_reviews<'a>(login: &str, reviews: &'a [Value], attempt: &Attempt) -> Vec<&'a Value> {
    let earliest = attempt.started_unix_ms / 1000 - CLOCK_SLACK_SECONDS;
    reviews
        .iter()
        .filter(|review| {
            review
                .pointer("/user/login")
                .and_then(Value::as_str)
                .is_some_and(|by| by.eq_ignore_ascii_case(login))
                && review.get("commit_id").and_then(Value::as_str) == Some(attempt.commit_id.as_str())
                && review.get("body").and_then(Value::as_str).unwrap_or_default() == attempt.body
                && review
                    .get("submitted_at")
                    .and_then(Value::as_str)
                    .and_then(unix_seconds)
                    .is_some_and(|at| at >= earliest)
        })
        .collect()
}

impl PullRequests {
    /// D287: for every pull request whose review was being posted when the
    /// daemon stopped, looks on GitHub for it. A review that is the one
    /// posted marks its drafts posted; none leaves the drafts as drafts,
    /// marked unconfirmed for the user to decide. Never posts. A GitHub that
    /// can't be reached leaves the attempt to the next start.
    pub async fn reconcile_submits(&self) {
        let store = self.inner.store.clone();
        let bindings = match blocking(move || store.review_sessions()).await {
            Ok(Ok(bindings)) => bindings,
            Ok(Err(error)) => {
                say!("humatad2: couldn't look for reviews being posted: {}", chain(&error));
                return;
            }
            Err(error) => {
                say!("humatad2: couldn't look for reviews being posted: {}", error.message());
                return;
            }
        };
        for binding in bindings {
            for pr in &binding.prs {
                if pr.submit_started_unix_ms.is_some() {
                    self.reconcile_one(&binding.session, pr).await;
                }
            }
        }
    }

    async fn reconcile_one(&self, session: &str, pr: &ReviewPr) {
        let name = key_of(pr);
        let note = self.attempt_path(session, pr);
        let attempt = blocking(move || {
            std::fs::read(note)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|value| Attempt::from_json(&value))
        })
        .await
        .ok()
        .flatten();
        let (owner, repo, number) = (pr.owner.clone(), pr.name.clone(), pr.number);
        let listed = blocking(move || -> Result<_, github::GithubError> {
            Ok((github::signed_in_login()?, github::list_reviews(&owner, &repo, number)?))
        })
        .await;
        let (login, reviews) = match listed {
            Ok(Ok(found)) => found,
            Ok(Err(error)) => {
                say!("humatad2: couldn't ask GitHub whether the review of {name} was posted: {}", chain(&error));
                return;
            }
            Err(error) => {
                say!("humatad2: couldn't ask GitHub whether the review of {name} was posted: {}", error.message());
                return;
            }
        };

        let mut posted: Vec<String> = Vec::new();
        let mut url = String::new();
        let mut unconfirmed: Vec<String> = Vec::new();
        match &attempt {
            Some(attempt) => {
                let found = matching_reviews(&login, &reviews, attempt);
                let review = (found.len() == 1).then(|| found[0]);
                match review {
                    Some(review) => {
                        let id = review.get("id").and_then(Value::as_u64).unwrap_or_default();
                        url = review.get("html_url").and_then(Value::as_str).unwrap_or_default().to_owned();
                        let (owner, repo) = (pr.owner.clone(), pr.name.clone());
                        let comments = match blocking(move || github::review_comments(&owner, &repo, number, id)).await {
                            Ok(Ok(comments)) => comments,
                            Ok(Err(error)) => {
                                say!("humatad2: couldn't read the review {id} of {name}: {}", chain(&error));
                                return;
                            }
                            Err(error) => {
                                say!("humatad2: couldn't read the review {id} of {name}: {}", error.message());
                                return;
                            }
                        };
                        for (draft, path, line, text) in &attempt.drafts {
                            let taken = comments.iter().any(|comment| {
                                comment.get("path").and_then(Value::as_str) == Some(path.as_str())
                                    && comment.get("body").and_then(Value::as_str) == Some(text.as_str())
                                    && comment
                                        .get("line")
                                        .and_then(Value::as_u64)
                                        .is_none_or(|at| at == u64::from(*line))
                            });
                            if taken { posted.push(draft.clone()) } else { unconfirmed.push(draft.clone()) }
                        }
                    }
                    None => unconfirmed = attempt.drafts.iter().map(|(id, ..)| id.clone()).collect(),
                }
            }
            // Nothing says which drafts were in it: every draft of the PR.
            None => match self.inner.review.pr_comments(session).await {
                Ok(comments) => {
                    unconfirmed = comments
                        .iter()
                        .filter(|comment| is_draft_for(comment, pr))
                        .map(|comment| id_of(comment).to_owned())
                        .collect();
                }
                Err(error) => {
                    say!("humatad2: couldn't read the drafts of {name}: {}", error.message());
                    return;
                }
            },
        }

        if !posted.is_empty()
            && let Err(error) = self.inner.review.mark_pr_posted(session, &posted, &url).await
        {
            say!("humatad2: couldn't mark the drafts of {name} posted: {}", error.message());
            return;
        }
        if !unconfirmed.is_empty()
            && let Err(error) = self.inner.review.mark_pr_unconfirmed(session, &unconfirmed).await
        {
            say!("humatad2: couldn't mark the drafts of {name} unconfirmed: {}", error.message());
            return;
        }
        self.settle(session, pr).await;
    }
}
