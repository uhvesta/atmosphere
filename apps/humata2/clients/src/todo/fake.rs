//! A scripted [`TodoClient`]: it keeps the list the way the host does.
//! Clones share it, so two clones stand for two apps open at once. No
//! timers, no real IO.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use humata2_todo::{
    Ask, AskDetail, AskKind, ChannelFilter, CheckpointState, Day, Evidence, OwnerKind, Review,
    RunEntry, State, Step, Summary, Todo, TodoComment, TodoOwner, word_diff,
};
use tokio::sync::mpsc;

use super::{
    BlockerGoneResolution, BulkUpdateFailure, BulkUpdateItem, BulkUpdateResult, Entry, FoldResult,
    MigrationNotice, NewTodo, SplitResult, TodoClient, TodoFilter, TodoHistoryEntry, TodoLists,
    TodoPatch, TodoRunsResult,
};
use crate::daemon::{DaemonError, Reply};

/// A call a test can read back, one per trait method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// `list`.
    List(TodoFilter),
    /// `create`.
    Create(NewTodo),
    /// `update`, with the revision last.
    Update(String, TodoPatch, u64),
    /// `answer`.
    Answer(String, bool),
    /// `migration`.
    Migration,
    /// `dismiss_migration`.
    DismissMigration,
    /// `watch`.
    Watch,
    /// `add_comment`.
    AddComment(String, String, Option<String>),
    /// `split`.
    Split(String, Vec<String>, bool),
    /// `fold_into_step`.
    FoldIntoStep(String, String),
    /// `bulk_update`.
    BulkUpdate(Vec<BulkUpdateItem>),
    /// `history`.
    History(String),
    /// `runs`.
    Runs(String),
    /// `answer_question`.
    AnswerQuestion(String, String, Vec<String>),
    /// `answer_action`.
    AnswerAction(String, bool, String),
    /// `answer_review`.
    AnswerReview(String, bool, String),
    /// `resolve_ownership`.
    ResolveOwnership(String, bool, bool),
    /// `resolve_blocker_gone`.
    ResolveBlockerGone(String, String, BlockerGoneResolution),
    /// `resolve_lifecycle_prompt`.
    ResolveLifecyclePrompt(String, String, String),
}

/// Lists a watcher can fall behind by before the fake drops it.
const WATCH_BUFFER: usize = 64;

struct Inner {
    entries: Vec<Entry>,
    today: Day,
    next_number: u32,
    next_comment_id: u32,
    next_step_id: u32,
    migration: Option<MigrationNotice>,
    watchers: Vec<mpsc::Sender<Vec<Entry>>>,
    failure: Option<DaemonError>,
    calls: Vec<Call>,
    history: HashMap<String, Vec<TodoHistoryEntry>>,
    runs: HashMap<String, Vec<RunEntry>>,
}

impl Inner {
    /// Tells every watcher that is still there.
    fn changed(&mut self) {
        let list = self.entries.clone();
        self.watchers.retain(|watcher| watcher.try_send(list.clone()).is_ok());
    }

    fn entry_mut(&mut self, key: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.todo.key == key)
    }

    /// Returns true if adding `key -> dep` would introduce a cycle in `depends_on`.
    fn would_cycle(&self, key: &str, dep: &str) -> bool {
        if key == dep {
            return true;
        }
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(dep.to_owned());
        while let Some(curr) = queue.pop_front() {
            if !visited.insert(curr.clone()) {
                continue;
            }
            if let Some(entry) = self.entries.iter().find(|e| e.todo.key == curr) {
                for next in &entry.todo.depends_on {
                    if next == key {
                        return true;
                    }
                    queue.push_back(next.clone());
                }
            }
        }
        false
    }

    fn check_cycle(&self, key: &str, deps: &[String]) -> Result<(), DaemonError> {
        for dep in deps {
            if self.would_cycle(key, dep) {
                return Err(DaemonError::Refused {
                    what: key.to_owned(),
                    reason: format!("adding dependency on {dep} would create a dependency cycle"),
                });
            }
        }
        Ok(())
    }

    fn push_history(&mut self, key: &str, entry: TodoHistoryEntry) {
        self.history.entry(key.to_owned()).or_default().push(entry);
    }

    fn record_patch_history(&mut self, key: &str, before: &Todo, after: &Todo) {
        let today = Some(self.today);
        if before.title != after.title {
            self.push_history(
                key,
                TodoHistoryEntry {
                    at_day: today,
                    actor: "you".to_owned(),
                    kind: "changed".to_owned(),
                    field: "title".to_owned(),
                    before: before.title.clone(),
                    after: after.title.clone(),
                    change: format!("title {} → {}", before.title, after.title),
                    word_diff: word_diff(&before.title, &after.title),
                },
            );
        }
        if before.description != after.description {
            self.push_history(
                key,
                TodoHistoryEntry {
                    at_day: today,
                    actor: "you".to_owned(),
                    kind: "changed".to_owned(),
                    field: "description".to_owned(),
                    before: before.description.clone(),
                    after: after.description.clone(),
                    change: "description edited".to_owned(),
                    word_diff: word_diff(&before.description, &after.description),
                },
            );
        }
        if before.state != after.state {
            self.push_history(
                key,
                TodoHistoryEntry {
                    at_day: today,
                    actor: "you".to_owned(),
                    kind: "changed".to_owned(),
                    field: "state".to_owned(),
                    before: format!("{:?}", before.state),
                    after: format!("{:?}", after.state),
                    change: format!("state {:?} → {:?}", before.state, after.state),
                    word_diff: Vec::new(),
                },
            );
        }
    }
}

#[allow(clippy::expect_used)]
fn today_default() -> Day {
    // Thu 19 Nov 2026, the day of the signed-off frames.
    Day::from_ymd(2026, 11, 19).expect("19 Nov 2026 is a date")
}

