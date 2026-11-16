//! Board items over `BoardService` (BD-1..16): what the daemon sends as the
//! app's [`Item`]s, and the calls that change them.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_clients::DaemonError;
use humata2_clients::model::{
    ChannelId, FieldChange, Item, ItemChange, ItemDetails, ItemId, ItemKind, ItemStatus, Scope,
    SessionId,
};
use humata2_grpc::humata::v1 as pb;

use crate::connection::{Shared, fresh_key};

const PAGE_SIZE: u32 = 200;
/// Days in the dashboard's reporting window.
const WINDOW_DAYS: i64 = 14;

pub(crate) fn status(column: i32) -> ItemStatus {
    match pb::BoardColumn::try_from(column) {
        Ok(pb::BoardColumn::ToDo) => ItemStatus::Todo,
        Ok(pb::BoardColumn::InProgress) => ItemStatus::InProgress,
        Ok(pb::BoardColumn::InReview) => ItemStatus::InReview,
        Ok(pb::BoardColumn::Done) => ItemStatus::Done,
        Ok(pb::BoardColumn::Backlog | pb::BoardColumn::Unspecified) | Err(_) => ItemStatus::Backlog,
    }
}

/// The wire column for a status, or `None` for one the daemon has no column for.
fn column(status: ItemStatus) -> Option<pb::BoardColumn> {
    match status {
        ItemStatus::Backlog => Some(pb::BoardColumn::Backlog),
        ItemStatus::Todo => Some(pb::BoardColumn::ToDo),
        ItemStatus::InProgress => Some(pb::BoardColumn::InProgress),
        ItemStatus::InReview => Some(pb::BoardColumn::InReview),
        ItemStatus::Done => Some(pb::BoardColumn::Done),
        ItemStatus::Blocked | ItemStatus::WontDo => None,
    }
}

fn scope(item: &pb::BoardItem) -> Scope {
    let channel = match item.channel.as_ref() {
        Some(channel) if !channel.value.is_empty() => ChannelId::new(channel.value.clone()),
        _ => return Scope::Global,
    };
    match item.board.as_ref().and_then(|board| board.board.as_ref()) {
        Some(pb::board_ref::Board::Session(id)) => Scope::Session {
            channel,
            session: SessionId::new(id.value.clone()),
        },
        _ => Scope::Channel(channel),
    }
}

pub(crate) fn item(wire: &pb::BoardItem) -> Item {
    Item {
        id: item_id(wire.id.as_ref()),
        scope: scope(wire),
        kind: ItemKind::Task,
        title: wire.title.clone(),
        status: status(wire.column),
        depends_on: wire.depends_on.iter().map(|edge| item_id(edge.id.as_ref())).collect(),
        session: wire.sessions.first().map(|id| SessionId::new(id.value.clone())),
        parent: None,
    }
}

fn item_id(id: Option<&pb::BoardItemId>) -> ItemId {
    ItemId::new(id.map(|id| id.value.clone()).unwrap_or_default())
}

/// What a pending ask asks for, as the app shows it.
fn requested(changes: &[pb::FieldChange], deletes: bool) -> (Option<ItemStatus>, Option<ItemChange>) {
    if deletes {
        return (None, Some(ItemChange::Delete));
    }
    let mut moved = None;
    let mut edits = Vec::new();
    for change in changes {
        if matches!(change.field.to_lowercase().as_str(), "column" | "status") {
            moved = Some(column_named(&change.after));
        } else {
            edits.push(FieldChange {
                field: change.field.clone(),
                before: change.before.clone(),
                after: change.after.clone(),
            });
        }
    }
    (moved, (!edits.is_empty()).then_some(ItemChange::Edit(edits)))
}

/// A column from the name the daemon words it with: "In review".
fn column_named(name: &str) -> ItemStatus {
    let wanted = name.trim().to_lowercase().replace(['_', '-'], " ");
    ItemStatus::ALL
        .into_iter()
        .find(|status| status.name().to_lowercase() == wanted)
        .unwrap_or(ItemStatus::Backlog)
}

/// Day in the fourteen-day window (0 oldest, 13 today) for an item done
/// `done` seconds since the epoch, when that is within the window.
fn done_day(now: i64, done: i64) -> Option<u8> {
    let days = now.saturating_sub(done).max(0) / 86_400;
    if days >= WINDOW_DAYS {
        return None;
    }
    u8::try_from(WINDOW_DAYS - 1 - days).ok()
}

