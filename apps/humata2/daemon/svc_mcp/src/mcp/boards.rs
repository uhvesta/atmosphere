//! The board tools of `humatad2 mcp` (MCP.md 6.1): an agent lists, reads,
//! makes, changes, moves and deletes board items through BoardService, which
//! decides what its session may do. A write the rules send to the Inbox
//! comes back from the daemon as a pending approval, which `call` turns into
//! a pending reply.

use humata2_grpc::humata::v1 as pb;
use humata2_mcp::{Call, Reply, ToolError};
use serde_json::{Map, Value, json};
use tonic::transport::Channel;

use super::{GrpcDaemon, applied, key, lower, text};
use crate::ports::stale;

impl GrpcDaemon {
    fn board_client(&self) -> pb::board_service_client::BoardServiceClient<Channel> {
        pb::board_service_client::BoardServiceClient::new(self.channel.clone())
    }

    /// A tool's `board`: `session` (the default) and `channel` are the
    /// caller's own, `global` is the Global board, and `{session}` names
    /// another session's.
    async fn board_ref(&self, arguments: &Map<String, Value>) -> Result<pb::BoardRef, ToolError> {
        use pb::board_ref::Board;
        let board = match arguments.get("board") {
            Some(Value::String(scope)) if scope == "channel" => {
                let (_, channel) = self.agent().await?;
                Board::Channel(channel.unwrap_or_default())
            }
            Some(Value::String(scope)) if scope == "global" => Board::Global(pb::GlobalBoard {}),
            Some(Value::Object(other)) => Board::Session(pb::SessionId {
                value: text(other, "session"),
            }),
            _ => Board::Session(self.own_session().await?),
        };
        Ok(pb::BoardRef { board: Some(board) })
    }

    /// `board_items_list`.
    pub(super) async fn board_items_list(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let board = self.board_ref(arguments).await?;
        let request = self.request(pb::ListItemsRequest {
            view: Some(pb::BoardView {
                view: Some(pb::board_view::View::Board(board)),
            }),
            columns: columns(arguments.get("columns")),
            page: page(arguments),
        })?;
        let response = self
            .board_client()
            .list_items(request)
            .await
            .map_err(|status| self.failed(&status))?;
        let stale_as_of = stale_as_of(response.metadata());
        let listed = response.into_inner();
        let mut counts = Map::new();
        for count in &listed.counts {
            counts.insert(column_name(count.column), json!(count.items));
        }
        let mut result = Map::new();
        result.insert(
            "items".to_owned(),
            Value::Array(listed.items.iter().map(item_json).collect()),
        );
        result.insert("counts".to_owned(), Value::Object(counts));
        next_page(&mut result, listed.page);
        Ok(read_reply(Value::Object(result), stale_as_of))
    }

