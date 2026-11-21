//! The To-do list (BOARDSIMPLE): one ranked list for every channel and
//! session, which are filters on it. Items have three states, a computed
//! rank with reasons, and Pin; an agent's pending ask shows as Needs
//! attention with Yes and No; Do next opens the top item.
//!
//! Ranking is `humata2_todo::rank`, run over what is shown whenever the
//! view asks, so the order is never kept in the state. Writes show at once
//! and are reconciled with the host's reply; a refused write puts the item
//! back and says why.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::todo::{
    BlockerGoneResolution, BulkUpdateItem, BulkUpdateResult, Entry, FoldResult, MigrationNotice,
    NewTodo, SplitResult, TodoClient, TodoFilter, TodoHistoryEntry, TodoPatch, TodoRunsResult,
};
use humata2_todo::{
    ChannelFilter, CheckpointState, Day, Draft, DueFilter, Effort, ExternalBlock, Filter, FuzzyDue,
    Impact, Need, Only, OwnerKind, ParseDayError, Ranked, SessionFilter, SortBy, SourceAnchor,
    State, Step, Todo as Item, TodoComment, TodoOwner, apply, parse_day, rank, rank_with,
};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

const WATCH: &str = "todo-watch";

/// A known workspace channel's ID and human-readable display name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelRef {
    /// Stable channel ID stored on items and filters.
    pub id: String,
    /// Human-readable channel name shown in the UI.
    pub name: String,
}

/// Which channel the list is narrowed to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelPick {
    /// One channel.
    Channel(String),
    /// Items with no channel.
    NoChannel,
}

/// Which session the list is narrowed to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionPick {
    /// One session.
    Session(String),
    /// Items not in any session yet.
    NoneYet,
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

/// One removable active-filter chip shown in the filter bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterChip {
    Channel(ChannelPick),
    Session(SessionPick),
    Due(DuePick),
    Tag(String),
    Effort(Effort),
    Impact(Impact),
    Need(Need),
    Only(OnlyPick),
    IncludeTerminal,
    Search(String),
}

impl FilterChip {
    /// Human-readable label for this chip.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            FilterChip::Channel(ChannelPick::Channel(c)) => c.clone(),
            FilterChip::Channel(ChannelPick::NoChannel) => "No channel".to_owned(),
            FilterChip::Session(SessionPick::Session(s)) => format!("Session: {s}"),
            FilterChip::Session(SessionPick::NoneYet) => "No session yet".to_owned(),
            FilterChip::Due(DuePick::Overdue) => "Overdue".to_owned(),
            FilterChip::Due(DuePick::Next7Days) => "Next 7 days".to_owned(),
            FilterChip::Due(DuePick::AfterThisWeek) => "After this week".to_owned(),
            FilterChip::Due(DuePick::NoDate) => "No date".to_owned(),
            FilterChip::Tag(t) => format!("#{t}"),
            FilterChip::Effort(Effort::S) => "Effort: S".to_owned(),
            FilterChip::Effort(Effort::M) => "Effort: M".to_owned(),
            FilterChip::Effort(Effort::L) => "Effort: L".to_owned(),
            FilterChip::Impact(Impact::Low) => "Impact: Low".to_owned(),
            FilterChip::Impact(Impact::Medium) => "Impact: Medium".to_owned(),
            FilterChip::Impact(Impact::High) => "Impact: High".to_owned(),
            FilterChip::Need(n) => format!("Need: {}", n.label()),
            FilterChip::Only(OnlyPick::NeedsAttention) => "Needs attention".to_owned(),
            FilterChip::Only(OnlyPick::PullRequests) => "Pull requests".to_owned(),
            FilterChip::IncludeTerminal => "Won't do / Became".to_owned(),
            FilterChip::Search(q) => format!("\"{q}\""),
        }
    }

    /// Human-readable label for this chip, resolving channel and session IDs.
    #[must_use]
    pub fn label_with<'a>(
        &'a self,
        channel_name: impl Fn(&'a str) -> &'a str,
        session_name: impl Fn(&str) -> Option<String>,
    ) -> String {
        match self {
            FilterChip::Channel(ChannelPick::Channel(id)) => channel_name(id).to_owned(),
            FilterChip::Session(SessionPick::Session(id)) => {
                let name = session_name(id).unwrap_or_else(|| id.clone());
                format!("Session: {name}")
            }
            _ => self.label(),
        }
    }
}