pub(crate) fn details_at(wire: &pb::BoardItem, now: i64) -> ItemDetails {
    let created_by = match wire.created_by.as_ref().and_then(|actor| actor.actor.as_ref()) {
        Some(pb::actor::Actor::Agent(agent)) => agent.session.as_ref().map(|id| SessionId::new(id.value.clone())),
        _ => None,
    };
    let mut details = ItemDetails {
        description: wire.description.clone(),
        sessions: wire.sessions.iter().map(|id| SessionId::new(id.value.clone())).collect(),
        created_by,
        done_day: wire.done_at.as_ref().and_then(|at| done_day(now, at.seconds)),
        ..ItemDetails::default()
    };
    if let Some(ask) = wire.pending.first() {
        details.request = ask.request.as_ref().map(|id| humata2_clients::model::RequestId::new(id.value.clone()));
        details.request_summary.clone_from(&ask.reason);
        if let Some(change) = ask.change.as_ref() {
            let (moved, other) = requested(&change.changes, change.deletes_item);
            details.requested_status = moved;
            details.requested_change = other;
        }
    }
    details
}

pub(crate) fn details(wire: &pb::BoardItem) -> ItemDetails {
    details_at(wire, now_secs())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

/// How long ago, in seconds, as people read it: "now", "9m ago", "3h ago",
/// "2d ago". The boards and the Desk use it.
pub(crate) fn ago(now: i64, then: i64) -> String {
    let seconds = now.saturating_sub(then).max(0);
    match seconds {
        0..=59 => "now".to_owned(),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86_399 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

fn history_line(entry: &pb::ItemHistoryEntry, now: i64) -> String {
    match entry.at.as_ref().and_then(|stamp| stamp.at.as_ref()) {
        Some(at) if at.seconds > 0 => format!("{} · {}", entry.change, ago(now, at.seconds)),
        _ => entry.change.clone(),
    }
}

fn wire_id(id: &ItemId) -> Option<pb::BoardItemId> {
    Some(pb::BoardItemId {
        value: id.as_str().to_owned(),
    })
}

fn session_wire(id: &SessionId) -> pb::SessionId {
    pb::SessionId {
        value: id.as_str().to_owned(),
    }
}

fn board_ref(scope: &Scope) -> pb::BoardRef {
    let board = match scope {
        Scope::Global => pb::board_ref::Board::Global(pb::GlobalBoard {}),
        Scope::Channel(channel) => pb::board_ref::Board::Channel(pb::ChannelId {
            value: channel.as_str().to_owned(),
        }),
        Scope::Session { session, .. } => pb::board_ref::Board::Session(session_wire(session)),
    };
    pb::BoardRef { board: Some(board) }
}

impl Shared {
    /// Every item the user can see, and its details, for the first load.
    pub(crate) async fn load_items(
        &self,
        channels: &[ChannelId],
    ) -> Result<(Vec<Item>, std::collections::BTreeMap<ItemId, ItemDetails>), DaemonError> {
        use pb::board_view::View;
        let mut views = vec![View::GlobalWithChannels(pb::GlobalWithChannels {
            global_only: false,
            channel: None,
        })];
        views.extend(channels.iter().map(|channel| {
            View::ChannelWithSessions(pb::ChannelId {
                value: channel.as_str().to_owned(),
            })
        }));
        let mut items = Vec::new();
        let mut details = std::collections::BTreeMap::new();
        let mut seen = BTreeSet::new();
        let now = now_secs();
        for view in views {
            let mut token = String::new();
            loop {
                let reply = self
                    .boards()
                    .list_items(pb::ListItemsRequest {
                        view: Some(pb::BoardView { view: Some(view.clone()) }),
                        columns: Vec::new(),
                        page: Some(pb::PageRequest {
                            page_size: PAGE_SIZE,
                            page_token: token,
                        }),
                    })
                    .await
                    .map_err(|status| self.error(&status, "The board items"))?
                    .into_inner();
                for wire in &reply.items {
                    let mapped = item(wire);
                    if seen.insert(mapped.id.clone()) {
                        details.insert(mapped.id.clone(), details_at(wire, now));
                        items.push(mapped);
                    }
                }
                token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
                if token.is_empty() {
                    break;
                }
            }
        }
        Ok((items, details))
    }

    /// Reads one item again and tells the app, for changes the daemon
    /// doesn't send the item with (an ask coming or going).
    pub(crate) async fn refresh_item(&self, id: &pb::BoardItemId) {
        if let Ok(reply) = self.boards().get_item(pb::GetItemRequest { id: Some(id.clone()) }).await {
            let wire = reply.into_inner();
            self.publish(humata2_clients::Event::Item {
                item: item(&wire),
                details: details(&wire),
            });
        }
    }

    pub(crate) async fn move_item(&self, id: &ItemId, to: ItemStatus) -> Result<(), DaemonError> {
        let Some(wire_column) = column(to) else {
            return Err(DaemonError::Refused {
                what: "Moving the item".to_owned(),
                reason: format!("the board has no {} column. Pick another column", to.name()),
            });
        };
        self.boards()
            .move_item(pb::MoveItemRequest {
                idempotency_key: fresh_key(),
                id: wire_id(id),
                column: wire_column as i32,
                after: None,
                expected_revision: None,
                reason: String::new(),
            })
            .await
            .map_err(|status| self.error(&status, "Moving the item"))?;
        Ok(())
    }

    pub(crate) async fn create_item(&self, scope: &Scope, title: &str) -> Result<ItemId, DaemonError> {
        const WHAT: &str = "The new board item";
        let reply = self
            .boards()
            .create_item(pb::CreateItemRequest {
                idempotency_key: fresh_key(),
                board: Some(board_ref(scope)),
                title: title.to_owned(),
                column: pb::BoardColumn::ToDo as i32,
                ..pb::CreateItemRequest::default()
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        reply
            .item
            .and_then(|created| created.id)
            .map(|id| ItemId::new(id.value))
            .ok_or_else(|| DaemonError::Refused {
                what: WHAT.to_owned(),
                reason: "the daemon took it but didn't name it, so it may be waiting to sync. Reload to see whether it exists".to_owned(),
            })
    }

    async fn patch_item(&self, id: &ItemId, patch: pb::ItemPatch, what: &str) -> Result<(), DaemonError> {
        self.boards()
            .update_item(pb::UpdateItemRequest {
                idempotency_key: fresh_key(),
                id: wire_id(id),
                patch: Some(patch),
                expected_revision: None,
                reason: String::new(),
            })
            .await
            .map_err(|status| self.error(&status, what))?;
        Ok(())
    }

    pub(crate) async fn set_dependency(&self, id: &ItemId, on: &ItemId, add: bool) -> Result<(), DaemonError> {
        let edge = on.as_str().to_owned();
        let edge = pb::BoardItemId { value: edge };
        let patch = if add {
            pb::ItemPatch {
                depends_on_add: vec![edge],
                ..pb::ItemPatch::default()
            }
        } else {
            pb::ItemPatch {
                depends_on_remove: vec![edge],
                ..pb::ItemPatch::default()
            }
        };
        let what = if add { "Adding the dependency" } else { "Removing the dependency" };
        self.patch_item(id, patch, what).await
    }

    pub(crate) async fn set_item_session(
        &self,
        id: &ItemId,
        session: &SessionId,
        working: bool,
    ) -> Result<(), DaemonError> {
        let wire = vec![session_wire(session)];
        let patch = if working {
            pb::ItemPatch {
                sessions_add: wire,
                ..pb::ItemPatch::default()
            }
        } else {
            pb::ItemPatch {
                sessions_remove: wire,
                ..pb::ItemPatch::default()
            }
        };
        self.patch_item(id, patch, "Changing the item's sessions").await
    }

    /// SE-3: a session started from an item works on it, and starts it.
    /// Best effort; the session is already running.
    pub(crate) async fn start_item_work(&self, id: &ItemId, session: &SessionId) {
        let current = self.boards().get_item(pb::GetItemRequest { id: wire_id(id) }).await;
        let _ = self.set_item_session(id, session, true).await;
        if let Ok(reply) = current {
            if matches!(status(reply.into_inner().column), ItemStatus::Backlog | ItemStatus::Todo) {
                let _ = self.move_item(id, ItemStatus::InProgress).await;
            }
        }
    }

    pub(crate) async fn item_history(&self, id: &ItemId) -> Result<Vec<String>, DaemonError> {
        let mut entries = Vec::new();
        let mut token = String::new();
        loop {
            let reply = self
                .boards()
                .get_item_history(pb::GetItemHistoryRequest {
                    id: wire_id(id),
                    page: Some(pb::PageRequest {
                        page_size: PAGE_SIZE,
                        page_token: token,
                    }),
                })
                .await
                .map_err(|status| self.error(&status, "The item's history"))?
                .into_inner();
            entries.extend(reply.entries);
            token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if token.is_empty() {
                break;
            }
        }
        let now = now_secs();
        let mut lines: Vec<(i64, String)> = entries
            .iter()
            .map(|entry| {
                let at = entry.at.as_ref().and_then(|stamp| stamp.at.as_ref()).map_or(0, |at| at.seconds);
                (at, history_line(entry, now))
            })
            .collect();
        // Newest first; entries at the same moment keep the daemon's order reversed.
        lines.reverse();
        lines.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        Ok(lines.into_iter().map(|(_, line)| line).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> Option<pb::BoardItemId> {
        Some(pb::BoardItemId { value: value.to_owned() })
    }

    fn wire(column: pb::BoardColumn) -> pb::BoardItem {
        pb::BoardItem {
            id: id("K-1"),
            title: "Ship".to_owned(),
            column: column as i32,
            ..pb::BoardItem::default()
        }
    }

    #[test]
    fn global_item_maps_to_global_scope_and_its_column() {
        let mapped = item(&wire(pb::BoardColumn::InReview));
        assert_eq!(mapped.scope, Scope::Global);
        assert_eq!(mapped.status, ItemStatus::InReview);
        assert_eq!(mapped.id, ItemId::new("K-1"));
    }

    #[test]
    fn channel_item_keeps_its_channel_and_first_session() {
        let mut item_wire = wire(pb::BoardColumn::ToDo);
        item_wire.channel = Some(pb::ChannelId { value: "c1".to_owned() });
        item_wire.sessions = vec![
            pb::SessionId { value: "s1".to_owned() },
            pb::SessionId { value: "s2".to_owned() },
        ];
        item_wire.depends_on = vec![pb::Dependency { id: id("K-0"), ..pb::Dependency::default() }];
        let mapped = item(&item_wire);
        assert_eq!(mapped.scope, Scope::Channel(ChannelId::new("c1")));
        assert_eq!(mapped.session, Some(SessionId::new("s1")));
        assert_eq!(mapped.depends_on, vec![ItemId::new("K-0")]);
        assert_eq!(details_at(&item_wire, 0).sessions.len(), 2);
    }

    #[test]
    fn done_day_counts_back_from_today() {
        let now = 100 * 86_400;
        assert_eq!(done_day(now, now), Some(13));
        assert_eq!(done_day(now, now - 13 * 86_400), Some(0));
        assert_eq!(done_day(now, now - 14 * 86_400), None);
    }

    #[test]
    fn a_pending_move_ask_reads_as_a_requested_status() {
        let mut item_wire = wire(pb::BoardColumn::ToDo);
        item_wire.pending = vec![pb::PendingAsk {
            request: Some(pb::RequestId { value: "r1".to_owned() }),
            reason: "it is done".to_owned(),
            change: Some(pb::BoardChangePreview {
                changes: vec![pb::FieldChange {
                    field: "status".to_owned(),
                    before: "To do".to_owned(),
                    after: "In review".to_owned(),
                }],
                ..pb::BoardChangePreview::default()
            }),
            ..pb::PendingAsk::default()
        }];
        let details = details_at(&item_wire, 0);
        assert_eq!(details.requested_status, Some(ItemStatus::InReview));
        assert_eq!(details.requested_change, None);
        assert_eq!(details.request_summary, "it is done");
    }

    #[test]
    fn a_pending_edit_or_delete_ask_reads_as_a_requested_change() {
        let edit = requested(
            &[pb::FieldChange {
                field: "title".to_owned(),
                before: "a".to_owned(),
                after: "b".to_owned(),
            }],
            false,
        );
        assert_eq!(
            edit.1,
            Some(ItemChange::Edit(vec![FieldChange {
                field: "title".to_owned(),
                before: "a".to_owned(),
                after: "b".to_owned()
            }]))
        );
        assert_eq!(requested(&[], true).1, Some(ItemChange::Delete));
    }

    #[test]
    fn columns_without_a_wire_column_are_not_written() {
        assert_eq!(column(ItemStatus::Blocked), None);
        assert_eq!(column(ItemStatus::Done), Some(pb::BoardColumn::Done));
    }
}
