//! The repo tools of `humatad2 mcp` (RL-5, RL-11; MCP.md 6.2): an agent
//! searches the repo library and GitHub, and lists the library, through
//! RepoService, which treats both as global reads. A search may widen its
//! scope for that one call; saving the scope and removing a repo have no
//! tool (D479).

use humata2_grpc::humata::v1 as pb;
use humata2_mcp::{Call, ErrorKind, Reply, ToolError};
use pb::repo_scope::Kind;
use serde_json::{Map, Value, json};
use tonic::transport::Channel;

use super::boards::{next_page, page};
use super::{GrpcDaemon, applied, lower, text};
use crate::history_service::rfc3339;

impl GrpcDaemon {
    fn repo_client(&self) -> pb::repo_service_client::RepoServiceClient<Channel> {
        pb::repo_service_client::RepoServiceClient::new(self.channel.clone())
    }

    /// `repo_search`.
    pub(super) async fn repo_search(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let request = self.request(pb::SearchReposRequest {
            query: text(arguments, "query"),
            scope: scope(arguments)?,
            page: page(arguments),
        })?;
        let found = self
            .repo_client()
            .search_repos(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let mut result = Map::new();
        result.insert(
            "library".to_owned(),
            Value::Array(found.library.iter().map(library_json).collect()),
        );
        result.insert(
            "github".to_owned(),
            Value::Array(found.github.iter().map(found_json).collect()),
        );
        result.insert("scope".to_owned(), scope_json(found.scope.as_ref()));
        result.insert(
            "problem".to_owned(),
            found.problem.as_ref().map_or(Value::Null, problem_json),
        );
        result.insert("unknown_owners".to_owned(), json!(found.unknown_owners));
        next_page(&mut result, found.page);
        Ok(applied(Value::Object(result), None))
    }

    /// `repo_library`.
    pub(super) async fn repo_library(&self) -> Result<Reply, ToolError> {
        let request = self.request(pb::ListRepoLibraryRequest {})?;
        let listed = self
            .repo_client()
            .list_repo_library(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(
            json!({ "repos": listed.repos.iter().map(library_json).collect::<Vec<_>>() }),
            None,
        ))
    }
}

/// The scope this one search asks for: the named owners, All of GitHub, or
/// none, which leaves the saved scope to the daemon.
fn scope(arguments: &Map<String, Value>) -> Result<Option<pb::RepoScope>, ToolError> {
    let owners: Vec<String> = arguments
        .get("owners")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let everything = arguments
        .get("all_of_github")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let kind = match (owners.is_empty(), everything) {
        (true, false) => return Ok(None),
        (false, false) => Kind::Owners(pb::RepoOwners {
            owners,
            // D474: listed owners plus your own repos, as the saved scope defaults.
            include_me: true,
        }),
        (true, true) => Kind::AllOfGithub(true),
        (false, true) => {
            let mut details = Map::new();
            details.insert("field".to_owned(), json!("all_of_github"));
            details.insert("problem".to_owned(), json!("conflict"));
            return Err(ToolError {
                kind: ErrorKind::InvalidField,
                message: "owners and all_of_github can't be used together".to_owned(),
                fix: "name the owners to search, or set all_of_github, not both; leave both out for the user's saved scope"
                    .to_owned(),
                retry: None,
                details,
            });
        }
    };
    Ok(Some(pb::RepoScope { kind: Some(kind) }))
}

fn full_name(repo: Option<&pb::RepoRef>) -> String {
    repo.map(|repo| format!("{}/{}", repo.owner, repo.name))
        .unwrap_or_default()
}

fn library_json(repo: &pb::LibraryRepo) -> Value {
    let kind = repo.last_used_kind();
    let used_in = match kind {
        pb::RepoPlaceKind::Unspecified => None,
        other => Some(lower(other.as_str_name(), "REPO_PLACE_KIND_")),
    };
    json!({
        "repo": full_name(repo.repo.as_ref()),
        "remote": repo.remote,
        "default_branch": repo.default_branch,
        "added_at": repo.added_at.as_ref().map(rfc3339::format),
        "last_used_at": repo.last_used_at.as_ref().map(rfc3339::format),
        "last_used_in": used_in,
        "last_used_channel": repo
            .last_used_channel
            .as_ref()
            .map(|id| id.value.clone())
            .filter(|id| !id.is_empty()),
    })
}

fn found_json(repo: &pb::FoundRepo) -> Value {
    json!({
        "repo": full_name(repo.repo.as_ref()),
        "description": repo.description,
        "private": repo.is_private,
    })
}

fn problem_json(problem: &pb::RepoSearchProblem) -> Value {
    json!({
        "kind": lower(problem.kind().as_str_name(), "KIND_"),
        "message": problem.message,
        "retry_at": problem.retry_at.as_ref().map(rfc3339::format),
    })
}

fn scope_json(scope: Option<&pb::RepoScope>) -> Value {
    match scope.and_then(|scope| scope.kind.as_ref()) {
        Some(Kind::YourRepos(_)) => json!({ "kind": "your_repos" }),
        Some(Kind::Owners(owners)) => json!({
            "kind": "owners",
            "owners": owners.owners,
            "include_me": owners.include_me,
        }),
        Some(Kind::AllOfGithub(_)) => json!({ "kind": "all_of_github" }),
        None => Value::Null,
    }
}