/// A to-do list that starts empty (see [`FakeTodo::frame_b1_1`] for the
/// signed-off one). Clones share it. A change raises the changed item's
/// revision by one and sends the whole list to every watcher before the
/// reply comes.
#[derive(Clone)]
pub struct FakeTodo {
    shared: Arc<Mutex<Inner>>,
}

impl Default for FakeTodo {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeTodo {
    /// An empty list, on Thu 19 Nov 2026, with no migration notice.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Inner {
                entries: Vec::new(),
                today: today_default(),
                next_number: 20,
                next_comment_id: 1,
                next_step_id: 1,
                migration: None,
                watchers: Vec::new(),
                failure: None,
                calls: Vec::new(),
                history: HashMap::new(),
                runs: HashMap::new(),
            })),
        }
    }

    /// The list of frame b1-1: ten open items across four channels, the
    /// INF-6 pin, the PR review, three asks, one suggestion, two waiting
    /// and three hidden until a day. Today is Thu 19 Nov.
    #[must_use]
    pub fn frame_b1_1() -> Self {
        let fake = Self::new();
        fake.inner().entries = b1_1_entries(today_default());
        fake
    }

    /// The list of frame b1-1 with items tied to the sessions `workspace`
    /// has, so a session's To-do panel shows its slice: Ledger schema
    /// migration (`ledger`) works on PAY-3, whose close it is asking about,
    /// PAY-7 and PAY-9; Token refresh retry (`s-auth`) on A-1 to A-3; the
    /// Desk's Plan Q4 (`desk-plan-q4`) on TD-3 and TD-4.
    #[must_use]
    pub fn for_workspace(workspace: &crate::Workspace) -> Self {
        let fake = Self::frame_b1_1();
        let has = |id: &str| {
            let id = crate::model::SessionId::new(id);
            workspace.session(&id).is_some() || workspace.desk.sessions.contains_key(&id)
        };
        let today = fake.today();
        let mut inner = fake.inner();
        let mut link = |keys: &[&str], session: &str| {
            for e in inner.entries.iter_mut().filter(|e| keys.contains(&e.todo.key.as_str())) {
                e.todo.sessions = vec![session.to_owned()];
            }
        };
        if has("ledger") {
            link(&["PAY-3", "PAY-7", "PAY-9"], "ledger");
        }
        if has("desk-plan-q4") {
            link(&["TD-3"], "desk-plan-q4");
        }
        if has("ledger")
            && let Some(pay3) = inner.entries.iter_mut().find(|e| e.todo.key == "PAY-3")
        {
            // Frame 191: the panel beside the asking terminal.
            if let Some(ask) = pay3.todo.ask.as_mut() {
                ask.by = "claude in this session".to_owned();
            }
            pay3.todo.ask_detail.asked_minutes_ago = 1;
        }
        let add = |key: &str, title: &str, channel: Option<&str>, session: &str| {
            let mut todo = Todo::new(key, title, today.add_days(-3));
            todo.channel = channel.map(str::to_owned);
            todo.sessions = vec![session.to_owned()];
            todo
        };
        let mut added = Vec::new();
        if has("s-auth") {
            let mut a1 = add("A-1", "Token refresh retry", Some("api"), "s-auth");
            a1.state = State::InProgress;
            added.push(a1);
            added.push(add("A-2", "Rate limit headers", Some("api"), "s-auth"));
            added.push(add("A-3", "v1 tables cleanup", Some("api"), "s-auth"));
        }
        if has("desk-plan-q4") {
            added.push(add("TD-4", "Share the Q4 plan with the team", None, "desk-plan-q4"));
        }
        inner.entries.extend(added.into_iter().map(|todo| Entry { todo, revision: 0, ask_id: None }));
        drop(inner);
        fake
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The day the fake calls today.
    #[must_use]
    pub fn today(&self) -> Day {
        self.inner().today
    }

    /// Adds `todo` as it is, under `ask_id` if it has an ask, as another
    /// client would: watchers are told.
    pub fn insert(&self, todo: Todo, ask_id: Option<&str>) {
        let mut inner = self.inner();
        inner.entries.push(Entry {
            todo,
            revision: 0,
            ask_id: ask_id.map(str::to_owned),
        });
        inner.changed();
    }

    /// Makes another client's change to `key` that changes nothing the app
    /// shows: its revision goes up and watchers are told. A write from an
    /// older revision is then refused.
    pub fn touch(&self, key: &str) {
        let mut inner = self.inner();
        if let Some(entry) = inner.entry_mut(key) {
            entry.revision += 1;
            inner.changed();
        }
    }

    /// Sets what `migration` answers with.
    pub fn set_migration(&self, notice: Option<MigrationNotice>) {
        self.inner().migration = notice;
    }

    /// Makes the next call fail with `error`, whichever method it is, and
    /// change nothing. The call is still recorded.
    pub fn fail_next(&self, error: DaemonError) {
        self.inner().failure = Some(error);
    }

    /// The calls made so far through any clone, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.inner().calls.clone()
    }

    /// The items now, as the host holds them.
    #[must_use]
    pub fn entries(&self) -> Vec<Entry> {
        self.inner().entries.clone()
    }

    /// Records a run in the fake ledger and updates the item's run summary.
    pub fn add_run(&self, key: &str, run: RunEntry) {
        let mut inner = self.inner();
        let cost = run.cost_micros;
        inner.runs.entry(key.to_owned()).or_default().push(run);
        if let Some(entry) = inner.entry_mut(key) {
            entry.todo.run_count = entry.todo.run_count.saturating_add(1);
            // Unknown costs stay unknown; the total sums the known ones.
            entry.todo.total_cost_micros = match (entry.todo.total_cost_micros, cost) {
                (Some(total), Some(cost)) => Some(total.saturating_add(cost)),
                (total, None) => total,
                (None, cost) => cost,
            };
            entry.revision += 1;
            inner.changed();
        }
    }

    /// Records `call`, then answers with the scripted failure or `run`.
    fn call<T: Send + 'static>(
        &self,
        call: Call,
        run: impl FnOnce(&mut Inner) -> Result<T, DaemonError>,
    ) -> Reply<T> {
        let mut inner = self.inner();
        inner.calls.push(call);
        let result = match inner.failure.take() {
            Some(error) => Err(error),
            None => run(&mut inner),
        };
        Box::pin(std::future::ready(result))
    }
}

