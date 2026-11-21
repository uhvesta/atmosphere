//! ATM-22, ATM-27: Fetch, parse, filter, collapse, and deduplicate GitHub PR
//! feedback in a single GraphQL query per pull request.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::github::{self, GithubError};

/// Single GraphQL query per PR requesting review threads, top-level reviews,
/// and PR lifecycle metadata.
pub const PR_FEEDBACK_QUERY: &str = r#"
query PrFeedback($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      state
      merged
      url
      title
      headRefOid
      reviewThreads(first: 100) {
        nodes {
          id
          isResolved
          isOutdated
          path
          line
          originalLine
          comments(first: 100) {
            nodes {
              id
              databaseId
              body
              url
              createdAt
              author {
                login
                __typename
              }
            }
          }
        }
      }
      reviews(first: 100) {
        nodes {
          id
          state
          body
          url
          submittedAt
          author {
            login
            __typename
          }
        }
      }
    }
  }
}
"#;

/// Marker embedded in automated stack-history comments that must be filtered out.
const STACK_HISTORY_MARKER: &str = "<!-- stack-history -->";

/// Lifecycle state of a pull request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrLifecycleState {
    #[default]
    Unknown,
    Open,
    Closed,
    Merged,
}

impl PrLifecycleState {
    /// Whether polling should stop for a PR in this state.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Merged)
    }
}

/// One comment inside a review thread.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadComment {
    pub id: String,
    pub database_id: Option<u64>,
    pub author: String,
    pub body: String,
    pub url: String,
    pub created_at: String,
}

/// One human review thread on a pull request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackThread {
    pub pr_key: String,
    pub thread_id: String,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub path: String,
    pub line: Option<u32>,
    pub original_line: Option<u32>,
    pub reviewer: String,
    pub body: String,
    pub url: String,
    pub created_at: String,
    pub comments: Vec<ThreadComment>,
}

/// One top-level human review on a pull request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopLevelReview {
    pub pr_key: String,
    pub review_id: String,
    pub state: String,
    pub reviewer: String,
    pub body: String,
    pub url: String,
    pub submitted_at: String,
}

/// Quiet summary of all bot comments collapsed out of the main human feed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollapsedBots {
    pub count: usize,
    pub authors: Vec<String>,
    pub label: String,
}

impl CollapsedBots {
    pub(crate) fn from_counts(count: usize, authors: BTreeSet<String>) -> Option<Self> {
        if count == 0 {
            return None;
        }
        let label = if count == 1 {
            "1 bot comment collapsed".to_owned()
        } else {
            format!("{count} bot comments collapsed")
        };
        Some(Self {
            count,
            authors: authors.into_iter().collect(),
            label,
        })
    }
}

/// Normalized feedback for one pull request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrFeedback {
    pub pr_key: String,
    pub owner: String,
    pub name: String,
    pub number: u32,
    pub title: String,
    pub state: PrLifecycleState,
    pub merged: bool,
    pub url: String,
    pub head_ref_oid: String,
    pub threads: Vec<FeedbackThread>,
    pub reviews: Vec<TopLevelReview>,
    pub collapsed_bots: Option<CollapsedBots>,
}

