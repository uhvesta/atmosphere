//! Where an item sits: boards and columns in the store's, the policy's and
//! the API's words (D155), and who made a change.

use humata2_access::{BoardRef, ChannelId, MachineId, SessionId, SessionRef};
use humata2_grpc::humata::v1 as pb;
use humata2_store::BoardKey;
use pb::invalid_field::Problem;
use tonic::Status;

use crate::identity::Caller;
use crate::status;

/// The board as the API names it.
pub(super) fn board_pb(board: &BoardKey) -> pb::BoardRef {
    pb::BoardRef {
        board: Some(match board {
            BoardKey::Global => pb::board_ref::Board::Global(pb::GlobalBoard {}),
            BoardKey::Channel(id) => pb::board_ref::Board::Channel(pb::ChannelId {
                value: id.clone(),
            }),
            BoardKey::Session(id) => pb::board_ref::Board::Session(pb::SessionId {
                value: id.clone(),
            }),
        }),
    }
}

/// The board as the policy names it. `channel` is the item's channel, which
/// a session board needs.
pub(super) fn board_for_policy(board: &BoardKey, channel: &str, machine: &str) -> BoardRef {
    match board {
        BoardKey::Global => BoardRef::Global,
        BoardKey::Channel(id) => BoardRef::Channel(ChannelId::new(id.as_str())),
        BoardKey::Session(id) => BoardRef::Session(SessionRef {
            id: SessionId::new(id.as_str()),
            channel: ChannelId::new(channel),
            machine: MachineId::new(machine),
        }),
    }
}

/// The session that made an item, if an agent did: what an edit's rule is
/// judged by (BD-9).
pub(super) fn made_by_session(created_by: &str) -> Option<SessionId> {
    (!created_by.is_empty()).then(|| SessionId::new(created_by))
}

/// The value the store keeps for who made a change: empty for the user
/// (and scripts), else the agent's session.
pub(super) fn by_of(caller: &Caller) -> String {
    match caller {
        Caller::Agent(agent) => agent.session.id.to_string(),
        _ => String::new(),
    }
}

/// The store's column number, 1 to 5.
pub(super) fn column_number(column: i32, field: &str) -> Result<i32, Status> {
    match pb::BoardColumn::try_from(column) {
        Ok(pb::BoardColumn::Unspecified) | Err(_) => Err(status::invalid_field(
            field,
            Problem::UnknownValue,
            format!("{field} names no board column"),
            "use Backlog, To do, In progress, In review or Done",
        )),
        Ok(known) => Ok(known as i32),
    }
}

/// The API's column for the store's number.
pub(super) fn column_pb(column: i32) -> pb::BoardColumn {
    pb::BoardColumn::try_from(column).unwrap_or(pb::BoardColumn::Unspecified)
}

/// The column as the app shows it.
pub(super) fn column_label(column: i32) -> &'static str {
    match column_pb(column) {
        pb::BoardColumn::Backlog => "Backlog",
        pb::BoardColumn::ToDo => "To do",
        pb::BoardColumn::InProgress => "In progress",
        pb::BoardColumn::InReview => "In review",
        pb::BoardColumn::Done => "Done",
        pb::BoardColumn::Unspecified => "",
    }
}

/// `assigned_session` was replaced by `sessions` (BD-16); the old field is
/// refused, not read.
#[allow(deprecated)]
pub(super) fn refuse_assigned_in_create(request: &pb::CreateItemRequest) -> Result<(), Status> {
    if request.assigned_session.is_some() {
        return Err(refuse_assigned("sessions"));
    }
    Ok(())
}

/// As [`refuse_assigned_in_create`], for a patch.
#[allow(deprecated)]
pub(super) fn refuse_assigned_in_patch(patch: &pb::ItemPatch) -> Result<(), Status> {
    if patch.assigned_session.is_some() || patch.clear_assignment {
        return Err(refuse_assigned("sessions_add"));
    }
    Ok(())
}

fn refuse_assigned(instead: &str) -> Status {
    status::invalid_field(
        "assigned_session",
        Problem::NotAllowedHere,
        format!("assigned_session is replaced by {instead}, so the old field isn't taken"),
        format!("send the session in {instead} (and sessions_remove to take one off) instead"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_round_trip_and_unknown_ones_are_refused() {
        for number in 1..=5 {
            assert_eq!(column_number(number, "column").ok(), Some(number));
            assert!(!column_label(number).is_empty());
        }
        assert!(column_number(0, "column").is_err());
        assert!(column_number(9, "column").is_err());
    }

    #[test]
    fn only_an_agent_item_has_a_session_that_made_it() {
        assert!(made_by_session("").is_none());
        assert_eq!(made_by_session("se_a").map(|id| id.to_string()), Some("se_a".to_owned()));
    }

    #[test]
    #[allow(deprecated)]
    fn the_old_assignment_fields_are_refused() {
        let mut patch = pb::ItemPatch::default();
        assert!(refuse_assigned_in_patch(&patch).is_ok());
        patch.clear_assignment = true;
        assert!(refuse_assigned_in_patch(&patch).is_err());
    }
}
