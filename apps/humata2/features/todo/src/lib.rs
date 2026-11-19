//! The To-do list (BOARDSIMPLE): one ranked list for every channel and
//! session, which are filters on it. Items have three states, a computed
//! rank with reasons, and Pin; an agent's pending ask shows as Needs
//! attention with Yes and No; Do next opens the top item.
//!
//! Ranking is `humata2_todo::rank`, run over what is shown whenever the
//! view asks, so the order is never kept in the state. Writes show at once
//! and are reconciled with the host's reply; a refused write puts the item
//! back and says why.

use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::todo::{Entry, MigrationNotice, NewTodo, TodoClient, TodoFilter, TodoPatch};
use humata2_todo::{
    ChannelFilter, Day, Draft, DueFilter, Effort, Filter, Impact, Only, Ranked, State, Step,
    Todo as Item, apply, rank,
};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

const WATCH: &str = "todo-watch";

/// Which channel the list is narrowed to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelPick {
    /// One channel.
    Channel(String),
    /// Items with no channel.
    NoChannel,
}

/// A due filter chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DuePick {
    /// Past due.
    Overdue,
    /// Due in the next 7 days.
    Next7Days,
    /// Due later than that.
    AfterThisWeek,
    /// No due date.
    NoDate,
}

/// The "Only" filter's choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnlyPick {
    /// An agent asks about it, or it is suggested.
    NeedsAttention,
    /// A pull request waiting for review.
    PullRequests,
}

/// What narrows the list: channel and session, then due, tags and search.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewFilter {
    /// A channel, or the items with none; unset is every channel.
    pub channel: Option<ChannelPick>,
    /// One session's items.
    pub session: Option<String>,
    /// Due chips; an item matches any of them.
    pub due: Vec<DuePick>,
    /// Tags; an item matches any of them.
    pub tags: Vec<String>,
    /// Text in the title, key or a tag.
    pub search: String,
    /// Efforts; an item matches any of them.
    pub effort: Vec<Effort>,
    /// Impacts; an item matches any of them.
    pub impact: Vec<Impact>,
    /// Kinds of item; an item matches any of them.
    pub only: Vec<OnlyPick>,
}

impl ViewFilter {
    /// The rules' filter for this.
    #[must_use]
    pub fn to_filter(&self) -> Filter {
        Filter {
            channel: self.channel.as_ref().map(|c| match c {
                ChannelPick::Channel(name) => ChannelFilter::Channel(name.clone()),
                ChannelPick::NoChannel => ChannelFilter::NoChannel,
            }),
            session: self.session.clone(),
            due: self
                .due
                .iter()
                .map(|d| match d {
                    DuePick::Overdue => DueFilter::Overdue,
                    DuePick::Next7Days => DueFilter::Next7Days,
                    DuePick::AfterThisWeek => DueFilter::AfterThisWeek,
                    DuePick::NoDate => DueFilter::NoDate,
                })
                .collect(),
            tags: self.tags.clone(),
            search: self.search.clone(),
            effort: self.effort.clone(),
            impact: self.impact.clone(),
            only: self
                .only
                .iter()
                .map(|o| match o {
                    OnlyPick::NeedsAttention => Only::NeedsAttention,
                    OnlyPick::PullRequests => Only::PullRequests,
                })
                .collect(),
        }
    }
}

/// Presentation retained across restarts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    /// What narrows the list.
    pub filter: ViewFilter,
    /// Closed items from the last 7 days are shown.
    pub show_closed: bool,
}

/// What a typed quick-add line would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    /// What was read off the line.
    pub draft: Draft,
    /// Its place among the Next rows of the whole list, 1 is the top.
    pub position: Option<u32>,
}

impl Preview {
    /// Where it would land, as the line under the field says it.
    #[must_use]
    pub fn text(&self) -> String {
        match self.position {
            Some(n) => format!("It would be {} in your list", ordinal(n)),
            None => "It would be added to your list".to_owned(),
        }
    }
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// The line that adds an item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuickAdd {
    /// What is typed.
    pub line: String,
    /// What it reads as; unset while no title is typed.
    pub preview: Option<Preview>,
}