    /// `board_item_get`.
    pub(super) async fn board_item_get(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::GetItemRequest {
            id: Some(item_id(&call.arguments, "id")),
        })?;
        let response = self
            .board_client()
            .get_item(request)
            .await
            .map_err(|status| self.failed(&status))?;
        let stale_as_of = stale_as_of(response.metadata());
        let item = response.into_inner();
        Ok(read_reply(item_json(&item), stale_as_of))
    }

    /// `board_item_history`.
    pub(super) async fn board_item_history(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let request = self.request(pb::GetItemHistoryRequest {
            id: Some(item_id(arguments, "id")),
            page: page(arguments),
        })?;
        let response = self
            .board_client()
            .get_item_history(request)
            .await
            .map_err(|status| self.failed(&status))?;
        let stale_as_of = stale_as_of(response.metadata());
        let history = response.into_inner();
        let mut result = Map::new();
        result.insert(
            "entries".to_owned(),
            Value::Array(history.entries.iter().map(entry_json).collect()),
        );
        next_page(&mut result, history.page);
        Ok(read_reply(Value::Object(result), stale_as_of))
    }

    /// `board_item_create`.
    pub(super) async fn board_item_create(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        // No board named: the daemon picks the caller's session board (VF-21).
        let board = match arguments.get("board") {
            Some(_) => Some(self.board_ref(arguments).await?),
            None => None,
        };
        let request = self.request(pb::CreateItemRequest {
            idempotency_key: Some(key(&call)),
            board,
            title: text(arguments, "title"),
            column: arguments
                .get("column")
                .and_then(Value::as_str)
                .and_then(column_of)
                .unwrap_or(pb::BoardColumn::Unspecified) as i32,
            description: text(arguments, "description"),
            reason: text(arguments, "reason"),
            sessions: session_ids(arguments, "sessions"),
            depends_on: item_ids(arguments, "depends_on"),
            ..pb::CreateItemRequest::default()
        })?;
        let written = self
            .board_client()
            .create_item(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(write_reply(&written, written_json(&written)))
    }

    /// `board_item_update`.
    pub(super) async fn board_item_update(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let patch = pb::ItemPatch {
            title: optional_text(arguments, "title"),
            description: optional_text(arguments, "description"),
            sessions_add: session_ids(arguments, "sessions_add"),
            sessions_remove: session_ids(arguments, "sessions_remove"),
            depends_on_add: item_ids(arguments, "depends_on_add"),
            depends_on_remove: item_ids(arguments, "depends_on_remove"),
            ..pb::ItemPatch::default()
        };
        let request = self.request(pb::UpdateItemRequest {
            idempotency_key: Some(key(&call)),
            id: Some(item_id(arguments, "id")),
            patch: Some(patch),
            expected_revision: expected_revision(arguments),
            reason: text(arguments, "reason"),
        })?;
        let written = self
            .board_client()
            .update_item(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(write_reply(&written, written_json(&written)))
    }

    /// `board_item_move`.
    pub(super) async fn board_item_move(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let request = self.request(pb::MoveItemRequest {
            idempotency_key: Some(key(&call)),
            id: Some(item_id(arguments, "id")),
            column: arguments
                .get("column")
                .and_then(Value::as_str)
                .and_then(column_of)
                .unwrap_or(pb::BoardColumn::Unspecified) as i32,
            after: Some(item_id(arguments, "after")).filter(|id| !id.value.is_empty()),
            expected_revision: expected_revision(arguments),
            reason: text(arguments, "reason"),
        })?;
        let written = self
            .board_client()
            .move_item(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(write_reply(&written, written_json(&written)))
    }

    /// `board_item_delete`.
    pub(super) async fn board_item_delete(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let request = self.request(pb::DeleteItemRequest {
            idempotency_key: Some(key(&call)),
            id: Some(item_id(arguments, "id")),
            expected_revision: expected_revision(arguments),
            reason: text(arguments, "reason"),
        })?;
        let written = self
            .board_client()
            .delete_item(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(write_reply(&written, json!({ "deleted": true })))
    }
}

fn item_id(arguments: &Map<String, Value>, field: &str) -> pb::BoardItemId {
    pb::BoardItemId {
        value: text(arguments, field),
    }
}

/// The strings of an array argument.
pub(super) fn strings(arguments: &Map<String, Value>, field: &str) -> Vec<String> {
    arguments
        .get(field)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn session_ids(arguments: &Map<String, Value>, field: &str) -> Vec<pb::SessionId> {
    strings(arguments, field)
        .into_iter()
        .map(|value| pb::SessionId { value })
        .collect()
}

fn item_ids(arguments: &Map<String, Value>, field: &str) -> Vec<pb::BoardItemId> {
    strings(arguments, field)
        .into_iter()
        .map(|value| pb::BoardItemId { value })
        .collect()
}

/// A string argument that was sent at all, so an empty one still clears.
fn optional_text(arguments: &Map<String, Value>, field: &str) -> Option<String> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub(super) fn expected_revision(arguments: &Map<String, Value>) -> Option<pb::Revision> {
    arguments
        .get("expected_revision")
        .and_then(Value::as_u64)
        .map(|value| pb::Revision { value })
}

pub(super) fn page(arguments: &Map<String, Value>) -> Option<pb::PageRequest> {
    let token = text(arguments, "page_token");
    (!token.is_empty()).then_some(pb::PageRequest {
        page_size: 0,
        page_token: token,
    })
}

pub(super) fn next_page(result: &mut Map<String, Value>, page: Option<pb::PageResponse>) {
    if let Some(page) = page.filter(|page| !page.next_page_token.is_empty()) {
        result.insert("next_page_token".to_owned(), json!(page.next_page_token));
    }
}

/// `backlog`, `to_do`, `in_progress`, `in_review` or `done`.
fn column_of(name: &str) -> Option<pb::BoardColumn> {
    let column = pb::BoardColumn::from_str_name(&format!(
        "BOARD_COLUMN_{}",
        name.to_ascii_uppercase()
    ))?;
    (column != pb::BoardColumn::Unspecified).then_some(column)
}

fn columns(value: Option<&Value>) -> Vec<i32> {
    value
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .filter_map(column_of)
                .map(|column| column as i32)
                .collect()
        })
        .unwrap_or_default()
}

fn column_name(value: i32) -> String {
    pb::BoardColumn::try_from(value)
        .map(|column| lower(column.as_str_name(), "BOARD_COLUMN_"))
        .unwrap_or_default()
}

/// Who did it: `user`, or the agent's session id.
fn actor_name(actor: Option<&pb::Actor>) -> String {
    match actor.and_then(|actor| actor.actor.as_ref()) {
        Some(pb::actor::Actor::Agent(agent)) => agent
            .session
            .as_ref()
            .map(|session| session.value.clone())
            .unwrap_or_default(),
        Some(pb::actor::Actor::System(_)) => "system".to_owned(),
        Some(pb::actor::Actor::Script(script)) => script.name.clone(),
        Some(pb::actor::Actor::User(_)) | None => "user".to_owned(),
    }
}

/// `Board` of MCP.md, as the item names it: `global`, or the channel's or
/// session's id under its kind.
fn board_json(board: Option<&pb::BoardRef>) -> Value {
    match board.and_then(|board| board.board.as_ref()) {
        Some(pb::board_ref::Board::Channel(channel)) => json!({ "channel": channel.value }),
        Some(pb::board_ref::Board::Session(session)) => json!({ "session": session.value }),
        Some(pb::board_ref::Board::Global(_)) | None => json!("global"),
    }
}

fn item_json(item: &pb::BoardItem) -> Value {
    let depends_on: Vec<Value> = item
        .depends_on
        .iter()
        .map(|dependency| {
            json!({
                "id": dependency.id.as_ref().map(|id| id.value.clone()),
                "title": dependency.title,
                "done": dependency.column == pb::BoardColumn::Done as i32,
            })
        })
        .collect();
    let mut result = Map::new();
    result.insert("id".to_owned(), json!(item.id.as_ref().map(|id| id.value.clone())));
    result.insert("board".to_owned(), board_json(item.board.as_ref()));
    result.insert("title".to_owned(), json!(item.title));
    result.insert("description".to_owned(), json!(item.description));
    result.insert("column".to_owned(), json!(column_name(item.column)));
    result.insert("rank".to_owned(), json!(item.rank));
    result.insert(
        "sessions".to_owned(),
        json!(
            item.sessions
                .iter()
                .map(|session| session.value.clone())
                .collect::<Vec<_>>()
        ),
    );
    result.insert("depends_on".to_owned(), Value::Array(depends_on));
    if let Some(done) = &item.done_at {
        result.insert("done_at".to_owned(), json!(rfc3339(done.seconds)));
    }
    result.insert("created_by".to_owned(), json!(actor_name(item.created_by.as_ref())));
    result.insert(
        "revision".to_owned(),
        json!(item.revision.as_ref().map(|revision| revision.value)),
    );
    result.insert("pinned".to_owned(), json!(item.pinned));
    result.insert("vip".to_owned(), json!(item.vip));
    Value::Object(result)
}

/// A read's reply: marked with when the host gave the answer if the daemon
/// answered it from its last copy because the host is away (AW-7 row 2).
pub(super) fn read_reply(result: Value, stale_as_of: Option<std::time::SystemTime>) -> Reply {
    Reply::Applied {
        result,
        request: None,
        stale_as_of,
    }
}

/// The time a read's response header says the host answered it, if it does.
pub(super) fn stale_as_of(metadata: &tonic::metadata::MetadataMap) -> Option<std::time::SystemTime> {
    metadata
        .get(stale::HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(stale::parse)
}

/// A write's reply: journaled, with where, when the host was away and the
/// daemon kept the write for it (AW-7 row 3); else applied.
fn write_reply(written: &pb::WriteItemResponse, result: Value) -> Reply {
    match (written.delivery(), &written.journal) {
        (pb::Delivery::Journaled, Some(position)) => Reply::Journaled {
            result,
            journal: humata2_mcp::JournalPosition {
                machine: position
                    .machine
                    .as_ref()
                    .map(|machine| machine.value.clone())
                    .unwrap_or_default(),
                seq: position.seq,
            },
        },
        _ => applied(result, None),
    }
}

/// What a write answered: the item, or, for a journaled write, nothing: a new
/// item has no id until the host replays it.
fn written_json(written: &pb::WriteItemResponse) -> Value {
    match &written.item {
        Some(item) => item_json(item),
        None => json!({}),
    }
}

fn entry_json(entry: &pb::ItemHistoryEntry) -> Value {
    let kind = pb::HistoryKind::try_from(entry.kind)
        .map(|kind| lower(kind.as_str_name(), "HISTORY_KIND_"))
        .unwrap_or_default();
    json!({
        "at": entry
            .at
            .as_ref()
            .and_then(|at| at.at.as_ref())
            .map(|at| rfc3339(at.seconds)),
        "by": actor_name(entry.at.as_ref().and_then(|at| at.by.as_ref())),
        "change": entry.change,
        "lost_conflict": entry.lost_conflict,
        "revision": entry.revision.as_ref().map(|revision| revision.value),
        "kind": kind,
        "field": entry.field,
        "before": entry.before,
        "after": entry.after,
        "cause": Some(entry.cause.as_ref().map(|id| id.value.clone()).unwrap_or_default())
            .filter(|cause| !cause.is_empty()),
    })
}

/// Seconds since the epoch as UTC `2026-11-15T09:30:00Z`.
fn rfc3339(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    // Civil date from a day count (proleptic Gregorian, from 0000-03-01).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 { shifted + 3 } else { shifted - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timestamp_is_written_as_utc_date_and_time() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        // A leap day, and a time of day.
        assert_eq!(rfc3339(1_709_210_096), "2024-02-29T12:34:56Z");
        assert_eq!(rfc3339(-1), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn a_column_is_read_by_its_snake_name() {
        assert_eq!(column_of("in_review"), Some(pb::BoardColumn::InReview));
        assert_eq!(column_of("to_do"), Some(pb::BoardColumn::ToDo));
        assert_eq!(column_of("nowhere"), None);
    }

    #[test]
    fn an_item_shows_its_dependencies_and_when_it_was_done() {
        let item = pb::BoardItem {
            id: Some(pb::BoardItemId { value: "PAY-7".to_owned() }),
            column: pb::BoardColumn::Done as i32,
            done_at: Some(Default::default()),
            depends_on: vec![pb::Dependency {
                id: Some(pb::BoardItemId { value: "PAY-3".to_owned() }),
                title: "schema".to_owned(),
                column: pb::BoardColumn::Done as i32,
            }],
            ..pb::BoardItem::default()
        };
        let shown = item_json(&item);
        assert_eq!(shown["id"], "PAY-7");
        assert_eq!(shown["column"], "done");
        assert_eq!(shown["depends_on"][0]["id"], "PAY-3");
        assert_eq!(shown["depends_on"][0]["done"], true);
        assert!(shown.get("done_at").is_some());
    }
}
