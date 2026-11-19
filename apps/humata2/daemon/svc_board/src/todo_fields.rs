//! The to-do fields kept beside each board item (BOARDSIMPLE, D832).
//!
//! The board item stays in the store as it always was; what the list adds
//! to it (due, effort, impact, tags, steps, hide until, pin order,
//! suggested, where it stood on the boards, the review request and when it
//! closed) lives in one saved document of this crate, keyed by item key.
//! Every field is additive with a default, so a document written by an
//! older build still reads, and an item with no entry reads as a plain old
//! board item whose state comes from its column.

use std::collections::BTreeMap;

use humata2_store::{Store, StoreError};
use humata2_todo::{Day, Effort, Impact, State};
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

/// A step inside an item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StepRow {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub done: bool,
    /// The session that ticked it.
    #[serde(default)]
    pub ticked_by: Option<String>,
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
pub(super) struct Fields {
    /// The state the list set, good while the item is still in
    /// `state_column`; a board move after that makes the column win again.
    #[serde(default)]
    pub state: Option<State>,
    #[serde(default)]
    pub state_column: Option<i32>,
    #[serde(default)]
    pub due: Option<Day>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default)]
    pub impact: Option<Impact>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub steps: Vec<StepRow>,
    #[serde(default)]
    pub hide_until: Option<Day>,
    /// Pinned items, smaller first; 0 is unpinned.
    #[serde(default)]
    pub pin_order: u32,
    #[serde(default)]
    pub suggested: bool,
    #[serde(default)]
    pub start: Option<StartRow>,
    #[serde(default)]
    pub review: Option<ReviewRow>,
    #[serde(default)]
    pub closed_at_ms: Option<i64>,
    /// The channel the list moved the item to; an empty text is no channel.
    #[serde(default)]
    pub channel: Option<String>,
    /// Applied changes that left the board item alone, so the revision still
    /// goes up with every change.
    #[serde(default)]
    pub extra_revisions: u64,
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
            State::InProgress => IN_PROGRESS_COLUMN,
            State::Closed => DONE_COLUMN,
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