/// A field of the open item being typed into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditField {
    /// The title.
    Title,
    /// A new tag.
    Tag,
    /// A new step.
    Step,
    /// The description.
    Description,
}

/// The field being typed into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editing {
    /// The item.
    pub key: String,
    /// The field.
    pub field: EditField,
    /// What is typed.
    pub text: String,
}

/// Everything the To-do page and panel draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodoState {
    /// Every item the host listed, with its revision.
    pub items: Vec<Entry>,
    /// Today, as the reducer last read it.
    pub today: Day,
    /// Presentation retained across restarts.
    pub view: View,
    /// The item open in the drawer.
    pub open: Option<String>,
    /// The line that adds an item.
    pub quick_add: QuickAdd,
    /// The drawer field being typed into.
    pub editing: Option<Editing>,
    /// What the boards became, until dismissed.
    pub migration: Option<MigrationNotice>,
    /// The item the Do next overlay offers.
    pub do_next: Option<String>,
    /// The migration banner shows what changed in full.
    pub what_changed: bool,
    /// Whether the list has loaded once.
    pub loaded: bool,
    /// Why the last call failed, for the person to fix.
    pub error: Option<DaemonError>,
}

impl TodoState {
    /// An empty list on `today`, shown as `view`.
    #[must_use]
    pub fn new(view: View, today: Day) -> Self {
        Self {
            items: Vec::new(),
            today,
            view,
            open: None,
            quick_add: QuickAdd::default(),
            editing: None,
            migration: None,
            do_next: None,
            what_changed: false,
            loaded: false,
            error: None,
        }
    }

    fn all(&self) -> Vec<Item> {
        self.items.iter().map(|e| e.todo.clone()).collect()
    }

    /// The items the filter shows; closed ones only when asked for.
    #[must_use]
    pub fn shown(&self) -> Vec<Item> {
        let mut shown = apply(&self.all(), &self.view.filter.to_filter(), self.today);
        if !self.view.show_closed {
            shown.retain(|t| t.state != State::Closed);
        }
        shown
    }

    /// The shown items in their groups, in rank order.
    #[must_use]
    pub fn ranked(&self) -> Ranked {
        rank(&self.shown(), self.today)
    }

    /// Open items and channels among them, for "10 open across 4 channels".
    #[must_use]
    pub fn header(&self) -> (usize, usize) {
        humata2_todo::header(&self.shown(), self.today)
    }

    /// The channels items are in, for the quick-add parser and the chips.
    #[must_use]
    pub fn channels(&self) -> Vec<String> {
        let mut channels: Vec<String> = Vec::new();
        for c in self.items.iter().filter_map(|e| e.todo.channel.as_ref()) {
            if !channels.contains(c) {
                channels.push(c.clone());
            }
        }
        channels
    }

    /// Open items per channel for the chip row, largest first.
    #[must_use]
    pub fn channel_counts(&self) -> Vec<(Option<String>, usize)> {
        humata2_todo::channel_counts(&self.all(), self.today)
    }

    /// Items the due filter drops that every other filter keeps.
    #[must_use]
    pub fn left_out(&self) -> Vec<Item> {
        humata2_todo::left_out(&self.all(), &self.view.filter.to_filter(), self.today)
    }

    fn position(&self, key: &str) -> Option<usize> {
        self.items.iter().position(|e| e.todo.key == key)
    }

    fn replace(&mut self, entry: Entry) {
        match self.position(&entry.todo.key) {
            Some(at) => self.items[at] = entry,
            None => self.items.push(entry),
        }
    }

    /// What a typed line would add, and where it would land.
    fn preview(&self, line: &str) -> Option<Preview> {
        let draft = humata2_todo::parse(line, self.today, &self.channels())?;
        const NEW: &str = "\u{0}new";
        let mut item = Item::new(NEW, &draft.title, self.today);
        item.channel.clone_from(&draft.channel);
        item.due = draft.due;
        item.effort = draft.effort;
        item.impact = draft.impact;
        item.tags.clone_from(&draft.tags);
        let mut all = self.all();
        all.push(item);
        let position = rank(&all, self.today)
            .next
            .iter()
            .find(|r| r.key == NEW)
            .and_then(|r| r.position);
        Some(Preview { draft, position })
    }
}

