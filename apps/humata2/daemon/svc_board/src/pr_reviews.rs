//! Pull requests that ask for the user's review become To-do items
//! (BOARDSIMPLE). The list is read from GitHub gently, grouped so a stack
//! is one item, and written through the same to-do paths a person's edits
//! take, so revisions, history and the event feed work as for any item.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use humata2_daemon_ports::{Caller, UserVia};
use humata2_rpc_kit::{blocking, internal, invalid_field, lock, now_ms, say};
use humata2_todo::State;
use serde::{Deserialize, Serialize};
use tonic::Status;

use crate::pb;
use crate::pb::invalid_field::Problem;
use crate::ports::{FoundPr, ReviewRequestsPort};
use crate::todo::{Entry, TodoApplied};
use crate::todo_fields::{self as saved, PrRow, ReviewRow};
use crate::BoardSvc;

/// How long a read of GitHub is reused before the next one.
const STALE_MS: i64 = 10 * 60 * 1000;

/// Store setting key for manually watched remote PR URLs.
const MANUAL_WATCHED_PRS_KEY: &str = "todo.manual_watched_prs";

/// A remote PR manually added to the Reviews inbox by URL or `owner/repo#N`
/// without auto-creating a To-do.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ManualWatchedPr {
    pub owner: String,
    pub name: String,
    pub number: u32,
    pub url: String,
    pub added_at_ms: i64,
}

/// Parses either `https://github.com/owner/repo/pull/123` (or `http://...`)
/// or `owner/repo#123` into `(owner, name, number, canonical_url)`.
fn parse_manual_pr_ref(input: &str) -> Option<(String, String, u32, String)> {
    let trimmed = input.trim();
    if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
    {
        let mut parts = rest.split('/');
        let owner = parts.next()?.trim();
        let name = parts.next()?.trim();
        let pull = parts.next()?.trim();
        let num_part = parts.next()?.split(['?', '#']).next()?.trim();
        if owner.is_empty() || name.is_empty() || pull != "pull" {
            return None;
        }
        let number: u32 = num_part.parse().ok()?;
        if number == 0 {
            return None;
        }
        let url = format!("https://github.com/{owner}/{name}/pull/{number}");
        return Some((owner.to_owned(), name.to_owned(), number, url));
    }
    let (repo, num_str) = trimmed.split_once('#')?;
    let (owner, name) = repo.split_once('/')?;
    let owner = owner.trim();
    let name = name.trim();
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    let number: u32 = num_str.trim().parse().ok()?;
    if number == 0 {
        return None;
    }
    let url = format!("https://github.com/{owner}/{name}/pull/{number}");
    Some((owner.to_owned(), name.to_owned(), number, url))
}

/// The pull requests of one repository that are reviewed together: one
/// pull request, or a stack of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReviewGroup {
    /// Bottom (closest to the default branch) to top.
    pub prs: Vec<FoundPr>,
    pub title: String,
    pub author: String,
    pub requested_unix_ms: i64,
}

impl ReviewGroup {
    fn row(&self) -> ReviewRow {
        ReviewRow {
            pull_requests: self
                .prs
                .iter()
                .map(|pr| PrRow {
                    owner: pr.owner.clone(),
                    name: pr.name.clone(),
                    number: pr.number,
                    note: pr.title.clone(),
                })
                .collect(),
            author: self.author.clone(),
            requested_unix_ms: self.requested_unix_ms,
        }
    }

