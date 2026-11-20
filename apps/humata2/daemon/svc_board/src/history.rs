//! An item's history as the API shows it (BD-3, SL-6, TODO2): the store's
//! entries, each said in words for the CLI and structured for the app (D155),
//! with word-level diffs for text changes.

use crate::pb;
use humata2_rpc_kit::timestamp_ms;
use humata2_store::{BoardHistoryEntry, BoardHistoryKind};
use humata2_todo::{WordDiffKind, word_diff};

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

/// Converts a pure word-level diff between `before` and `after` to proto spans.
pub(super) fn word_diff_pb(before: &str, after: &str) -> Vec<pb::WordDiffSpan> {
    if before.is_empty() && after.is_empty() {
        return Vec::new();
    }
    word_diff(before, after)
        .into_iter()
        .map(|span| pb::WordDiffSpan {
            kind: match span.kind {
                WordDiffKind::Equal => pb::WordDiffKind::Equal as i32,
                WordDiffKind::Added => pb::WordDiffKind::Added as i32,
                WordDiffKind::Removed => pb::WordDiffKind::Removed as i32,
            },
            text: span.text,
        })
        .collect()
}

fn is_text_field(field: &str) -> bool {
    matches!(
        field,
        "title" | "description" | "steps" | "step" | "comment" | "wont_do_reason"
    )
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
    let before = shown(&entry.field, &entry.before);
    let after = shown(&entry.field, &entry.after);
    let diff = if entry.kind == BoardHistoryKind::FieldChanged && is_text_field(&entry.field) {
        word_diff_pb(&before, &after)
    } else {
        Vec::new()
    };
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
        before,
        after,
        cause: (!entry.cause.is_empty()).then(|| pb::SessionId {
            value: entry.cause.clone(),
        }),
        word_diff: diff,
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
        assert!(moved.word_diff.is_empty());
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

    #[test]
    fn title_and_description_changes_include_word_diff() {
        let changed = entry_pb(
            &entry(
                BoardHistoryKind::FieldChanged,
                "title",
                "Fix login loop",
                "Fix auth loop",
            ),
            pb::Actor::default(),
        );
        assert_eq!(changed.word_diff.len(), 4);
        assert_eq!(changed.word_diff[0].text, "Fix");
        assert_eq!(changed.word_diff[1].kind, pb::WordDiffKind::Removed as i32);
        assert_eq!(changed.word_diff[1].text, "login");
        assert_eq!(changed.word_diff[2].kind, pb::WordDiffKind::Added as i32);
        assert_eq!(changed.word_diff[2].text, "auth");
        assert_eq!(changed.word_diff[3].text, "loop");
    }
}
