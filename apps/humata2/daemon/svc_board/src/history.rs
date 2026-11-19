//! An item's history as the API shows it (BD-3, SL-6): the store's entries,
//! each said in words for the CLI and structured for the app (D155).

use crate::pb;
use humata2_rpc_kit::timestamp_ms;
use humata2_store::{BoardHistoryEntry, BoardHistoryKind};

use super::scope::column_label;

fn kind_pb(kind: BoardHistoryKind) -> pb::HistoryKind {
    match kind {
        BoardHistoryKind::Created => pb::HistoryKind::Created,
        BoardHistoryKind::FieldChanged => pb::HistoryKind::Changed,
        BoardHistoryKind::Moved => pb::HistoryKind::Moved,
        BoardHistoryKind::Deleted => pb::HistoryKind::Deleted,
        BoardHistoryKind::SessionsChanged => pb::HistoryKind::Sessions,
        BoardHistoryKind::DependenciesChanged => pb::HistoryKind::Dependencies,
    }
}

/// A stored value as shown: columns by name, the rest as kept.
fn shown(field: &str, value: &str) -> String {
    if field == "column" {
        value
            .parse::<i32>()
            .map(|column| column_label(column).to_owned())
            .unwrap_or_else(|_| value.to_owned())
    } else {
        value.to_owned()
    }
}

/// The entry in words: "created", "column To do → In review", "deleted".
fn words(entry: &BoardHistoryEntry) -> String {
    let before = shown(&entry.field, &entry.before);
    let after = shown(&entry.field, &entry.after);
    let text = match entry.kind {
        BoardHistoryKind::Created => "created".to_owned(),
        BoardHistoryKind::Deleted => "deleted".to_owned(),
        BoardHistoryKind::Moved if entry.field == "rank" => "moved within its column".to_owned(),
        BoardHistoryKind::FieldChanged if entry.field == "description" => {
            "description changed".to_owned()
        }
        _ => format!("{} {before} → {after}", entry.field),
    };
    if entry.lost {
        format!("lost to a later write: {text}")
    } else {
        text
    }
}

/// `entry` for the API; `by` is who made it.
pub(super) fn entry_pb(entry: &BoardHistoryEntry, by: pb::Actor) -> pb::ItemHistoryEntry {
    pb::ItemHistoryEntry {
        at: Some(pb::Timestamped {
            at: Some(timestamp_ms(entry.at_unix_ms)),
            by: Some(by),
        }),
        change: words(entry),
        lost_conflict: entry.lost,
        journal: None,
        revision: Some(pb::Revision {
            value: entry.revision,
        }),
        kind: kind_pb(entry.kind) as i32,
        field: entry.field.clone(),
        before: shown(&entry.field, &entry.before),
        after: shown(&entry.field, &entry.after),
        cause: (!entry.cause.is_empty()).then(|| pb::SessionId {
            value: entry.cause.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: BoardHistoryKind, field: &str, before: &str, after: &str) -> BoardHistoryEntry {
        BoardHistoryEntry {
            n: 1,
            at_unix_ms: 0,
            by: String::new(),
            kind,
            field: field.to_owned(),
            before: before.to_owned(),
            after: after.to_owned(),
            lost: false,
            revision: 1,
            cause: String::new(),
        }
    }

    #[test]
    fn a_column_move_names_both_columns_and_marks_its_kind() {
        let moved = entry_pb(
            &entry(BoardHistoryKind::Moved, "column", "2", "4"),
            pb::Actor::default(),
        );
        assert_eq!(moved.kind(), pb::HistoryKind::Moved);
        assert_eq!(moved.before, "To do");
        assert_eq!(moved.after, "In review");
        assert!(moved.change.contains("To do") && moved.change.contains("In review"));
    }

    #[test]
    fn a_lost_write_is_flagged() {
        let mut lost = entry(BoardHistoryKind::FieldChanged, "title", "a", "b");
        lost.lost = true;
        assert!(entry_pb(&lost, pb::Actor::default()).lost_conflict);
    }

    #[test]
    fn a_cause_is_set_only_when_a_session_start_made_the_change() {
        let mut made = entry(BoardHistoryKind::Created, "", "", "");
        assert!(entry_pb(&made, pb::Actor::default()).cause.is_none());
        made.cause = "se_a".to_owned();
        assert!(entry_pb(&made, pb::Actor::default()).cause.is_some());
    }
}
