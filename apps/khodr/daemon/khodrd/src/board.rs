//! The board: each project's tasks and subtasks, and how its sessions move
//! them.

use std::collections::HashMap;

use khodr_wire::pb;
use serde::{Deserialize, Serialize};

/// Where an item is on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Todo,
    InProgress,
    WaitingOnYou,
    InReview,
    Done,
    WontDo,
}

impl Status {
    /// Done and Won't do are only ever set by hand, and sessions don't move
    /// an item out of them.
    fn closed(self) -> bool {
        matches!(self, Self::Done | Self::WontDo)
    }

    pub fn wire(self) -> pb::ItemStatus {
        match self {
            Self::Todo => pb::ItemStatus::Todo,
            Self::InProgress => pb::ItemStatus::InProgress,
            Self::WaitingOnYou => pb::ItemStatus::WaitingOnYou,
            Self::InReview => pb::ItemStatus::InReview,
            Self::Done => pb::ItemStatus::Done,
            Self::WontDo => pb::ItemStatus::WontDo,
        }
    }

    pub fn from_wire(status: pb::ItemStatus) -> Self {
        match status {
            pb::ItemStatus::Unspecified | pb::ItemStatus::Todo => Self::Todo,
            pb::ItemStatus::InProgress => Self::InProgress,
            pb::ItemStatus::WaitingOnYou => Self::WaitingOnYou,
            pb::ItemStatus::InReview => Self::InReview,
            pb::ItemStatus::Done => Self::Done,
            pb::ItemStatus::WontDo => Self::WontDo,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    #[default]
    None,
    Low,
    Medium,
    High,
    Urgent,
}

impl Priority {
    pub fn wire(self) -> pb::Priority {
        match self {
            Self::None => pb::Priority::None,
            Self::Low => pb::Priority::Low,
            Self::Medium => pb::Priority::Medium,
            Self::High => pb::Priority::High,
            Self::Urgent => pb::Priority::Urgent,
        }
    }

    pub fn from_wire(priority: pb::Priority) -> Self {
        match priority {
            pb::Priority::None => Self::None,
            pb::Priority::Low => Self::Low,
            pb::Priority::Medium => Self::Medium,
            pb::Priority::High => Self::High,
            pb::Priority::Urgent => Self::Urgent,
        }
    }
}

/// What the board's changes record of an item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemFields {
    /// The task a subtask is under; None for a task.
    pub parent_id: Option<String>,
    pub title: String,
    pub spec: String,
    pub status: Status,
    pub priority: Priority,
}

impl ItemFields {
    pub fn wire(&self) -> pb::ItemFields {
        pb::ItemFields {
            parent_id: self.parent_id.clone().unwrap_or_default(),
            title: self.title.clone(),
            spec: self.spec.clone(),
            status: self.status.wire().into(),
            priority: self.priority.wire().into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub project_id: String,
    pub fields: ItemFields,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
    /// The sessions on it, archived ones too.
    pub session_ids: Vec<String>,
}

impl Item {
    /// The item as the protocol has it, its status still as set.
    pub fn wire(&self) -> pb::Item {
        pb::Item {
            id: self.id.clone(),
            project_id: self.project_id.clone(),
            parent_id: self.fields.parent_id.clone().unwrap_or_default(),
            title: self.fields.title.clone(),
            spec: self.fields.spec.clone(),
            status: self.fields.status.wire().into(),
            given_status: self.fields.status.wire().into(),
            priority: self.fields.priority.wire().into(),
            created_unix_ms: self.created_unix_ms,
            updated_unix_ms: self.updated_unix_ms,
            session_ids: self.session_ids.clone(),
        }
    }
}

/// What to change in an item; None leaves a field as it is.
#[derive(Clone, Debug, Default)]
pub struct ItemPatch {
    pub title: Option<String>,
    pub spec: Option<String>,
    pub status: Option<Status>,
    pub priority: Option<Priority>,
}

/// One change to a board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub id: u64,
    pub item_id: String,
    pub time_unix_ms: i64,
    /// None when it added the item.
    pub before: Option<ItemFields>,
    /// None when it removed it.
    pub after: Option<ItemFields>,
    pub reverts: Option<u64>,
}

impl Change {
    pub fn wire(&self) -> pb::BoardChange {
        pb::BoardChange {
            id: self.id,
            item_id: self.item_id.clone(),
            time_unix_ms: self.time_unix_ms,
            before: self.before.as_ref().map(ItemFields::wire),
            after: self.after.as_ref().map(ItemFields::wire),
            reverts: self.reverts.unwrap_or_default(),
        }
    }
}

/// What one session says about its item, if anything: waiting on an
/// answer or failed is Waiting on you, working is In progress, and having
/// finished a turn is In review. Archived sessions say nothing.
fn from_session(session: &pb::SessionInfo) -> Option<Status> {
    if session.archived {
        return None;
    }
    match session.state() {
        pb::SessionState::WaitingForApproval | pb::SessionState::Failed => {
            Some(Status::WaitingOnYou)
        }
        pb::SessionState::Working => Some(Status::InProgress),
        pb::SessionState::Idle if session.turns > 0 => Some(Status::InReview),
        pb::SessionState::Idle | pb::SessionState::Unspecified => None,
    }
}

/// The most urgent of what sessions say: Waiting on you, then In progress,
/// then In review.
fn most_urgent(statuses: impl IntoIterator<Item = Status>) -> Option<Status> {
    let rank = |status: &Status| match status {
        Status::WaitingOnYou => 3,
        Status::InProgress => 2,
        Status::InReview => 1,
        _ => 0,
    };
    statuses.into_iter().max_by_key(rank)
}

/// Sets each item's shown status from its sessions, and each task's also
/// from its subtasks', unless it is Done or Won't do.
pub fn show(items: &mut [pb::Item], sessions: &[pb::SessionInfo]) {
    let mut said: HashMap<&str, Vec<Status>> = HashMap::new();
    for session in sessions {
        if let Some(status) = from_session(session) {
            said.entry(session.item_id.as_str())
                .or_default()
                .push(status);
        }
    }
    // A subtask's sessions speak for its task too.
    let parents: HashMap<String, String> = items
        .iter()
        .filter(|item| !item.parent_id.is_empty())
        .map(|item| (item.id.clone(), item.parent_id.clone()))
        .collect();
    let mut rolled: HashMap<String, Vec<Status>> = HashMap::new();
    for (item, statuses) in &said {
        rolled
            .entry((*item).to_owned())
            .or_default()
            .extend(statuses);
        if let Some(parent) = parents.get(*item) {
            rolled.entry(parent.clone()).or_default().extend(statuses);
        }
    }
    for item in items {
        let set = Status::from_wire(item.given_status());
        let shown = if set.closed() {
            set
        } else {
            rolled.remove(&item.id).and_then(most_urgent).unwrap_or(set)
        };
        item.status = shown.wire().into();
    }
}

/// Why a session is in the inbox, if it is: it waits on an approval, it
/// failed, or it finished a turn and isn't archived or on a closed item.
pub fn inbox_reason(session: &pb::SessionInfo, item: Option<&pb::Item>) -> Option<pb::InboxReason> {
    if session.archived {
        return None;
    }
    match session.state() {
        pb::SessionState::WaitingForApproval => Some(pb::InboxReason::Approval),
        pb::SessionState::Failed => Some(pb::InboxReason::Failed),
        pb::SessionState::Idle if session.turns > 0 => {
            let closed = item.is_some_and(|item| Status::from_wire(item.status()).closed());
            (!closed).then_some(pb::InboxReason::Reply)
        }
        pb::SessionState::Idle | pb::SessionState::Working | pb::SessionState::Unspecified => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, parent: &str, set: Status) -> pb::Item {
        pb::Item {
            id: id.into(),
            parent_id: parent.into(),
            given_status: set.wire().into(),
            status: set.wire().into(),
            ..Default::default()
        }
    }

    fn session(item: &str, state: pb::SessionState, turns: u32) -> pb::SessionInfo {
        pb::SessionInfo {
            item_id: item.into(),
            state: state.into(),
            turns,
            ..Default::default()
        }
    }

    fn shown(items: &[pb::Item]) -> Vec<Status> {
        items
            .iter()
            .map(|item| Status::from_wire(item.status()))
            .collect()
    }

    #[test]
    fn a_working_subtask_moves_it_and_its_task_to_in_progress() {
        let mut items = [
            item("task", "", Status::Todo),
            item("sub", "task", Status::Todo),
            item("other", "task", Status::Todo),
        ];
        show(&mut items, &[session("sub", pb::SessionState::Working, 0)]);
        assert_eq!(
            shown(&items),
            [Status::InProgress, Status::InProgress, Status::Todo]
        );
    }

    #[test]
    fn waiting_on_you_wins_over_working_across_subtasks() {
        let mut items = [
            item("task", "", Status::Todo),
            item("a", "task", Status::Todo),
            item("b", "task", Status::Todo),
        ];
        show(
            &mut items,
            &[
                session("a", pb::SessionState::Working, 1),
                session("b", pb::SessionState::WaitingForApproval, 0),
            ],
        );
        assert_eq!(
            shown(&items),
            [
                Status::WaitingOnYou,
                Status::InProgress,
                Status::WaitingOnYou
            ]
        );
    }

    #[test]
    fn a_finished_turn_is_in_review_and_a_fresh_session_says_nothing() {
        let mut items = [
            item("reviewed", "", Status::Todo),
            item("fresh", "", Status::Todo),
        ];
        show(
            &mut items,
            &[
                session("reviewed", pb::SessionState::Idle, 1),
                session("fresh", pb::SessionState::Idle, 0),
            ],
        );
        assert_eq!(shown(&items), [Status::InReview, Status::Todo]);
    }

    #[test]
    fn closed_items_and_archived_sessions_stay_put() {
        let mut items = [
            item("done", "", Status::Done),
            item("archived", "", Status::InReview),
        ];
        let mut archived = session("archived", pb::SessionState::Working, 0);
        archived.archived = true;
        show(
            &mut items,
            &[session("done", pb::SessionState::Working, 0), archived],
        );
        assert_eq!(shown(&items), [Status::Done, Status::InReview]);
    }

    #[test]
    fn the_inbox_has_what_waits_on_you_and_not_what_is_closed_or_busy() {
        let open = item("open", "", Status::Todo);
        let done = item("done", "", Status::Done);
        let reason = |state, turns, item| inbox_reason(&session("", state, turns), item);
        assert_eq!(
            reason(pb::SessionState::WaitingForApproval, 0, None),
            Some(pb::InboxReason::Approval)
        );
        assert_eq!(
            reason(pb::SessionState::Failed, 1, Some(&open)),
            Some(pb::InboxReason::Failed)
        );
        assert_eq!(
            reason(pb::SessionState::Idle, 1, Some(&open)),
            Some(pb::InboxReason::Reply)
        );
        assert_eq!(reason(pb::SessionState::Idle, 1, Some(&done)), None);
        assert_eq!(reason(pb::SessionState::Idle, 0, None), None);
        assert_eq!(reason(pb::SessionState::Working, 1, None), None);
    }
}