fn not_found(what: &str) -> DaemonError {
    DaemonError::NotFound { what: what.to_owned() }
}

impl TodoClient for FakeTodo {
    fn today(&self) -> Day {
        FakeTodo::today(self)
    }

    fn list(&self, filter: &TodoFilter) -> Reply<Vec<Entry>> {
        self.call(Call::List(filter.clone()), |inner| {
            Ok(inner
                .entries
                .iter()
                .filter(|e| {
                    let t = &e.todo;
                    let channel = match &filter.channel {
                        None => true,
                        Some(ChannelFilter::NoChannel) => t.channel.is_none(),
                        Some(ChannelFilter::Channel(c)) => t.channel.as_deref() == Some(c),
                    };
                    let session = filter.session.as_ref().is_none_or(|s| t.sessions.contains(s));
                    let closed = filter.show_closed || !t.state.is_terminal();
                    channel && session && closed
                })
                .cloned()
                .collect())
        })
    }

    fn create(&self, draft: NewTodo) -> Reply<Entry> {
        self.call(Call::Create(draft.clone()), |inner| {
            let key = format!("T-{}", inner.next_number);
            inner.next_number += 1;
            let mut todo = Todo::new(&key, &draft.title, inner.today);
            todo.channel = draft.channel;
            todo.due = draft.due;
            todo.fuzzy_due = draft.fuzzy_due;
            todo.effort = draft.effort;
            todo.impact = draft.impact;
            todo.need = draft.need;
            todo.tags = draft.tags;
            todo.description = draft.description;
            todo.source_anchor = draft.source_anchor;
            todo.from_keys = draft.from_keys;
            todo.external_block = draft.external_block;
            let entry = Entry { todo, revision: 0, ask_id: None };
            inner.entries.push(entry.clone());
            inner.push_history(
                &key,
                TodoHistoryEntry {
                    at_day: Some(inner.today),
                    actor: "you".to_owned(),
                    kind: "created".to_owned(),
                    field: String::new(),
                    before: String::new(),
                    after: entry.todo.title.clone(),
                    change: "created".to_owned(),
                    word_diff: Vec::new(),
                },
            );
            inner.changed();
            Ok(entry)
        })
    }

    fn update(&self, key: &str, patch: TodoPatch, expected_revision: u64) -> Reply<Entry> {
        self.call(Call::Update(key.to_owned(), patch.clone(), expected_revision), |inner| {
            inner.check_cycle(key, &patch.depends_on_add)?;
            let today = inner.today;
            let entry = inner.entry_mut(key).ok_or_else(|| not_found(key))?;
            if entry.revision != expected_revision {
                return Err(DaemonError::Conflict {
                    what: key.to_owned(),
                    reason: "it changed on another screen or by an agent after you opened it; \
                             reload the list and make the change again"
                        .to_owned(),
                });
            }
            let before = entry.todo.clone();
            patch.apply(&mut entry.todo, today);
            entry.revision += 1;
            let done = entry.clone();
            inner.record_patch_history(key, &before, &done.todo);
            inner.changed();
            Ok(done)
        })
    }

    fn answer(&self, ask_id: &str, yes: bool) -> Reply<()> {
        self.call(Call::Answer(ask_id.to_owned(), yes), |inner| {
            let today = inner.today;
            let entry = inner
                .entries
                .iter_mut()
                .find(|e| e.ask_id.as_deref() == Some(ask_id))
                .ok_or_else(|| not_found("This ask"))?;
            if let Some(ask) = entry.todo.ask.take()
                && yes
                && matches!(ask.kind, AskKind::Close)
            {
                TodoPatch { state: Some(State::Closed), ..TodoPatch::default() }
                    .apply(&mut entry.todo, today);
            }
            entry.ask_id = None;
            entry.revision += 1;
            inner.changed();
            Ok(())
        })
    }

    fn migration(&self) -> Reply<Option<MigrationNotice>> {
        self.call(Call::Migration, |inner| Ok(inner.migration.clone()))
    }

    fn dismiss_migration(&self) -> Reply<()> {
        self.call(Call::DismissMigration, |inner| {
            inner.migration = None;
            Ok(())
        })
    }

    fn watch(&self) -> Reply<TodoLists> {
        self.call(Call::Watch, |inner| {
            let (send, receive) = mpsc::channel(WATCH_BUFFER);
            inner.watchers.push(send);
            Ok(receive)
        })
    }

    fn add_comment(
        &self,
        key: &str,
        body: String,
        step_id: Option<String>,
    ) -> Reply<(Entry, TodoComment)> {
        self.call(Call::AddComment(key.to_owned(), body.clone(), step_id.clone()), |inner| {
            let today = inner.today;
            let comment_id = format!("cmt-{}", inner.next_comment_id);
            inner.next_comment_id += 1;
            let entry = inner.entry_mut(key).ok_or_else(|| not_found(key))?;
            let comment = TodoComment {
                id: comment_id,
                author: "you".to_owned(),
                body: body.clone(),
                created: Some(today),
                step_id,
            };
            entry.todo.comments.push(comment.clone());
            entry.revision += 1;
            let updated = entry.clone();
            inner.push_history(
                key,
                TodoHistoryEntry {
                    at_day: Some(today),
                    actor: "you".to_owned(),
                    kind: "commented".to_owned(),
                    field: "comments".to_owned(),
                    before: String::new(),
                    after: body,
                    change: "commented".to_owned(),
                    word_diff: Vec::new(),
                },
            );
            inner.changed();
            Ok((updated, comment))
        })
    }

