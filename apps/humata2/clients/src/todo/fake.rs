//! A scripted [`TodoClient`]: it keeps the list the way the host does.
//! Clones share it, so two clones stand for two apps open at once. No
//! timers, no real IO.

use std::sync::{Arc, Mutex, MutexGuard};

use humata2_todo::{
    Ask, AskDetail, AskKind, ChannelFilter, Evidence, Day, Review, State, Step, Summary, Todo,
};
use tokio::sync::mpsc;

use super::{Entry, MigrationNotice, NewTodo, TodoClient, TodoFilter, TodoLists, TodoPatch};
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
}

/// Lists a watcher can fall behind by before the fake drops it.
const WATCH_BUFFER: usize = 64;

struct Inner {
    entries: Vec<Entry>,
    today: Day,
    next_number: u32,
    migration: Option<MigrationNotice>,
    watchers: Vec<mpsc::Sender<Vec<Entry>>>,
    failure: Option<DaemonError>,
    calls: Vec<Call>,
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
                migration: None,
                watchers: Vec::new(),
                failure: None,
                calls: Vec::new(),
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

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The day the fake calls today.
    #[must_use]
    pub fn today(&self) -> Day {
        self.inner().today
    }

    /// Adds `todo` as it is, under `ask_id` if it has an ask.
    pub fn insert(&self, todo: Todo, ask_id: Option<&str>) {
        self.inner().entries.push(Entry {
            todo,
            revision: 0,
            ask_id: ask_id.map(str::to_owned),
        });
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
                    let closed = filter.show_closed || t.state != State::Closed;
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
            todo.effort = draft.effort;
            todo.impact = draft.impact;
            todo.tags = draft.tags;
            todo.description = draft.description;
            let entry = Entry { todo, revision: 0, ask_id: None };
            inner.entries.push(entry.clone());
            inner.changed();
            Ok(entry)
        })
    }

    fn update(&self, key: &str, patch: TodoPatch, expected_revision: u64) -> Reply<Entry> {
        self.call(Call::Update(key.to_owned(), patch.clone(), expected_revision), |inner| {
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
            patch.apply(&mut entry.todo, today);
            entry.revision += 1;
            let done = entry.clone();
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
            .map(|i| Step { text: format!("Step {}", i + 1), done: i < done })
            .collect::<Vec<_>>()
    };
    let tags = |t: &[&str]| t.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let mut out: Vec<(Todo, Option<&str>)> = Vec::new();

    let mut pay3 = item("PAY-3", "Ledger schema v2", Some("payments-rewrite"), ago(14));
    pay3.state = State::InProgress;
    pay3.due = day(11, 20);
    pay3.tags = tags(&["migration", "db"]);
    pay3.steps = steps(4, 5);
    pay3.effort = Some(humata2_todo::Effort::M);
    pay3.impact = Some(humata2_todo::Impact::High);
    pay3.sessions = vec!["Ledger entries".to_owned()];
    pay3.ask = Some(Ask {
        kind: AskKind::Close,
        by: "claude in Ledger entries".to_owned(),
        reason: "Migration and backfill pass on staging.".to_owned(),
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