/// Errors returned while fetching or parsing PR feedback.
#[derive(Debug, thiserror::Error)]
pub enum FeedbackError {
    #[error("couldn't fetch feedback for {owner}/{name}#{number} from GitHub")]
    Github {
        owner: String,
        name: String,
        number: u32,
        #[source]
        source: GithubError,
    },
    #[error("GitHub GraphQL returned errors for {pr_key}: {}", messages.join("; "))]
    GraphQlErrors {
        pr_key: String,
        messages: Vec<String>,
    },
    #[error("GitHub feedback response for {pr_key} is missing `{field}`")]
    MissingField { pr_key: String, field: &'static str },
}

/// Canonical key for a pull request: `owner/name#number`.
pub fn pr_key(owner: &str, name: &str, number: u32) -> String {
    format!("{owner}/{name}#{number}")
}

/// Returns true if a comment or review body is an automated stack-history block.
pub fn is_stack_history(body: &str) -> bool {
    body.contains(STACK_HISTORY_MARKER)
}

/// Returns true if an author represents a bot account.
pub fn is_bot_author(login: &str, typename: Option<&str>, known_bot: bool) -> bool {
    if known_bot {
        return true;
    }
    if typename.is_some_and(|t| t.eq_ignore_ascii_case("Bot")) {
        return true;
    }
    let lower = login.trim().to_ascii_lowercase();
    lower.ends_with("[bot]") || lower.ends_with("-bot")
}

/// Fetches PR feedback using a single `gh api graphql` call. Blocking.
pub fn fetch_pr_feedback(
    owner: &str,
    name: &str,
    number: u32,
) -> Result<PrFeedback, FeedbackError> {
    let query_arg = format!("query={PR_FEEDBACK_QUERY}");
    let owner_arg = format!("owner={owner}");
    let name_arg = format!("name={name}");
    let number_arg = format!("number={number}");
    let value = github::gh_json(&[
        "api",
        "graphql",
        "-f",
        query_arg.as_str(),
        "-f",
        owner_arg.as_str(),
        "-f",
        name_arg.as_str(),
        "-F",
        number_arg.as_str(),
    ])
    .map_err(|source| FeedbackError::Github {
        owner: owner.to_owned(),
        name: name.to_owned(),
        number,
        source,
    })?;
    parse_pr_feedback(owner, name, number, &value)
}

/// Parses a GraphQL JSON response into normalized [`PrFeedback`].
pub fn parse_pr_feedback(
    owner: &str,
    name: &str,
    number: u32,
    response: &Value,
) -> Result<PrFeedback, FeedbackError> {
    let key = pr_key(owner, name, number);

    if let Some(errors) = response.get("errors").and_then(Value::as_array) {
        let messages: Vec<String> = errors
            .iter()
            .filter_map(|e| e.get("message").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        if !messages.is_empty() {
            return Err(FeedbackError::GraphQlErrors {
                pr_key: key,
                messages,
            });
        }
    }

    let pr = response
        .pointer("/data/repository/pullRequest")
        .filter(|v| !v.is_null())
        .ok_or_else(|| FeedbackError::MissingField {
            pr_key: key.clone(),
            field: "data.repository.pullRequest",
        })?;

    let merged = pr.get("merged").and_then(Value::as_bool).unwrap_or(false);
    let state = match (pr.get("state").and_then(Value::as_str), merged) {
        (_, true) | (Some("MERGED"), _) => PrLifecycleState::Merged,
        (Some("OPEN"), _) => PrLifecycleState::Open,
        (Some("CLOSED"), _) => PrLifecycleState::Closed,
        _ => PrLifecycleState::Unknown,
    };
    let url = str_field(pr, "url");
    let title = str_field(pr, "title");
    let head_ref_oid = str_field(pr, "headRefOid");

    let mut bot_count = 0_usize;
    let mut bot_authors = BTreeSet::new();

    // Parse review threads.
    let mut raw_threads = Vec::new();
    if let Some(nodes) = pr.pointer("/reviewThreads/nodes").and_then(Value::as_array) {
        for node in nodes {
            let thread_id = str_field(node, "id");
            if thread_id.is_empty() {
                continue;
            }
            let is_resolved = node
                .get("isResolved")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let is_outdated = node
                .get("isOutdated")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let path = str_field(node, "path");
            let line = u32_field(node, "line");
            let original_line = u32_field(node, "originalLine");

            let mut human_comments = Vec::new();
            if let Some(comment_nodes) = node.pointer("/comments/nodes").and_then(Value::as_array) {
                for comment in comment_nodes {
                    let body = str_field(comment, "body");
                    if is_stack_history(&body) {
                        continue;
                    }
                    let (author, typename, known_bot) = parse_author(comment.get("author"));
                    if is_bot_author(&author, typename.as_deref(), known_bot) {
                        bot_count += 1;
                        if !author.is_empty() {
                            bot_authors.insert(author);
                        }
                        continue;
                    }
                    human_comments.push(ThreadComment {
                        id: str_field(comment, "id"),
                        database_id: comment.get("databaseId").and_then(Value::as_u64),
                        author,
                        body,
                        url: str_field(comment, "url"),
                        created_at: str_field(comment, "createdAt"),
                    });
                }
            }

            if let Some(first) = human_comments.first() {
                let latest = human_comments.last().unwrap_or(first);
                raw_threads.push(FeedbackThread {
                    pr_key: key.clone(),
                    thread_id,
                    is_resolved,
                    is_outdated,
                    path,
                    line,
                    original_line,
                    reviewer: first.author.clone(),
                    body: first.body.clone(),
                    url: first.url.clone(),
                    created_at: latest.created_at.clone(),
                    comments: human_comments,
                });
            }
        }
    }

    let threads = deduplicate_threads(raw_threads);

    // Parse top-level reviews.
    let mut reviews = Vec::new();
    if let Some(nodes) = pr.pointer("/reviews/nodes").and_then(Value::as_array) {
        for node in nodes {
            let review_id = str_field(node, "id");
            if review_id.is_empty() {
                continue;
            }
            let body = str_field(node, "body");
            if is_stack_history(&body) {
                continue;
            }
            let (author, typename, known_bot) = parse_author(node.get("author"));
            if is_bot_author(&author, typename.as_deref(), known_bot) {
                bot_count += 1;
                if !author.is_empty() {
                    bot_authors.insert(author);
                }
                continue;
            }
            reviews.push(TopLevelReview {
                pr_key: key.clone(),
                review_id,
                state: str_field(node, "state"),
                reviewer: author,
                body,
                url: str_field(node, "url"),
                submitted_at: str_field(node, "submittedAt"),
            });
        }
    }

    Ok(PrFeedback {
        pr_key: key,
        owner: owner.to_owned(),
        name: name.to_owned(),
        number,
        title,
        state,
        merged,
        url,
        head_ref_oid,
        threads,
        reviews,
        collapsed_bots: CollapsedBots::from_counts(bot_count, bot_authors),
    })
}

/// Deduplicates feedback threads by `(pr_key, thread_id)`, keeping the richest
/// or latest entry while preserving deterministic ordering.
pub fn deduplicate_threads(threads: Vec<FeedbackThread>) -> Vec<FeedbackThread> {
    let mut by_key: BTreeMap<(String, String), FeedbackThread> = BTreeMap::new();
    for thread in threads {
        let key = (thread.pr_key.clone(), thread.thread_id.clone());
        match by_key.get(&key) {
            Some(existing) if !should_replace_thread(existing, &thread) => {}
            _ => {
                by_key.insert(key, thread);
            }
        }
    }
    by_key.into_values().collect()
}

fn should_replace_thread(existing: &FeedbackThread, candidate: &FeedbackThread) -> bool {
    if candidate.comments.len() != existing.comments.len() {
        return candidate.comments.len() > existing.comments.len();
    }
    if candidate.created_at != existing.created_at {
        return candidate.created_at > existing.created_at;
    }
    candidate.body.len() > existing.body.len()
}

fn parse_author(author: Option<&Value>) -> (String, Option<String>, bool) {
    let Some(author) = author.filter(|v| !v.is_null()) else {
        return (String::new(), None, false);
    };
    let login = str_field(author, "login");
    let typename = author
        .get("__typename")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let known_bot = author
        .get("isBot")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    (login, typename, known_bot)
}

fn str_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn u32_field(value: &Value, field: &str) -> Option<u32> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn graphql_query_requests_all_feedback_in_one_query() {
        assert_eq!(PR_FEEDBACK_QUERY.matches("query ").count(), 1);
        for token in [
            "reviewThreads",
            "isResolved",
            "isOutdated",
            "originalLine",
            "databaseId",
            "__typename",
            "reviews",
            "submittedAt",
            "headRefOid",
            "merged",
        ] {
            assert!(
                PR_FEEDBACK_QUERY.contains(token),
                "PR_FEEDBACK_QUERY missing `{token}`"
            );
        }
    }

    #[test]
    fn filters_stack_history_from_threads_and_reviews() {
        let payload = json!({
            "data": {
                "repository": {
                    "pullRequest": {
                        "state": "OPEN",
                        "merged": false,
                        "url": "https://github.com/acme/api/pull/42",
                        "title": "Add feedback polling",
                        "headRefOid": "abc123",
                        "reviewThreads": {
                            "nodes": [
                                {
                                    "id": "T_stack",
                                    "isResolved": false,
                                    "isOutdated": false,
                                    "path": "src/lib.rs",
                                    "line": 10,
                                    "originalLine": 10,
                                    "comments": {
                                        "nodes": [{
                                            "id": "C_stack",
                                            "databaseId": 1,
                                            "body": "<!-- stack-history -->\nStack updated",
                                            "url": "https://github.com/acme/api/pull/42#discussion_r1",
                                            "createdAt": "2026-11-20T10:00:00Z",
                                            "author": { "login": "graphite-app[bot]", "__typename": "Bot" }
                                        }]
                                    }
                                },
                                {
                                    "id": "T_human",
                                    "isResolved": false,
                                    "isOutdated": false,
                                    "path": "src/main.rs",
                                    "line": 24,
                                    "originalLine": 20,
                                    "comments": {
                                        "nodes": [{
                                            "id": "C_human",
                                            "databaseId": 2,
                                            "body": "Please handle empty input.",
                                            "url": "https://github.com/acme/api/pull/42#discussion_r2",
                                            "createdAt": "2026-11-20T10:05:00Z",
                                            "author": { "login": "alice", "__typename": "User" }
                                        }]
                                    }
                                }
                            ]
                        },
                        "reviews": {
                            "nodes": [
                                {
                                    "id": "R_stack",
                                    "state": "COMMENTED",
                                    "body": "<!-- stack-history --> summary",
                                    "url": "https://github.com/acme/api/pull/42#pullrequestreview-1",
                                    "submittedAt": "2026-11-20T10:01:00Z",
                                    "author": { "login": "alice", "__typename": "User" }
                                },
                                {
                                    "id": "R_human",
                                    "state": "CHANGES_REQUESTED",
                                    "body": "Needs a test for edge cases.",
                                    "url": "https://github.com/acme/api/pull/42#pullrequestreview-2",
                                    "submittedAt": "2026-11-20T10:06:00Z",
                                    "author": { "login": "bob", "__typename": "User" }
                                }
                            ]
                        }
                    }
                }
            }
        });

        let feedback = parse_pr_feedback("acme", "api", 42, &payload).expect("parse feedback");
        assert_eq!(feedback.threads.len(), 1);
        assert_eq!(feedback.threads[0].thread_id, "T_human");
        assert_eq!(feedback.reviews.len(), 1);
        assert_eq!(feedback.reviews[0].review_id, "R_human");
        // Stack-history bot comment is filtered completely, not counted as a collapsed bot.
        assert!(feedback.collapsed_bots.is_none());
    }

    #[test]
    fn collapses_bot_comments_into_one_quiet_summary() {
        let payload = json!({
            "data": {
                "repository": {
                    "pullRequest": {
                        "state": "OPEN",
                        "merged": false,
                        "url": "https://github.com/acme/api/pull/7",
                        "title": "Bot noise test",
                        "headRefOid": "def456",
                        "reviewThreads": {
                            "nodes": [
                                {
                                    "id": "T_bot_1",
                                    "isResolved": false,
                                    "isOutdated": false,
                                    "path": "src/a.rs",
                                    "line": 5,
                                    "originalLine": 5,
                                    "comments": {
                                        "nodes": [{
                                            "id": "C_b1",
                                            "databaseId": 10,
                                            "body": "Lint warning",
                                            "url": "https://github.com/acme/api/pull/7#discussion_r10",
                                            "createdAt": "2026-11-20T11:00:00Z",
                                            "author": { "login": "coderabbitai[bot]", "__typename": "Bot" }
                                        }]
                                    }
                                },
                                {
                                    "id": "T_mixed",
                                    "isResolved": true,
                                    "isOutdated": false,
                                    "path": "src/b.rs",
                                    "line": 12,
                                    "originalLine": 12,
                                    "comments": {
                                        "nodes": [
                                            {
                                                "id": "C_h1",
                                                "databaseId": 11,
                                                "body": "Looks good after rename",
                                                "url": "https://github.com/acme/api/pull/7#discussion_r11",
                                                "createdAt": "2026-11-20T11:01:00Z",
                                                "author": { "login": "carol", "__typename": "User" }
                                            },
                                            {
                                                "id": "C_b2",
                                                "databaseId": 12,
                                                "body": "Coverage +1%",
                                                "url": "https://github.com/acme/api/pull/7#discussion_r12",
                                                "createdAt": "2026-11-20T11:02:00Z",
                                                "author": { "login": "codecov-bot", "__typename": "User" }
                                            }
                                        ]
                                    }
                                }
                            ]
                        },
                        "reviews": {
                            "nodes": [{
                                "id": "R_bot",
                                "state": "COMMENTED",
                                "body": "Automated review summary",
                                "url": "https://github.com/acme/api/pull/7#pullrequestreview-9",
                                "submittedAt": "2026-11-20T11:03:00Z",
                                "author": { "login": "github-actions[bot]", "__typename": "Bot" }
                            }]
                        }
                    }
                }
            }
        });

        let feedback = parse_pr_feedback("acme", "api", 7, &payload).expect("parse feedback");
        assert_eq!(feedback.threads.len(), 1);
        assert_eq!(feedback.threads[0].thread_id, "T_mixed");
        assert_eq!(feedback.threads[0].comments.len(), 1);
        assert_eq!(feedback.threads[0].reviewer, "carol");
        assert!(feedback.reviews.is_empty());

        let collapsed = feedback.collapsed_bots.expect("collapsed bots summary");
        assert_eq!(collapsed.count, 3);
        assert_eq!(collapsed.label, "3 bot comments collapsed");
        assert_eq!(
            collapsed.authors,
            vec!["codecov-bot", "coderabbitai[bot]", "github-actions[bot]"]
        );
    }

    #[test]
    fn deduplicates_by_pr_key_and_thread_id_preserving_metadata() {
        let older = FeedbackThread {
            pr_key: "acme/api#42".to_owned(),
            thread_id: "T_1".to_owned(),
            is_resolved: false,
            is_outdated: false,
            path: "src/lib.rs".to_owned(),
            line: Some(15),
            original_line: Some(14),
            reviewer: "alice".to_owned(),
            body: "Initial comment".to_owned(),
            url: "https://github.com/acme/api/pull/42#discussion_r1".to_owned(),
            created_at: "2026-11-20T10:00:00Z".to_owned(),
            comments: vec![ThreadComment {
                id: "C_1".to_owned(),
                database_id: Some(1),
                author: "alice".to_owned(),
                body: "Initial comment".to_owned(),
                url: "https://github.com/acme/api/pull/42#discussion_r1".to_owned(),
                created_at: "2026-11-20T10:00:00Z".to_owned(),
            }],
        };
        let richer = FeedbackThread {
            is_resolved: true,
            is_outdated: true,
            created_at: "2026-11-20T10:15:00Z".to_owned(),
            comments: vec![
                older.comments[0].clone(),
                ThreadComment {
                    id: "C_2".to_owned(),
                    database_id: Some(2),
                    author: "bob".to_owned(),
                    body: "Fixed in latest push".to_owned(),
                    url: "https://github.com/acme/api/pull/42#discussion_r2".to_owned(),
                    created_at: "2026-11-20T10:15:00Z".to_owned(),
                },
            ],
            ..older.clone()
        };

        let deduped = deduplicate_threads(vec![older, richer]);
        assert_eq!(deduped.len(), 1);
        let kept = &deduped[0];
        assert_eq!(kept.pr_key, "acme/api#42");
        assert_eq!(kept.thread_id, "T_1");
        assert!(kept.is_resolved);
        assert!(kept.is_outdated);
        assert_eq!(kept.path, "src/lib.rs");
        assert_eq!(kept.line, Some(15));
        assert_eq!(kept.original_line, Some(14));
        assert_eq!(kept.reviewer, "alice");
        assert_eq!(kept.url, "https://github.com/acme/api/pull/42#discussion_r1");
        assert_eq!(kept.comments.len(), 2);
    }
}
