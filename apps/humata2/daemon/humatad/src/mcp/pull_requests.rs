//! The pull request review tools of `humatad2 mcp` (PR-9, PR-11, PR-23):
//! the agent drafts, edits and drops GitHub drafts, drafts the summary,
//! starts a review and reads which commits the user reviewed (D299). Each is a call on the daemon, which decides what the
//! session may do; nothing here reaches GitHub.

use humata2_grpc::humata::v1 as pb;
use humata2_mcp::{Call, ErrorKind, Reply, ToolError};
use serde_json::{Map, Value, json};
use tonic::transport::Channel;

use super::{GrpcDaemon, applied, key, text};
use crate::history_service::rfc3339;

/// `owner/repo#number` as a pull request, or the tool error that says how
/// to write it.
fn pull_request(given: &str) -> Result<pb::PullRequestRef, ToolError> {
    let parsed = given.split_once('#').and_then(|(repo, number)| {
        let (owner, name) = repo.split_once('/')?;
        let number = number.trim().parse::<u32>().ok().filter(|number| *number > 0)?;
        (!owner.is_empty() && !name.is_empty() && !name.contains('/')).then(|| pb::PullRequestRef {
            repo: Some(pb::RepoRef {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
            number,
            ..pb::PullRequestRef::default()
        })
    });
    parsed.ok_or_else(|| ToolError {
        kind: ErrorKind::InvalidField,
        message: format!("{given:?} is not a pull request written as owner/repo#number"),
        fix: "write the pull request like acme/api#482".to_owned(),
        retry: None,
        details: Map::new(),
    })
}

/// Who marked, as the tools write it.
fn by_json(by: Option<&pb::ReviewedBy>) -> Value {
    by.map_or(Value::Null, |by| {
        let actor = match by.actor.as_ref().and_then(|actor| actor.actor.as_ref()) {
            Some(pb::actor::Actor::User(_)) => "user",
            Some(_) => "agent",
            None => "unknown",
        };
        json!({ "actor": actor, "gh_login": by.gh_login })
    })
}

/// `PullRequestCommits` in the tools' snake_case, enums lowercase without
/// their prefix.
fn commits_json(listed: &pb::PullRequestCommits) -> Value {
    let commits: Vec<Value> = listed
        .commits
        .iter()
        .map(|commit| {
            let state = match pb::pr_commit::State::try_from(commit.state) {
                Ok(pb::pr_commit::State::Reviewed) => "reviewed",
                Ok(pb::pr_commit::State::Carried) => "carried",
                _ => "unreviewed",
            };
            json!({
                "sha": commit.sha,
                "subject": commit.subject,
                "patch_id": commit.patch_id,
                "state": state,
                "reviewed_as_sha": commit.reviewed_as_sha,
                "reviewed_at": commit.reviewed_at.as_ref().map(rfc3339::format),
                "reviewed_head_sha": commit.reviewed_head_sha,
                "by": by_json(commit.by.as_ref()),
            })
        })
        .collect();
    let history: Vec<Value> = listed
        .history
        .iter()
        .map(|entry| {
            let kind = match pb::commit_review_entry::Kind::try_from(entry.kind) {
                Ok(pb::commit_review_entry::Kind::Marked) => "marked",
                Ok(pb::commit_review_entry::Kind::Unmarked) => "unmarked",
                Ok(pb::commit_review_entry::Kind::Carried) => "carried",
                Ok(pb::commit_review_entry::Kind::WholePr) => "whole_pr",
                _ => "unknown",
            };
            json!({
                "seq": entry.seq,
                "at": entry.at.as_ref().map(rfc3339::format),
                "kind": kind,
                "commits": entry.commits,
                "head_sha": entry.head_sha,
                "snapshot_n": entry.snapshot_n,
                "by": by_json(entry.by.as_ref()),
            })
        })
        .collect();
    json!({ "head_sha": listed.head_sha, "commits": commits, "history": history })
}

/// A tool's `lines` as a range.
fn lines(value: Option<&Value>) -> Option<pb::LineRange> {
    let number = |field: &str| {
        value
            .and_then(|lines| lines.get(field))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    Some(pb::LineRange {
        first: number("first")?,
        last: number("last")?,
    })
}

impl GrpcDaemon {
    fn pull_request_client(
        &self,
    ) -> pb::pull_request_review_service_client::PullRequestReviewServiceClient<Channel> {
        pb::pull_request_review_service_client::PullRequestReviewServiceClient::new(self.channel.clone())
    }

    /// `review_draft {pr, path, lines, side, quoted, text}` -> `{comment_id, status: "draft"}`.
    pub(super) async fn review_draft(&self, call: Call) -> Result<Reply, ToolError> {
        let session = self.own_session().await?;
        let arguments = &call.arguments;
        let pr = pull_request(&text(arguments, "pr"))?;
        let side = if text(arguments, "side") == "old" {
            pb::DiffSide::Old
        } else {
            pb::DiffSide::New
        };
        let request = self.request(pb::CreateCommentRequest {
            idempotency_key: Some(key(&call)),
            session: Some(session),
            anchor: Some(pb::CommentAnchor {
                file: Some(pb::RepoPath {
                    repo: pr.repo.clone(),
                    path: text(arguments, "path"),
                }),
                lines: lines(arguments.get("lines")),
                side: side as i32,
                quoted: text(arguments, "quoted"),
                ..pb::CommentAnchor::default()
            }),
            text: text(arguments, "text"),
            destination: Some(pb::CommentDestination {
                destination: Some(pb::comment_destination::Destination::PullRequest(pr)),
            }),
        })?;
        let draft = self
            .comments()
            .create_comment(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let result = json!({ "comment_id": draft.id.map(|id| id.value), "status": "draft" });
        Ok(applied(result, None))
    }

    /// `review_draft_edit {comment_id, text}`.
    pub(super) async fn review_draft_edit(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::EditCommentRequest {
            idempotency_key: Some(key(&call)),
            id: Some(pb::CommentId {
                value: text(&call.arguments, "comment_id"),
            }),
            text: text(&call.arguments, "text"),
            expected_revision: None,
        })?;
        let draft = self
            .comments()
            .edit_comment(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let result = json!({ "comment_id": draft.id.map(|id| id.value), "status": "draft" });
        Ok(applied(result, None))
    }

    /// `review_draft_drop {comment_id}` -> `{dropped: true}`.
    pub(super) async fn review_draft_drop(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::DropCommentRequest {
            idempotency_key: Some(key(&call)),
            id: Some(pb::CommentId {
                value: text(&call.arguments, "comment_id"),
            }),
            expected_revision: None,
        })?;
        self.comments()
            .drop_comment(request)
            .await
            .map_err(|status| self.failed(&status))?;
        Ok(applied(json!({ "dropped": true }), None))
    }

    /// `review_summary_draft {pr, summary}`, through DraftSummary, which
    /// keeps it for Submit review.
    pub(super) async fn review_summary_draft(&self, call: Call) -> Result<Reply, ToolError> {
        let session = self.own_session().await?;
        let request = self.request(pb::DraftSummaryRequest {
            idempotency_key: Some(key(&call)),
            session: Some(session),
            pr: Some(pull_request(&text(&call.arguments, "pr"))?),
            summary: text(&call.arguments, "summary"),
        })?;
        self.pull_request_client()
            .draft_summary(request)
            .await
            .map_err(|status| self.failed(&status))?;
        Ok(applied(json!({}), None))
    }

    /// `review_commits {pr}` -> `{head_sha, commits, history}`: which of a
    /// pull request's commits the user reviewed. It only reads; no tool
    /// marks commits.
    pub(super) async fn review_commits(&self, call: Call) -> Result<Reply, ToolError> {
        let session = self.own_session().await?;
        let request = self.request(pb::ListPullRequestCommitsRequest {
            session: Some(session),
            pr: Some(pull_request(&text(&call.arguments, "pr"))?),
        })?;
        let listed = self
            .pull_request_client()
            .list_pull_request_commits(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(commits_json(&listed), None))
    }

    /// `pr_review_start {prs, machine, agent}`: StartReview as the agent.
    /// The daemon asks the user first (D288); an answer pending comes back
    /// as a pending reply.
    pub(super) async fn pr_review_start(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let prs = arguments
            .get("prs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(pull_request)
            .collect::<Result<Vec<_>, _>>()?;
        let agent = text(arguments, "agent");
        let request = self.request(pb::StartReviewRequest {
            idempotency_key: Some(key(&call)),
            prs,
            machine: Some(pb::MachineId {
                value: text(arguments, "machine"),
            }),
            agent: (!agent.is_empty()).then_some(pb::AgentId { value: agent }),
            ..pb::StartReviewRequest::default()
        })?;
        let operation = self
            .pull_request_client()
            .start_review(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(
            json!({ "operation": operation.id.map(|id| id.value) }),
            None,
        ))
    }
}
