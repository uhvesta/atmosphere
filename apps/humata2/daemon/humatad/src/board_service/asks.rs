//! An agent's board write that waits in the Inbox (BD-15, D155): the card's
//! change field by field, and the same requests shown on the item itself.

use humata2_grpc::humata::v1 as pb;
use humata2_store::BoardItemRecord;

use super::scope::{column_label, column_pb};
use super::{FIRST_COLUMN, Op};
use crate::inbox::BoardAsk;

pub(super) fn field_change(field: &str, before: &str, after: &str) -> pb::FieldChange {
    pb::FieldChange {
        field: field.to_owned(),
        before: before.to_owned(),
        after: after.to_owned(),
    }
}

/// The card for a change to `item`; `deletes` for a deletion.
pub(super) fn change_ask(
    item: &BoardItemRecord,
    changes: Vec<pb::FieldChange>,
    deletes: bool,
    reason: &str,
) -> BoardAsk {
    BoardAsk {
        preview: pb::request_preview::Preview::BoardChange(pb::BoardChangePreview {
            item: Some(pb::BoardItemId {
                value: item.key.clone(),
            }),
            item_title: item.title.clone(),
            changes,
            deletes_item: deletes,
        }),
        reason: reason.to_owned(),
    }
}

/// The card for a new item.
pub(super) fn new_item_ask(preview: pb::NewBoardItemPreview, reason: &str) -> BoardAsk {
    BoardAsk {
        preview: pb::request_preview::Preview::NewBoardItem(preview),
        reason: reason.to_owned(),
    }
}