    fn split(&self, key: &str, titles: Vec<String>, keep_age: bool) -> Reply<SplitResult> {
        self.call(Call::Split(key.to_owned(), titles.clone(), keep_age), |inner| {
            if titles.is_empty() {
                return Err(DaemonError::Refused {
                    what: key.to_owned(),
                    reason: "split requires at least one new title".to_owned(),
                });
            }
            let today = inner.today;
            let source_snapshot = inner
                .entries
                .iter()
                .find(|e| e.todo.key == key)
                .ok_or_else(|| not_found(key))?
                .todo
                .clone();

            let mut created = Vec::with_capacity(titles.len());
            let mut created_keys = Vec::with_capacity(titles.len());
            for title in titles {
                let new_key = format!("T-{}", inner.next_number);
                inner.next_number += 1;
                let created_day = if keep_age { source_snapshot.created } else { today };
                let mut child = Todo::new(&new_key, &title, created_day);
                child.channel = source_snapshot.channel.clone();
                child.tags = source_snapshot.tags.clone();
                child.need = source_snapshot.need;
                child.impact = source_snapshot.impact;
                child.from_keys.push(key.to_owned());
                let entry = Entry { todo: child, revision: 0, ask_id: None };
                inner.entries.push(entry.clone());
                created_keys.push(new_key);
                created.push(entry);
            }

            let source = inner.entry_mut(key).ok_or_else(|| not_found(key))?;
            source.todo.state = State::Became;
            source.todo.closed = Some(today);
            source.todo.pin = None;
            for k in &created_keys {
                if !source.todo.became_keys.contains(k) {
                    source.todo.became_keys.push(k.clone());
                }
            }
            source.revision += 1;
            let source_out = source.clone();

            inner.push_history(
                key,
                TodoHistoryEntry {
                    at_day: Some(today),
                    actor: "you".to_owned(),
                    kind: "split".to_owned(),
                    field: "became_keys".to_owned(),
                    before: String::new(),
                    after: created_keys.join(", "),
                    change: format!("split into {}", created_keys.join(", ")),
                    word_diff: Vec::new(),
                },
            );
            inner.changed();
            Ok(SplitResult { source: source_out, created })
        })
    }

    fn fold_into_step(&self, source_key: &str, target_key: &str) -> Reply<FoldResult> {
        self.call(
            Call::FoldIntoStep(source_key.to_owned(), target_key.to_owned()),
            |inner| {
                if source_key == target_key {
                    return Err(DaemonError::Refused {
                        what: source_key.to_owned(),
                        reason: "cannot fold an item into itself".to_owned(),
                    });
                }
                let today = inner.today;
                let src_snap = inner
                    .entries
                    .iter()
                    .find(|e| e.todo.key == source_key)
                    .ok_or_else(|| not_found(source_key))?
                    .todo
                    .clone();
                if !inner.entries.iter().any(|e| e.todo.key == target_key) {
                    return Err(not_found(target_key));
                }

                let step_id = format!("step-{}", inner.next_step_id);
                inner.next_step_id += 1;

                let source_out = {
                    let src = inner.entry_mut(source_key).ok_or_else(|| not_found(source_key))?;
                    src.todo.state = State::Became;
                    src.todo.closed = Some(today);
                    src.todo.pin = None;
                    if !src.todo.became_keys.iter().any(|k| k == target_key) {
                        src.todo.became_keys.push(target_key.to_owned());
                    }
                    src.revision += 1;
                    src.clone()
                };

                let target_out = {
                    let tgt = inner.entry_mut(target_key).ok_or_else(|| not_found(target_key))?;
                    tgt.todo.steps.push(Step {
                        id: step_id,
                        text: src_snap.title.clone(),
                        done: false,
                        markdown_context: (!src_snap.description.is_empty())
                            .then_some(src_snap.description),
                        dropped: false,
                        from_key: Some(source_key.to_owned()),
                        source_anchor: src_snap.source_anchor,
                    });
                    if !tgt.todo.from_keys.iter().any(|k| k == source_key) {
                        tgt.todo.from_keys.push(source_key.to_owned());
                    }
                    tgt.revision += 1;
                    tgt.clone()
                };

                inner.push_history(
                    source_key,
                    TodoHistoryEntry {
                        at_day: Some(today),
                        actor: "you".to_owned(),
                        kind: "folded".to_owned(),
                        field: "became_keys".to_owned(),
                        before: String::new(),
                        after: target_key.to_owned(),
                        change: format!("folded into {target_key}"),
                        word_diff: Vec::new(),
                    },
                );
                inner.changed();
                Ok(FoldResult { source: source_out, target: target_out })
            },
        )
    }

    fn bulk_update(&self, items: Vec<BulkUpdateItem>) -> Reply<BulkUpdateResult> {
        self.call(Call::BulkUpdate(items.clone()), |inner| {
            let today = inner.today;
            let mut updated = Vec::new();
            let mut failed = Vec::new();
            for item in items {
                if let Err(err) = inner.check_cycle(&item.key, &item.patch.depends_on_add) {
                    failed.push(BulkUpdateFailure { key: item.key, reason: err.to_string() });
                    continue;
                }
                let Some(entry) = inner.entry_mut(&item.key) else {
                    failed.push(BulkUpdateFailure {
                        key: item.key,
                        reason: "item not found".to_owned(),
                    });
                    continue;
                };
                if entry.revision != item.expected_revision {
                    failed.push(BulkUpdateFailure {
                        key: item.key,
                        reason: "revision conflict".to_owned(),
                    });
                    continue;
                }
                let before = entry.todo.clone();
                item.patch.apply(&mut entry.todo, today);
                entry.revision += 1;
                let done = entry.clone();
                inner.record_patch_history(&item.key, &before, &done.todo);
                updated.push(done);
            }
            if !updated.is_empty() {
                inner.changed();
            }
            Ok(BulkUpdateResult { updated, failed })
        })
    }

