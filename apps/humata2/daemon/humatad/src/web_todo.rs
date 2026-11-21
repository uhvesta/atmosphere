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
use humata2_web_todo_wire::{BoardSnapshot, ChannelInfo, SessionInfo};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::events::Feed;

const FIELDS_KEY: &str = "todo.fields";
const DAY_SECS: i64 = 86_400;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct SavedFieldsDoc {
    items: BTreeMap<String, SavedItemFields>,
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
}

fn today_utc() -> Day {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(0))
        .unwrap_or(0);
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
        if let Some(pr) = &fields.watched_pr {
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

/// [`BoardSource`] backed by `humatad2`'s `Store` and push `Feed`.
pub(crate) struct DaemonBoardSource {
    store: Arc<Store>,
    updates: broadcast::Sender<()>,
    revision: AtomicU64,
}

impl DaemonBoardSource {
    pub(crate) fn new(store: Arc<Store>, feed: &Arc<Feed>) -> Arc<Self> {
        let (updates, _) = broadcast::channel(64);
        let source = Arc::new(Self {
            store,
            updates,
            revision: AtomicU64::new(1),
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
                            src.revision.fetch_add(1, Ordering::Relaxed);
                            let _ = src.updates.send(());
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let Some(src) = weak.upgrade() else {
                            break;
                        };
                        src.revision.fetch_add(1, Ordering::Relaxed);
                        let _ = src.updates.send(());
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        source
    }
}

impl BoardSource for DaemonBoardSource {
    fn snapshot(&self) -> BoardSnapshot {
        let today = today_utc();
        let revision = self.revision.load(Ordering::Relaxed);

        let fields_doc: SavedFieldsDoc = self
            .store
            .setting(FIELDS_KEY)
            .ok()
            .flatten()
            .and_then(|saved| serde_json::from_str(&saved.value).ok())
            .unwrap_or_default();

        let items: Vec<Todo> = self
            .store
            .all_board_items_but_sessions()
            .unwrap_or_default()
            .into_iter()
            .filter(|rec| !rec.deleted)
            .map(|rec| {
                let extra = fields_doc.items.get(&rec.key);
                todo_from_record(&rec, extra, today)
            })
            .collect();

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

        BoardSnapshot {
            today,
            items,
            channels,
            sessions,
            revision,
            reviews: Vec::new(),
            watched_prs: Vec::new(),
            github_checked_ago_secs: None,
            review_details: Vec::new(),
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<()> {
        self.updates.subscribe()
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