/// The card for a write made while the host is away, from the call alone:
/// the host's board isn't there to read. A new item shows as it will be
/// made; an edit names the item and the values it sets, with no "before",
/// which the host fills in when it shows the card (BD-15, D268).
pub(super) fn away_ask(op: &Op) -> BoardAsk {
    let change = |id: &Option<pb::BoardItemId>, changes: Vec<pb::FieldChange>, deletes: bool, reason: &str| BoardAsk {
        preview: pb::request_preview::Preview::BoardChange(pb::BoardChangePreview {
            item: id.clone(),
            item_title: String::new(),
            changes,
            deletes_item: deletes,
        }),
        reason: reason.to_owned(),
    };
    match op {
        Op::Create(request) => new_item_ask(
            pb::NewBoardItemPreview {
                board: request.board.clone(),
                title: request.title.trim().to_owned(),
                column: match request.column {
                    0 => column_pb(FIRST_COLUMN) as i32,
                    column => column,
                },
                description: request.description.clone(),
            },
            &request.reason,
        ),
        Op::Update(request) => {
            let patch = request.patch.clone().unwrap_or_default();
            let mut changes = Vec::new();
            if let Some(title) = &patch.title {
                changes.push(field_change("title", "", title));
            }
            if let Some(text) = &patch.description {
                changes.push(field_change("description", "", text));
            }
            let signed = |add: &[String], remove: &[String]| {
                add.iter()
                    .map(|id| format!("+{id}"))
                    .chain(remove.iter().map(|id| format!("-{id}")))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let ids = |ids: &[pb::SessionId]| ids.iter().map(|id| id.value.clone()).collect::<Vec<_>>();
            let items = |ids: &[pb::BoardItemId]| ids.iter().map(|id| id.value.clone()).collect::<Vec<_>>();
            let sessions = signed(&ids(&patch.sessions_add), &ids(&patch.sessions_remove));
            if !sessions.is_empty() {
                changes.push(field_change("sessions", "", &sessions));
            }
            let depends = signed(&items(&patch.depends_on_add), &items(&patch.depends_on_remove));
            if !depends.is_empty() {
                changes.push(field_change("depends_on", "", &depends));
            }
            change(&request.id, changes, false, &request.reason)
        }
        Op::Move(request) => change(
            &request.id,
            vec![field_change("status", "", column_label(request.column))],
            false,
            &request.reason,
        ),
        Op::Delete(request) => change(&request.id, Vec::new(), true, &request.reason),
    }
}

/// What `field` holds on `item` now, in the words a card uses.
fn now(field: &str, item: &BoardItemRecord) -> Option<String> {
    match field {
        "status" => Some(column_label(item.column).to_owned()),
        "title" => Some(item.title.clone()),
        "description" => Some(item.description.clone()),
        "sessions" => Some(item.sessions.join(", ")),
        "depends_on" => Some(item.depends_on.join(", ")),
        "position" => Some(String::new()),
        _ => None,
    }
}

/// The pending requests among `asks` that are about `item`, as it stands:
/// each change's "before" is read from the item now, since it may have
/// changed since the agent asked.
pub(super) fn pending_for(item: &BoardItemRecord, asks: &[pb::InboxRequest]) -> Vec<pb::PendingAsk> {
    asks.iter()
        .filter_map(|ask| {
            let Some(pb::request_preview::Preview::BoardChange(change)) =
                ask.preview.as_ref().and_then(|preview| preview.preview.as_ref())
            else {
                return None;
            };
            if change.item.as_ref().map(|id| id.value.as_str()) != Some(item.key.as_str()) {
                return None;
            }
            let mut change = change.clone();
            for each in &mut change.changes {
                if let Some(before) = now(&each.field, item) {
                    each.before = before;
                }
            }
            let requester = ask.requester.clone();
            Some(pb::PendingAsk {
                request: ask.id.clone(),
                session: requester.as_ref().and_then(|agent| agent.session.clone()),
                asked_by: requester.map(|agent| pb::Actor {
                    actor: Some(pb::actor::Actor::Agent(agent)),
                }),
                change: Some(change),
                reason: ask.reason.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use humata2_store::BoardKey;

    use super::*;

    fn item(column: i32) -> BoardItemRecord {
        BoardItemRecord {
            key: "PAY-1".to_owned(),
            board: BoardKey::Global,
            channel: String::new(),
            title: "t".to_owned(),
            description: String::new(),
            column,
            rank: "n".to_owned(),
            created_by: String::new(),
            created_unix_ms: 0,
            updated_unix_ms: 0,
            revision: 1,
            done_unix_ms: None,
            deleted: false,
            sessions: Vec::new(),
            depends_on: Vec::new(),
        }
    }

    fn request_about(key: &str) -> pb::InboxRequest {
        let ask = change_ask(
            &BoardItemRecord {
                key: key.to_owned(),
                ..item(2)
            },
            vec![field_change("status", "To do", "Done")],
            false,
            "why",
        );
        pb::InboxRequest {
            preview: Some(pb::RequestPreview {
                preview: Some(ask.preview),
            }),
            reason: ask.reason,
            requester: Some(pb::AgentActor::default()),
            ..Default::default()
        }
    }

    #[test]
    fn an_away_edits_card_names_the_item_and_the_new_values_with_no_before() {
        let ask = away_ask(&Op::Move(pb::MoveItemRequest {
            id: Some(pb::BoardItemId {
                value: "PAY-7".to_owned(),
            }),
            column: pb::BoardColumn::InReview as i32,
            reason: "ready".to_owned(),
            ..Default::default()
        }));
        let pb::request_preview::Preview::BoardChange(change) = ask.preview else {
            panic!("an edit's card shows a board change");
        };
        assert_eq!(change.item.map(|id| id.value), Some("PAY-7".to_owned()));
        assert_eq!(change.changes.len(), 1);
        assert_eq!(change.changes[0].before, "");
        assert_eq!(change.changes[0].after, "In review");
        assert_eq!(ask.reason, "ready");
    }

    #[test]
    fn an_away_adds_card_shows_the_new_item() {
        let ask = away_ask(&Op::Create(pb::CreateItemRequest {
            title: "  Fix login ".to_owned(),
            ..Default::default()
        }));
        let pb::request_preview::Preview::NewBoardItem(made) = ask.preview else {
            panic!("an add's card shows the new item");
        };
        assert_eq!(made.title, "Fix login");
        assert_eq!(made.column, pb::BoardColumn::Backlog as i32);
    }

    #[test]
    fn only_requests_about_the_item_are_pending_on_it() {
        let asks = [request_about("PAY-1"), request_about("PAY-2")];
        assert_eq!(pending_for(&item(2), &asks).len(), 1);
    }

    #[test]
    fn the_before_of_a_pending_change_is_the_item_as_it_is_now() {
        let asks = [request_about("PAY-1")];
        let pending = pending_for(&item(3), &asks);
        let change = pending[0].change.clone().unwrap_or_default();
        assert_eq!(change.changes[0].before, "In progress");
        assert_eq!(change.changes[0].after, "Done");
    }
}