    fn history(&self, key: &str) -> Reply<Vec<TodoHistoryEntry>> {
        self.call(Call::History(key.to_owned()), |inner| {
            Ok(inner.history.get(key).cloned().unwrap_or_default())
        })
    }

    fn runs(&self, key: &str) -> Reply<TodoRunsResult> {
        self.call(Call::Runs(key.to_owned()), |inner| {
            let runs = inner.runs.get(key).cloned().unwrap_or_default();
            Ok(TodoRunsResult::from_runs(runs))
        })
    }

    fn answer_question(
        &self,
        ask_id: &str,
        text: String,
        selected_choices: Vec<String>,
    ) -> Reply<()> {
        self.call(
            Call::AnswerQuestion(ask_id.to_owned(), text.clone(), selected_choices.clone()),
            |inner| {
                let today = inner.today;
                let entry = inner
                    .entries
                    .iter_mut()
                    .find(|e| e.ask_id.as_deref() == Some(ask_id))
                    .ok_or_else(|| not_found("This ask"))?;
                let key = entry.todo.key.clone();
                entry.todo.ask = None;
                entry.ask_id = None;
                entry.revision += 1;
                let reply_summary = if selected_choices.is_empty() {
                    text
                } else if text.is_empty() {
                    selected_choices.join(", ")
                } else {
                    format!("{} ({text})", selected_choices.join(", "))
                };
                inner.push_history(
                    &key,
                    TodoHistoryEntry {
                        at_day: Some(today),
                        actor: "you".to_owned(),
                        kind: "ask_answered".to_owned(),
                        field: "question".to_owned(),
                        before: String::new(),
                        after: reply_summary.clone(),
                        change: format!("answered question: {reply_summary}"),
                        word_diff: Vec::new(),
                    },
                );
                inner.changed();
                Ok(())
            },
        )
    }

    fn answer_action(&self, ask_id: &str, done: bool, note: String) -> Reply<()> {
        self.call(Call::AnswerAction(ask_id.to_owned(), done, note.clone()), |inner| {
            let today = inner.today;
            let entry = inner
                .entries
                .iter_mut()
                .find(|e| e.ask_id.as_deref() == Some(ask_id))
                .ok_or_else(|| not_found("This ask"))?;
            let key = entry.todo.key.clone();
            entry.todo.ask = None;
            entry.ask_id = None;
            entry.revision += 1;
            let verdict = if done { "Done" } else { "Can't be done" };
            inner.push_history(
                &key,
                TodoHistoryEntry {
                    at_day: Some(today),
                    actor: "you".to_owned(),
                    kind: "ask_answered".to_owned(),
                    field: "action".to_owned(),
                    before: String::new(),
                    after: note,
                    change: format!("action request: {verdict}"),
                    word_diff: Vec::new(),
                },
            );
            inner.changed();
            Ok(())
        })
    }

    fn answer_review(&self, ask_id: &str, approved: bool, note: String) -> Reply<()> {
        self.call(Call::AnswerReview(ask_id.to_owned(), approved, note.clone()), |inner| {
            let today = inner.today;
            let entry = inner
                .entries
                .iter_mut()
                .find(|e| e.ask_id.as_deref() == Some(ask_id))
                .ok_or_else(|| not_found("This ask"))?;
            let key = entry.todo.key.clone();
            let ask = entry.todo.ask.take();
            entry.ask_id = None;
            if approved {
                if let Some(Ask { kind: AskKind::ReviewRequest { checkpoint: Some(cp), .. }, .. }) =
                    ask
                {
                    entry.todo.checkpoint = Some(cp);
                } else if entry.todo.checkpoint == Some(CheckpointState::Defined) {
                    entry.todo.checkpoint = Some(CheckpointState::Done);
                }
            }
            entry.revision += 1;
            let verdict = if approved { "Approved" } else { "Changes asked" };
            inner.push_history(
                &key,
                TodoHistoryEntry {
                    at_day: Some(today),
                    actor: "you".to_owned(),
                    kind: "ask_answered".to_owned(),
                    field: "review".to_owned(),
                    before: String::new(),
                    after: note,
                    change: format!("review: {verdict}"),
                    word_diff: Vec::new(),
                },
            );
            inner.changed();
            Ok(())
        })
    }

    fn resolve_ownership(&self, key: &str, take_back: bool, retry: bool) -> Reply<Entry> {
        self.call(Call::ResolveOwnership(key.to_owned(), take_back, retry), |inner| {
            let today = inner.today;
            let entry = inner.entry_mut(key).ok_or_else(|| not_found(key))?;
            if take_back {
                entry.todo.owner = Some(TodoOwner {
                    kind: OwnerKind::User,
                    label: "You".to_owned(),
                    session: None,
                });
                entry.todo.stuck_agent = None;
            } else if retry {
                entry.todo.stuck_agent = None;
            }
            entry.revision += 1;
            let out = entry.clone();
            inner.push_history(
                key,
                TodoHistoryEntry {
                    at_day: Some(today),
                    actor: "you".to_owned(),
                    kind: "owner_changed".to_owned(),
                    field: "owner".to_owned(),
                    before: String::new(),
                    after: if take_back { "You".to_owned() } else { "Retried".to_owned() },
                    change: if take_back {
                        "took back ownership".to_owned()
                    } else {
                        "retried stuck agent".to_owned()
                    },
                    word_diff: Vec::new(),
                },
            );
            inner.changed();
            Ok(out)
        })
    }