/// What the To-do list is told.
#[derive(Clone, Debug, PartialEq)]
pub enum TodoAction {
    /// Lists everything and reads the migration notice.
    Load,
    /// The list, and the migration notice if there is one.
    Loaded(Vec<Entry>, Option<MigrationNotice>),
    /// Loading failed.
    Failed(DaemonError),
    /// Follows the host's changes from now on.
    Watch,
    /// The host's list after a change, from any screen.
    Updated(Vec<Entry>),

    /// Narrows the list.
    SetFilter(ViewFilter),
    /// Sets the search text and keeps the rest of the filter.
    SetSearch(String),
    /// Shows everything again; Show closed stays as it is.
    ClearFilter,
    /// Shows or hides this week's closed items.
    ShowClosed(bool),

    /// The quick-add line changed.
    QuickAddChanged(String),
    /// Adds what the quick-add line says.
    QuickAddSubmit,
    /// The host's answer to an add.
    Created(Result<Entry, DaemonError>),

    /// Opens the item's drawer.
    Open(String),
    /// Closes the drawer.
    Close,
    /// Starts typing into a drawer field.
    StartEdit(String, EditField),
    /// The typed text changed.
    EditChanged(String),
    /// Saves the typed text into its field.
    CommitEdit,
    /// Stops typing without saving.
    CancelEdit,

    /// Sets the state: open, in progress, or closed; closed reopens.
    SetState(String, State),
    /// Pins after the other pins.
    Pin(String),
    /// Takes the pin off.
    Unpin(String),
    /// Puts a pin at a place among the pins, 0 is first.
    MovePin(String, usize),
    /// Sets the due date.
    SetDue(String, Day),
    /// Takes the due date off.
    ClearDue(String),
    /// Sets or clears the effort.
    SetEffort(String, Option<Effort>),
    /// Sets or clears the impact.
    SetImpact(String, Option<Impact>),
    /// Adds a tag.
    AddTag(String, String),
    /// Takes a tag off.
    RemoveTag(String, String),
    /// Ticks or unticks the step at this place.
    ToggleStep(String, usize),
    /// Hides until a day, or stops hiding.
    HideUntil(String, Option<Day>),
    /// Says yes to a suggested item.
    AcceptSuggestion(String),
    /// Says no to a suggested item.
    DismissSuggestion(String),
    /// The host's answer to a write. `before` is the item as it was, to
    /// put back if the write was refused.
    Written {
        /// The item before the change.
        before: Entry,
        /// What the host answered.
        result: Result<Entry, DaemonError>,
    },

    /// Answers an agent's ask by its request id.
    Answer(String, bool),
    /// The host's answer to an answer; `before` is the item as it was.
    Answered {
        /// The item before the answer.
        before: Entry,
        /// What the host answered.
        result: Result<(), DaemonError>,
    },

    /// Opens the Do next overlay on the top Next row among those shown.
    DoNext,
    /// Moves the overlay on to the next row.
    SkipDoNext,
    /// Closes the overlay.
    CloseDoNext,

    /// Asks the agent why. The app routes it; the list does not change.
    AskWhy(String),
    /// Opens the session an item belongs to. The app routes it.
    OpenSession(String),
    /// Opens an item's history. The app routes it.
    ShowHistory(String),
    /// Shows or hides the full account of what the migration changed.
    ShowWhatChanged,
    /// Dismisses the migration banner.
    DismissMigration,
    /// The host's answer to a dismissal; `before` is the notice.
    Dismissed {
        /// The notice that was shown.
        before: Option<MigrationNotice>,
        /// What the host answered.
        result: Result<(), DaemonError>,
    },
}

/// Reads the date the list is ranked on.
type Today = Arc<dyn Fn() -> Day + Send + Sync>;

