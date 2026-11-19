//! The history tools of `humatad2 mcp` (MCP.md 6.2): an agent searches and
//! reads the conversations the host keeps through HistoryService, which
//! decides what its session may see, so the tool and the CLI give the same
//! results for the same query (LD-25).

use humata2_grpc::humata::v1 as pb;
use humata2_mcp::{Call, ErrorKind, Reply, ToolError};
use serde_json::{Map, Value, json};
use tonic::transport::Channel;

use super::boards::{next_page, page};
use super::{GrpcDaemon, applied, lower, text};
use humata2_rpc_kit::rfc3339;

impl GrpcDaemon {
    fn history_client(&self) -> pb::history_service_client::HistoryServiceClient<Channel> {
        pb::history_service_client::HistoryServiceClient::new(self.channel.clone())
    }

    /// `sessions_search`.
    pub(super) async fn sessions_search(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let request = self.request(pb::SearchSessionsRequest {
            query: text(arguments, "query"),
            channel: named(arguments, "channel").map(|value| pb::ChannelId { value }),
            machine: named(arguments, "machine").map(|value| pb::MachineId { value }),
            agent: text(arguments, "agent"),
            since: time(arguments, "since")?,
            until: time(arguments, "until")?,
            page: page(arguments),
        })?;
        let found = self
            .history_client()
            .search_sessions(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let mut result = Map::new();
        result.insert(
            "hits".to_owned(),
            Value::Array(found.hits.iter().map(hit_json).collect()),
        );
        next_page(&mut result, found.page);
        Ok(applied(Value::Object(result), None))
    }

    /// `session_read`.
    pub(super) async fn session_read(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let request = self.request(pb::ReadTurnsRequest {
            session: Some(pb::SessionId {
                value: text(arguments, "session"),
            }),
            first_turn: turn(arguments, "first_turn"),
            last_turn: turn(arguments, "last_turn"),
            page: page(arguments),
        })?;
        let read = self
            .history_client()
            .read_turns(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let mut result = Map::new();
        result.insert(
            "turns".to_owned(),
            Value::Array(read.entries.iter().map(entry_json).collect()),
        );
        next_page(&mut result, read.page);
        Ok(applied(Value::Object(result), None))
    }
}

/// A non-empty string argument.
fn named(arguments: &Map<String, Value>, field: &str) -> Option<String> {
    Some(text(arguments, field)).filter(|value| !value.is_empty())
}

fn turn(arguments: &Map<String, Value>, field: &str) -> Option<u32> {
    arguments
        .get(field)
        .and_then(Value::as_u64)
        .map(|value| u32::try_from(value).unwrap_or(u32::MAX))
}

/// A time argument, written as RFC 3339.
fn time(
    arguments: &Map<String, Value>,
    field: &str,
) -> Result<Option<humata2_grpc::Timestamp>, ToolError> {
    let Some(written) = named(arguments, field) else {
        return Ok(None);
    };
    match rfc3339::parse(&written) {
        Some(at) => Ok(Some(at)),
        None => {
            let mut details = Map::new();
            details.insert("field".to_owned(), json!(field));
            details.insert("problem".to_owned(), json!("malformed"));
            Err(ToolError {
                kind: ErrorKind::InvalidField,
                message: format!("{field} {written:?} isn't a time"),
                fix: format!(
                    "write {field} as an RFC 3339 time such as 2026-11-15T09:30:00Z, with a Z or an offset"
                ),
                retry: None,
                details,
            })
        }
    }
}

fn hit_json(hit: &pb::SearchHit) -> Value {
    let id = |value: Option<&str>| value.unwrap_or_default().to_owned();
    json!({
        "session": id(hit.session.as_ref().map(|id| id.value.as_str())),
        "session_title": hit.session_title,
        "channel": id(hit.channel.as_ref().map(|id| id.value.as_str())),
        "channel_name": hit.channel_name,
        "machine": id(hit.machine.as_ref().map(|id| id.value.as_str())),
        "machine_name": hit.machine_name,
        "agent": hit.agent,
        "turn": hit.turn,
        "seq": hit.seq,
        "snippet": hit.snippet,
        "at": hit.at.as_ref().map(rfc3339::format),
        "as_of": hit.as_of.as_ref().map(rfc3339::format),
    })
}

fn entry_json(entry: &pb::MirrorEntry) -> Value {
    let role = pb::mirror_entry::Role::try_from(entry.role)
        .map(|role| lower(role.as_str_name(), "ROLE_"))
        .unwrap_or_default();
    json!({
        "seq": entry.seq,
        "turn": entry.turn,
        "role": role,
        "text": entry.text,
        "at": entry.at.as_ref().map(rfc3339::format),
    })
}
