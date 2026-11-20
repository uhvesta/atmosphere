//! The to-do fields kept beside each board item (BOARDSIMPLE, D832, TODO2).
//!
//! The board item stays in the store as it always was; what the list adds
//! to it lives in one saved document of this crate, keyed by item key.
//! Every field is additive with a default, so a document written by an
//! older build still reads, and an item with no entry reads as a plain old
//! board item whose state comes from its column.

use std::collections::BTreeMap;

use humata2_store::{Store, StoreError};
use humata2_todo::{CheckpointState, Day, Effort, FuzzyDue, Impact, Need, OwnerKind, State};
use serde::{Deserialize, Serialize};

const FIELDS_KEY: &str = "todo.fields";
const MIGRATION_KEY: &str = "todo.migration";

/// The Done column, where a closed item sits.
pub(super) const DONE_COLUMN: i32 = 5;
/// The To do column, where an open item sits.
pub(super) const TODO_COLUMN: i32 = 2;
/// The In progress column.
pub(super) const IN_PROGRESS_COLUMN: i32 = 3;

/// Why the saved to-do fields couldn't be read or kept.
#[derive(Debug, thiserror::Error)]
pub(super) enum FieldsError {
    #[error("couldn't read the saved to-do {what}")]
    Read {
        what: &'static str,
        #[source]
        source: StoreError,
    },
    #[error("couldn't save the to-do {what}")]
    Write {
        what: &'static str,
        #[source]
        source: StoreError,
    },
    #[error(
        "the saved to-do {what} isn't readable ({source}); restore the state folder from a backup, or ask for help before deleting it"
    )]
    Corrupt {
        what: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("couldn't write the to-do {what} as text")]
    Encode {
        what: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

/// Optional diff or markdown source anchor on an item or step.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SourceAnchorRow {
    pub repo: String,
    pub path: String,
    pub line: u32,
    pub end_line: u32,
    pub comment_id: String,
}

/// A step inside an item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct StepRow {
    pub id: String,
    pub text: String,
    pub done: bool,
    /// The session that ticked it.
    pub ticked_by: Option<String>,
    pub markdown_context: Option<String>,
    pub dropped: bool,
    pub from_key: Option<String>,
    pub source_anchor: Option<SourceAnchorRow>,
}

/// Free-text external blocker reason.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct ExternalBlockRow {
    pub reason: String,
    pub since_ms: Option<i64>,
}

/// Append-only comment on an item or step.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct TodoCommentRow {
    pub id: String,
    pub author: String,
    pub body: String,
    pub created_at_ms: i64,
    pub step_id: Option<String>,
}

/// Explicit current owner (user or agent session).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TodoOwnerRow {
    pub kind: OwnerKind,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub session: Option<String>,
}

/// Notice when an assigned agent appears stuck.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct StuckAgentRow {
    pub session: String,
    pub agent_label: String,
    pub reason: String,
    pub since_ms: i64,
}

/// Notice when a dependency closed as Won't do or Became.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct BlockerGoneRow {
    pub blocker_key: String,
    pub blocker_state: State,
    #[serde(default)]
    pub replacement_keys: Vec<String>,
}

/// A pull request or stack watched by a to-do item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct WatchedPrRow {
    pub owner: String,
    pub name: String,
    pub number: u32,
    pub note: String,
    pub title: String,
    pub ci_state: String,
    pub unresolved_comments: u32,
    pub has_conflicts: bool,
    pub upstream_merged: bool,
    pub merged: bool,
    pub closed_without_merge: bool,
    pub stack_key: String,
}

impl WatchedPrRow {
    pub(super) fn repo_pr(&self) -> String {
        if self.owner.is_empty() && self.name.is_empty() {
            format!("#{}", self.number)
        } else {
            format!("{}/{}#{}", self.owner, self.name, self.number)
        }
    }
}

/// Compact close-readiness summary.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct CloseReadinessRow {
    pub steps_done: u32,
    pub steps_total: u32,
    pub unresolved_comments: u32,
    pub ci_status: String,
    pub commits: u32,
}

/// Quiet live session signal on an In progress item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SessionSignalRow {
    pub session: String,
    pub label: String,
    pub status: String,
    pub idle_days: u32,
    pub active_turn: bool,
}

/// Quiet 1-click lifecycle prompt (PR merge, idle session, etc.).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct LifecyclePromptRow {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub choices: Vec<String>,
    pub pr_ref: String,
    pub session: Option<String>,
}

/// Extra history entry recorded for to-do-only operations (comments, split,
/// fold, ownership, PR sync, etc.).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct TodoHistoryRow {
    pub at_unix_ms: i64,
    pub by: String,
    pub kind: i32,
    pub field: String,
    pub before: String,
    pub after: String,
    pub change: String,
    pub revision: u64,
}