    fn resolve_blocker_gone(
        &self,
        key: &str,
        blocker_key: &str,
        resolution: BlockerGoneResolution,
    ) -> Reply<Entry> {
        self.call(
            Call::ResolveBlockerGone(key.to_owned(), blocker_key.to_owned(), resolution.clone()),
            |inner| {
                if let BlockerGoneResolution::Rewire(ref targets) = resolution {
                    inner.check_cycle(key, targets)?;
                }
                let today = inner.today;
                let entry = inner.entry_mut(key).ok_or_else(|| not_found(key))?;
                entry.todo.depends_on.retain(|d| d != blocker_key);
                entry.todo.blocker_gone.retain(|b| b.blocker_key != blocker_key);
                match resolution {
                    BlockerGoneResolution::Unblock => {}
                    BlockerGoneResolution::CloseWontDo => {
                        entry.todo.state = State::WontDo;
                        entry.todo.closed = Some(today);
                        entry.todo.pin = None;
                    }
                    BlockerGoneResolution::Rewire(targets) => {
                        for t in targets {
                            if !entry.todo.depends_on.contains(&t) {
                                entry.todo.depends_on.push(t);
                            }
                        }
                    }
                }
                entry.revision += 1;
                let out = entry.clone();
                inner.changed();
                Ok(out)
            },
        )
    }

    fn resolve_lifecycle_prompt(&self, key: &str, prompt_id: &str, choice: &str) -> Reply<Entry> {
        self.call(
            Call::ResolveLifecyclePrompt(key.to_owned(), prompt_id.to_owned(), choice.to_owned()),
            |inner| {
                let today = inner.today;
                let entry = inner.entry_mut(key).ok_or_else(|| not_found(key))?;
                entry.todo.lifecycle_prompts.retain(|p| p.id != prompt_id);
                match choice {
                    "close" => {
                        entry.todo.state = State::Closed;
                        entry.todo.closed = Some(today);
                        entry.todo.pin = None;
                    }
                    "wont_do" => {
                        entry.todo.state = State::WontDo;
                        entry.todo.closed = Some(today);
                        entry.todo.pin = None;
                    }
                    "open" => {
                        entry.todo.state = State::Open;
                        entry.todo.closed = None;
                    }
                    _ => {}
                }
                entry.revision += 1;
                let out = entry.clone();
                inner.changed();
                Ok(out)
            },
        )
    }
}

/// The notice of a migration of `items` items from `boards` boards.
#[must_use]
pub fn notice(open: usize, in_progress: usize, closed: usize, boards: usize) -> MigrationNotice {
    MigrationNotice {
        summary: Summary { items: open + in_progress + closed, open, in_progress, closed, boards },
    }
}

