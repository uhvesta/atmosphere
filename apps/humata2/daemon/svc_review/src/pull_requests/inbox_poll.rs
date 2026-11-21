//! ATM-75, ATM-76 + owner addition: Review inbox polling with 15-minute default
//! cadence, app-focus and manual refresh triggers, ETag/watermark deduplication,
//! terminal PR retirement, direct-only review request filtering, and manually
//! watched remote PR URLs.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::feedback::{FeedbackError, PrFeedback, PrLifecycleState, pr_key};

/// Default polling cadence for the review inbox (15 minutes).
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Why a poll cycle was requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollTrigger {
    Scheduled,
    AppFocus,
    Manual,
}

/// Per-PR conditional polling and notification watermark state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrPollCursor {
    pub etag: Option<String>,
    pub last_fingerprint: Option<String>,
    pub seen_thread_ids: BTreeSet<String>,
    pub seen_review_ids: BTreeSet<String>,
    pub state: PrLifecycleState,
}

/// A manually watched remote pull request added by URL or `owner/repo#N`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WatchedPr {
    pub owner: String,
    pub name: String,
    pub number: u32,
    pub url: String,
}

impl WatchedPr {
    pub fn key(&self) -> String {
        pr_key(&self.owner, &self.name, self.number)
    }
}

/// One candidate from a `review-requested:@me` query.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRequestEntry {
    pub owner: String,
    pub name: String,
    pub number: u32,
    pub title: String,
    pub author: String,
    pub url: String,
    /// True when the signed-in user is directly requested as a reviewer.
    pub direct_requested: bool,
    /// Team slugs requested on this PR (e.g. `"platform"` or `"acme/platform"`).
    pub requested_teams: Vec<String>,
}

impl ReviewRequestEntry {
    pub fn key(&self) -> String {
        pr_key(&self.owner, &self.name, self.number)
    }
}

/// Result of a conditional fetch for one PR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PollFetchResult {
    /// HTTP 304 Not Modified (or unchanged ETag) — no work needed.
    NotModified,
    /// Updated feedback payload with optional new ETag.
    Updated {
        etag: Option<String>,
        feedback: PrFeedback,
    },
}

/// Category of an inbox notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InboxNotificationKind {
    NewFeedback,
    ReviewRequested,
    WatchedPrUpdate,
}

/// Structured notification produced by a poll cycle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxNotification {
    pub kind: InboxNotificationKind,
    pub pr_key: String,
    pub title: String,
    pub summary: String,
    pub url: String,
    pub actor: String,
    pub count: usize,
}

/// Errors produced by inbox polling operations.
#[derive(Debug, thiserror::Error)]
pub enum InboxPollError {
    #[error("{given:?} is not a GitHub pull request URL or owner/repo#number")]
    InvalidPrUrl { given: String },
    #[error("couldn't fetch pull request feedback")]
    Feedback(#[from] FeedbackError),
}

/// Persistent/inspectable state for [`ReviewInboxPoller`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxPollState {
    pub cursors: BTreeMap<String, PrPollCursor>,
    pub opted_in_teams: BTreeSet<String>,
    pub watched_prs: BTreeMap<String, WatchedPr>,
    pub submitted_prs: BTreeMap<String, WatchedPr>,
    pub seen_review_requests: BTreeSet<String>,
}

/// Coordinates polling across submitted PRs, direct review requests, and
/// manually watched remote PR URLs.
#[derive(Clone, Debug)]
pub struct ReviewInboxPoller {
    poll_interval: Duration,
    last_polled: Option<Instant>,
    state: InboxPollState,
}

impl Default for ReviewInboxPoller {
    fn default() -> Self {
        Self::new(DEFAULT_POLL_INTERVAL)
    }
}

impl ReviewInboxPoller {
    pub fn new(poll_interval: Duration) -> Self {
        Self {
            poll_interval,
            last_polled: None,
            state: InboxPollState::default(),
        }
    }

    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    pub fn state(&self) -> &InboxPollState {
        &self.state
    }