/// Where a migrated item stood on the boards.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StartRow {
    /// The store's column number, 1 to 5.
    #[serde(default)]
    pub column: i32,
    #[serde(default)]
    pub position: u32,
    /// The channel of the board it was on; none for the Global board.
    #[serde(default)]
    pub board_channel: Option<String>,
    #[serde(default)]
    pub was_vip: bool,
}

/// One pull request of a review request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PrRow {
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub note: String,
}

/// A pull request review request item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ReviewRow {
    #[serde(default)]
    pub pull_requests: Vec<PrRow>,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub requested_unix_ms: i64,
}

/// What the list keeps for one item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Fields {
    /// The state the list set, good while the item is still in
    /// `state_column`; a board move after that makes the column win again.
    pub state: Option<State>,
    pub state_column: Option<i32>,
    pub due: Option<Day>,
    pub effort: Option<Effort>,
    pub impact: Option<Impact>,
    pub tags: Vec<String>,
    pub steps: Vec<StepRow>,
    pub hide_until: Option<Day>,
    /// Pinned items, smaller first; 0 is unpinned.
    pub pin_order: u32,
    pub suggested: bool,
    pub start: Option<StartRow>,
    pub review: Option<ReviewRow>,
    pub closed_at_ms: Option<i64>,
    /// The channel the list moved the item to; an empty text is no channel.
    pub channel: Option<String>,
    /// Applied changes that left the board item alone, so the revision still
    /// goes up with every change.
    pub extra_revisions: u64,
    // TODO2 extended fields
    pub need: Option<Need>,
    pub fuzzy_due: Option<FuzzyDue>,
    pub external_block: Option<ExternalBlockRow>,
    pub from_keys: Vec<String>,
    pub became_keys: Vec<String>,
    pub source_anchor: Option<SourceAnchorRow>,
    pub comments: Vec<TodoCommentRow>,
    pub owner: Option<TodoOwnerRow>,
    pub checkpoint: Option<CheckpointState>,
    pub stuck_agent: Option<StuckAgentRow>,
    pub blocker_gone: Vec<BlockerGoneRow>,
    pub watched_prs: Vec<WatchedPrRow>,
    pub close_readiness: Option<CloseReadinessRow>,
    pub session_signals: Vec<SessionSignalRow>,
    pub lifecycle_prompts: Vec<LifecyclePromptRow>,
    /// The sessions a lifecycle prompt acts on, by prompt id.
    pub prompt_sessions: BTreeMap<String, Vec<String>>,
    pub wont_do_reason: Option<String>,
    pub extra_history: Vec<TodoHistoryRow>,
}

/// What a board column says about state, for an item with no say of its own.
pub(super) fn state_of_column(column: i32) -> State {
    match column {
        DONE_COLUMN => State::Closed,
        IN_PROGRESS_COLUMN | 4 => State::InProgress,
        _ => State::Open,
    }
}

/// The column a state sits in: the item's own when it already agrees,
/// else the state's usual one.
pub(super) fn column_for(state: State, current: i32) -> i32 {
    if state_of_column(current) == state {
        current
    } else {
        match state {
            State::Open => TODO_COLUMN,
            State::InProgress | State::WatchingPr => IN_PROGRESS_COLUMN,
            State::Closed | State::WontDo | State::Became => DONE_COLUMN,
        }
    }
}

impl Fields {
    /// The state now, given the item's column.
    pub(super) fn state(&self, column: i32) -> State {
        match (self.state, self.state_column) {
            (Some(state), Some(set_in)) if set_in == column => state,
            _ => state_of_column(column),
        }
    }
}

/// What the move from boards came to (the Migrate card and the banner).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct MigrationRow {
    #[serde(default)]
    pub migrated_at_ms: i64,
    #[serde(default)]
    pub items: u32,
    #[serde(default)]
    pub open: u32,
    #[serde(default)]
    pub in_progress: u32,
    #[serde(default)]
    pub closed: u32,
    #[serde(default)]
    pub boards: u32,
    #[serde(default)]
    pub banner_dismissed: bool,
}

fn read<T: for<'a> Deserialize<'a> + Default>(
    store: &Store,
    key: &str,
    what: &'static str,
) -> Result<Option<(T, u64)>, FieldsError> {
    let saved = store
        .setting(key)
        .map_err(|source| FieldsError::Read { what, source })?;
    saved
        .map(|saved| {
            serde_json::from_str(&saved.value)
                .map(|value| (value, saved.revision))
                .map_err(|source| FieldsError::Corrupt { what, source })
        })
        .transpose()
}