/// What narrows the list: channel and session, then due, tags and search.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewFilter {
    /// A channel, or the items with none; unset is every channel.
    pub channel: Option<ChannelPick>,
    /// One session's items, or items not in any session yet.
    pub session: Option<SessionPick>,
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
    /// Need levels; an item matches any of them.
    pub need: Vec<Need>,
    /// Kinds of item; an item matches any of them.
    pub only: Vec<OnlyPick>,
    /// Include terminal `Won't do` and `Became` items.
    pub include_terminal: bool,
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
            session: self.session.as_ref().map(|s| match s {
                SessionPick::Session(id) => SessionFilter::Session(id.clone()),
                SessionPick::NoneYet => SessionFilter::NoneYet,
            }),
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
            need: self.need.clone(),
            only: self
                .only
                .iter()
                .map(|o| match o {
                    OnlyPick::NeedsAttention => Only::NeedsAttention,
                    OnlyPick::PullRequests => Only::PullRequests,
                })
                .collect(),
            include_terminal: self.include_terminal,
        }
    }

    /// Returns removable active-filter chips for every criterion currently set.
    #[must_use]
    pub fn active_chips(&self) -> Vec<FilterChip> {
        let mut chips = Vec::new();
        if let Some(ch) = &self.channel {
            chips.push(FilterChip::Channel(ch.clone()));
        }
        if let Some(sess) = &self.session {
            chips.push(FilterChip::Session(sess.clone()));
        }
        for &d in &self.due {
            chips.push(FilterChip::Due(d));
        }
        for tag in &self.tags {
            chips.push(FilterChip::Tag(tag.clone()));
        }
        for &e in &self.effort {
            chips.push(FilterChip::Effort(e));
        }
        for &i in &self.impact {
            chips.push(FilterChip::Impact(i));
        }
        for &n in &self.need {
            chips.push(FilterChip::Need(n));
        }
        for &o in &self.only {
            chips.push(FilterChip::Only(o));
        }
        if self.include_terminal {
            chips.push(FilterChip::IncludeTerminal);
        }
        let search = self.search.trim();
        if !search.is_empty() {
            chips.push(FilterChip::Search(search.to_owned()));
        }
        chips
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
    /// A typed due date ("fri", "nov 28", "2026-12-01").
    Due,
    /// The Markdown context of the step at this index.
    StepContext(usize),
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

/// Conditional "Start next" offer carried by [`UndoToast`], populated ONLY
/// when no other item is currently `InProgress`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartNextOffer {
    /// Key of the top `Next` item.
    pub key: String,
    /// Title of the top `Next` item.
    pub title: String,
}

/// Undo toast state shown after quick state transitions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoToast {
    /// Human-readable message, e.g. "Closed PAY-5".
    pub message: String,
    /// Key of the item that was changed.
    pub key: String,
    /// Patch that restores the item's previous state.
    pub undo_patch: TodoPatch,
    /// Conditional "Start next" offer, populated ONLY when no other item is `InProgress`.
    pub start_next: Option<StartNextOffer>,
}

/// Editable draft answer for an open Question ask.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestionDraft {
    /// Free-text answer or note.
    pub text: String,
    /// Selected single or multiple choices.
    pub selected_choices: Vec<String>,
}

/// Everything the To-do page and panel draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodoState {
    /// Every item the host listed, with its revision.
    pub items: Vec<Entry>,
    /// Known workspace channels for resolving channel IDs to display names.
    pub known_channels: Vec<ChannelRef>,
    /// Today, as the reducer last read it.
    pub today: Day,
    /// Presentation retained across restarts.
    pub view: View,
    /// Temporary sort-by override (defaults to `SortBy::Priority`).
    pub sort: SortBy,
    /// The item open in the drawer.
    pub open: Option<String>,
    /// Keyboard triage cursor in the list.
    pub focused: Option<String>,
    /// Multi-selected item keys for bulk actions.
    pub selected: BTreeSet<String>,
    /// Range-selection anchor for Shift-click / `Shift+↑`/`↓`.
    pub selection_anchor: Option<String>,
    /// Per-item refusal messages from the last bulk action.
    pub bulk_errors: BTreeMap<String, String>,
    /// Undo toast with conditional `Start next` offer.
    pub undo: Option<UndoToast>,
    /// Item key awaiting *Take back* confirmation.
    pub confirm_take_back: Option<String>,
    /// Cached item history (with word-level diffs) keyed by item key.
    pub history: BTreeMap<String, Vec<TodoHistoryEntry>>,
    /// Cached agent runs & cost ledger keyed by item key.
    pub runs: BTreeMap<String, TodoRunsResult>,
    /// Editable drafts for open Question asks, keyed by `ask_id`.
    pub question_drafts: BTreeMap<String, QuestionDraft>,
    /// NOTIFY2 (Fable ruling 4): ask IDs whose inline "Why?" explanation is expanded.
    pub expanded_why: BTreeSet<String>,
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
    /// Why a typed due date couldn't be read, cleared on the next valid date or edit.
    pub due_error: Option<ParseDayError>,
}