    /// Registers a PR submitted by the user so its feedback is polled.
    pub fn track_submitted_pr(&mut self, owner: &str, name: &str, number: u32) {
        let entry = WatchedPr {
            owner: owner.to_owned(),
            name: name.to_owned(),
            number,
            url: format!("https://github.com/{owner}/{name}/pull/{number}"),
        };
        self.state.submitted_prs.insert(entry.key(), entry);
    }

    /// Opts into team-wide review requests for `team_slug`.
    pub fn opt_in_team(&mut self, team_slug: impl Into<String>) {
        self.state
            .opted_in_teams
            .insert(team_slug.into().trim().to_ascii_lowercase());
    }

    /// Opts out of team-wide review requests for `team_slug`.
    pub fn opt_out_team(&mut self, team_slug: &str) -> bool {
        self.state
            .opted_in_teams
            .remove(&team_slug.trim().to_ascii_lowercase())
    }

    /// Parses a GitHub PR URL (`https://github.com/owner/repo/pull/N`) or
    /// `owner/repo#N` and adds it to the watched PR list.
    pub fn watch_pr_url(&mut self, url: &str) -> Result<WatchedPr, InboxPollError> {
        let watched = parse_pr_url(url)?;
        self.state.watched_prs.insert(watched.key(), watched.clone());
        Ok(watched)
    }

    /// Removes a manually watched PR.
    pub fn unwatch_pr(&mut self, owner: &str, name: &str, number: u32) -> bool {
        let key = pr_key(owner, name, number);
        self.state.watched_prs.remove(&key).is_some()
    }

    /// Lists all manually watched PRs.
    pub fn watched_prs(&self) -> Vec<WatchedPr> {
        self.state.watched_prs.values().cloned().collect()
    }

    /// Returns whether a poll should run at `now` for `trigger`.
    pub fn should_poll(&self, now: Instant, trigger: PollTrigger) -> bool {
        match trigger {
            PollTrigger::Manual | PollTrigger::AppFocus => true,
            PollTrigger::Scheduled => match self.last_polled {
                None => true,
                Some(last) => now.saturating_duration_since(last) >= self.poll_interval,
            },
        }
    }

    /// Returns all active (non-terminal) PR targets that should be polled on
    /// this cycle, combining submitted PRs and manually watched PRs.
    pub fn active_poll_targets(&self) -> Vec<WatchedPr> {
        let mut combined: BTreeMap<String, WatchedPr> = BTreeMap::new();
        for (key, pr) in &self.state.submitted_prs {
            if !self.is_terminal(key) {
                combined.insert(key.clone(), pr.clone());
            }
        }
        for (key, pr) in &self.state.watched_prs {
            if !self.is_terminal(key) {
                combined.entry(key.clone()).or_insert_with(|| pr.clone());
            }
        }
        combined.into_values().collect()
    }

    /// Returns the stored ETag for a PR, if any.
    pub fn etag_for(&self, pr_key: &str) -> Option<&str> {
        self.state
            .cursors
            .get(pr_key)
            .and_then(|cursor| cursor.etag.as_deref())
    }

    /// Filters `review-requested:@me` entries to direct requests only, unless
    /// one of the PR's requested teams is in `opted_in_teams`.
    pub fn filter_review_requests(&self, entries: &[ReviewRequestEntry]) -> Vec<ReviewRequestEntry> {
        entries
            .iter()
            .filter(|entry| {
                entry.direct_requested
                    || entry
                        .requested_teams
                        .iter()
                        .any(|team| self.state.opted_in_teams.contains(&team.to_ascii_lowercase()))
            })
            .cloned()
            .collect()
    }

    /// Runs a scheduled poll if the 15-minute staleness window has elapsed.
    pub fn poll_if_due<F>(
        &mut self,
        now: Instant,
        review_requests: &[ReviewRequestEntry],
        fetcher: F,
    ) -> Result<Option<Vec<InboxNotification>>, InboxPollError>
    where
        F: FnMut(&WatchedPr, Option<&str>) -> Result<PollFetchResult, FeedbackError>,
    {
        if !self.should_poll(now, PollTrigger::Scheduled) {
            return Ok(None);
        }
        self.run_poll(now, review_requests, fetcher).map(Some)
    }