fn write<T: Serialize>(
    store: &Store,
    key: &str,
    what: &'static str,
    value: &T,
    revision: u64,
) -> Result<(), FieldsError> {
    let text =
        serde_json::to_string(value).map_err(|source| FieldsError::Encode { what, source })?;
    store
        .put_setting(key, &text, revision)
        .map_err(|source| FieldsError::Write { what, source })
}

/// Every item's fields, by item key.
pub(super) fn load(store: &Store) -> Result<BTreeMap<String, Fields>, FieldsError> {
    Ok(read::<BTreeMap<String, Fields>>(store, FIELDS_KEY, "fields")?
        .map(|(map, _)| map)
        .unwrap_or_default())
}

/// Saves every item's fields. The caller holds the write lock, so no other
/// write lands between its read and this.
pub(super) fn save(store: &Store, map: &BTreeMap<String, Fields>) -> Result<(), FieldsError> {
    let revision = store
        .setting(FIELDS_KEY)
        .map_err(|source| FieldsError::Read {
            what: "fields",
            source,
        })?
        .map_or(0, |saved| saved.revision);
    write(store, FIELDS_KEY, "fields", map, revision + 1)
}

/// The move's record, or none when it hasn't run.
pub(super) fn load_migration(store: &Store) -> Result<Option<MigrationRow>, FieldsError> {
    Ok(read::<MigrationRow>(store, MIGRATION_KEY, "migration record")?.map(|(row, _)| row))
}

pub(super) fn save_migration(store: &Store, row: &MigrationRow) -> Result<(), FieldsError> {
    let revision = store
        .setting(MIGRATION_KEY)
        .map_err(|source| FieldsError::Read {
            what: "migration record",
            source,
        })?
        .map_or(0, |saved| saved.revision);
    write(store, MIGRATION_KEY, "migration record", row, revision + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_from_an_older_build_with_fewer_fields_still_reads() {
        let old = r#"{"PAY-1":{"due":null,"pin_order":2}}"#;
        let map: BTreeMap<String, Fields> = serde_json::from_str(old).unwrap();
        let fields = &map["PAY-1"];
        assert_eq!(fields.pin_order, 2);
        assert!(fields.tags.is_empty());
        assert!(!fields.suggested);
        assert_eq!(fields.need, None);
        assert!(fields.comments.is_empty());
    }

    #[test]
    fn a_row_with_no_fields_at_all_reads_with_defaults() {
        let map: BTreeMap<String, Fields> = serde_json::from_str(r#"{"A-1":{}}"#).unwrap();
        assert_eq!(map["A-1"], Fields::default());
    }

    #[test]
    fn an_old_board_item_takes_its_state_from_its_column() {
        for (column, state) in [
            (1, State::Open),
            (2, State::Open),
            (3, State::InProgress),
            (4, State::InProgress),
            (5, State::Closed),
        ] {
            assert_eq!(Fields::default().state(column), state);
        }
    }

    #[test]
    fn a_state_the_list_set_holds_until_the_item_moves_on_a_board() {
        let fields = Fields {
            state: Some(State::Closed),
            state_column: Some(2),
            ..Fields::default()
        };
        assert_eq!(fields.state(2), State::Closed);
        assert_eq!(fields.state(3), State::InProgress);
    }

    #[test]
    fn a_state_keeps_the_column_that_already_says_it() {
        assert_eq!(column_for(State::InProgress, 4), 4);
        assert_eq!(column_for(State::InProgress, 1), IN_PROGRESS_COLUMN);
        assert_eq!(column_for(State::Open, 1), 1);
        assert_eq!(column_for(State::Closed, 2), DONE_COLUMN);
        assert_eq!(column_for(State::WontDo, 2), DONE_COLUMN);
        assert_eq!(column_for(State::Became, 2), DONE_COLUMN);
        assert_eq!(column_for(State::WatchingPr, 2), IN_PROGRESS_COLUMN);
    }

    #[test]
    fn saved_fields_and_the_migration_record_round_trip_in_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("state")).unwrap();
        assert!(load(&store).unwrap().is_empty());
        assert_eq!(load_migration(&store).unwrap(), None);
        let mut map = BTreeMap::new();
        map.insert(
            "PAY-1".to_owned(),
            Fields {
                tags: vec!["a".to_owned()],
                need: Some(Need::Critical),
                fuzzy_due: Some(FuzzyDue::NextWeek),
                ..Fields::default()
            },
        );
        save(&store, &map).unwrap();
        save(&store, &map).unwrap();
        assert_eq!(load(&store).unwrap(), map);
        let row = MigrationRow {
            items: 3,
            ..MigrationRow::default()
        };
        save_migration(&store, &row).unwrap();
        assert_eq!(load_migration(&store).unwrap(), Some(row));
    }
}
