//! The To-do list, as the host keeps it (BOARDSIMPLE).
//!
//! One list for every channel and session; channel and session are filters
//! on it. The app ranks what it gets with `humata2_todo::rank`, so the
//! client only stores, filters and writes. [`FakeTodo`] is the only
//! implementation until the live client serves it.

use humata2_todo::{
    ChannelFilter, CheckpointState, Day, Draft, Effort, ExternalBlock, FuzzyDue, Impact, Need,
    RunEntry, SourceAnchor, State, Step, Summary, Todo, TodoComment, TodoOwner, WatchedPr,
    WordDiffSpan,
};
use tokio::sync::mpsc;

use crate::daemon::{DaemonError, Reply};

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
    /// Loose/fuzzy due date.
    pub fuzzy_due: Option<FuzzyDue>,
    /// How big the job is.
    pub effort: Option<Effort>,
    /// How much it matters.
    pub impact: Option<Impact>,
    /// How needed it is.
    pub need: Option<Need>,
    /// Its tags.
    pub tags: Vec<String>,
    /// Its longer text.
    pub description: String,
    /// Optional diff or markdown source anchor.
    pub source_anchor: Option<SourceAnchor>,
    /// Lineage keys this item was spun off or split from.
    pub from_keys: Vec<String>,
    /// Optional external block reason.
    pub external_block: Option<ExternalBlock>,
}

impl From<Draft> for NewTodo {
    fn from(draft: Draft) -> Self {
        Self {
            title: draft.title,
            channel: draft.channel,
            due: draft.due,
            fuzzy_due: draft.fuzzy_due,
            effort: draft.effort,
            impact: draft.impact,
            need: draft.need,
            tags: draft.tags,
            description: String::new(),
            source_anchor: None,
            from_keys: Vec::new(),
            external_block: None,
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
    /// A new channel, or none.
    pub channel: Option<Option<String>>,
    /// A new due date, or none.
    pub due: Option<Option<Day>>,
    /// A new fuzzy due date, or none.
    pub fuzzy_due: Option<Option<FuzzyDue>>,
    /// A new effort, or none.
    pub effort: Option<Option<Effort>>,
    /// A new impact, or none.
    pub impact: Option<Option<Impact>>,
    /// A new need level, or none.
    pub need: Option<Option<Need>>,
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
    /// External block reason to set or clear.
    pub external_block: Option<Option<ExternalBlock>>,
    /// Owner to set or clear.
    pub owner: Option<Option<TodoOwner>>,
    /// Checkpoint state to set or clear.
    pub checkpoint: Option<Option<CheckpointState>>,
    /// Dependency keys to add.
    pub depends_on_add: Vec<String>,
    /// Dependency keys to remove.
    pub depends_on_remove: Vec<String>,
    /// Reason when setting state to Won't do.
    pub wont_do_reason: Option<String>,
    /// Watched PRs to add.
    pub watched_prs_add: Vec<WatchedPr>,
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
            if state.is_terminal() {
                todo.closed = Some(today);
                todo.pin = None;
            } else {
                todo.closed = None;
            }
        }
        if let Some(channel) = &self.channel {
            todo.channel.clone_from(channel);
        }
        if let Some(due) = self.due {
            todo.due = due;
            if due.is_some() && self.fuzzy_due.is_none() {
                todo.fuzzy_due = None;
            }
        }
        if let Some(fuzzy) = self.fuzzy_due {
            todo.fuzzy_due = fuzzy;
            if fuzzy.is_some() && self.due.is_none() {
                todo.due = None;
            }
        }
        if let Some(effort) = self.effort {
            todo.effort = effort;
        }
        if let Some(impact) = self.impact {
            todo.impact = impact;
        }
        if let Some(need) = self.need {
            todo.need = need;
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
        todo.tags
            .retain(|t| !self.tags_remove.iter().any(|r| r.eq_ignore_ascii_case(t)));
        if let Some(steps) = &self.steps {
            todo.steps.clone_from(steps);
        }
        if let Some(description) = &self.description {
            todo.description.clone_from(description);
        }
        if self.accept_suggestion {
            todo.suggested = false;
        }
        if let Some(ext) = &self.external_block {
            todo.external_block.clone_from(ext);
        }
        if let Some(owner) = &self.owner {
            todo.owner.clone_from(owner);
        }
        if let Some(checkpoint) = self.checkpoint {
            todo.checkpoint = checkpoint;
        }
        for dep in &self.depends_on_add {
            if !todo.depends_on.contains(dep) {
                todo.depends_on.push(dep.clone());
            }
        }
        todo.depends_on
            .retain(|d| !self.depends_on_remove.contains(d));
        if let Some(reason) = &self.wont_do_reason {
            todo.wont_do_reason = Some(reason.clone());
        }
        for pr in &self.watched_prs_add {
            if !todo.watched_prs.iter().any(|w| w.repo_pr == pr.repo_pr) {
                todo.watched_prs.push(pr.clone());
            }
        }
    }
}

/// Result of splitting a to-do item into multiple items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitResult {
    pub source: Entry,
    pub created: Vec<Entry>,
}

/// Result of folding one to-do item into a step on another item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldResult {
    pub source: Entry,
    pub target: Entry,
}

/// One entry in a bulk update request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BulkUpdateItem {
    pub key: String,
    pub patch: TodoPatch,
    pub expected_revision: u64,
}

/// One failed item in a bulk update response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BulkUpdateFailure {
    pub key: String,
    pub reason: String,
}

/// Result of a bulk update: items that succeeded and per-item refusals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BulkUpdateResult {
    pub updated: Vec<Entry>,
    pub failed: Vec<BulkUpdateFailure>,
}