    /// Runs a poll triggered by app focus.
    pub fn poll_on_focus<F>(
        &mut self,
        now: Instant,
        review_requests: &[ReviewRequestEntry],
        fetcher: F,
    ) -> Result<Vec<InboxNotification>, InboxPollError>
    where
        F: FnMut(&WatchedPr, Option<&str>) -> Result<PollFetchResult, FeedbackError>,
    {
        self.run_poll(now, review_requests, fetcher)
    }

    /// Runs a manual refresh poll, bypassing the staleness window.
    pub fn poll_manual<F>(
        &mut self,
        now: Instant,
        review_requests: &[ReviewRequestEntry],
        fetcher: F,
    ) -> Result<Vec<InboxNotification>, InboxPollError>
    where
        F: FnMut(&WatchedPr, Option<&str>) -> Result<PollFetchResult, FeedbackError>,
    {
        self.run_poll(now, review_requests, fetcher)
    }

    fn run_poll<F>(
        &mut self,
        now: Instant,
        review_requests: &[ReviewRequestEntry],
        mut fetcher: F,
    ) -> Result<Vec<InboxNotification>, InboxPollError>
    where
        F: FnMut(&WatchedPr, Option<&str>) -> Result<PollFetchResult, FeedbackError>,
    {
        self.last_polled = Some(now);
        let mut notifications = Vec::new();

        // 1. Poll active submitted and watched PRs.
        let targets = self.active_poll_targets();
        for target in targets {
            let key = target.key();
            let etag = self.etag_for(&key).map(str::to_owned);
            let is_watched_only = self.state.watched_prs.contains_key(&key)
                && !self.state.submitted_prs.contains_key(&key);
            match fetcher(&target, etag.as_deref())? {
                PollFetchResult::NotModified => {}
                PollFetchResult::Updated { etag, feedback } => {
                    if let Some(notification) =
                        self.ingest_feedback(&key, etag, &feedback, is_watched_only)
                    {
                        notifications.push(notification);
                    }
                }
            }
        }

        // 2. Process filtered review requests (direct-only unless team opted in).
        for req in self.filter_review_requests(review_requests) {
            let key = req.key();
            if self.state.seen_review_requests.insert(key.clone()) {
                notifications.push(InboxNotification {
                    kind: InboxNotificationKind::ReviewRequested,
                    pr_key: key.clone(),
                    title: req.title.clone(),
                    summary: format!(
                        "Review requested from you · by {} · {}/{}#{}",
                        req.author, req.owner, req.name, req.number
                    ),
                    url: req.url.clone(),
                    actor: req.author.clone(),
                    count: 1,
                });
            }
        }

        Ok(notifications)
    }

    fn is_terminal(&self, pr_key: &str) -> bool {
        self.state
            .cursors
            .get(pr_key)
            .is_some_and(|cursor| cursor.state.is_terminal())
    }

    fn ingest_feedback(
        &mut self,
        key: &str,
        etag: Option<String>,
        feedback: &PrFeedback,
        is_watched_only: bool,
    ) -> Option<InboxNotification> {
        let cursor = self.state.cursors.entry(key.to_owned()).or_default();
        if let Some(new_etag) = etag {
            cursor.etag = Some(new_etag);
        }
        cursor.state = feedback.state;

        let fingerprint = feedback_fingerprint(feedback);
        if cursor.last_fingerprint.as_deref() == Some(fingerprint.as_str()) {
            return None;
        }
        cursor.last_fingerprint = Some(fingerprint);

        let mut new_count = 0_usize;
        let mut latest_actor = String::new();

        for thread in &feedback.threads {
            if cursor.seen_thread_ids.insert(thread.thread_id.clone()) {
                new_count += 1;
                if !thread.reviewer.is_empty() {
                    latest_actor = thread.reviewer.clone();
                }
            }
        }
        for review in &feedback.reviews {
            if cursor.seen_review_ids.insert(review.review_id.clone()) {
                new_count += 1;
                if !review.reviewer.is_empty() {
                    latest_actor = review.reviewer.clone();
                }
            }
        }

        if new_count == 0 {
            return None;
        }

        if is_watched_only {
            let title = if feedback.title.is_empty() {
                feedback.pr_key.clone()
            } else {
                feedback.title.clone()
            };
            Some(InboxNotification {
                kind: InboxNotificationKind::WatchedPrUpdate,
                pr_key: feedback.pr_key.clone(),
                title: title.clone(),
                summary: format!(
                    "Watched PR · {title} · {}/{}#{}",
                    feedback.owner, feedback.name, feedback.number
                ),
                url: feedback.url.clone(),
                actor: latest_actor,
                count: new_count,
            })
        } else {
            let noun = if new_count == 1 {
                "comment"
            } else {
                "comments"
            };
            let actor = if latest_actor.is_empty() {
                "reviewer".to_owned()
            } else {
                latest_actor
            };
            Some(InboxNotification {
                kind: InboxNotificationKind::NewFeedback,
                pr_key: feedback.pr_key.clone(),
                title: feedback.title.clone(),
                summary: format!("New feedback · {new_count} {noun} from {actor}"),
                url: feedback.url.clone(),
                actor,
                count: new_count,
            })
        }
    }
}