fn b1_1_entries(today: Day) -> Vec<Entry> {
    let day = |m: u32, d: u32| Day::from_ymd(2026, m, d);
    let ago = |n: i32| today.add_days(-n);
    let item = |key: &str, title: &str, channel: Option<&str>, created: Day| {
        let mut t = Todo::new(key, title, created);
        t.channel = channel.map(str::to_owned);
        t
    };
    let steps = |done: usize, total: usize| {
        (0..total)
            .map(|i| Step {
                id: format!("s-{}", i + 1),
                text: format!("Step {}", i + 1),
                done: i < done,
                ..Step::default()
            })
            .collect::<Vec<_>>()
    };
    let tags = |t: &[&str]| t.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let mut out: Vec<(Todo, Option<&str>)> = Vec::new();

    let mut pay3 = item("PAY-3", "Ledger schema v2", Some("payments-rewrite"), ago(14));
    pay3.state = State::InProgress;
    pay3.due = day(11, 20);
    pay3.tags = tags(&["migration", "db"]);
    pay3.steps = [
        "Add ledger_entries and ledger_balances",
        "Dual-write from the posting path",
        "Backfill the old table in batches",
        "Verify the balances match on staging",
        "Cut production reads over",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, text)| Step {
        id: format!("pay3-s-{}", i + 1),
        text: text.to_owned(),
        done: i < 4,
        ..Step::default()
    })
    .collect();
    pay3.effort = Some(humata2_todo::Effort::M);
    pay3.impact = Some(humata2_todo::Impact::High);
    pay3.sessions = vec!["Ledger entries".to_owned()];
    pay3.ask = Some(Ask {
        kind: AskKind::Close,
        by: "claude in Ledger entries".to_owned(),
        reason: "Migration and backfill pass on staging: 3.1M rows in 4m12s. Steps 1 to 4 are done; step 5, the production cut-over, is yours.".to_owned(),
    });
    pay3.ask_detail = AskDetail {
        evidence: vec![
            Evidence { text: "3 commits".to_owned(), link: Some("commits".to_owned()) },
            Evidence { text: "acme/ledger#92".to_owned(), link: Some("pull".to_owned()) },
            Evidence { text: "212 tests passed".to_owned(), link: Some("checks".to_owned()) },
        ],
        affects: vec![
            "WEB-13 stops waiting".to_owned(),
            "this session keeps running".to_owned(),
        ],
        change: None,
        asked_minutes_ago: 20,
        session_ended: false,
    };
    pay3.description = "Split postings into ledger_entries and ledger_balances, backfill from the old table in batches, and keep both in step until cut-over.".to_owned();
    out.push((pay3, Some("req-pay-3")));

    let mut inf2 = item("INF-2", "Postgres 17 upgrade", Some("infra"), ago(10));
    inf2.state = State::InProgress;
    inf2.due = day(11, 17);
    inf2.tags = tags(&["db"]);
    inf2.steps = steps(1, 4);
    inf2.effort = Some(humata2_todo::Effort::L);
    inf2.impact = Some(humata2_todo::Impact::High);
    inf2.sessions = vec!["Upgrade Postgres 17".to_owned()];
    inf2.ask = Some(Ask {
        kind: AskKind::Edit {
            field: "due".to_owned(),
            before: "Tue 17 Nov".to_owned(),
            after: "Fri 4 Dec".to_owned(),
        },
        by: "claude in Upgrade Postgres 17".to_owned(),
        reason: "The staging rehearsal slipped.".to_owned(),
    });
    inf2.ask_detail = AskDetail {
        asked_minutes_ago: 95,
        session_ended: true,
        ..AskDetail::default()
    };
    out.push((inf2, Some("req-inf-2")));

    let mut sug = item("SUG-1", "Idempotency keys for refund retries", Some("payments-rewrite"), ago(1));
    sug.suggested = true;
    sug.tags = tags(&["refunds"]);
    sug.effort = Some(humata2_todo::Effort::S);
    sug.impact = Some(humata2_todo::Impact::High);
    out.push((sug, None));

    let mut web8 = item("WEB-8", "Checkout page shows the new ledger totals", Some("web"), ago(8));
    web8.state = State::InProgress;
    web8.due = day(11, 27);
    web8.tags = tags(&["checkout"]);
    web8.steps = steps(2, 3);
    web8.effort = Some(humata2_todo::Effort::M);
    web8.impact = Some(humata2_todo::Impact::Medium);
    web8.sessions = vec!["Ledger totals".to_owned(), "Totals, another way".to_owned()];
    out.push((web8, None));

    let mut inf6 = item("INF-6", "Rotate the staging database credentials", Some("infra"), ago(7));
    inf6.pin = Some(0);
    inf6.due = Some(today);
    inf6.tags = tags(&["security"]);
    inf6.effort = Some(humata2_todo::Effort::S);
    inf6.impact = Some(humata2_todo::Impact::Medium);
    out.push((inf6, None));

    let mut pay7 = item("PAY-7", "Refund retry with backoff", Some("payments-rewrite"), ago(9));
    pay7.due = day(11, 18);
    pay7.tags = tags(&["refunds", "bug"]);
    pay7.effort = Some(humata2_todo::Effort::S);
    pay7.impact = Some(humata2_todo::Impact::High);
    out.push((pay7, None));

    let mut rev = item("REV-1", "Review: Ledger reads behind a flag", Some("pr-reviews"), ago(2));
    rev.review = Some(Review {
        repo_prs: vec!["acme/api#483".to_owned(), "acme/web#210".to_owned()],
        author: "dana-k".to_owned(),
        waiting_days: 2,
    });
    out.push((rev, None));

    let mut pay5 = item("PAY-5", "API rate limiter", Some("payments-rewrite"), ago(12));
    pay5.due = day(11, 23);
    pay5.tags = tags(&["api"]);
    pay5.steps = steps(0, 3);
    pay5.effort = Some(humata2_todo::Effort::M);
    pay5.impact = Some(humata2_todo::Impact::High);
    out.push((pay5, None));

    let mut td3 = item("TD-3", "Renew the Apple developer certificate", None, ago(6));
    td3.due = day(12, 14);
    td3.tags = tags(&["admin"]);
    td3.effort = Some(humata2_todo::Effort::S);
    td3.impact = Some(humata2_todo::Impact::High);
    out.push((td3, None));

    let mut pay9 = item("PAY-9", "Backfill progress bar", Some("payments-rewrite"), ago(9));
    pay9.depends_on = vec!["PAY-7".to_owned()];
    out.push((pay9, None));
    let mut web13 = item("WEB-13", "Show totals on the receipt page", Some("web"), ago(5));
    web13.depends_on = vec!["PAY-3".to_owned()];
    out.push((web13, None));

    for (key, title, until) in [
        ("OPS-1", "Check the Stripe fee change", day(11, 23)),
        ("OPS-2", "Archive last quarter's exports", day(11, 30)),
        ("OPS-3", "Plan the December on-call rota", day(12, 1)),
    ] {
        let mut hidden = item(key, title, None, ago(4));
        hidden.hide_until = until;
        out.push((hidden, None));
    }

    out.into_iter()
        .map(|(todo, ask)| Entry { todo, revision: 0, ask_id: ask.map(str::to_owned) })
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use humata2_todo::{BlockerGone, LifecyclePrompt, RunLink, StuckAgent, WordDiffKind};

    #[tokio::test]
    async fn fake_rejects_dependency_cycles() {
        let fake = FakeTodo::new();
        let a = fake
            .create(NewTodo { title: "A".to_owned(), ..NewTodo::default() })
            .await
            .expect("create A");
        let b = fake
            .create(NewTodo { title: "B".to_owned(), ..NewTodo::default() })
            .await
            .expect("create B");

        // Self cycle rejected.
        let self_err = fake
            .update(
                &a.todo.key,
                TodoPatch { depends_on_add: vec![a.todo.key.clone()], ..TodoPatch::default() },
                a.revision,
            )
            .await;
        assert!(self_err.is_err());

        // A -> B succeeds.
        let a2 = fake
            .update(
                &a.todo.key,
                TodoPatch { depends_on_add: vec![b.todo.key.clone()], ..TodoPatch::default() },
                a.revision,
            )
            .await
            .expect("A depends on B");
        assert_eq!(a2.todo.depends_on, vec![b.todo.key.clone()]);

        // B -> A rejected as cycle.
        let cycle_err = fake
            .update(
                &b.todo.key,
                TodoPatch { depends_on_add: vec![a.todo.key.clone()], ..TodoPatch::default() },
                b.revision,
            )
            .await;
        assert!(cycle_err.is_err());
    }

    #[tokio::test]
    async fn fake_bulk_update_retains_partial_failures() {
        let fake = FakeTodo::new();
        let a = fake
            .create(NewTodo { title: "A".to_owned(), ..NewTodo::default() })
            .await
            .expect("create A");
        let b = fake
            .create(NewTodo { title: "B".to_owned(), ..NewTodo::default() })
            .await
            .expect("create B");

        // Bump B's revision so the bulk update sees a conflict on B only.
        fake.touch(&b.todo.key);

        let res = fake
            .bulk_update(vec![
                BulkUpdateItem {
                    key: a.todo.key.clone(),
                    patch: TodoPatch { state: Some(State::Closed), ..TodoPatch::default() },
                    expected_revision: a.revision,
                },
                BulkUpdateItem {
                    key: b.todo.key.clone(),
                    patch: TodoPatch { state: Some(State::Closed), ..TodoPatch::default() },
                    expected_revision: b.revision,
                },
            ])
            .await
            .expect("bulk update");

        assert_eq!(res.updated.len(), 1);
        assert_eq!(res.updated[0].todo.key, a.todo.key);
        assert_eq!(res.updated[0].todo.state, State::Closed);
        assert_eq!(res.failed.len(), 1);
        assert_eq!(res.failed[0].key, b.todo.key);
    }

    #[tokio::test]
    async fn fake_split_and_fold_maintain_bidirectional_lineage() {
        let fake = FakeTodo::new();
        let orig = fake
            .create(NewTodo { title: "Big task".to_owned(), ..NewTodo::default() })
            .await
            .expect("create");

        let split = fake
            .split(&orig.todo.key, vec!["Part 1".to_owned(), "Part 2".to_owned()], true)
            .await
            .expect("split");
        assert_eq!(split.source.todo.state, State::Became);
        assert_eq!(split.created.len(), 2);
        assert_eq!(
            split.source.todo.became_keys,
            vec![split.created[0].todo.key.clone(), split.created[1].todo.key.clone()]
        );
        assert_eq!(split.created[0].todo.from_keys, vec![orig.todo.key.clone()]);
        assert_eq!(split.created[0].todo.created, orig.todo.created);

        let folded = fake
            .fold_into_step(&split.created[1].todo.key, &split.created[0].todo.key)
            .await
            .expect("fold");
        assert_eq!(folded.source.todo.state, State::Became);
        assert_eq!(folded.source.todo.became_keys, vec![split.created[0].todo.key.clone()]);
        assert!(folded.target.todo.from_keys.contains(&split.created[1].todo.key));
        assert_eq!(folded.target.todo.steps.len(), 1);
        assert_eq!(
            folded.target.todo.steps[0].from_key.as_deref(),
            Some(split.created[1].todo.key.as_str())
        );
    }

    #[tokio::test]
    async fn fake_records_word_diff_history_runs_and_structured_asks() {
        let fake = FakeTodo::new();
        let a = fake
            .create(NewTodo { title: "Fix login loop".to_owned(), ..NewTodo::default() })
            .await
            .expect("create");
        let a = fake
            .update(
                &a.todo.key,
                TodoPatch { title: Some("Fix auth loop".to_owned()), ..TodoPatch::default() },
                a.revision,
            )
            .await
            .expect("update");
        let hist = fake.history(&a.todo.key).await.expect("history");
        let title_edit = hist.iter().find(|h| h.field == "title").expect("title edit");
        assert!(title_edit.word_diff.iter().any(|s| s.kind == WordDiffKind::Removed && s.text == "login"));
        assert!(title_edit.word_diff.iter().any(|s| s.kind == WordDiffKind::Added && s.text == "auth"));

        fake.add_run(
            &a.todo.key,
            RunEntry {
                id: "run-1".to_owned(),
                tool: "claude".to_owned(),
                model: "opus".to_owned(),
                outcome: "ok".to_owned(),
                cost_micros: Some(1_250_000),
                started: Some(fake.today()),
                links: vec![RunLink { label: "PR #1".to_owned(), target: "pr:1".to_owned() }],
                session: Some("s-1".to_owned()),
            },
        );
        let runs = fake.runs(&a.todo.key).await.expect("runs");
        assert_eq!(runs.runs.len(), 1);
        assert_eq!(runs.total_cost_micros, Some(1_250_000));

        // Structured asks & resolution methods.
        let mut q = Todo::new("Q-1", "Needs answer", fake.today());
        q.ask = Some(Ask {
            kind: AskKind::Question {
                question: "Which DB?".to_owned(),
                choices: vec!["Postgres".to_owned(), "SQLite".to_owned()],
                multi_select: false,
                step_id: None,
                details: None,
                editable: true,
            },
            by: "claude".to_owned(),
            reason: "schema".to_owned(),
        });
        q.stuck_agent = Some(StuckAgent {
            session: "s-1".to_owned(),
            agent_label: "claude".to_owned(),
            reason: "looping".to_owned(),
            since_minutes: 15,
        });
        q.blocker_gone.push(BlockerGone {
            blocker_key: "OLD-1".to_owned(),
            blocker_state: State::WontDo,
            replacement_keys: Vec::new(),
        });
        q.lifecycle_prompts.push(LifecyclePrompt {
            id: "lp-1".to_owned(),
            kind: "pr_merged".to_owned(),
            title: "PR merged".to_owned(),
            detail: String::new(),
            choices: vec!["close".to_owned(), "keep".to_owned()],
            pr_ref: None,
            session: None,
        });
        fake.insert(q, Some("ask-q-1"));

        fake.answer_question("ask-q-1", String::new(), vec!["Postgres".to_owned()])
            .await
            .expect("answer question");
        let taken = fake.resolve_ownership("Q-1", true, false).await.expect("take back");
        assert_eq!(taken.todo.owner.as_ref().map(|o| o.kind), Some(OwnerKind::User));
        assert!(taken.todo.stuck_agent.is_none());

        let unblocked = fake
            .resolve_blocker_gone("Q-1", "OLD-1", BlockerGoneResolution::Unblock)
            .await
            .expect("resolve blocker");
        assert!(unblocked.todo.blocker_gone.is_empty());

        let closed = fake
            .resolve_lifecycle_prompt("Q-1", "lp-1", "close")
            .await
            .expect("resolve lifecycle");
        assert_eq!(closed.todo.state, State::Closed);
        assert!(closed.todo.lifecycle_prompts.is_empty());
    }
}