/// The To-do reducer, over the host's client.
pub struct Todo {
    client: Arc<dyn TodoClient>,
    today: Today,
}

impl Todo {
    /// A reducer over `client`, ranking on the day `today` answers.
    pub fn new(
        client: Arc<dyn TodoClient>,
        today: impl Fn() -> Day + Send + Sync + 'static,
    ) -> Self {
        Self { client, today: Arc::new(today) }
    }

    fn fetch(&self) -> Effect<TodoAction> {
        let client = self.client.clone();
        // Closed items come too: Show closed is a view, not another list.
        let list = client.list(&TodoFilter { show_closed: true, ..TodoFilter::default() });
        let migration = client.migration();
        Effect::future(async move {
            match list.await {
                // A notice that can't be read is not worth blocking the list on.
                Ok(items) => TodoAction::Loaded(items, migration.await.unwrap_or(None)),
                Err(error) => TodoAction::Failed(error),
            }
        })
    }

    /// Shows `patch` on the item now and asks the host for it.
    fn write(&self, state: &mut TodoState, key: &str, patch: TodoPatch) -> Effect<TodoAction> {
        let Some(at) = state.position(key) else {
            return Effect::none();
        };
        let before = state.items[at].clone();
        patch.apply(&mut state.items[at].todo, state.today);
        // The host's revision after this write, so a second write before the
        // reply names the right one.
        state.items[at].revision += 1;
        state.error = None;
        let reply = self.client.update(key, patch, before.revision);
        Effect::future(async move { TodoAction::Written { before, result: reply.await } })
    }

    fn patch(&self, state: &mut TodoState, key: &str, patch: TodoPatch) -> Effect<TodoAction> {
        self.write(state, key, patch)
    }

    fn pins(state: &TodoState) -> Vec<String> {
        let mut pinned: Vec<(u32, &str)> = state
            .items
            .iter()
            .filter_map(|e| e.todo.pin.map(|p| (p, e.todo.key.as_str())))
            .collect();
        pinned.sort();
        pinned.into_iter().map(|(_, k)| k.to_owned()).collect()
    }
}

impl Reducer for Todo {
    type State = TodoState;
    type Action = TodoAction;