    fn description(&self) -> String {
        self.prs
            .iter()
            .map(|pr| format!("{}/{}#{} {}\n{}", pr.owner, pr.name, pr.number, pr.title, pr.url))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// How many pull requests below `at` its base branch leads through.
fn depth(prs: &[&FoundPr], at: &FoundPr) -> usize {
    let mut depth = 0;
    let mut base = at.base_ref.as_str();
    while let Some(below) = prs.iter().find(|pr| pr.head_ref == base && pr.head_ref != pr.base_ref)
    {
        depth += 1;
        base = below.base_ref.as_str();
        if depth > prs.len() {
            break;
        }
    }
    depth
}

/// Groups `found` into stacks: pull requests of one repository where one's
/// base branch is another's head branch are reviewed together.
pub(crate) fn group_stacks(found: &[FoundPr]) -> Vec<ReviewGroup> {
    let mut repos: Vec<(&str, &str)> = found
        .iter()
        .map(|pr| (pr.owner.as_str(), pr.name.as_str()))
        .collect();
    repos.sort_unstable();
    repos.dedup();
    let mut groups = Vec::new();
    for (owner, name) in repos {
        let prs: Vec<&FoundPr> = found
            .iter()
            .filter(|pr| pr.owner == owner && pr.name == name)
            .collect();
        // Union the pull requests that sit on one another.
        let mut parent: Vec<usize> = (0..prs.len()).collect();
        fn root(parent: &[usize], mut at: usize) -> usize {
            while parent[at] != at {
                at = parent[at];
            }
            at
        }
        for (i, upper) in prs.iter().enumerate() {
            for (j, lower) in prs.iter().enumerate() {
                if i != j && upper.base_ref == lower.head_ref && lower.head_ref != lower.base_ref {
                    let (a, b) = (root(&parent, i), root(&parent, j));
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
        let mut roots: Vec<usize> = (0..prs.len()).map(|i| root(&parent, i)).collect();
        roots.sort_unstable();
        roots.dedup();
        for at in roots {
            let mut layers: Vec<&FoundPr> = prs
                .iter()
                .enumerate()
                .filter(|(i, _)| root(&parent, *i) == at)
                .map(|(_, pr)| *pr)
                .collect();
            layers.sort_by_key(|pr| (depth(&prs, pr), pr.number));
            let (Some(bottom), Some(top)) = (layers.first(), layers.last()) else {
                continue;
            };
            groups.push(ReviewGroup {
                title: format!("Review: {}", bottom.title),
                author: top.author.clone(),
                requested_unix_ms: layers
                    .iter()
                    .map(|pr| pr.requested_unix_ms)
                    .min()
                    .unwrap_or(0),
                prs: layers.into_iter().cloned().collect(),
            });
        }
    }
    groups.sort_by_key(|group| (group.requested_unix_ms, group.prs[0].number));
    groups
}

/// What one sync of the review requests did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReviewSync {
    pub created: usize,
    pub updated: usize,
    pub closed: usize,
    pub reopened: usize,
}

fn identity(row: &ReviewRow) -> BTreeSet<(String, String, u32)> {
    row.pull_requests
        .iter()
        .map(|pr| (pr.owner.clone(), pr.name.clone(), pr.number))
        .collect()
}

fn group_identity(group: &ReviewGroup) -> BTreeSet<(String, String, u32)> {
    identity(&group.row())
}

/// The item each group is kept by: the same pull requests first, then one
/// that shares a pull request (a stack that gained or lost a layer).
fn match_items(groups: &[ReviewGroup], items: &[&Entry]) -> Vec<Option<usize>> {
    let ids: Vec<BTreeSet<(String, String, u32)>> = items
        .iter()
        .map(|entry| entry.fields.review.as_ref().map(identity).unwrap_or_default())
        .collect();
    let mut taken = vec![false; items.len()];
    let mut matched = vec![None; groups.len()];
    for exact in [true, false] {
        for (g, group) in groups.iter().enumerate() {
            if matched[g].is_some() {
                continue;
            }
            let wanted = group_identity(group);
            let found = (0..items.len()).find(|&i| {
                !taken[i]
                    && if exact {
                        ids[i] == wanted
                    } else {
                        ids[i].intersection(&wanted).next().is_some()
                    }
            });
            if let Some(i) = found {
                taken[i] = true;
                matched[g] = Some(i);
            }
        }
    }
    matched
}

impl BoardSvc {
    /// Makes the list's review items match `found`: one item per stack, new
    /// ones added, ones whose request is gone closed, ones that came back
    /// reopened. Pin, Hide until and every other item are left as they are.
    pub(crate) fn sync_review_requests(
        &self,
        found: &[FoundPr],
        now: i64,
    ) -> Result<ReviewSync, Status> {
        let user = Caller::User(UserVia::App);
        let groups = group_stacks(found);
        let entries = self.entries(&user)?;
        let mut items: Vec<&Entry> = entries
            .iter()
            .filter(|entry| entry.fields.review.is_some())
            .collect();
        // An open item is the one a group's pull requests belong to.
        items.sort_by_key(|entry| entry.state() == State::Closed);
        let matched = match_items(&groups, &items);
        let mut done = ReviewSync::default();
        let mut pending: Vec<(String, ReviewRow, bool)> = Vec::new();

        let outcome = (|| -> Result<(), Status> {
            for (group, at) in groups.iter().zip(&matched) {
                match at.map(|i| items[i]) {
                    None => {
                        let key = self.add_review_item(&user, group, now)?;
                        pending.push((key, group.row(), false));
                        done.created += 1;
                    }
                    Some(entry) => {
                        let reopen = entry.state() == State::Closed;
                        let row = group.row();
                        let new_row = entry.fields.review.as_ref() != Some(&row);
                        let retitle = entry.record.title != group.title;
                        if !(reopen || new_row || retitle) {
                            continue;
                        }
                        let key = entry.record.key.clone();
                        if reopen || retitle {
                            let patch = pb::TodoPatch {
                                title: retitle.then(|| group.title.clone()),
                                state: reopen.then_some(pb::TodoState::Open as i32),
                                ..Default::default()
                            };
                            self.patch_review_item(&user, &key, patch, now)?;
                        }
                        if new_row {
                            pending.push((key, row, true));
                        }
                        if reopen {
                            done.reopened += 1;
                        } else {
                            done.updated += 1;
                        }
                    }
                }
            }

            for (i, entry) in items.iter().enumerate() {
                if matched.contains(&Some(i)) || entry.state() == State::Closed {
                    continue;
                }
                let patch = pb::TodoPatch {
                    state: Some(pb::TodoState::Closed as i32),
                    ..Default::default()
                };
                self.patch_review_item(&user, &entry.record.key, patch, now)?;
                done.closed += 1;
            }
            Ok(())
        })();

        match outcome {
            Ok(()) => {
                self.flush_reviews(&user, pending)?;
                Ok(done)
            }
            Err(error) => {
                // Flush any review rows already created/updated before returning the error.
                let _ = self.flush_reviews(&user, pending);
                Err(error)
            }
        }
    }

    fn add_review_item(
        &self,
        user: &Caller,
        group: &ReviewGroup,
        now: i64,
    ) -> Result<String, Status> {
        let request = pb::CreateTodoRequest {
            title: group.title.clone(),
            description: group.description(),
            ..Default::default()
        };
        let TodoApplied::Done(done) = self.create_todo_in(user, &request, false, now)? else {
            return Err(internal(
                "a review request item was sent to Ask me, which the user never is",
                "report this",
            ));
        };
        done.item
            .and_then(|todo| todo.item)
            .and_then(|item| item.id)
            .map(|id| id.value)
            .ok_or_else(|| internal("the new review item came back with no id", "report this"))
    }

    fn patch_review_item(
        &self,
        user: &Caller,
        key: &str,
        patch: pb::TodoPatch,
        now: i64,
    ) -> Result<(), Status> {
        let request = pb::UpdateTodoRequest {
            id: Some(pb::BoardItemId {
                value: key.to_owned(),
            }),
            patch: Some(patch),
            ..Default::default()
        };
        self.update_todo_in(user, &request, false, now).map(|_| ())
    }

    /// Saves all pending review rows in a single `todo_fields` JSON write
    /// under `self.writes`; for entries with `changed == true`, bumps their
    /// revision and tells the feed after the lock is released.
    fn flush_reviews(
        &self,
        user: &Caller,
        pending: Vec<(String, ReviewRow, bool)>,
    ) -> Result<(), Status> {
        if pending.is_empty() {
            return Ok(());
        }
        let mut notify = Vec::new();
        {
            let _write = lock(&self.writes);
            let mut map = saved::load(&self.store)?;
            for (key, row, changed) in pending {
                let fields = map.entry(key.clone()).or_default();
                fields.review = Some(row);
                if changed {
                    fields.extra_revisions += 1;
                    notify.push(key);
                }
            }
            saved::save(&self.store, &map)?;
        }
        if !notify.is_empty() {
            let archived = self.archived()?;
            for key in notify {
                let record = self.load(user, &key, false)?;
                self.tell(user, pb::Change::Updated, &record, &archived);
            }
        }
        Ok(())
    }

    /// Reads GitHub again when the last read is older than ten minutes (or
    /// there was none), in the background: one read at a time, and the
    /// caller never waits for it.
    pub(crate) fn refresh_review_requests_if_stale(&self, now: i64) {
        let Some(feed) = self.review_requests.clone() else {
            return;
        };
        {
            let mut state = lock(&feed.state);
            if state.running || state.last_ms.is_some_and(|last| now - last < STALE_MS) {
                return;
            }
            state.running = true;
        }
        let me = self.clone();
        tokio::spawn(async move {
            me.refresh_review_requests(&feed, now).await;
        });
    }

    async fn refresh_review_requests(&self, feed: &Arc<ReviewFeed>, started: i64) {
        match feed.port.fetch().await {
            Ok(found) => {
                let me = self.clone();
                match blocking(move || me.sync_review_requests(&found, now_ms())).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(status)) => say!(
                        "humatad2: couldn't keep the review requests in the To-do list: {}",
                        status.message()
                    ),
                    Err(status) => say!(
                        "humatad2: reading the review requests stopped: {}",
                        status.message()
                    ),
                }
            }
            // The items stay as they are; the next read is in ten minutes.
            Err(error) => say!("humatad2: {error}"),
        }
        let mut state = lock(&feed.state);
        state.last_ms = Some(started);
        state.running = false;
    }

    /// Manually watches a remote PR URL (`https://github.com/owner/repo/pull/N`
    /// or `owner/repo#N`) for the Reviews inbox without auto-creating a To-do.
    pub fn watch_remote_pr_url(
        &self,
        pr_url_or_ref: &str,
        now: i64,
    ) -> Result<ManualWatchedPr, Status> {
        let Some((owner, name, number, url)) = parse_manual_pr_ref(pr_url_or_ref) else {
            return Err(invalid_field(
                "pr_url",
                Problem::BadFormat,
                format!("'{pr_url_or_ref}' is not a valid GitHub PR URL or owner/repo#N reference"),
                "use https://github.com/owner/repo/pull/123 or owner/repo#123",
            ));
        };
        let _write = lock(&self.writes);
        let (mut list, rev) = self.load_manual_watched_prs_locked()?;
        if let Some(existing) = list
            .iter()
            .find(|pr| pr.owner == owner && pr.name == name && pr.number == number)
        {
            return Ok(existing.clone());
        }
        let entry = ManualWatchedPr {
            owner,
            name,
            number,
            url,
            added_at_ms: now,
        };
        list.push(entry.clone());
        self.save_manual_watched_prs_locked(&list, rev + 1)?;
        Ok(entry)
    }

    /// Stops manually watching a remote PR. Returns `true` if it was present.
    pub fn unwatch_remote_pr(
        &self,
        owner: &str,
        name: &str,
        number: u32,
    ) -> Result<bool, Status> {
        let _write = lock(&self.writes);
        let (mut list, rev) = self.load_manual_watched_prs_locked()?;
        let before = list.len();
        list.retain(|pr| !(pr.owner == owner && pr.name == name && pr.number == number));
        let removed = list.len() != before;
        if removed {
            self.save_manual_watched_prs_locked(&list, rev + 1)?;
        }
        Ok(removed)
    }

    /// Returns all manually watched remote PRs for the Reviews inbox.
    pub fn manual_watched_prs(&self) -> Result<Vec<ManualWatchedPr>, Status> {
        let _write = lock(&self.writes);
        let (list, _) = self.load_manual_watched_prs_locked()?;
        Ok(list)
    }

    fn load_manual_watched_prs_locked(&self) -> Result<(Vec<ManualWatchedPr>, u64), Status> {
        let saved = self
            .store
            .setting(MANUAL_WATCHED_PRS_KEY)
            .map_err(|e| internal(format!("couldn't read manual watched PRs: {e}"), ""))?;
        match saved {
            Some(row) => {
                let list: Vec<ManualWatchedPr> = serde_json::from_str(&row.value).map_err(|e| {
                    internal(format!("saved manual watched PRs are unreadable: {e}"), "")
                })?;
                Ok((list, row.revision))
            }
            None => Ok((Vec::new(), 0)),
        }
    }

    fn save_manual_watched_prs_locked(
        &self,
        list: &[ManualWatchedPr],
        revision: u64,
    ) -> Result<(), Status> {
        let encoded = serde_json::to_string(list)
            .map_err(|e| internal(format!("couldn't encode manual watched PRs: {e}"), ""))?;
        self.store
            .put_setting(MANUAL_WATCHED_PRS_KEY, &encoded, revision)
            .map_err(|e| internal(format!("couldn't save manual watched PRs: {e}"), ""))
    }
}

/// Where review requests are read from, and when they last were.
pub(crate) struct ReviewFeed {
    port: Arc<dyn ReviewRequestsPort>,
    state: Mutex<FeedState>,
}

#[derive(Default)]
struct FeedState {
    last_ms: Option<i64>,
    running: bool,
}

impl ReviewFeed {
    pub(crate) fn new(port: Arc<dyn ReviewRequestsPort>) -> Self {
        Self {
            port,
            state: Mutex::new(FeedState::default()),
        }
    }

    #[cfg(test)]
    fn idle(&self) -> bool {
        !lock(&self.state).running
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::ports::ReviewFetchError;
    use crate::test_support::test_svc;

    fn pr(name: &str, number: u32, head: &str, base: &str, asked: i64) -> FoundPr {
        FoundPr {
            owner: "acme".to_owned(),
            name: name.to_owned(),
            number,
            title: format!("title {number}"),
            author: format!("author-{number}"),
            url: format!("https://example.test/{name}/{number}"),
            head_ref: head.to_owned(),
            base_ref: base.to_owned(),
            requested_unix_ms: asked,
        }
    }

    fn user() -> Caller {
        Caller::User(UserVia::App)
    }

    fn review_entries(svc: &BoardSvc) -> Vec<Entry> {
        svc.entries(&user())
            .unwrap()
            .into_iter()
            .filter(|entry| entry.fields.review.is_some())
            .collect()
    }

    fn open_count(svc: &BoardSvc) -> usize {
        review_entries(svc)
            .iter()
            .filter(|entry| entry.state() != State::Closed)
            .count()
    }

    #[test]
    fn a_stack_is_one_group_bottom_first_with_the_top_authors_name() {
        // Listed out of order, with one lone pull request in the same repo.
        let found = [
            pr("api", 2, "b", "a", 300),
            pr("api", 1, "a", "main", 200),
            pr("api", 7, "solo", "main", 500),
        ];
        let groups = group_stacks(&found);
        assert_eq!(groups.len(), 2);
        let stack = &groups[0];
        assert_eq!(stack.prs.iter().map(|p| p.number).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(stack.author, "author-2");
        assert_eq!(stack.requested_unix_ms, 200);
        assert_eq!(stack.title, "Review: title 1");
        assert_eq!(groups[1].prs.len(), 1);
    }

    #[test]
    fn pull_requests_of_different_repos_never_group() {
        let found = [pr("api", 1, "a", "main", 1), pr("web", 2, "b", "a", 1)];
        assert_eq!(group_stacks(&found).len(), 2);
    }

    #[test]
    fn a_new_request_adds_one_item_and_a_stack_of_two_is_one_item() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let found = [
            pr("api", 1, "a", "main", 100),
            pr("api", 2, "b", "a", 100),
            pr("web", 5, "w", "main", 100),
        ];
        let done = svc.sync_review_requests(&found, 1000).unwrap();
        assert_eq!(done.created, 2);
        let items = review_entries(&svc);
        assert_eq!(items.len(), 2);
        let stack = items
            .iter()
            .find(|entry| entry.fields.review.as_ref().unwrap().pull_requests.len() == 2)
            .unwrap();
        assert_eq!(stack.fields.review.as_ref().unwrap().author, "author-2");
    }

    #[test]
    fn a_stack_that_gains_a_layer_keeps_its_item() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        svc.sync_review_requests(&[pr("api", 1, "a", "main", 100)], 1000)
            .unwrap();
        let before = review_entries(&svc)[0].record.key.clone();
        let done = svc
            .sync_review_requests(
                &[pr("api", 1, "a", "main", 100), pr("api", 2, "b", "a", 150)],
                2000,
            )
            .unwrap();
        assert_eq!((done.created, done.updated), (0, 1));
        let items = review_entries(&svc);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].record.key, before);
        assert_eq!(items[0].fields.review.as_ref().unwrap().pull_requests.len(), 2);
    }