fn feedback_fingerprint(feedback: &PrFeedback) -> String {
    let thread_ids: Vec<&str> = feedback
        .threads
        .iter()
        .map(|t| t.thread_id.as_str())
        .collect();
    let review_ids: Vec<&str> = feedback
        .reviews
        .iter()
        .map(|r| r.review_id.as_str())
        .collect();
    format!(
        "{:?}|{}|{}|{}",
        feedback.state,
        feedback.head_ref_oid,
        thread_ids.join(","),
        review_ids.join(",")
    )
}

/// Parses `https://github.com/owner/repo/pull/N` (with optional scheme, `www.`,
/// or trailing path/query/fragment) or `owner/repo#N`.
pub fn parse_pr_url(given: &str) -> Result<WatchedPr, InboxPollError> {
    let text = given.trim();
    if let Some(parsed) = parse_github_url(text).or_else(|| parse_short_ref(text)) {
        return Ok(parsed);
    }
    Err(InboxPollError::InvalidPrUrl {
        given: given.to_owned(),
    })
}

fn valid_slug_part(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn parse_github_url(text: &str) -> Option<WatchedPr> {
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .unwrap_or(text);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let mut parts = rest.strip_prefix("github.com/")?.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if parts.next()? != "pull" {
        return None;
    }
    let digits: String = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let number: u32 = digits.parse().ok().filter(|n| *n > 0)?;
    (valid_slug_part(owner) && valid_slug_part(name)).then(|| WatchedPr {
        owner: owner.to_owned(),
        name: name.to_owned(),
        number,
        url: format!("https://github.com/{owner}/{name}/pull/{number}"),
    })
}

fn parse_short_ref(text: &str) -> Option<WatchedPr> {
    let (repo, num_str) = text.split_once('#')?;
    let (owner, name) = repo.split_once('/')?;
    let number: u32 = num_str.parse().ok().filter(|n| *n > 0)?;
    (valid_slug_part(owner) && valid_slug_part(name)).then(|| WatchedPr {
        owner: owner.to_owned(),
        name: name.to_owned(),
        number,
        url: format!("https://github.com/{owner}/{name}/pull/{number}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pull_requests::feedback::{FeedbackThread, ThreadComment};

    fn sample_feedback(
        owner: &str,
        name: &str,
        number: u32,
        state: PrLifecycleState,
        thread_ids: &[&str],
    ) -> PrFeedback {
        let key = pr_key(owner, name, number);
        PrFeedback {
            pr_key: key.clone(),
            owner: owner.to_owned(),
            name: name.to_owned(),
            number,
            title: format!("PR #{number}"),
            state,
            merged: state == PrLifecycleState::Merged,
            url: format!("https://github.com/{owner}/{name}/pull/{number}"),
            head_ref_oid: "sha1".to_owned(),
            threads: thread_ids
                .iter()
                .map(|id| FeedbackThread {
                    pr_key: key.clone(),
                    thread_id: (*id).to_owned(),
                    is_resolved: false,
                    is_outdated: false,
                    path: "src/lib.rs".to_owned(),
                    line: Some(10),
                    original_line: Some(10),
                    reviewer: "alice".to_owned(),
                    body: "Nit: rename".to_owned(),
                    url: format!("https://github.com/{owner}/{name}/pull/{number}#discussion_{id}"),
                    created_at: "2026-11-20T12:00:00Z".to_owned(),
                    comments: vec![ThreadComment {
                        id: format!("C_{id}"),
                        database_id: Some(1),
                        author: "alice".to_owned(),
                        body: "Nit: rename".to_owned(),
                        url: format!(
                            "https://github.com/{owner}/{name}/pull/{number}#discussion_{id}"
                        ),
                        created_at: "2026-11-20T12:00:00Z".to_owned(),
                    }],
                })
                .collect(),
            reviews: Vec::new(),
            collapsed_bots: None,
        }
    }

    #[test]
    fn respects_15m_staleness_vs_focus_vs_manual_refresh() {
        let mut poller = ReviewInboxPoller::default();
        assert_eq!(poller.poll_interval(), Duration::from_secs(15 * 60));
        poller.track_submitted_pr("acme", "api", 10);

        let t0 = Instant::now();
        // First scheduled poll runs immediately.
        let first = poller
            .poll_if_due(t0, &[], |_, _| Ok(PollFetchResult::NotModified))
            .expect("first poll");
        assert!(first.is_some());

        // 5 minutes later: scheduled poll is skipped.
        let t5m = t0 + Duration::from_secs(5 * 60);
        let skipped = poller
            .poll_if_due(t5m, &[], |_, _| Ok(PollFetchResult::NotModified))
            .expect("skipped poll");
        assert!(skipped.is_none());

        // App focus runs immediately even inside the 15m window.
        let mut focus_calls = 0;
        poller
            .poll_on_focus(t5m, &[], |_, _| {
                focus_calls += 1;
                Ok(PollFetchResult::NotModified)
            })
            .expect("focus poll");
        assert_eq!(focus_calls, 1);

        // Manual refresh also bypasses the staleness window.
        let mut manual_calls = 0;
        poller
            .poll_manual(t5m + Duration::from_secs(10), &[], |_, _| {
                manual_calls += 1;
                Ok(PollFetchResult::NotModified)
            })
            .expect("manual poll");
        assert_eq!(manual_calls, 1);

        // 15 minutes after the last poll, scheduled poll runs again.
        let t20m = t5m + Duration::from_secs(10) + Duration::from_secs(15 * 60);
        let due = poller
            .poll_if_due(t20m, &[], |_, _| Ok(PollFetchResult::NotModified))
            .expect("due poll");
        assert!(due.is_some());
    }

    #[test]
    fn etag_and_seen_watermark_prevent_duplicate_notifications() {
        let mut poller = ReviewInboxPoller::default();
        poller.track_submitted_pr("acme", "api", 12);
        let t0 = Instant::now();

        // First poll returns 2 threads and an ETag.
        let first = poller
            .poll_manual(t0, &[], |_, etag| {
                assert!(etag.is_none());
                Ok(PollFetchResult::Updated {
                    etag: Some("\"etag-1\"".to_owned()),
                    feedback: sample_feedback("acme", "api", 12, PrLifecycleState::Open, &["T1", "T2"]),
                })
            })
            .expect("first poll");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].summary, "New feedback · 2 comments from alice");

        // Second poll receives ETag and returns 304 NotModified -> no notifications.
        let second = poller
            .poll_manual(t0 + Duration::from_secs(1), &[], |_, etag| {
                assert_eq!(etag, Some("\"etag-1\""));
                Ok(PollFetchResult::NotModified)
            })
            .expect("second poll");
        assert!(second.is_empty());

        // Third poll returns same threads with a new head SHA (fingerprint changed) -> still no re-notification.
        let third = poller
            .poll_manual(t0 + Duration::from_secs(2), &[], |_, _| {
                let mut fb = sample_feedback("acme", "api", 12, PrLifecycleState::Open, &["T1", "T2"]);
                fb.head_ref_oid = "sha2".to_owned();
                Ok(PollFetchResult::Updated {
                    etag: Some("\"etag-2\"".to_owned()),
                    feedback: fb,
                })
            })
            .expect("third poll");
        assert!(third.is_empty());
    }

    #[test]
    fn merged_and_closed_prs_stop_polling() {
        let mut poller = ReviewInboxPoller::default();
        poller.track_submitted_pr("acme", "api", 20);
        poller.track_submitted_pr("acme", "api", 21);
        let t0 = Instant::now();

        poller
            .poll_manual(t0, &[], |target, _| {
                let state = if target.number == 20 {
                    PrLifecycleState::Merged
                } else {
                    PrLifecycleState::Closed
                };
                Ok(PollFetchResult::Updated {
                    etag: None,
                    feedback: sample_feedback(&target.owner, &target.name, target.number, state, &[]),
                })
            })
            .expect("initial poll");

        assert!(poller.active_poll_targets().is_empty());
        let mut called = 0;
        poller
            .poll_manual(t0 + Duration::from_secs(1), &[], |_, _| {
                called += 1;
                Ok(PollFetchResult::NotModified)
            })
            .expect("subsequent poll");
        assert_eq!(called, 0);
    }

    #[test]
    fn team_review_requests_ignored_unless_opted_in() {
        let mut poller = ReviewInboxPoller::default();
        let requests = vec![
            ReviewRequestEntry {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
                number: 100,
                title: "Direct review".to_owned(),
                author: "carol".to_owned(),
                url: "https://github.com/acme/api/pull/100".to_owned(),
                direct_requested: true,
                requested_teams: vec![],
            },
            ReviewRequestEntry {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
                number: 101,
                title: "Team review".to_owned(),
                author: "dave".to_owned(),
                url: "https://github.com/acme/api/pull/101".to_owned(),
                direct_requested: false,
                requested_teams: vec!["platform".to_owned()],
            },
        ];

        let t0 = Instant::now();
        let first = poller
            .poll_manual(t0, &requests, |_, _| Ok(PollFetchResult::NotModified))
            .expect("poll");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].pr_key, "acme/api#100");
        assert_eq!(
            first[0].summary,
            "Review requested from you · by carol · acme/api#100"
        );

        // Opt into `platform` team -> now PR #101 notifies, while #100 does not re-notify.
        poller.opt_in_team("platform");
        let second = poller
            .poll_manual(t0 + Duration::from_secs(1), &requests, |_, _| {
                Ok(PollFetchResult::NotModified)
            })
            .expect("poll after opt-in");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].pr_key, "acme/api#101");
    }

    #[test]
    fn manually_watched_remote_pr_url_polls_on_same_cycle() {
        let mut poller = ReviewInboxPoller::default();
        let w1 = poller
            .watch_pr_url("https://github.com/other/repo/pull/55/files")
            .expect("watch full URL");
        let w2 = poller
            .watch_pr_url("other/lib#9")
            .expect("watch short ref");
        assert_eq!(w1.key(), "other/repo#55");
        assert_eq!(w2.key(), "other/lib#9");
        assert_eq!(poller.watched_prs().len(), 2);

        let t0 = Instant::now();
        let mut polled_keys = Vec::new();
        let notifications = poller
            .poll_manual(t0, &[], |target, _| {
                polled_keys.push(target.key());
                Ok(PollFetchResult::Updated {
                    etag: None,
                    feedback: sample_feedback(
                        &target.owner,
                        &target.name,
                        target.number,
                        PrLifecycleState::Open,
                        &["TW1"],
                    ),
                })
            })
            .expect("poll watched");

        assert_eq!(polled_keys, vec!["other/lib#9", "other/repo#55"]);
        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[0].kind, InboxNotificationKind::WatchedPrUpdate);
        assert_eq!(
            notifications[0].summary,
            "Watched PR · PR #9 · other/lib#9"
        );

        assert!(poller.unwatch_pr("other", "repo", 55));
        assert_eq!(poller.watched_prs().len(), 1);
    }
}