/// One history entry for a to-do item, with optional word-level diff spans.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TodoHistoryEntry {
    pub at_day: Option<Day>,
    pub actor: String,
    pub kind: String,
    pub field: String,
    pub before: String,
    pub after: String,
    pub change: String,
    pub word_diff: Vec<WordDiffSpan>,
}

/// Runs ledger for a to-do item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TodoRunsResult {
    pub runs: Vec<RunEntry>,
    /// `None` when no run has a known cost.
    pub total_cost_micros: Option<u64>,
}

impl TodoRunsResult {
    /// Builds a ledger whose total sums the known costs only.
    #[must_use]
    pub fn from_runs(runs: Vec<RunEntry>) -> Self {
        let total_cost_micros = known_cost_total(runs.iter().map(|r| r.cost_micros));
        Self { runs, total_cost_micros }
    }
}

/// Sums the known costs; `None` when none is known.
pub(crate) fn known_cost_total(costs: impl IntoIterator<Item = Option<u64>>) -> Option<u64> {
    costs.into_iter().flatten().fold(None, |acc, c| Some(acc.unwrap_or(0).saturating_add(c)))
}

/// How to resolve a BlockerGone notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockerGoneResolution {
    Unblock,
    CloseWontDo,
    Rewire(Vec<String>),
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
    /// The day the list is ranked and dated on: this computer's (UTC) day.
    /// Fakes answer the fixed day their frames were signed off on, so a
    /// capture doesn't change with the wall clock.
    fn today(&self) -> Day {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Day::from_epoch_days(i32::try_from(secs / 86_400).unwrap_or(0))
    }

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

    /// Appends a comment to `key` (optionally anchored to `step_id`).
    fn add_comment(
        &self,
        _key: &str,
        _body: String,
        _step_id: Option<String>,
    ) -> Reply<(Entry, TodoComment)> {
        unsupported("Adding a comment")
    }

    /// Splits `key` into new items with `titles`, recording `from_keys` / `became_keys`.
    fn split(&self, _key: &str, _titles: Vec<String>, _keep_age: bool) -> Reply<SplitResult> {
        unsupported("Splitting a to-do")
    }

    /// Folds `source_key` into a step on `target_key`, recording `from_keys` / `became_keys`.
    fn fold_into_step(&self, _source_key: &str, _target_key: &str) -> Reply<FoldResult> {
        unsupported("Folding a to-do into a step")
    }

    /// Applies multiple updates, returning per-item failures without aborting valid items.
    fn bulk_update(&self, _items: Vec<BulkUpdateItem>) -> Reply<BulkUpdateResult> {
        unsupported("Bulk updating to-dos")
    }

    /// Returns item history including word-level diffs.
    fn history(&self, _key: &str) -> Reply<Vec<TodoHistoryEntry>> {
        unsupported("Loading to-do history")
    }

    /// Returns recorded agent runs and total cost for `key`.
    fn runs(&self, _key: &str) -> Reply<TodoRunsResult> {
        unsupported("Loading to-do runs")
    }

    /// Answers a structured Question ask (free-text and/or selected choices).
    fn answer_question(
        &self,
        _ask_id: &str,
        _text: String,
        _selected_choices: Vec<String>,
    ) -> Reply<()> {
        unsupported("Answering a question")
    }

    /// Answers an ActionRequest ask (`done = true` for Done, `false` for Can't be done).
    fn answer_action(&self, _ask_id: &str, _done: bool, _note: String) -> Reply<()> {
        unsupported("Answering an action request")
    }

    /// Answers a ReviewRequest ask (`approved = true` for Approved, `false` for Changes asked).
    fn answer_review(&self, _ask_id: &str, _approved: bool, _note: String) -> Reply<()> {
        unsupported("Answering a review request")
    }

    /// Resolves ownership / stuck-agent state (take back to user or retry agent).
    fn resolve_ownership(&self, _key: &str, _take_back: bool, _retry: bool) -> Reply<Entry> {
        unsupported("Resolving to-do ownership")
    }

    /// Resolves a BlockerGone notice on `key`.
    fn resolve_blocker_gone(
        &self,
        _key: &str,
        _blocker_key: &str,
        _resolution: BlockerGoneResolution,
    ) -> Reply<Entry> {
        unsupported("Resolving a gone blocker")
    }

    /// Resolves a quiet lifecycle prompt on `key`.
    fn resolve_lifecycle_prompt(&self, _key: &str, _prompt_id: &str, _choice: &str) -> Reply<Entry> {
        unsupported("Resolving a lifecycle prompt")
    }
}

fn unsupported<T: Send + 'static>(what: &str) -> Reply<T> {
    Box::pin(std::future::ready(Err(DaemonError::Refused {
        what: what.to_owned(),
        reason: "not supported by this client".to_owned(),
    })))
}

#[cfg(test)]
mod cost_tests {
    use super::*;

    fn run(cost: Option<u64>) -> RunEntry {
        RunEntry { cost_micros: cost, ..RunEntry::default() }
    }

    #[test]
    fn total_sums_known_costs_and_skips_unknown() {
        let res = TodoRunsResult::from_runs(vec![run(Some(1_250_000)), run(None)]);
        assert_eq!(res.total_cost_micros, Some(1_250_000));
    }

    #[test]
    fn total_is_unknown_when_no_cost_is_known() {
        let res = TodoRunsResult::from_runs(vec![run(None), run(None)]);
        assert_eq!(res.total_cost_micros, None);
        assert_eq!(TodoRunsResult::from_runs(vec![]).total_cost_micros, None);
    }
}