    #[test]
    fn a_request_that_is_gone_closes_its_item_and_a_new_request_reopens_it() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let one = [pr("api", 1, "a", "main", 100)];
        svc.sync_review_requests(&one, 1000).unwrap();
        let done = svc.sync_review_requests(&[], 2000).unwrap();
        assert_eq!(done.closed, 1);
        assert_eq!(open_count(&svc), 0);
        // Closed items stay closed while the request is gone.
        assert_eq!(svc.sync_review_requests(&[], 3000).unwrap(), ReviewSync::default());
        let done = svc.sync_review_requests(&one, 4000).unwrap();
        assert_eq!((done.created, done.reopened), (0, 1));
        assert_eq!(open_count(&svc), 1);
        assert_eq!(review_entries(&svc).len(), 1);
    }

    #[test]
    fn a_pin_and_a_hide_until_survive_a_sync() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        svc.sync_review_requests(&[pr("api", 1, "a", "main", 100)], 1000)
            .unwrap();
        let key = review_entries(&svc)[0].record.key.clone();
        let mut map = saved::load(&svc.store).unwrap();
        let fields = map.get_mut(&key).unwrap();
        fields.pin_order = 3;
        fields.hide_until = humata2_todo::Day::from_ymd(2030, 1, 1);
        saved::save(&svc.store, &map).unwrap();
        // The author changes and a layer arrives.
        let mut changed = pr("api", 1, "a", "main", 100);
        changed.author = "someone-else".to_owned();
        svc.sync_review_requests(&[changed, pr("api", 2, "b", "a", 120)], 2000)
            .unwrap();
        let item = &review_entries(&svc)[0];
        assert_eq!(item.fields.pin_order, 3);
        assert!(item.fields.hide_until.is_some());
        assert_eq!(item.fields.review.as_ref().unwrap().pull_requests.len(), 2);
    }

    #[test]
    fn other_items_are_never_touched() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let request = pb::CreateTodoRequest {
            title: "Buy milk".to_owned(),
            ..Default::default()
        };
        svc.create_todo_in(&user(), &request, false, 1).unwrap();
        svc.sync_review_requests(&[], 2000).unwrap();
        let all = svc.entries(&user()).unwrap();
        assert_eq!(all.len(), 1);
        assert_ne!(all[0].state(), State::Closed);
    }

    #[test]
    fn the_same_input_twice_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let found = [pr("api", 1, "a", "main", 100), pr("api", 2, "b", "a", 100)];
        svc.sync_review_requests(&found, 1000).unwrap();
        let revisions = |svc: &BoardSvc| -> Vec<(String, u64)> {
            review_entries(svc)
                .iter()
                .map(|entry| (entry.record.key.clone(), entry.revision()))
                .collect()
        };
        let before = revisions(&svc);
        let done = svc.sync_review_requests(&found, 5000).unwrap();
        assert_eq!(done, ReviewSync::default());
        assert_eq!(revisions(&svc), before);
    }

    struct Fake {
        calls: AtomicUsize,
        reply: Mutex<Option<Vec<FoundPr>>>,
    }

    #[tonic::async_trait]
    impl ReviewRequestsPort for Fake {
        async fn fetch(&self) -> Result<Vec<FoundPr>, ReviewFetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            lock(&self.reply)
                .clone()
                .ok_or(ReviewFetchError::RateLimited { wait_seconds: None })
        }
    }

    async fn settle(svc: &BoardSvc) {
        let feed = svc.review_requests.clone().unwrap();
        for _ in 0..100_000 {
            if feed.idle() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the review refresh never finished");
    }

    #[tokio::test]
    async fn two_lists_within_ten_minutes_read_github_once() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake {
            calls: AtomicUsize::new(0),
            reply: Mutex::new(Some(vec![pr("api", 1, "a", "main", 100)])),
        });
        let svc = test_svc(&dir).with_review_requests(fake.clone());
        svc.refresh_review_requests_if_stale(1_000);
        settle(&svc).await;
        svc.refresh_review_requests_if_stale(1_000 + 5 * 60_000);
        settle(&svc).await;
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
        assert_eq!(review_entries(&svc).len(), 1);
        svc.refresh_review_requests_if_stale(1_000 + 11 * 60_000);
        settle(&svc).await;
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failed_read_closes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake {
            calls: AtomicUsize::new(0),
            reply: Mutex::new(None),
        });
        let svc = test_svc(&dir).with_review_requests(fake.clone());
        svc.sync_review_requests(&[pr("api", 1, "a", "main", 100)], 500)
            .unwrap();
        svc.refresh_review_requests_if_stale(1_000);
        settle(&svc).await;
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
        assert_eq!(open_count(&svc), 1);
    }

    #[test]
    fn manual_watched_pr_url_tracks_without_creating_a_todo() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);

        let added = svc
            .watch_remote_pr_url("https://github.com/acme/api/pull/412", 1_000)
            .unwrap();
        assert_eq!(added.owner, "acme");
        assert_eq!(added.name, "api");
        assert_eq!(added.number, 412);
        assert_eq!(added.url, "https://github.com/acme/api/pull/412");

        // Adding by short ref is idempotent.
        let dup = svc.watch_remote_pr_url("acme/api#412", 2_000).unwrap();
        assert_eq!(dup, added);
        assert_eq!(svc.manual_watched_prs().unwrap().len(), 1);

        // No To-do was auto-created.
        assert!(svc.entries(&user()).unwrap().is_empty());

        // Invalid input is rejected.
        assert!(svc.watch_remote_pr_url("not-a-pr", 3_000).is_err());

        // Unwatch removes it.
        assert!(svc.unwatch_remote_pr("acme", "api", 412).unwrap());
        assert!(!svc.unwatch_remote_pr("acme", "api", 412).unwrap());
        assert!(svc.manual_watched_prs().unwrap().is_empty());
    }
}
