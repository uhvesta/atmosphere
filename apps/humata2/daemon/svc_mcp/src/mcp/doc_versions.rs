//! The document-version tools of `humatad2 mcp` (MCP.md 6.1, DV-9): the
//! agent lists the versions of a file in a Desk session's folder and reads
//! one. DeskService decides what the caller may see (its own session's
//! folder freely, another's under FILES_READ_OTHER_SESSION or
//! FILES_READ_OTHER_MACHINE), so the tool and the app answer the same way.
//! There is no tool that saves or restores: the agent commits with git.

use crate::pb;
use humata2_mcp::{Call, Reply, ToolError};
use serde_json::{Map, Value, json};
use tonic::transport::Channel;

use super::{GrpcDaemon, applied, review, text};

impl GrpcDaemon {
    fn desk_client(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.channel.clone())
    }

    /// The session a doc tool reads: `location.session`, else the caller's.
    async fn doc_session(&self, arguments: &Map<String, Value>) -> Result<pb::SessionId, ToolError> {
        let named = arguments
            .get("location")
            .and_then(Value::as_object)
            .map(|location| text(location, "session"))
            .unwrap_or_default();
        if named.is_empty() {
            self.own_session().await
        } else {
            Ok(pb::SessionId { value: named })
        }
    }

    /// `doc_versions_list`.
    pub(super) async fn doc_versions_list(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::ListDocVersionsRequest {
            session: Some(self.doc_session(&call.arguments).await?),
            path: text(&call.arguments, "path"),
        })?;
        let listed = self
            .desk_client()
            .list_doc_versions(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let versions: Vec<Value> = listed.versions.iter().map(version_json).collect();
        Ok(applied(
            json!({ "versions": versions, "unsaved_changes": listed.unsaved_changes }),
            None,
        ))
    }

    /// `doc_version_read`.
    pub(super) async fn doc_version_read(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::ReadDocVersionRequest {
            session: Some(self.doc_session(&call.arguments).await?),
            path: text(&call.arguments, "path"),
            version: text(&call.arguments, "version"),
        })?;
        let read = self
            .desk_client()
            .read_doc_version(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let mut result = Map::new();
        result.insert("text".to_owned(), json!(read.text));
        if read.comments_recorded {
            let comments: Vec<Value> = read.comments.iter().map(review::comment_json).collect();
            result.insert("comments".to_owned(), Value::Array(comments));
        }
        Ok(applied(Value::Object(result), None))
    }
}

/// One version as `doc_versions_list` shows it: `by` is "you" or the
/// agent's name.
fn version_json(version: &pb::DocVersion) -> Value {
    let by = if version.by == pb::DocVersionBy::You as i32 {
        "you".to_owned()
    } else {
        version.who.clone()
    };
    let unix_ms = version
        .at
        .as_ref()
        .map_or(0, |at| at.seconds * 1000 + i64::from(at.nanos) / 1_000_000);
    json!({
        "id": version.id,
        "number": version.number,
        "unix_ms": unix_ms,
        "by": by,
        "message": version.message,
        "added": version.added,
        "removed": version.removed,
        "has_comments": version.has_comments,
    })
}
