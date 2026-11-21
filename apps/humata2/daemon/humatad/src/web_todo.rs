//! Host daemon adapter for the loopback To-do web UI server (WEBTODO, D905).
//!
//! Bridges `humata2_store::Store` and `crate::events::Feed` into
//! `humata2_web_todo_server::BoardSource` with pure push notifications (never
//! polls).

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_grpc::humata::v1 as pb;
use humata2_store::{BoardItemRecord, Store};
use humata2_todo::{
    CheckpointState, Day, Effort, FuzzyDue, Impact, Need, OwnerKind, State, Step, Todo, TodoOwner,
    WatchedPr,
};
use humata2_web_todo_server::{Assets, BoardSource, WebTodoController, WebTodoStatus};
use humata2_web_todo_wire::{
    BoardSnapshot, ChannelInfo, LayerStatus, ReviewDetail, ReviewInboxItem, ReviewKind,
    ReviewLayer, SessionInfo, WatchedPrEntry,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::events::Feed;

const FIELDS_KEY: &str = "todo.fields";
const MANUAL_WATCHED_PRS_KEY: &str = "todo.manual_watched_prs";
const DAY_SECS: i64 = 86_400;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct SavedFieldsDoc {
    items: BTreeMap<String, SavedItemFields>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
enum SavedLocalReviewState {
    #[default]
    Draft,
    LocalReviewRequested,
    ChangesRequested,
    Submitted,
    Approved,
    Done,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct SavedLocalReviewLayer {
    repo: String,
    base_branch: String,
    head_branch: String,
    title: String,
    pr_number: Option<u32>,
    rebased_diff_unchanged: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct SavedLocalReview {
    state: SavedLocalReviewState,
    layers: Vec<SavedLocalReviewLayer>,
    summary_what: String,
    summary_why: String,
    summary_details: String,
    owner_approved_push: bool,
    fixes_pushed_since_review: bool,
    attachment_reason: Option<String>,
    auto_attached: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct SavedPrRow {
    owner: String,
    name: String,
    number: u32,
    note: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct SavedReviewRow {
    pull_requests: Vec<SavedPrRow>,
    author: String,
    requested_unix_ms: i64,
}

/// Manually watched remote PR stored under [`MANUAL_WATCHED_PRS_KEY`]
/// (matches `humata2_svc_board::ManualWatchedPr` JSON schema).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct ManualWatchedPr {
    owner: String,
    name: String,
    number: u32,
    url: String,
    added_at_ms: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct SavedItemFields {
    state: Option<State>,
    state_column: Option<i32>,
    due: Option<Day>,
    effort: Option<Effort>,
    impact: Option<Impact>,
    need: Option<Need>,
    fuzzy_due: Option<FuzzyDue>,
    checkpoint: Option<CheckpointState>,
    tags: Vec<String>,
    steps: Vec<SavedStep>,
    hide_until: Option<Day>,
    pinned: bool,
    owner: Option<SavedOwner>,
    watched_pr: Option<SavedWatchedPr>,
    watched_prs: Vec<SavedWatchedPr>,
    review: Option<SavedReviewRow>,
    local_review: Option<SavedLocalReview>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct SavedStep {
    id: String,
    text: String,
    done: bool,
    dropped: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct SavedOwner {
    kind: OwnerKind,
    #[serde(default)]
    label: String,
    #[serde(default)]
    session: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct SavedWatchedPr {
    owner: String,
    name: String,
    number: u32,
    title: String,
    ci_state: String,
    unresolved_comments: u32,
    has_conflicts: bool,
    upstream_merged: bool,
    merged: bool,
    closed_without_merge: bool,
    stack_key: String,
}

impl SavedWatchedPr {
    fn repo_pr(&self) -> String {
        if self.owner.is_empty() && self.name.is_empty() {
            format!("#{}", self.number)
        } else {
            format!("{}/{}#{}", self.owner, self.name, self.number)
        }
    }

    fn repo_slug(&self) -> String {
        if self.owner.is_empty() {
            self.name.clone()
        } else if self.name.is_empty() {
            self.owner.clone()
        } else {
            format!("{}/{}", self.owner, self.name)
        }
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(0))
        .unwrap_or(0)
}

fn today_utc() -> Day {
    let secs = i64::try_from(now_unix_secs()).unwrap_or(0);
    day_of_secs(secs).unwrap_or_else(|| Day::from_ymd(2026, 1, 1).expect("epoch fallback"))
}

fn day_of_secs(secs: i64) -> Option<Day> {
    let z = secs.div_euclid(DAY_SECS) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Day::from_ymd(
        i32::try_from(y).ok()?,
        u32::try_from(m).ok()?,
        u32::try_from(d).ok()?,
    )
}

fn state_from_column(column: i32) -> State {
    match column {
        3 => State::InProgress,
        5 => State::Closed,
        _ => State::Open,
    }
}

/// Parses either `https://github.com/<owner>/<repo>/pull/<number>` (or `http://...`)
/// or `<owner>/<repo>#<number>` into `(owner, repo, number, canonical_url)`.
pub(crate) fn parse_github_pr_url(input: &str) -> Result<(String, String, u32, String), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("PR URL cannot be empty".to_owned());
    }
    if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
    {
        let mut parts = rest.split('/');
        let owner = parts.next().unwrap_or("").trim();
        let name = parts.next().unwrap_or("").trim();
        let pull = parts.next().unwrap_or("").trim();
        let num_part = parts
            .next()
            .unwrap_or("")
            .split(['?', '#'])
            .next()
            .unwrap_or("")
            .trim();
        if owner.is_empty() || name.is_empty() || pull != "pull" {
            return Err(format!(
                "invalid GitHub PR URL '{trimmed}': expected https://github.com/<owner>/<repo>/pull/<number>"
            ));
        }
        let number: u32 = num_part.parse().map_err(|_| {
            format!("invalid PR number '{num_part}' in '{trimmed}'")
        })?;
        if number == 0 {
            return Err(format!("PR number must be greater than 0 in '{trimmed}'"));
        }
        let url = format!("https://github.com/{owner}/{name}/pull/{number}");
        return Ok((owner.to_owned(), name.to_owned(), number, url));
    }
    let Some((repo, num_str)) = trimmed.split_once('#') else {
        return Err(format!(
            "invalid GitHub PR reference '{trimmed}': use https://github.com/<owner>/<repo>/pull/<number> or <owner>/<repo>#<number>"
        ));
    };
    let Some((owner, name)) = repo.split_once('/') else {
        return Err(format!(
            "invalid repository '{repo}' in '{trimmed}': expected <owner>/<repo>#<number>"
        ));
    };
    let owner = owner.trim();
    let name = name.trim();
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return Err(format!(
            "invalid repository '{repo}' in '{trimmed}': expected <owner>/<repo>#<number>"
        ));
    }
    let number: u32 = num_str.trim().parse().map_err(|_| {
        format!("invalid PR number '{}' in '{trimmed}'", num_str.trim())
    })?;
    if number == 0 {
        return Err(format!("PR number must be greater than 0 in '{trimmed}'"));
    }
    let url = format!("https://github.com/{owner}/{name}/pull/{number}");
    Ok((owner.to_owned(), name.to_owned(), number, url))
}

fn load_saved_fields(store: &Store) -> BTreeMap<String, SavedItemFields> {
    let Some(saved) = store.setting(FIELDS_KEY).ok().flatten() else {
        return BTreeMap::new();
    };
    if let Ok(doc) = serde_json::from_str::<SavedFieldsDoc>(&saved.value)
        && !doc.items.is_empty()
    {
        return doc.items;
    }
    serde_json::from_str::<BTreeMap<String, SavedItemFields>>(&saved.value).unwrap_or_default()
}

fn load_manual_watched_prs(store: &Store) -> (Vec<ManualWatchedPr>, u64) {
    match store.setting(MANUAL_WATCHED_PRS_KEY).ok().flatten() {
        Some(saved) => {
            let list = serde_json::from_str::<Vec<ManualWatchedPr>>(&saved.value).unwrap_or_default();
            (list, saved.revision)
        }
        None => (Vec::new(), 0),
    }
}

fn age_label_from_ms(ts_ms: i64, now_ms: i64) -> String {
    if ts_ms <= 0 || now_ms <= ts_ms {
        return "just now".to_owned();
    }
    let secs = (now_ms - ts_ms) / 1000;
    if secs < 60 {
        "just now".to_owned()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

fn todo_from_record(rec: &BoardItemRecord, extra: Option<&SavedItemFields>, today: Day) -> Todo {
    let created = day_of_secs(rec.created_unix_ms / 1000).unwrap_or(today);
    let mut todo = Todo::new(&rec.key, &rec.title, created);
    todo.description = rec.description.clone();
    todo.channel = (!rec.channel.is_empty()).then(|| rec.channel.clone());
    todo.sessions = rec.sessions.clone();
    todo.depends_on = rec.depends_on.clone();
    todo.closed = rec.done_unix_ms.and_then(|ms| day_of_secs(ms / 1000));
    todo.state = state_from_column(rec.column);

    if let Some(fields) = extra {
        if let Some(saved_state) = fields.state
            && fields.state_column.is_none_or(|c| c == rec.column)
        {
            todo.state = saved_state;
        }
        todo.due = fields.due;
        todo.effort = fields.effort;
        todo.impact = fields.impact;
        todo.need = fields.need;
        todo.fuzzy_due = fields.fuzzy_due;
        todo.checkpoint = fields.checkpoint;
        todo.tags = fields.tags.clone();
        todo.hide_until = fields.hide_until;
        todo.steps = fields
            .steps
            .iter()
            .map(|s| Step {
                id: s.id.clone(),
                text: s.text.clone(),
                done: s.done,
                dropped: s.dropped,
                ..Step::default()
            })
            .collect();
        if let Some(owner) = &fields.owner {
            todo.owner = Some(TodoOwner {
                kind: owner.kind,
                label: owner.label.clone(),
                session: owner.session.clone(),
            });
        }
        for pr in fields.watched_pr.iter().chain(fields.watched_prs.iter()) {
            todo.watched_prs.push(WatchedPr {
                repo_pr: pr.repo_pr(),
                title: pr.title.clone(),
                ci_state: pr.ci_state.clone(),
                unresolved_comments: pr.unresolved_comments,
                has_conflicts: pr.has_conflicts,
                upstream_merged: pr.upstream_merged,
                merged: pr.merged,
                closed_without_merge: pr.closed_without_merge,
                stack_key: (!pr.stack_key.is_empty()).then(|| pr.stack_key.clone()),
            });
        }
    }
    todo
}

fn build_local_review(
    rec: &BoardItemRecord,
    lr: &SavedLocalReview,
    channel_name: Option<String>,
    agent: Option<String>,
    now_ms: i64,
) -> (ReviewInboxItem, ReviewDetail) {
    let layer_count = u32::try_from(lr.layers.len().max(1)).unwrap_or(1);
    let layers_summary = (layer_count > 1).then(|| format!("{layer_count} layers"));
    let mut repos: Vec<String> = lr
        .layers
        .iter()
        .map(|l| l.repo.clone())
        .filter(|r| !r.is_empty())
        .collect();
    repos.sort();
    repos.dedup();
    let pr_refs: Vec<String> = lr
        .layers
        .iter()
        .filter_map(|l| {
            l.pr_number.map(|n| {
                if l.repo.is_empty() {
                    format!("#{n}")
                } else {
                    format!("{}#{n}", l.repo)
                }
            })
        })
        .collect();

    let (chip_label, chip_tone) = if lr.fixes_pushed_since_review {
        ("Fixes pushed", "amber")
    } else {
        match lr.state {
            SavedLocalReviewState::ChangesRequested => ("Changes requested", "red"),
            SavedLocalReviewState::Approved => ("Approved", "green"),
            SavedLocalReviewState::Submitted => ("Submitted", "blue"),
            SavedLocalReviewState::LocalReviewRequested => ("Ready to push", "blue"),
            SavedLocalReviewState::Draft | SavedLocalReviewState::Done => ("Draft", "neutral"),
        }
    };

    let layer_status = if lr.fixes_pushed_since_review {
        LayerStatus::FixesPushed
    } else if matches!(
        lr.state,
        SavedLocalReviewState::Approved | SavedLocalReviewState::Submitted
    ) {
        LayerStatus::LookedAt
    } else {
        LayerStatus::NotLookedAt
    };

    let layers: Vec<ReviewLayer> = if lr.layers.is_empty() {
        vec![ReviewLayer {
            number: 1,
            title: rec.title.clone(),
            branch: String::new(),
            base_branch: "main".to_owned(),
            repos: repos.clone(),
            status: layer_status,
            rebased_diff_unchanged: false,
            files: Vec::new(),
        }]
    } else {
        lr.layers
            .iter()
            .enumerate()
            .map(|(idx, layer)| ReviewLayer {
                number: u32::try_from(idx + 1).unwrap_or(1),
                title: if layer.title.is_empty() {
                    format!("Layer {}", idx + 1)
                } else {
                    layer.title.clone()
                },
                branch: layer.head_branch.clone(),
                base_branch: layer.base_branch.clone(),
                repos: (!layer.repo.is_empty())
                    .then(|| vec![layer.repo.clone()])
                    .unwrap_or_default(),
                status: layer_status.clone(),
                rebased_diff_unchanged: layer.rebased_diff_unchanged,
                files: Vec::new(),
            })
            .collect()
    };

    let mut subtitle_parts = Vec::new();
    if let Some(ch) = &channel_name {
        subtitle_parts.push(ch.clone());
    }
    if let Some(ls) = &layers_summary {
        subtitle_parts.push(ls.clone());
    }

    let inbox = ReviewInboxItem {
        id: rec.key.clone(),
        kind: ReviewKind::Local,
        title: rec.title.clone(),
        channel: channel_name,
        agent,
        layers_summary,
        repos,
        pr_refs,
        subtitle: subtitle_parts.join(" · "),
        chip_label: chip_label.to_owned(),
        chip_tone: chip_tone.to_owned(),
        age_label: age_label_from_ms(rec.updated_unix_ms, now_ms),
        added: 0,
        removed: 0,
        layer_count,
    };

    let detail = ReviewDetail {
        id: rec.key.clone(),
        title: rec.title.clone(),
        is_local: true,
        layers,
        summary_what: lr.summary_what.clone(),
        summary_why: lr.summary_why.clone(),
        summary_details: lr.summary_details.clone(),
        pr_count: layer_count,
        open_notes: 0,
    };

    (inbox, detail)
}

fn build_review_request(
    rec: &BoardItemRecord,
    row: &SavedReviewRow,
    channel_name: Option<String>,
    now_ms: i64,
) -> (ReviewInboxItem, ReviewDetail) {
    let layer_count = u32::try_from(row.pull_requests.len().max(1)).unwrap_or(1);
    let layers_summary = (layer_count > 1).then(|| format!("{layer_count} layers"));
    let mut repos: Vec<String> = row
        .pull_requests
        .iter()
        .map(|pr| format!("{}/{}", pr.owner, pr.name))
        .filter(|r| r != "/")
        .collect();
    repos.sort();
    repos.dedup();
    let pr_refs: Vec<String> = row
        .pull_requests
        .iter()
        .map(|pr| format!("{}/{}#{}", pr.owner, pr.name, pr.number))
        .collect();

    let layers: Vec<ReviewLayer> = row
        .pull_requests
        .iter()
        .enumerate()
        .map(|(idx, pr)| ReviewLayer {
            number: u32::try_from(idx + 1).unwrap_or(1),
            title: if pr.note.is_empty() {
                format!("{}/{}#{}", pr.owner, pr.name, pr.number)
            } else {
                pr.note.clone()
            },
            branch: String::new(),
            base_branch: String::new(),
            repos: vec![format!("{}/{}", pr.owner, pr.name)],
            status: LayerStatus::NotLookedAt,
            rebased_diff_unchanged: false,
            files: Vec::new(),
        })
        .collect();

    let ts = if row.requested_unix_ms > 0 {
        row.requested_unix_ms
    } else {
        rec.updated_unix_ms
    };

    let inbox = ReviewInboxItem {
        id: rec.key.clone(),
        kind: ReviewKind::ReviewRequested,
        title: rec.title.clone(),
        channel: channel_name,
        agent: None,
        layers_summary,
        repos,
        pr_refs,
        subtitle: row.author.clone(),
        chip_label: "Review requested".to_owned(),
        chip_tone: "blue".to_owned(),
        age_label: age_label_from_ms(ts, now_ms),
        added: 0,
        removed: 0,
        layer_count,
    };

    let detail = ReviewDetail {
        id: rec.key.clone(),
        title: rec.title.clone(),
        is_local: false,
        layers,
        summary_what: rec.description.clone(),
        summary_why: String::new(),
        summary_details: String::new(),
        pr_count: layer_count,
        open_notes: 0,
    };

    (inbox, detail)
}

fn build_github_feedback(
    rec: &BoardItemRecord,
    prs: &[&SavedWatchedPr],
    channel_name: Option<String>,
    now_ms: i64,
) -> (ReviewInboxItem, ReviewDetail) {
    let layer_count = u32::try_from(prs.len().max(1)).unwrap_or(1);
    let layers_summary = (layer_count > 1).then(|| format!("{layer_count} layers"));
    let mut repos: Vec<String> = prs
        .iter()
        .map(|pr| pr.repo_slug())
        .filter(|r| !r.is_empty())
        .collect();
    repos.sort();
    repos.dedup();
    let pr_refs: Vec<String> = prs.iter().map(|pr| pr.repo_pr()).collect();

    let total_comments: u32 = prs.iter().map(|pr| pr.unresolved_comments).sum();
    let has_conflicts = prs.iter().any(|pr| pr.has_conflicts);
    let (chip_label, chip_tone) = if has_conflicts {
        ("Conflicts".to_owned(), "red".to_owned())
    } else if total_comments > 0 {
        (
            format!(
                "{total_comments} comment{}",
                if total_comments == 1 { "" } else { "s" }
            ),
            "amber".to_owned(),
        )
    } else {
        ("Open PR".to_owned(), "blue".to_owned())
    };

    let layers: Vec<ReviewLayer> = prs
        .iter()
        .enumerate()
        .map(|(idx, pr)| ReviewLayer {
            number: u32::try_from(idx + 1).unwrap_or(1),
            title: if pr.title.is_empty() {
                pr.repo_pr()
            } else {
                pr.title.clone()
            },
            branch: String::new(),
            base_branch: String::new(),
            repos: (!pr.repo_slug().is_empty())
                .then(|| vec![pr.repo_slug()])
                .unwrap_or_default(),
            status: if pr.unresolved_comments > 0 || pr.has_conflicts {
                LayerStatus::NotLookedAt
            } else {
                LayerStatus::LookedAt
            },
            rebased_diff_unchanged: false,
            files: Vec::new(),
        })
        .collect();

    let inbox = ReviewInboxItem {
        id: rec.key.clone(),
        kind: ReviewKind::GithubFeedback,
        title: rec.title.clone(),
        channel: channel_name,
        agent: None,
        layers_summary,
        repos,
        pr_refs,
        subtitle: String::new(),
        chip_label,
        chip_tone,
        age_label: age_label_from_ms(rec.updated_unix_ms, now_ms),
        added: 0,
        removed: 0,
        layer_count,
    };

    let detail = ReviewDetail {
        id: rec.key.clone(),
        title: rec.title.clone(),
        is_local: false,
        layers,
        summary_what: rec.description.clone(),
        summary_why: String::new(),
        summary_details: String::new(),
        pr_count: layer_count,
        open_notes: total_comments,
    };

    (inbox, detail)
}

/// [`BoardSource`] backed by `humatad2`'s `Store` and push `Feed`.
pub(crate) struct DaemonBoardSource {
    store: Arc<Store>,
    updates: broadcast::Sender<()>,
    revision: AtomicU64,
    last_reviews_refresh: AtomicU64,
}

impl DaemonBoardSource {
    pub(crate) fn new(store: Arc<Store>, feed: &Arc<Feed>) -> Arc<Self> {
        let (updates, _) = broadcast::channel(64);
        let source = Arc::new(Self {
            store,
            updates,
            revision: AtomicU64::new(1),
            last_reviews_refresh: AtomicU64::new(0),
        });
        let mut rx = feed.subscribe_events();
        let weak = Arc::downgrade(&source);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let relevant = matches!(
                            event.payload,
                            Some(
                                pb::event::Payload::BoardItem(_)
                                    | pb::event::Payload::Channel(_)
                                    | pb::event::Payload::Session(_)
                                    | pb::event::Payload::Resync(_)
                            )
                        );
                        if relevant {
                            let Some(src) = weak.upgrade() else {
                                break;
                            };
                            src.bump_and_notify();
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let Some(src) = weak.upgrade() else {
                            break;
                        };
                        src.bump_and_notify();
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        source
    }

    fn bump_and_notify(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
        let _ = self.updates.send(());
    }
}

impl BoardSource for DaemonBoardSource {
    fn snapshot(&self) -> BoardSnapshot {
        let today = today_utc();
        let now_ms = now_unix_ms();
        let revision = self.revision.load(Ordering::Relaxed);

        let fields_map = load_saved_fields(&self.store);

        let channels: Vec<ChannelInfo> = self
            .store
            .channels()
            .unwrap_or_default()
            .into_iter()
            .map(|ch| ChannelInfo {
                id: ch.id,
                name: ch.name,
            })
            .collect();
        let channel_names: BTreeMap<&str, &str> = channels
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str()))
            .collect();

        let sessions: Vec<SessionInfo> = self
            .store
            .sessions()
            .unwrap_or_default()
            .into_iter()
            .map(|s| SessionInfo {
                id: s.id,
                name: s.title,
                dot: 0x4ade80,
            })
            .collect();

        let records: Vec<BoardItemRecord> = self
            .store
            .all_board_items_but_sessions()
            .unwrap_or_default()
            .into_iter()
            .filter(|rec| !rec.deleted)
            .collect();

        let mut items = Vec::with_capacity(records.len());
        let mut reviews = Vec::new();
        let mut review_details = Vec::new();

        for rec in &records {
            let extra = fields_map.get(&rec.key);
            items.push(todo_from_record(rec, extra, today));

            let Some(fields) = extra else {
                continue;
            };
            let channel_name = (!rec.channel.is_empty()).then(|| {
                channel_names
                    .get(rec.channel.as_str())
                    .copied()
                    .unwrap_or(rec.channel.as_str())
                    .to_owned()
            });
            let agent = fields
                .owner
                .as_ref()
                .filter(|o| o.kind == OwnerKind::Agent && !o.label.is_empty())
                .map(|o| o.label.clone());

            if let Some(lr) = &fields.local_review
                && lr.state != SavedLocalReviewState::Done
            {
                let (inbox, detail) =
                    build_local_review(rec, lr, channel_name.clone(), agent, now_ms);
                reviews.push(inbox);
                review_details.push(detail);
                continue;
            }

            if let Some(review_row) = &fields.review
                && !review_row.pull_requests.is_empty()
            {
                let (inbox, detail) = build_review_request(rec, review_row, channel_name, now_ms);
                reviews.push(inbox);
                review_details.push(detail);
                continue;
            }

            let open_prs: Vec<&SavedWatchedPr> = fields
                .watched_pr
                .iter()
                .chain(fields.watched_prs.iter())
                .filter(|pr| !pr.merged && !pr.closed_without_merge)
                .collect();
            if !open_prs.is_empty() {
                let (inbox, detail) = build_github_feedback(rec, &open_prs, channel_name, now_ms);
                reviews.push(inbox);
                review_details.push(detail);
            }
        }

        let (manual_prs, _) = load_manual_watched_prs(&self.store);
        let mut watched_prs = Vec::with_capacity(manual_prs.len());
        for pr in manual_prs {
            let repo = format!("{}/{}", pr.owner, pr.name);
            let pr_ref = format!("{repo}#{}", pr.number);
            let id = format!("watched:{pr_ref}");
            watched_prs.push(WatchedPrEntry {
                url: pr.url.clone(),
                repo: repo.clone(),
                number: u64::from(pr.number),
                title: pr_ref.clone(),
            });
            reviews.push(ReviewInboxItem {
                id: id.clone(),
                kind: ReviewKind::Watched,
                title: pr_ref.clone(),
                channel: None,
                agent: None,
                layers_summary: None,
                repos: vec![repo.clone()],
                pr_refs: vec![pr_ref.clone()],
                subtitle: pr.url.clone(),
                chip_label: "Watching".to_owned(),
                chip_tone: "neutral".to_owned(),
                age_label: age_label_from_ms(pr.added_at_ms, now_ms),
                added: 0,
                removed: 0,
                layer_count: 1,
            });
            review_details.push(ReviewDetail {
                id,
                title: pr_ref.clone(),
                is_local: false,
                layers: vec![ReviewLayer {
                    number: 1,
                    title: pr_ref,
                    branch: String::new(),
                    base_branch: String::new(),
                    repos: vec![repo],
                    status: LayerStatus::NotLookedAt,
                    rebased_diff_unchanged: false,
                    files: Vec::new(),
                }],
                summary_what: pr.url,
                summary_why: String::new(),
                summary_details: String::new(),
                pr_count: 1,
                open_notes: 0,
            });
        }

        let last_refresh = self.last_reviews_refresh.load(Ordering::Relaxed);
        let github_checked_ago_secs =
            (last_refresh > 0).then(|| now_unix_secs().saturating_sub(last_refresh));

        BoardSnapshot {
            today,
            items,
            channels,
            sessions,
            revision,
            reviews,
            watched_prs,
            github_checked_ago_secs,
            review_details,
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<()> {
        self.updates.subscribe()
    }

    fn watch_pr_url(&self, url: &str) -> Result<(), String> {
        let (owner, name, number, canonical_url) = parse_github_pr_url(url)?;
        let (mut list, rev) = load_manual_watched_prs(&self.store);
        let already_present = list
            .iter()
            .any(|pr| pr.owner == owner && pr.name == name && pr.number == number);
        if !already_present {
            list.push(ManualWatchedPr {
                owner,
                name,
                number,
                url: canonical_url,
                added_at_ms: now_unix_ms(),
            });
            let encoded = serde_json::to_string(&list)
                .map_err(|e| format!("failed to encode watched PRs: {e}"))?;
            self.store
                .put_setting(MANUAL_WATCHED_PRS_KEY, &encoded, rev + 1)
                .map_err(|e| format!("failed to save watched PR: {e}"))?;
        }
        self.bump_and_notify();
        Ok(())
    }

    fn refresh_reviews(&self) {
        self.last_reviews_refresh
            .store(now_unix_secs(), Ordering::Relaxed);
        self.bump_and_notify();
    }
}

/// Loads the GPUI Wasm To-do bundle from `HUMATA2_WEB_TODO_BUNDLE` or next to
/// the `humatad2` executable, falling back to a minimal placeholder when the
/// bundle is not installed alongside the binary.
pub(crate) fn load_assets() -> Assets {
    if let Some(dir) = std::env::var_os("HUMATA2_WEB_TODO_BUNDLE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        && let Ok(assets) = Assets::from_dir(&dir)
    {
        return assets;
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        for candidate in [parent.join("web_todo_bundle"), parent.join("web_todo")] {
            if let Ok(assets) = Assets::from_dir(&candidate) {
                return assets;
            }
        }
    }
    Assets::fallback_placeholder()
}

/// Loopback bind address for `WebTodoController`. Honors `HUMATA2_WEB_TODO_BIND`
/// (e.g. `127.0.0.1:0` in tests) when it parses as a loopback `SocketAddr`.
pub(crate) fn bind_addr() -> SocketAddr {
    if let Ok(raw) = std::env::var("HUMATA2_WEB_TODO_BIND")
        && let Ok(addr) = raw.parse::<SocketAddr>()
        && addr.ip().is_loopback()
    {
        return addr;
    }
    WebTodoController::default_addr()
}

/// Reads whether the To-do web UI is saved as enabled in `config/settings.toml`.
pub(crate) fn saved_enabled(state: &Path) -> bool {
    crate::settings_service::web_todo_enabled_in(state)
}

/// Converts [`WebTodoStatus`] to the protobuf `WebTodoStatus` message.
pub(crate) fn status_to_wire(status: WebTodoStatus) -> pb::WebTodoStatus {
    use pb::web_todo_status::State;
    match status {
        WebTodoStatus::Off => pb::WebTodoStatus {
            state: State::Off as i32,
            url: String::new(),
            reason: String::new(),
        },
        WebTodoStatus::Starting => pb::WebTodoStatus {
            state: State::Starting as i32,
            url: String::new(),
            reason: String::new(),
        },
        WebTodoStatus::Listening { url } => pb::WebTodoStatus {
            state: State::Listening as i32,
            url,
            reason: String::new(),
        },
        WebTodoStatus::Failed { reason } => pb::WebTodoStatus {
            state: State::Failed as i32,
            url: String::new(),
            reason,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_store::{BoardKey, NewBoardItem};

    fn open_temp_store() -> (PathBuf, Arc<Store>) {
        let dir = std::env::temp_dir().join(format!(
            "humatad_web_todo_test_{}",
            uuid::Uuid::new_v4()
        ));
        let store = Arc::new(Store::open(&dir).expect("open temp store"));
        (dir, store)
    }

    #[test]
    fn parse_github_pr_url_valid_and_invalid() {
        assert_eq!(
            parse_github_pr_url("https://github.com/acme/payments-api/pull/412"),
            Ok((
                "acme".to_owned(),
                "payments-api".to_owned(),
                412,
                "https://github.com/acme/payments-api/pull/412".to_owned(),
            ))
        );
        assert_eq!(
            parse_github_pr_url("  https://github.com/acme/repo/pull/7/files?w=1#diff  "),
            Ok((
                "acme".to_owned(),
                "repo".to_owned(),
                7,
                "https://github.com/acme/repo/pull/7".to_owned(),
            ))
        );
        assert_eq!(
            parse_github_pr_url("acme/repo#99"),
            Ok((
                "acme".to_owned(),
                "repo".to_owned(),
                99,
                "https://github.com/acme/repo/pull/99".to_owned(),
            ))
        );

        for invalid in [
            "",
            "   ",
            "https://github.com/acme/repo/issues/12",
            "https://github.com/acme/repo/pull/0",
            "https://github.com/acme/repo/pull/abc",
            "https://gitlab.com/acme/repo/merge_requests/5",
            "acme#12",
            "acme/repo/extra#12",
            "acme/repo#0",
            "not-a-url",
        ] {
            assert!(
                parse_github_pr_url(invalid).is_err(),
                "expected invalid for {invalid:?}"
            );
        }
    }

    #[tokio::test]
    async fn snapshot_populates_reviews_watched_prs_and_details() {
        let (dir, store) = open_temp_store();
        let feed = Arc::new(Feed::default());
        let source = DaemonBoardSource::new(Arc::clone(&store), &feed);

        store
            .claim_board_prefix("global", "HUM")
            .expect("claim global prefix");
        let created = store
            .create_board_item(&NewBoardItem {
                board: BoardKey::Global,
                channel: String::new(),
                title: "Idempotent webhook delivery".to_owned(),
                description: "Adds idempotency keys".to_owned(),
                column: 3,
                rank: "a".to_owned(),
                created_by: String::new(),
                at_unix_ms: now_unix_ms(),
                sessions: Vec::new(),
                depends_on: Vec::new(),
            })
            .expect("create board item");
        let item_key = created.key;

        let fields_json = serde_json::json!({
            &item_key: {
                "local_review": {
                    "state": "LocalReviewRequested",
                    "fixes_pushed_since_review": true,
                    "summary_what": "Idempotent webhook ingestion",
                    "summary_why": "Prevents duplicate charges",
                    "summary_details": "2 layers",
                    "layers": [
                        {
                            "repo": "payments-api",
                            "base_branch": "main",
                            "head_branch": "pay-3/schema",
                            "title": "schema",
                            "rebased_diff_unchanged": true
                        },
                        {
                            "repo": "payments-api",
                            "base_branch": "pay-3/schema",
                            "head_branch": "pay-3/dual-write",
                            "title": "dual write",
                            "rebased_diff_unchanged": false
                        }
                    ]
                }
            }
        });
        store
            .put_setting(FIELDS_KEY, &fields_json.to_string(), 1)
            .expect("save todo.fields");

        let watched_json = serde_json::json!([
            {
                "owner": "acme",
                "name": "payments-api",
                "number": 412,
                "url": "https://github.com/acme/payments-api/pull/412",
                "added_at_ms": now_unix_ms()
            }
        ]);
        store
            .put_setting(MANUAL_WATCHED_PRS_KEY, &watched_json.to_string(), 1)
            .expect("save manual_watched_prs");

        let snap = source.snapshot();
        assert_eq!(snap.github_checked_ago_secs, None);
        assert_eq!(snap.watched_prs.len(), 1);
        assert_eq!(snap.watched_prs[0].repo, "acme/payments-api");
        assert_eq!(snap.watched_prs[0].number, 412);

        assert_eq!(snap.reviews.len(), 2);
        let local = snap
            .reviews
            .iter()
            .find(|r| r.id == item_key)
            .expect("local review row");
        assert_eq!(local.kind, ReviewKind::Local);
        assert_eq!(local.chip_label, "Fixes pushed");
        assert_eq!(local.layer_count, 2);
        assert_eq!(local.layers_summary.as_deref(), Some("2 layers"));

        let detail = snap
            .review_details
            .iter()
            .find(|d| d.id == item_key)
            .expect("local review detail");
        assert!(detail.is_local);
        assert_eq!(detail.layers.len(), 2);
        assert_eq!(detail.layers[0].status, LayerStatus::FixesPushed);
        assert!(detail.layers[0].rebased_diff_unchanged);
        assert_eq!(detail.summary_what, "Idempotent webhook ingestion");

        let watched = snap
            .reviews
            .iter()
            .find(|r| r.kind == ReviewKind::Watched)
            .expect("watched review row");
        assert_eq!(watched.title, "acme/payments-api#412");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn watch_pr_url_persists_deduplicates_rejects_invalid_and_notifies() {
        let (dir, store) = open_temp_store();
        let feed = Arc::new(Feed::default());
        let source = DaemonBoardSource::new(Arc::clone(&store), &feed);
        let mut rx = source.subscribe();

        assert!(source.watch_pr_url("not-a-valid-url").is_err());

        let rev_before = source.snapshot().revision;
        source
            .watch_pr_url("https://github.com/acme/repo/pull/42")
            .expect("watch valid URL");
        assert!(rx.try_recv().is_ok(), "expected broadcast notification");

        let snap = source.snapshot();
        assert!(snap.revision > rev_before);
        assert_eq!(snap.watched_prs.len(), 1);
        assert_eq!(snap.watched_prs[0].number, 42);

        // Deduplicates when added again via shorthand.
        source
            .watch_pr_url("acme/repo#42")
            .expect("deduplicated watch");
        assert!(rx.try_recv().is_ok());
        assert_eq!(source.snapshot().watched_prs.len(), 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn refresh_reviews_updates_timestamp_bumps_revision_and_notifies() {
        let (dir, store) = open_temp_store();
        let feed = Arc::new(Feed::default());
        let source = DaemonBoardSource::new(Arc::clone(&store), &feed);
        let mut rx = source.subscribe();

        let snap_before = source.snapshot();
        assert_eq!(snap_before.github_checked_ago_secs, None);

        source.refresh_reviews();
        assert!(rx.try_recv().is_ok(), "expected broadcast notification");

        let snap_after = source.snapshot();
        assert!(snap_after.revision > snap_before.revision);
        assert!(snap_after.github_checked_ago_secs.is_some());

        let _ = std::fs::remove_dir_all(dir);
    }
}