    fn reduce(&self, state: &mut TodoState, action: TodoAction) -> Effect<TodoAction> {
        state.today = (self.today)();
        match action {
            TodoAction::Load => self.fetch(),
            TodoAction::Loaded(items, migration) => {
                state.items = items;
                state.migration = migration;
                state.loaded = true;
                state.error = None;
                if state.open.as_deref().is_some_and(|k| state.position(k).is_none()) {
                    state.open = None;
                }
                Effect::none()
            }
            TodoAction::Failed(error) => {
                state.error = Some(error);
                Effect::none()
            }
            TodoAction::Watch => {
                let client = self.client.clone();
                Effect::run(move |send| async move {
                    let mut lists = match client.watch().await {
                        Ok(lists) => lists,
                        Err(error) => {
                            send.send(TodoAction::Failed(error));
                            return;
                        }
                    };
                    loop {
                        send.ready().await;
                        match lists.recv().await {
                            Some(items) => send.send(TodoAction::Updated(items)),
                            None => return,
                        }
                    }
                })
                .cancellable(WATCH, true)
            }
            TodoAction::Updated(items) => {
                state.items = items;
                if state.open.as_deref().is_some_and(|k| state.position(k).is_none()) {
                    state.open = None;
                }
                Effect::none()
            }

            TodoAction::SetFilter(filter) => {
                state.view.filter = filter;
                state.do_next = None;
                Effect::none()
            }
            TodoAction::SetSearch(text) => {
                state.view.filter.search = text;
                state.do_next = None;
                Effect::none()
            }
            TodoAction::AskWhy(_) | TodoAction::OpenSession(_) | TodoAction::ShowHistory(_) => {
                Effect::none()
            }
            TodoAction::ShowWhatChanged => {
                state.what_changed = !state.what_changed;
                Effect::none()
            }
            TodoAction::ClearFilter => {
                state.view.filter = ViewFilter::default();
                state.do_next = None;
                Effect::none()
            }
            TodoAction::ShowClosed(on) => {
                state.view.show_closed = on;
                Effect::none()
            }

            TodoAction::QuickAddChanged(line) => {
                state.quick_add.preview = state.preview(&line);
                state.quick_add.line = line;
                Effect::none()
            }
            TodoAction::QuickAddSubmit => {
                let Some(preview) = state.quick_add.preview.clone() else {
                    return Effect::none();
                };
                state.error = None;
                let reply = self.client.create(NewTodo::from(preview.draft));
                Effect::future(async move { TodoAction::Created(reply.await) })
            }
            TodoAction::Created(Ok(entry)) => {
                state.replace(entry);
                state.quick_add = QuickAdd::default();
                Effect::none()
            }
            TodoAction::Created(Err(error)) => {
                state.error = Some(error);
                Effect::none()
            }

            TodoAction::Open(key) => {
                if state.position(&key).is_some() {
                    state.open = Some(key);
                }
                state.editing = None;
                Effect::none()
            }
            TodoAction::Close => {
                state.open = None;
                state.editing = None;
                Effect::none()
            }
            TodoAction::StartEdit(key, field) => {
                let text = match (field, state.position(&key)) {
                    (EditField::Title, Some(at)) => state.items[at].todo.title.clone(),
                    (EditField::Description, Some(at)) => state.items[at].todo.description.clone(),
                    _ => String::new(),
                };
                state.editing = Some(Editing { key, field, text });
                Effect::none()
            }
            TodoAction::EditChanged(text) => {
                if let Some(editing) = &mut state.editing {
                    editing.text = text;
                }
                Effect::none()
            }
            TodoAction::CommitEdit => {
                let Some(editing) = state.editing.take() else {
                    return Effect::none();
                };
                let text = editing.text.trim().to_owned();
                // A description may be cleared; the other fields need text.
                if text.is_empty() && editing.field != EditField::Description {
                    return Effect::none();
                }
                let key = editing.key;
                match editing.field {
                    EditField::Title => self.patch(
                        state,
                        &key,
                        TodoPatch { title: Some(text), ..TodoPatch::default() },
                    ),
                    EditField::Description => self.patch(
                        state,
                        &key,
                        TodoPatch { description: Some(text), ..TodoPatch::default() },
                    ),
                    EditField::Tag => self.patch(
                        state,
                        &key,
                        TodoPatch { tags_add: vec![text], ..TodoPatch::default() },
                    ),
                    EditField::Step => {
                        let Some(at) = state.position(&key) else {
                            return Effect::none();
                        };
                        let mut steps = state.items[at].todo.steps.clone();
                        steps.push(Step { text, done: false });
                        self.patch(
                            state,
                            &key,
                            TodoPatch { steps: Some(steps), ..TodoPatch::default() },
                        )
                    }
                }
            }
            TodoAction::CancelEdit => {
                state.editing = None;
                Effect::none()
            }

            TodoAction::SetState(key, to) => {
                self.patch(state, &key, TodoPatch { state: Some(to), ..TodoPatch::default() })
            }
            TodoAction::Pin(key) => {
                let next = u32::try_from(Self::pins(state).len()).unwrap_or(u32::MAX);
                self.patch(state, &key, TodoPatch { pin: Some(Some(next)), ..TodoPatch::default() })
            }
            TodoAction::Unpin(key) => {
                self.patch(state, &key, TodoPatch { pin: Some(None), ..TodoPatch::default() })
            }
            TodoAction::MovePin(key, to) => {
                let mut order = Self::pins(state);
                let Some(from) = order.iter().position(|k| *k == key) else {
                    return Effect::none();
                };
                let moved = order.remove(from);
                order.insert(to.min(order.len()), moved);
                let mut writes = Vec::new();
                for (place, k) in order.iter().enumerate() {
                    let place = u32::try_from(place).unwrap_or(u32::MAX);
                    let current = state.position(k).and_then(|at| state.items[at].todo.pin);
                    if current != Some(place) {
                        writes.push(self.write(
                            state,
                            k,
                            TodoPatch { pin: Some(Some(place)), ..TodoPatch::default() },
                        ));
                    }
                }
                Effect::merge(writes)
            }
            TodoAction::SetDue(key, day) => {
                self.patch(state, &key, TodoPatch { due: Some(Some(day)), ..TodoPatch::default() })
            }
            TodoAction::ClearDue(key) => {
                self.patch(state, &key, TodoPatch { due: Some(None), ..TodoPatch::default() })
            }
            TodoAction::SetEffort(key, effort) => {
                self.patch(state, &key, TodoPatch { effort: Some(effort), ..TodoPatch::default() })
            }
            TodoAction::SetImpact(key, impact) => {
                self.patch(state, &key, TodoPatch { impact: Some(impact), ..TodoPatch::default() })
            }
            TodoAction::AddTag(key, tag) => {
                self.patch(state, &key, TodoPatch { tags_add: vec![tag], ..TodoPatch::default() })
            }
            TodoAction::RemoveTag(key, tag) => self.patch(
                state,
                &key,
                TodoPatch { tags_remove: vec![tag], ..TodoPatch::default() },
            ),
            TodoAction::ToggleStep(key, index) => {
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                let mut steps = state.items[at].todo.steps.clone();
                let Some(step) = steps.get_mut(index) else {
                    return Effect::none();
                };
                step.done = !step.done;
                self.patch(state, &key, TodoPatch { steps: Some(steps), ..TodoPatch::default() })
            }
            TodoAction::HideUntil(key, day) => self.patch(
                state,
                &key,
                TodoPatch { hide_until: Some(day), ..TodoPatch::default() },
            ),
            TodoAction::AcceptSuggestion(key) => self.patch(
                state,
                &key,
                TodoPatch { accept_suggestion: true, ..TodoPatch::default() },
            ),
            TodoAction::DismissSuggestion(key) => self.patch(
                state,
                &key,
                TodoPatch { state: Some(State::Closed), ..TodoPatch::default() },
            ),
            TodoAction::Written { before, result } => {
                match result {
                    Ok(entry) => state.replace(entry),
                    Err(error) => {
                        state.replace(before);
                        state.error = Some(error);
                    }
                }
                Effect::none()
            }

            TodoAction::Answer(ask_id, yes) => {
                let Some(at) = state
                    .items
                    .iter()
                    .position(|e| e.ask_id.as_deref() == Some(ask_id.as_str()))
                else {
                    return Effect::none();
                };
                let before = state.items[at].clone();
                state.items[at].todo.ask = None;
                state.items[at].ask_id = None;
                state.error = None;
                let reply = self.client.answer(&ask_id, yes);
                Effect::future(async move {
                    TodoAction::Answered { before, result: reply.await }
                })
            }
            TodoAction::Answered { before, result } => match result {
                // The host acted on the answer: ask for the list as it is now.
                Ok(()) => self.fetch(),
                Err(error) => {
                    state.replace(before);
                    state.error = Some(error);
                    Effect::none()
                }
            },

            TodoAction::DoNext => {
                state.do_next = state.ranked().do_next;
                Effect::none()
            }
            TodoAction::SkipDoNext => {
                let ranked = state.ranked();
                let at = ranked.next.iter().position(|r| Some(&r.key) == state.do_next.as_ref());
                if let Some(row) = at.and_then(|i| ranked.next.get(i + 1)) {
                    state.do_next = Some(row.key.clone());
                }
                Effect::none()
            }
            TodoAction::CloseDoNext => {
                state.do_next = None;
                Effect::none()
            }

            TodoAction::DismissMigration => {
                let before = state.migration.take();
                state.what_changed = false;
                state.error = None;
                let reply = self.client.dismiss_migration();
                Effect::future(async move {
                    TodoAction::Dismissed { before, result: reply.await }
                })
            }
            TodoAction::Dismissed { before, result } => {
                if let Err(error) = result {
                    state.migration = before;
                    state.error = Some(error);
                }
                Effect::none()
            }
        }
    }
}
