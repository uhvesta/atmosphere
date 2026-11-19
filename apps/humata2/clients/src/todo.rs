//! The To-do list, as the host keeps it (BOARDSIMPLE).
//!
//! One list for every channel and session; channel and session are filters
//! on it. The app ranks what it gets with `humata2_todo::rank`, so the
//! client only stores, filters and writes. [`FakeTodo`] is the only
//! implementation until the live client serves it.

use humata2_todo::{ChannelFilter, Day, Draft, Effort, Impact, State, Step, Summary, Todo};
use tokio::sync::mpsc;

use crate::daemon::Reply;

/// An item as the host stores it: the item, which version of it this is,
/// and the request an agent's pending ask waits on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The item.
    pub todo: Todo,
    /// Goes up with every change to this item, from any client. A write
    /// names the one it was made from.
    pub revision: u64,
    /// The inbox request a pending ask on this item waits on; answer it
    /// with [`TodoClient::answer`].
    pub ask_id: Option<String>,
}

/// What [`TodoClient::list`] keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TodoFilter {
    /// One channel, or the items with none; `None` is every channel.
    pub channel: Option<ChannelFilter>,
    /// Items a session works on.
    pub session: Option<String>,
    /// Closed items from the last 7 days too.
    pub show_closed: bool,
}

/// A new item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewTodo {
    /// Its title; the only thing needed.
    pub title: String,
    /// Its channel, or none.
    pub channel: Option<String>,
    /// When it is due.
    pub due: Option<Day>,
    /// How big the job is.
    pub effort: Option<Effort>,
    /// How much it matters.
    pub impact: Option<Impact>,
    /// Its tags.
    pub tags: Vec<String>,
    /// Its longer text.
    pub description: String,
}

impl From<Draft> for NewTodo {
    fn from(draft: Draft) -> Self {
        Self {
            title: draft.title,
            channel: draft.channel,
            due: draft.due,
            effort: draft.effort,
            impact: draft.impact,
            tags: draft.tags,
            description: String::new(),
        }
    }
}

/// A change to one item: only what is set changes. For the fields that can
/// be taken off, `Some(None)` takes it off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TodoPatch {
    /// A new title.
    pub title: Option<String>,
    /// A new state. Closing takes the pin off; reopening leaves it off.
    pub state: Option<State>,
    /// A new due date, or none.
    pub due: Option<Option<Day>>,
    /// A new effort, or none.
    pub effort: Option<Option<Effort>>,
    /// A new impact, or none.
    pub impact: Option<Option<Impact>>,
    /// Hide until a day, or stop hiding.
    pub hide_until: Option<Option<Day>>,
    /// Pin at this place among the pins (smaller first), or unpin.
    pub pin: Option<Option<u32>>,
    /// Tags to add.
    pub tags_add: Vec<String>,
    /// Tags to take off.
    pub tags_remove: Vec<String>,
    /// All the steps, replacing the old ones.
    pub steps: Option<Vec<Step>>,
    /// A new description.
    pub description: Option<String>,
    /// The person said yes to a suggested item.
    pub accept_suggestion: bool,
}

impl TodoPatch {
    /// Makes `todo` what it is after this change. `today` is the day a
    /// closing is dated. The host and the app both use it, so what the app
    /// shows before the host answers is what the host then holds.
    pub fn apply(&self, todo: &mut Todo, today: Day) {
        if let Some(title) = &self.title {
            todo.title.clone_from(title);
        }
        if let Some(state) = self.state {
            todo.state = state;
            if state == State::Closed {
                todo.closed = Some(today);
                todo.pin = None;
            } else {
                todo.closed = None;
            }
        }
        if let Some(due) = self.due {
            todo.due = due;
        }
        if let Some(effort) = self.effort {
            todo.effort = effort;
        }
        if let Some(impact) = self.impact {
            todo.impact = impact;
        }
        if let Some(until) = self.hide_until {
            todo.hide_until = until;
        }
        if let Some(pin) = self.pin {
            todo.pin = pin;
        }
        for tag in &self.tags_add {
            if !todo.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                todo.tags.push(tag.clone());
            }
        }
        todo.tags.retain(|t| !self.tags_remove.iter().any(|r| r.eq_ignore_ascii_case(t)));
        if let Some(steps) = &self.steps {
            todo.steps.clone_from(steps);
        }
        if let Some(description) = &self.description {
            todo.description.clone_from(description);
        }
        if self.accept_suggestion {
            todo.suggested = false;
        }
    }
}

/// What the boards became, shown until it is dismissed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationNotice {
    /// The counts of what moved.
    pub summary: Summary,
}

/// The stream [`TodoClient::watch`] returns: the whole list after each
/// change, from any client.
pub type TodoLists = mpsc::Receiver<Vec<Entry>>;

/// What the app asks the host about the To-do list.
pub trait TodoClient: Send + Sync + 'static {
    /// The items `filter` keeps.
    fn list(&self, filter: &TodoFilter) -> Reply<Vec<Entry>>;

    /// Adds an item. Answers with it, key and all.
    fn create(&self, draft: NewTodo) -> Reply<Entry>;

    /// Changes the item `key`. `expected_revision` is the revision the
    /// change was made from: when the item has changed since, on any
    /// client, the host refuses and nothing changes, so list again.
    fn update(&self, key: &str, patch: TodoPatch, expected_revision: u64) -> Reply<Entry>;

    /// Answers an agent's pending ask by its request id.
    fn answer(&self, ask_id: &str, yes: bool) -> Reply<()>;

    /// What the boards became, until it is dismissed.
    fn migration(&self) -> Reply<Option<MigrationNotice>>;

    /// Dismisses the migration notice for good.
    fn dismiss_migration(&self) -> Reply<()>;

    /// Follows the list from now on: all of it after each change.
    fn watch(&self) -> Reply<TodoLists>;
}