impl TodoState {
    /// An empty list on `today`, shown as `view`.
    #[must_use]
    pub fn new(view: View, today: Day) -> Self {
        Self {
            items: Vec::new(),
            known_channels: Vec::new(),
            today,
            view,
            sort: SortBy::Priority,
            open: None,
            focused: None,
            selected: BTreeSet::new(),
            selection_anchor: None,
            bulk_errors: BTreeMap::new(),
            undo: None,
            confirm_take_back: None,
            history: BTreeMap::new(),
            runs: BTreeMap::new(),
            question_drafts: BTreeMap::new(),
            expanded_why: BTreeSet::new(),
            quick_add: QuickAdd::default(),
            editing: None,
            migration: None,
            do_next: None,
            what_changed: false,
            loaded: false,
            error: None,
            due_error: None,
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

    /// The shown items in their groups, in the active sort order.
    #[must_use]
    pub fn ranked(&self) -> Ranked {
        rank_with(&self.shown(), self.sort, self.today)
    }

    /// Keys of all currently visible rows in display order.
    #[must_use]
    pub fn visible_order(&self) -> Vec<String> {
        let ranked = self.ranked();
        let mut order = Vec::new();
        for group in [
            &ranked.needs_attention,
            &ranked.in_progress,
            &ranked.watching_pr,
            &ranked.next,
            &ranked.waiting,
            &ranked.hidden,
            &ranked.closed,
        ] {
            for row in group {
                if !row.key.is_empty() {
                    order.push(row.key.clone());
                }
            }
        }
        order
    }

    /// Open items and channels among them, for "10 open across 4 channels".
    #[must_use]
    pub fn header(&self) -> (usize, usize) {
        humata2_todo::header(&self.shown(), self.today)
    }

    /// Updates the known workspace channels used to resolve channel IDs to display names.
    pub fn set_channels(&mut self, channels: Vec<ChannelRef>) {
        self.known_channels = channels;
    }

    /// Human-readable display name for a channel ID, falling back to `id` when unknown.
    #[must_use]
    pub fn channel_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.known_channels
            .iter()
            .find(|c| c.id == id && !c.name.is_empty())
            .map_or(id, |c| c.name.as_str())
    }

    /// All known workspace channels plus any channel IDs referenced by items
    /// that are not yet in `known_channels`.
    #[must_use]
    pub fn all_channels(&self) -> Vec<ChannelRef> {
        let mut out = self.known_channels.clone();
        for c in self.items.iter().filter_map(|e| e.todo.channel.as_ref()) {
            if !out.iter().any(|ch| ch.id == *c) {
                out.push(ChannelRef { id: c.clone(), name: c.clone() });
            }
        }
        out
    }

    /// Human-readable label for an active filter chip, resolving channel IDs.
    #[must_use]
    pub fn chip_label(&self, chip: &FilterChip) -> String {
        chip.label_with(|id| self.channel_name(id), |_| None)
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
        let channels: Vec<(String, String)> =
            self.all_channels().into_iter().map(|c| (c.id, c.name)).collect();
        let draft = humata2_todo::parse_with_channels(line, self.today, &channels)?;
        const NEW: &str = "\u{0}new";
        let mut item = Item::new(NEW, &draft.title, self.today);
        item.channel.clone_from(&draft.channel);
        item.due = draft.due;
        item.fuzzy_due = draft.fuzzy_due;
        item.effort = draft.effort;
        item.impact = draft.impact;
        item.need = draft.need;
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
    /// Syncs the known workspace channels used for display names and quick-add resolution.
    SetChannels(Vec<ChannelRef>),

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
    /// Parses a typed due date ("fri", "nov 28", "2026-12-01") and sets it, or
    /// records why it couldn't be read in [`TodoState::due_error`].
    SetDueText(String, String),
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

    /// Toggles the inline "Why?" explanation on a To-do ask (NOTIFY2, Fable ruling 4).
    AskWhy(String),
    /// Starts the review of a pull request review item (by key). The app
    /// routes it to the pull request review start; the list does not change.
    StartReview(String),
    /// Starts a new session for this item (by key). The app routes it to
    /// `AppAction::StartNewForItem`; the list does not change.
    StartSession(String),
    /// Opens the session an item belongs to. The app routes it.
    OpenSession(String),
    /// Opens the channel an item belongs to. The app routes it.
    OpenChannel(String),
    /// Opens an item's history. The app routes it.
    ShowHistory(String),
    /// Opens a link on an ask card's evidence row. The app routes it.
    OpenEvidence(String),
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

    // --- TODO2: Sort, Chips, Keyboard Triage, Bulk Selection, Undo ---

    /// Temporary sort-by override (`SortBy::Priority` default).
    SetSort(SortBy),
    /// Toggles a tag in the active filter (clickable tag chip).
    ToggleTagChip(String),
    /// Removes a specific active filter chip.
    RemoveFilterChip(FilterChip),

    /// Moves keyboard triage focus by `delta` (`+1` down, `-1` up).
    MoveFocus(i32),
    /// Steps the open drawer to the next/previous item without closing it.
    StepDrawer(i32),
    /// Toggles the focused item between closed and open.
    ToggleFocusDone,
    /// Starts the focused item (or the top `Next` item if none focused).
    StartOrNextFocused,

    /// Selects a row in bulk selection mode.
    SelectRow {
        key: String,
        shift: bool,
        toggle: bool,
    },
    /// Extends bulk selection by `delta` rows (`Shift+↑`/`↓`).
    ExtendSelection(i32),
    /// Clears bulk selection and per-item bulk errors.
    ClearSelection,
    /// Applies a state transition to all selected items.
    BulkSetState(State),
    /// Moves all selected items to `channel`.
    BulkMoveChannel(Option<String>),
    /// Host reply to a bulk update; keeps only refused keys selected.
    BulkUpdated {
        before: Vec<Entry>,
        result: Result<BulkUpdateResult, DaemonError>,
    },

    /// Reverses the last quick state change in `state.undo`.
    Undo,
    /// Dismisses the Undo toast.
    DismissUndo,
    /// Starts the `start_next` item offered on the Undo toast.
    StartNextFromUndo,

    // --- TODO2: Priority, Structure, Steps, Comments, Lineage, History, Runs ---

    /// Sets or clears the item's Need level.
    SetNeed(String, Option<Need>),
    /// Sets or clears the item's loose/fuzzy due date.
    SetFuzzyDue(String, Option<FuzzyDue>),
    /// Sets a free-text external block reason on an item.
    AddExternalBlock(String, String),
    /// Clears the external block on an item.
    RemoveExternalBlock(String),
    /// Appends a comment to an item (optionally anchored to a step).
    AddComment {
        key: String,
        body: String,
        step_id: Option<String>,
    },
    /// Host reply to `AddComment`.
    CommentAdded {
        result: Result<(Entry, TodoComment), DaemonError>,
    },
    /// Sets or clears optional Markdown context on a checklist step.
    SetStepMarkdown(String, usize, Option<String>),
    /// Reorders a checklist step from `from` to `to`.
    MoveStep(String, usize, usize),
    /// Soft-drops a checklist step.
    DropStep(String, usize),
    /// Restores a soft-dropped checklist step.
    RestoreStep(String, usize),
    /// Sets or clears the item's checkpoint handoff state.
    SetCheckpoint(String, Option<CheckpointState>),
    /// Splits an item into multiple tasks (`From`/`Became` lineage).
    SplitItem {
        key: String,
        titles: Vec<String>,
        keep_age: bool,
    },
    /// Host reply to `SplitItem`.
    ItemSplit(Result<SplitResult, DaemonError>),
    /// Folds `source_key` into a step on `target_key`.
    FoldIntoStep {
        source_key: String,
        target_key: String,
    },
    /// Host reply to `FoldIntoStep`.
    ItemFolded(Result<FoldResult, DaemonError>),
    /// Loads item history (including word-level diffs).
    LoadHistory(String),
    /// Host reply to `LoadHistory`.
    HistoryLoaded {
        key: String,
        result: Result<Vec<TodoHistoryEntry>, DaemonError>,
    },
    /// Loads agent runs & cost ledger for an item.
    LoadRuns(String),
    /// Host reply to `LoadRuns`.
    RunsLoaded {
        key: String,
        result: Result<TodoRunsResult, DaemonError>,
    },

    // --- TODO2: Structured Agent Collaboration & Lifecycle Sync ---

    /// Updates an editable open Question draft before submitting.
    UpdateQuestionDraft {
        ask_id: String,
        text: String,
        selected_choices: Vec<String>,
    },
    /// Toggles a choice on an editable open Question draft.
    ToggleQuestionChoice {
        ask_id: String,
        choice: String,
        multi_select: bool,
    },
    /// Submits an answer to a structured Question ask.
    AnswerQuestion {
        ask_id: String,
        text: String,
        selected_choices: Vec<String>,
    },
    /// Submits an answer to an ActionRequest ask (`Done` / `Can't be done`).
    AnswerAction {
        ask_id: String,
        done: bool,
        note: String,
    },
    /// Submits an answer to a ReviewRequest ask (`Approved` / `Changes asked`).
    AnswerReview {
        ask_id: String,
        approved: bool,
        note: String,
    },
    /// Retries a stuck agent on `key`.
    RetryStuck(String),
    /// Opens *Take back* confirmation for `key`.
    RequestTakeBack(String),
    /// Cancels *Take back* confirmation.
    CancelTakeBack,
    /// Confirms taking ownership of `key` back from the agent.
    ConfirmTakeBack(String),
    /// Resolves a *Blocker gone* recovery card.
    ResolveBlockerGone {
        key: String,
        blocker_key: String,
        resolution: BlockerGoneResolution,
    },
    /// Resolves a quiet lifecycle prompt on `key`.
    ResolveLifecyclePrompt {
        key: String,
        prompt_id: String,
        choice: String,
    },

    // --- TODO2: Diff & Markdown Comment Hooks ---

    /// Adds a checklist step to `key` anchored to a diff/markdown comment.
    AddStepFromComment {
        key: String,
        text: String,
        anchor: SourceAnchor,
    },
    /// Spins off a new To-do from a diff/markdown comment (`From: <key>` + anchor).
    SpinOffFromComment {
        from_key: Option<String>,
        title: String,
        anchor: SourceAnchor,
        channel: Option<String>,
    },
    /// Navigates to a `SourceAnchor` (routed by the app; list does not change).
    NavigateAnchor(SourceAnchor),
}

/// Reads the date the list is ranked on.
type Today = Arc<dyn Fn() -> Day + Send + Sync>;

/// The To-do reducer, over the host's client.
#[derive(Clone)]
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

    fn set_state_with_undo(
        &self,
        state: &mut TodoState,
        key: &str,
        to: State,
    ) -> Effect<TodoAction> {
        let prev = state.position(key).map(|at| {
            let t = &state.items[at].todo;
            (t.state, t.pin)
        });
        let effect = self.patch(
            state,
            key,
            TodoPatch { state: Some(to), ..TodoPatch::default() },
        );
        if let Some((prev_state, prev_pin)) = prev
            && to.is_terminal()
        {
            let ranked = state.ranked();
            let start_next = if ranked.in_progress.is_empty() {
                ranked.do_next.and_then(|next_key| {
                    state.position(&next_key).map(|at| StartNextOffer {
                        key: next_key,
                        title: state.items[at].todo.title.clone(),
                    })
                })
            } else {
                None
            };
            let verb = match to {
                State::WontDo => "Marked won't do",
                State::Became => "Marked became",
                _ => "Closed",
            };
            state.undo = Some(UndoToast {
                message: format!("{verb} {key}"),
                key: key.to_owned(),
                undo_patch: TodoPatch {
                    state: Some(prev_state),
                    pin: Some(prev_pin),
                    ..TodoPatch::default()
                },
                start_next,
            });
        }
        effect
    }

    fn bulk_patch(&self, state: &mut TodoState, patch: TodoPatch) -> Effect<TodoAction> {
        let mut items = Vec::new();
        let mut before = Vec::new();
        let keys: Vec<String> = state.selected.iter().cloned().collect();
        for key in keys {
            let Some(at) = state.position(&key) else {
                continue;
            };
            let snap = state.items[at].clone();
            items.push(BulkUpdateItem {
                key,
                patch: patch.clone(),
                expected_revision: snap.revision,
            });
            patch.apply(&mut state.items[at].todo, state.today);
            state.items[at].revision += 1;
            before.push(snap);
        }
        if items.is_empty() {
            return Effect::none();
        }
        state.error = None;
        state.bulk_errors.clear();
        let reply = self.client.bulk_update(items);
        Effect::future(async move {
            TodoAction::BulkUpdated { before, result: reply.await }
        })
    }

    fn step_in_order(order: &[String], current: Option<&str>, delta: i32) -> Option<String> {
        if order.is_empty() {
            return None;
        }
        let next_idx = match current.and_then(|k| order.iter().position(|x| x == k)) {
            Some(idx) => {
                let target = idx as isize + delta as isize;
                target.clamp(0, (order.len() - 1) as isize) as usize
            }
            None => {
                if delta < 0 {
                    order.len() - 1
                } else {
                    0
                }
            }
        };
        order.get(next_idx).cloned()
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
                    state.editing = None;
                    state.due_error = None;
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
                    state.editing = None;
                    state.due_error = None;
                }
                Effect::none()
            }
            TodoAction::SetChannels(channels) => {
                state.set_channels(channels);
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
            TodoAction::AskWhy(ask_id) => {
                if !state.expanded_why.remove(&ask_id) {
                    state.expanded_why.insert(ask_id);
                }
                Effect::none()
            }
            TodoAction::OpenSession(_)
            | TodoAction::OpenChannel(_)
            | TodoAction::StartReview(_)
            | TodoAction::StartSession(_)
            | TodoAction::ShowHistory(_)
            | TodoAction::OpenEvidence(_) => Effect::none(),
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
                state.due_error = None;
                Effect::none()
            }
            TodoAction::Close => {
                state.open = None;
                state.editing = None;
                state.due_error = None;
                Effect::none()
            }
            TodoAction::StartEdit(key, field) => {
                let text = match (field, state.position(&key)) {
                    (EditField::Title, Some(at)) => state.items[at].todo.title.clone(),
                    (EditField::Description, Some(at)) => state.items[at].todo.description.clone(),
                    (EditField::Due, Some(at)) => state.items[at]
                        .todo
                        .due
                        .map_or_else(String::new, |d| {
                            let (y, m, day) = d.ymd();
                            format!("{y:04}-{m:02}-{day:02}")
                        }),
                    (EditField::StepContext(i), Some(at)) => state.items[at]
                        .todo
                        .steps
                        .get(i)
                        .and_then(|s| s.markdown_context.clone())
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                state.editing = Some(Editing { key, field, text });
                state.due_error = None;
                Effect::none()
            }
            TodoAction::EditChanged(text) => {
                if let Some(editing) = &mut state.editing {
                    editing.text = text;
                }
                state.due_error = None;
                Effect::none()
            }
            TodoAction::CommitEdit => {
                let Some(editing) = state.editing.take() else {
                    return Effect::none();
                };
                let text = editing.text.trim().to_owned();
                if editing.field == EditField::Due {
                    if text.is_empty() {
                        state.due_error = None;
                        return self.patch(
                            state,
                            &editing.key,
                            TodoPatch { due: Some(None), ..TodoPatch::default() },
                        );
                    }
                    return match parse_day(&text, state.today) {
                        Ok(day) => {
                            state.due_error = None;
                            self.patch(
                                state,
                                &editing.key,
                                TodoPatch { due: Some(Some(day)), ..TodoPatch::default() },
                            )
                        }
                        Err(err) => {
                            state.editing = Some(editing);
                            state.due_error = Some(err);
                            Effect::none()
                        }
                    };
                }
                if let EditField::StepContext(index) = editing.field {
                    let Some(at) = state.position(&editing.key) else {
                        return Effect::none();
                    };
                    let had = state.items[at]
                        .todo
                        .steps
                        .get(index)
                        .and_then(|s| s.markdown_context.as_deref())
                        .unwrap_or_default();
                    if had.trim() == text {
                        return Effect::none();
                    }
                    let markdown = (!text.is_empty()).then_some(text);
                    return self.reduce(state, TodoAction::SetStepMarkdown(editing.key, index, markdown));
                }
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
                        steps.push(Step::new(text));
                        self.patch(
                            state,
                            &key,
                            TodoPatch { steps: Some(steps), ..TodoPatch::default() },
                        )
                    }
                    EditField::Due | EditField::StepContext(_) => unreachable!("handled above"),
                }
            }
            TodoAction::CancelEdit => {
                state.editing = None;
                state.due_error = None;
                Effect::none()
            }

            TodoAction::SetState(key, to) => self.set_state_with_undo(state, &key, to),
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
                state.due_error = None;
                self.patch(state, &key, TodoPatch { due: Some(Some(day)), ..TodoPatch::default() })
            }
            TodoAction::SetDueText(key, text) => match parse_day(&text, state.today) {
                Ok(day) => {
                    state.due_error = None;
                    self.patch(
                        state,
                        &key,
                        TodoPatch { due: Some(Some(day)), ..TodoPatch::default() },
                    )
                }
                Err(err) => {
                    state.due_error = Some(err);
                    Effect::none()
                }
            },
            TodoAction::ClearDue(key) => {
                state.due_error = None;
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

            // --- TODO2: Sort, Chips, Keyboard Triage, Bulk Selection, Undo ---
            TodoAction::SetSort(sort) => {
                state.sort = sort;
                state.do_next = None;
                Effect::none()
            }
            TodoAction::ToggleTagChip(tag) => {
                let tag = tag.trim().to_owned();
                if !tag.is_empty() {
                    if let Some(at) = state
                        .view
                        .filter
                        .tags
                        .iter()
                        .position(|t| t.eq_ignore_ascii_case(&tag))
                    {
                        state.view.filter.tags.remove(at);
                    } else {
                        state.view.filter.tags.push(tag);
                    }
                    state.do_next = None;
                }
                Effect::none()
            }
            TodoAction::RemoveFilterChip(chip) => {
                match chip {
                    FilterChip::Channel(_) => state.view.filter.channel = None,
                    FilterChip::Session(_) => state.view.filter.session = None,
                    FilterChip::Due(d) => state.view.filter.due.retain(|x| *x != d),
                    FilterChip::Tag(t) => {
                        state.view.filter.tags.retain(|x| !x.eq_ignore_ascii_case(&t));
                    }
                    FilterChip::Effort(e) => state.view.filter.effort.retain(|x| *x != e),
                    FilterChip::Impact(i) => state.view.filter.impact.retain(|x| *x != i),
                    FilterChip::Need(n) => state.view.filter.need.retain(|x| *x != n),
                    FilterChip::Only(o) => state.view.filter.only.retain(|x| *x != o),
                    FilterChip::IncludeTerminal => state.view.filter.include_terminal = false,
                    FilterChip::Search(_) => state.view.filter.search.clear(),
                }
                state.do_next = None;
                Effect::none()
            }

            TodoAction::MoveFocus(delta) => {
                let order = state.visible_order();
                state.focused = Self::step_in_order(&order, state.focused.as_deref(), delta);
                Effect::none()
            }
            TodoAction::StepDrawer(delta) => {
                if state.open.is_some() {
                    let order = state.visible_order();
                    if let Some(next_key) = Self::step_in_order(&order, state.open.as_deref(), delta)
                    {
                        state.open = Some(next_key.clone());
                        state.focused = Some(next_key);
                        state.editing = None;
                        state.due_error = None;
                        state.confirm_take_back = None;
                    }
                }
                Effect::none()
            }
            TodoAction::ToggleFocusDone => {
                let Some(key) = state.focused.clone().or_else(|| state.open.clone()) else {
                    return Effect::none();
                };
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                let to = if state.items[at].todo.state.is_terminal() {
                    State::Open
                } else {
                    State::Closed
                };
                self.set_state_with_undo(state, &key, to)
            }
            TodoAction::StartOrNextFocused => {
                let target = state
                    .focused
                    .clone()
                    .or_else(|| state.open.clone())
                    .or_else(|| state.ranked().do_next);
                let Some(key) = target else {
                    return Effect::none();
                };
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                if state.items[at].todo.state == State::InProgress {
                    if let Some(next) = state.ranked().do_next {
                        state.focused = Some(next.clone());
                        state.do_next = Some(next);
                    }
                    Effect::none()
                } else {
                    state.focused = Some(key.clone());
                    self.patch(
                        state,
                        &key,
                        TodoPatch { state: Some(State::InProgress), ..TodoPatch::default() },
                    )
                }
            }

            TodoAction::SelectRow { key, shift, toggle } => {
                state.bulk_errors.clear();
                state.focused = Some(key.clone());
                if shift {
                    let order = state.visible_order();
                    let anchor = state
                        .selection_anchor
                        .as_deref()
                        .unwrap_or(key.as_str());
                    if let (Some(a), Some(b)) = (
                        order.iter().position(|k| k == anchor),
                        order.iter().position(|k| k == &key),
                    ) {
                        if !toggle {
                            state.selected.clear();
                        }
                        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                        for k in &order[lo..=hi] {
                            state.selected.insert(k.clone());
                        }
                    } else {
                        state.selected.insert(key.clone());
                    }
                    if state.selection_anchor.is_none() {
                        state.selection_anchor = Some(key);
                    }
                } else if toggle {
                    if !state.selected.remove(&key) {
                        state.selected.insert(key.clone());
                    }
                    state.selection_anchor = Some(key);
                } else {
                    state.selected.clear();
                    state.selected.insert(key.clone());
                    state.selection_anchor = Some(key);
                }
                Effect::none()
            }
            TodoAction::ExtendSelection(delta) => {
                state.bulk_errors.clear();
                let order = state.visible_order();
                if order.is_empty() {
                    return Effect::none();
                }
                let cur_idx = state
                    .focused
                    .as_deref()
                    .and_then(|k| order.iter().position(|x| x == k))
                    .unwrap_or_else(|| if delta < 0 { order.len() - 1 } else { 0 });
                if state.selection_anchor.is_none() {
                    state.selection_anchor = Some(order[cur_idx].clone());
                }
                let new_idx = (cur_idx as isize + delta as isize)
                    .clamp(0, (order.len() - 1) as isize) as usize;
                state.focused = Some(order[new_idx].clone());
                let anchor_idx = state
                    .selection_anchor
                    .as_deref()
                    .and_then(|k| order.iter().position(|x| x == k))
                    .unwrap_or(cur_idx);
                let (lo, hi) = if anchor_idx <= new_idx {
                    (anchor_idx, new_idx)
                } else {
                    (new_idx, anchor_idx)
                };
                state.selected.clear();
                for k in &order[lo..=hi] {
                    state.selected.insert(k.clone());
                }
                Effect::none()
            }
            TodoAction::ClearSelection => {
                state.selected.clear();
                state.selection_anchor = None;
                state.bulk_errors.clear();
                Effect::none()
            }
            TodoAction::BulkSetState(to) => {
                self.bulk_patch(state, TodoPatch { state: Some(to), ..TodoPatch::default() })
            }
            TodoAction::BulkMoveChannel(channel) => {
                self.bulk_patch(state, TodoPatch { channel: Some(channel), ..TodoPatch::default() })
            }
            TodoAction::BulkUpdated { before, result } => {
                match result {
                    Ok(BulkUpdateResult { updated, failed }) => {
                        for entry in updated {
                            state.replace(entry);
                        }
                        state.bulk_errors.clear();
                        state.selected.clear();
                        for failure in failed {
                            if let Some(prev) =
                                before.iter().find(|e| e.todo.key == failure.key)
                            {
                                state.replace(prev.clone());
                            }
                            state.selected.insert(failure.key.clone());
                            state.bulk_errors.insert(failure.key, failure.reason);
                        }
                        if state.selected.is_empty() {
                            state.selection_anchor = None;
                        }
                    }
                    Err(error) => {
                        for prev in before {
                            state.replace(prev);
                        }
                        state.error = Some(error);
                    }
                }
                Effect::none()
            }

            TodoAction::Undo => {
                let Some(toast) = state.undo.take() else {
                    return Effect::none();
                };
                self.patch(state, &toast.key, toast.undo_patch)
            }
            TodoAction::DismissUndo => {
                state.undo = None;
                Effect::none()
            }
            TodoAction::StartNextFromUndo => {
                let Some(UndoToast { start_next: Some(offer), .. }) = state.undo.take() else {
                    return Effect::none();
                };
                state.focused = Some(offer.key.clone());
                self.patch(
                    state,
                    &offer.key,
                    TodoPatch { state: Some(State::InProgress), ..TodoPatch::default() },
                )
            }

            // --- TODO2: Priority, Structure, Steps, Comments, Lineage, History, Runs ---
            TodoAction::SetNeed(key, need) => {
                self.patch(state, &key, TodoPatch { need: Some(need), ..TodoPatch::default() })
            }
            TodoAction::SetFuzzyDue(key, fuzzy) => {
                state.due_error = None;
                self.patch(
                    state,
                    &key,
                    TodoPatch { fuzzy_due: Some(fuzzy), ..TodoPatch::default() },
                )
            }
            TodoAction::AddExternalBlock(key, reason) => {
                let reason = reason.trim().to_owned();
                if reason.is_empty() {
                    return Effect::none();
                }
                self.patch(
                    state,
                    &key,
                    TodoPatch {
                        external_block: Some(Some(ExternalBlock {
                            reason,
                            since: Some(state.today),
                        })),
                        ..TodoPatch::default()
                    },
                )
            }
            TodoAction::RemoveExternalBlock(key) => self.patch(
                state,
                &key,
                TodoPatch { external_block: Some(None), ..TodoPatch::default() },
            ),
            TodoAction::AddComment { key, body, step_id } => {
                let body = body.trim().to_owned();
                if body.is_empty() {
                    return Effect::none();
                }
                state.error = None;
                let reply = self.client.add_comment(&key, body, step_id);
                Effect::future(async move { TodoAction::CommentAdded { result: reply.await } })
            }
            TodoAction::CommentAdded { result } => {
                match result {
                    Ok((entry, _)) => state.replace(entry),
                    Err(error) => state.error = Some(error),
                }
                Effect::none()
            }
            TodoAction::SetStepMarkdown(key, index, markdown) => {
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                let mut steps = state.items[at].todo.steps.clone();
                let Some(step) = steps.get_mut(index) else {
                    return Effect::none();
                };
                step.markdown_context = markdown.filter(|m| !m.trim().is_empty());
                self.patch(state, &key, TodoPatch { steps: Some(steps), ..TodoPatch::default() })
            }
            TodoAction::MoveStep(key, from, to) => {
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                cancel_step_context_edit(state, &key);
                let mut steps = state.items[at].todo.steps.clone();
                if from >= steps.len() || from == to {
                    return Effect::none();
                }
                let step = steps.remove(from);
                let target = to.min(steps.len());
                steps.insert(target, step);
                self.patch(state, &key, TodoPatch { steps: Some(steps), ..TodoPatch::default() })
            }
            TodoAction::DropStep(key, index) => {
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                cancel_step_context_edit(state, &key);
                let mut steps = state.items[at].todo.steps.clone();
                let Some(step) = steps.get_mut(index) else {
                    return Effect::none();
                };
                step.dropped = true;
                self.patch(state, &key, TodoPatch { steps: Some(steps), ..TodoPatch::default() })
            }
            TodoAction::RestoreStep(key, index) => {
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                let mut steps = state.items[at].todo.steps.clone();
                let Some(step) = steps.get_mut(index) else {
                    return Effect::none();
                };
                step.dropped = false;
                self.patch(state, &key, TodoPatch { steps: Some(steps), ..TodoPatch::default() })
            }
            TodoAction::SetCheckpoint(key, checkpoint) => self.patch(
                state,
                &key,
                TodoPatch { checkpoint: Some(checkpoint), ..TodoPatch::default() },
            ),
            TodoAction::SplitItem { key, titles, keep_age } => {
                let titles: Vec<String> = titles
                    .into_iter()
                    .map(|t| t.trim().to_owned())
                    .filter(|t| !t.is_empty())
                    .collect();
                if titles.is_empty() {
                    return Effect::none();
                }
                state.error = None;
                let reply = self.client.split(&key, titles, keep_age);
                Effect::future(async move { TodoAction::ItemSplit(reply.await) })
            }
            TodoAction::ItemSplit(result) => {
                match result {
                    Ok(SplitResult { source, created }) => {
                        state.replace(source);
                        for entry in created {
                            state.replace(entry);
                        }
                    }
                    Err(error) => state.error = Some(error),
                }
                Effect::none()
            }
            TodoAction::FoldIntoStep { source_key, target_key } => {
                state.error = None;
                let reply = self.client.fold_into_step(&source_key, &target_key);
                Effect::future(async move { TodoAction::ItemFolded(reply.await) })
            }
            TodoAction::ItemFolded(result) => {
                match result {
                    Ok(FoldResult { source, target }) => {
                        state.replace(source);
                        state.replace(target);
                    }
                    Err(error) => state.error = Some(error),
                }
                Effect::none()
            }
            TodoAction::LoadHistory(key) => {
                let reply = self.client.history(&key);
                Effect::future(async move {
                    TodoAction::HistoryLoaded { key, result: reply.await }
                })
            }
            TodoAction::HistoryLoaded { key, result } => {
                match result {
                    Ok(entries) => {
                        state.history.insert(key, entries);
                    }
                    Err(error) => state.error = Some(error),
                }
                Effect::none()
            }
            TodoAction::LoadRuns(key) => {
                let reply = self.client.runs(&key);
                Effect::future(async move {
                    TodoAction::RunsLoaded { key, result: reply.await }
                })
            }
            TodoAction::RunsLoaded { key, result } => {
                match result {
                    Ok(runs) => {
                        state.runs.insert(key, runs);
                    }
                    Err(error) => state.error = Some(error),
                }
                Effect::none()
            }

            // --- TODO2: Structured Agent Collaboration & Lifecycle Sync ---
            TodoAction::UpdateQuestionDraft { ask_id, text, selected_choices } => {
                state.question_drafts.insert(ask_id, QuestionDraft { text, selected_choices });
                Effect::none()
            }
            TodoAction::ToggleQuestionChoice { ask_id, choice, multi_select } => {
                let draft = state.question_drafts.entry(ask_id).or_default();
                if multi_select {
                    if let Some(pos) = draft.selected_choices.iter().position(|c| c == &choice) {
                        draft.selected_choices.remove(pos);
                    } else {
                        draft.selected_choices.push(choice);
                    }
                } else if draft.selected_choices.as_slice() == [choice.as_str()] {
                    draft.selected_choices.clear();
                } else {
                    draft.selected_choices = vec![choice];
                }
                Effect::none()
            }
            TodoAction::AnswerQuestion { ask_id, text, selected_choices } => {
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
                state.question_drafts.remove(&ask_id);
                state.error = None;
                let reply = self.client.answer_question(&ask_id, text, selected_choices);
                Effect::future(async move {
                    TodoAction::Answered { before, result: reply.await }
                })
            }
            TodoAction::AnswerAction { ask_id, done, note } => {
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
                let reply = self.client.answer_action(&ask_id, done, note);
                Effect::future(async move {
                    TodoAction::Answered { before, result: reply.await }
                })
            }
            TodoAction::AnswerReview { ask_id, approved, note } => {
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
                let reply = self.client.answer_review(&ask_id, approved, note);
                Effect::future(async move {
                    TodoAction::Answered { before, result: reply.await }
                })
            }
            TodoAction::RetryStuck(key) => {
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                let before = state.items[at].clone();
                state.items[at].todo.stuck_agent = None;
                state.error = None;
                let reply = self.client.resolve_ownership(&key, false, true);
                Effect::future(async move {
                    TodoAction::Written { before, result: reply.await }
                })
            }
            TodoAction::RequestTakeBack(key) => {
                if state.position(&key).is_some() {
                    state.confirm_take_back = Some(key);
                }
                Effect::none()
            }
            TodoAction::CancelTakeBack => {
                state.confirm_take_back = None;
                Effect::none()
            }
            TodoAction::ConfirmTakeBack(key) => {
                state.confirm_take_back = None;
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                let before = state.items[at].clone();
                state.items[at].todo.owner = Some(TodoOwner {
                    kind: OwnerKind::User,
                    label: "You".to_owned(),
                    session: None,
                });
                state.items[at].todo.stuck_agent = None;
                state.error = None;
                let reply = self.client.resolve_ownership(&key, true, false);
                Effect::future(async move {
                    TodoAction::Written { before, result: reply.await }
                })
            }
            TodoAction::ResolveBlockerGone { key, blocker_key, resolution } => {
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                let before = state.items[at].clone();
                state.items[at].todo.blocker_gone.retain(|b| b.blocker_key != blocker_key);
                state.items[at].todo.depends_on.retain(|d| d != &blocker_key);
                state.error = None;
                let reply = self.client.resolve_blocker_gone(&key, &blocker_key, resolution);
                Effect::future(async move {
                    TodoAction::Written { before, result: reply.await }
                })
            }
            TodoAction::ResolveLifecyclePrompt { key, prompt_id, choice } => {
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                let before = state.items[at].clone();
                state.items[at].todo.lifecycle_prompts.retain(|p| p.id != prompt_id);
                state.error = None;
                let reply = self.client.resolve_lifecycle_prompt(&key, &prompt_id, &choice);
                Effect::future(async move {
                    TodoAction::Written { before, result: reply.await }
                })
            }

            // --- TODO2: Diff & Markdown Comment Hooks ---
            TodoAction::AddStepFromComment { key, text, anchor } => {
                let text = text.trim().to_owned();
                if text.is_empty() {
                    return Effect::none();
                }
                let Some(at) = state.position(&key) else {
                    return Effect::none();
                };
                let mut steps = state.items[at].todo.steps.clone();
                steps.push(Step {
                    text,
                    source_anchor: Some(anchor),
                    ..Step::default()
                });
                self.patch(state, &key, TodoPatch { steps: Some(steps), ..TodoPatch::default() })
            }
            TodoAction::SpinOffFromComment { from_key, title, anchor, channel } => {
                let title = title.trim().to_owned();
                if title.is_empty() {
                    return Effect::none();
                }
                state.error = None;
                let reply = self.client.create(NewTodo {
                    title,
                    channel,
                    source_anchor: Some(anchor),
                    from_keys: from_key.into_iter().collect(),
                    ..NewTodo::default()
                });
                Effect::future(async move { TodoAction::Created(reply.await) })
            }
            TodoAction::NavigateAnchor(_) => Effect::none(),
        }
    }
}

/// Steps moving or dropping shift the indices an open step-context editor
/// points at, so the editor closes without saving.
fn cancel_step_context_edit(state: &mut TodoState, key: &str) {
    if state
        .editing
        .as_ref()
        .is_some_and(|e| e.key == key && matches!(e.field, EditField::StepContext(_)))
    {
        state.editing = None;
    }
}
