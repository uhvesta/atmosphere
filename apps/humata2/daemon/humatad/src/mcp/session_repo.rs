//! `session_add_repo` (CH-21; MCP.md 6.1): an agent adds a repo its channel
//! already has to its own session. The daemon asks the user first when the
//! rule for changing the agent's own session says so.

use humata2_grpc::humata::v1 as pb;
use humata2_mcp::{Call, ErrorKind, Reply, ToolError};
use serde_json::{Map, json};

use super::{GrpcDaemon, applied, key, text};

/// `owner/name` as the tool takes it.
fn parse_repo(repo: &str) -> Result<pb::RepoRef, ToolError> {
    match repo.trim().split_once('/') {
        Some((owner, name)) if !owner.is_empty() && !name.is_empty() && !name.contains('/') => {
            Ok(pb::RepoRef { owner: owner.to_owned(), name: name.to_owned() })
        }
        _ => {
            let mut details = Map::new();
            details.insert("field".to_owned(), json!("repo"));
            details.insert("problem".to_owned(), json!("bad_format"));
            Err(ToolError {
                kind: ErrorKind::InvalidField,
                message: format!("repo {repo:?} isn't owner/name"),
                fix: "name the repo as owner/name, for example acme/api".to_owned(),
                retry: None,
                details,
            })
        }
    }
}

impl GrpcDaemon {
    /// `session_add_repo`.
    pub(super) async fn session_add_repo(&self, call: Call) -> Result<Reply, ToolError> {
        let repo = parse_repo(&text(&call.arguments, "repo"))?;
        let request = self.request(pb::AddRepoToSessionRequest {
            idempotency_key: Some(key(&call)),
            id: Some(self.own_session().await?),
            repo: Some(repo),
        })?;
        let operation = self
            .sessions()
            .add_repo_to_session(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(
            json!({ "operation": operation.id.map(|id| id.value) }),
            None,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_repo;

    #[test]
    fn a_repo_is_owner_slash_name() {
        let repo = parse_repo("acme/api").unwrap();
        assert_eq!((repo.owner.as_str(), repo.name.as_str()), ("acme", "api"));
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in ["", "acme", "acme/", "/api", "a/b/c"] {
            assert!(parse_repo(bad).is_err(), "{bad:?}");
        }
    }
}
