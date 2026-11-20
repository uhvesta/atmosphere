//! Watched pull requests read through the host's `gh` CLI (TODO2WIRE gap 4).
//!
//! One query reads the pull request itself (state, merge conflicts, checks,
//! unresolved review threads and its base branch). When the base isn't the
//! repository's default branch the pull request sits in a stack, and a
//! second query asks whether the pull request below it was merged.

use std::ffi::OsString;
use std::process::{Command, Stdio};

use humata2_rpc_kit::blocking;
use serde::Deserialize;

use crate::ports::{PrStatus, PrStatusError, PrStatusPort};

const PR_QUERY: &str = r#"query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    defaultBranchRef { name }
    pullRequest(number: $number) {
      title
      state
      merged
      mergeable
      baseRefName
      commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
      reviewThreads(first: 100) { nodes { isResolved } }
    }
  }
}"#;

const BELOW_QUERY: &str = r#"query($owner: String!, $name: String!, $base: String!) {
  repository(owner: $owner, name: $name) {
    pullRequests(headRefName: $base, states: [MERGED], first: 1) {
      nodes { number }
    }
  }
}"#;

/// Reads a watched pull request's state with `gh`.
pub struct GhPrStatus {
    program: OsString,
}

impl GhPrStatus {
    /// Runs `program` (a path or a name on `PATH`) as the GitHub CLI.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// The host's reader, or none for a test daemon whose rig gave it no
    /// stand-in `gh`: tests never reach the real GitHub.
    #[must_use]
    pub fn for_host() -> Option<Self> {
        (!humata2_rpc_kit::admin::is_admin_mode() || std::env::var_os("HUMATA2_GH").is_some())
            .then(Self::default)
    }

    async fn graphql(&self, args: Vec<(String, String, bool)>, repo_pr: &str) -> Result<String, PrStatusError> {
        let program = self.program.clone();
        let repo_pr = repo_pr.to_owned();
        match blocking(move || run(&program, &args, &repo_pr)).await {
            Ok(done) => done,
            Err(status) => Err(PrStatusError::Unreachable {
                first_line: status.message().to_owned(),
            }),
        }
    }
}

impl Default for GhPrStatus {
    /// `gh`; in an admin test build, the one `HUMATA2_GH` names.
    fn default() -> Self {
        let program = if humata2_rpc_kit::admin::is_admin_mode()
            && let Some(program) = std::env::var_os("HUMATA2_GH")
        {
            program
        } else {
            OsString::from("gh")
        };
        Self { program }
    }
}

#[tonic::async_trait]
impl PrStatusPort for GhPrStatus {
    async fn fetch(&self, owner: &str, name: &str, number: u32) -> Result<PrStatus, PrStatusError> {
        let repo_pr = format!("{owner}/{name}#{number}");
        let printed = self
            .graphql(
                vec![
                    ("query".to_owned(), PR_QUERY.to_owned(), false),
                    ("owner".to_owned(), owner.to_owned(), false),
                    ("name".to_owned(), name.to_owned(), false),
                    ("number".to_owned(), number.to_string(), true),
                ],
                &repo_pr,
            )
            .await?;
        let read = parse_pull_request(&printed, &repo_pr)?;
        let mut status = read.status;
        let in_a_stack = !read.base_ref.is_empty()
            && read.default_branch.as_deref().is_some_and(|default| default != read.base_ref);
        if in_a_stack && !status.merged && !status.closed_without_merge {
            let printed = self
                .graphql(
                    vec![
                        ("query".to_owned(), BELOW_QUERY.to_owned(), false),
                        ("owner".to_owned(), owner.to_owned(), false),
                        ("name".to_owned(), name.to_owned(), false),
                        ("base".to_owned(), read.base_ref, false),
                    ],
                    &repo_pr,
                )
                .await?;
            status.upstream_merged = parse_below_merged(&printed)?;
        }
        Ok(status)
    }
}

/// Runs `gh api graphql` with the `(key, value, typed)` fields; `typed`
/// fields go as `-F` so numbers stay numbers.
fn run(
    program: &OsString,
    fields: &[(String, String, bool)],
    repo_pr: &str,
) -> Result<String, PrStatusError> {
    let mut command = Command::new(program);
    command.args(["api", "graphql"]);
    for (key, value, typed) in fields {
        command.arg(if *typed { "-F" } else { "-f" });
        command.arg(format!("{key}={value}"));
    }
    let output = command
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|source| PrStatusError::GhMissing {
            program: program.to_string_lossy().into_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(classify(
            String::from_utf8_lossy(&output.stderr).trim(),
            repo_pr,
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// What `gh`'s stderr says went wrong.
fn classify(stderr: &str, repo_pr: &str) -> PrStatusError {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("rate limit") {
        PrStatusError::RateLimited {
            wait_seconds: wait_seconds(stderr),
        }
    } else if ["auth login", "not logged", "authentication", "http 401", "bad credentials"]
        .iter()
        .any(|sign| lower.contains(sign))
    {
        PrStatusError::SignedOut {
            stderr: stderr.to_owned(),
        }
    } else if ["could not resolve to", "http 404", "not found"]
        .iter()
        .any(|sign| lower.contains(sign))
    {
        PrStatusError::NotFound {
            repo_pr: repo_pr.to_owned(),
        }
    } else {
        PrStatusError::Unreachable {
            first_line: stderr.lines().next().unwrap_or_default().to_owned(),
        }
    }
}

/// The wait GitHub names in seconds ("retry in 30 seconds", "after 2
/// minutes"), if its message does.
fn wait_seconds(stderr: &str) -> Option<u64> {
    let words: Vec<&str> = stderr.split_whitespace().collect();
    words.windows(2).find_map(|pair| {
        let count: u64 = pair[0].parse().ok()?;
        match pair[1].trim_end_matches(['.', ',', ')']) {
            "second" | "seconds" => Some(count),
            "minute" | "minutes" => Some(count.saturating_mul(60)),
            _ => None,
        }
    })
}

#[derive(Deserialize)]
struct PrReply {
    data: Option<PrData>,
}

#[derive(Deserialize)]
struct PrData {
    repository: Option<PrRepository>,
}

#[derive(Deserialize)]
struct PrRepository {
    #[serde(rename = "defaultBranchRef")]
    default_branch: Option<Named>,
    #[serde(rename = "pullRequest")]
    pull_request: Option<PullRequest>,
}

#[derive(Deserialize)]
struct Named {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct PullRequest {
    #[serde(default)]
    title: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    merged: bool,
    #[serde(default)]
    mergeable: String,
    #[serde(default, rename = "baseRefName")]
    base_ref: String,
    #[serde(default)]
    commits: Nodes<CommitNode>,
    #[serde(default, rename = "reviewThreads")]
    review_threads: Nodes<Thread>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<T>>,
}

impl<T> Default for Nodes<T> {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

#[derive(Deserialize)]
struct CommitNode {
    commit: Option<Commit>,
}

#[derive(Deserialize)]
struct Commit {
    #[serde(rename = "statusCheckRollup")]
    rollup: Option<Rollup>,
}

#[derive(Deserialize)]
struct Rollup {
    #[serde(default)]
    state: String,
}

#[derive(Deserialize)]
struct Thread {
    #[serde(default, rename = "isResolved")]
    resolved: bool,
}

/// A pull request as read, with what the stack check needs.
struct Read {
    status: PrStatus,
    base_ref: String,
    default_branch: Option<String>,
}

fn parse_pull_request(printed: &str, repo_pr: &str) -> Result<Read, PrStatusError> {
    let reply: PrReply = serde_json::from_str(printed)
        .map_err(|source| PrStatusError::Unreadable { source })?;
    let not_found = || PrStatusError::NotFound {
        repo_pr: repo_pr.to_owned(),
    };
    let repository = reply
        .data
        .and_then(|data| data.repository)
        .ok_or_else(not_found)?;
    let pr = repository.pull_request.ok_or_else(not_found)?;
    let rollup = pr
        .commits
        .nodes
        .into_iter()
        .flatten()
        .filter_map(|node| node.commit)
        .find_map(|commit| commit.rollup)
        .map(|rollup| rollup.state)
        .unwrap_or_default();
    let ci_state = match rollup.as_str() {
        "SUCCESS" => "passing",
        "FAILURE" | "ERROR" => "failing",
        "PENDING" | "EXPECTED" => "pending",
        _ => "",
    };
    let unresolved = pr
        .review_threads
        .nodes
        .iter()
        .flatten()
        .filter(|thread| !thread.resolved)
        .count();
    let merged = pr.merged || pr.state == "MERGED";
    Ok(Read {
        status: PrStatus {
            title: pr.title,
            merged,
            closed_without_merge: pr.state == "CLOSED" && !merged,
            ci_state: ci_state.to_owned(),
            unresolved_comments: u32::try_from(unresolved).unwrap_or(u32::MAX),
            has_conflicts: pr.mergeable == "CONFLICTING",
            upstream_merged: false,
        },
        base_ref: pr.base_ref,
        default_branch: repository.default_branch.map(|branch| branch.name),
    })
}

#[derive(Deserialize)]
struct BelowReply {
    data: Option<BelowData>,
}

#[derive(Deserialize)]
struct BelowData {
    repository: Option<BelowRepository>,
}

#[derive(Deserialize)]
struct BelowRepository {
    #[serde(default, rename = "pullRequests")]
    pull_requests: Nodes<serde_json::Value>,
}

/// Whether a merged pull request has the base branch as its head.
fn parse_below_merged(printed: &str) -> Result<bool, PrStatusError> {
    let reply: BelowReply = serde_json::from_str(printed)
        .map_err(|source| PrStatusError::Unreadable { source })?;
    Ok(reply
        .data
        .and_then(|data| data.repository)
        .is_some_and(|repository| repository.pull_requests.nodes.iter().flatten().next().is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(state: &str, merged: bool, mergeable: &str, rollup: &str, base: &str) -> String {
        let checks = if rollup.is_empty() {
            r#"{"nodes":[{"commit":{"statusCheckRollup":null}}]}"#.to_owned()
        } else {
            format!(r#"{{"nodes":[{{"commit":{{"statusCheckRollup":{{"state":"{rollup}"}}}}}}]}}"#)
        };
        format!(
            r#"{{"data":{{"repository":{{"defaultBranchRef":{{"name":"main"}},"pullRequest":{{
              "title":"Add watcher","state":"{state}","merged":{merged},"mergeable":"{mergeable}",
              "baseRefName":"{base}","commits":{checks},
              "reviewThreads":{{"nodes":[{{"isResolved":false}},{{"isResolved":true}},{{"isResolved":false}}]}}
            }}}}}}}}"#
        )
    }

    #[test]
    fn an_open_failing_pull_request_reads_with_its_unresolved_threads() {
        let read = parse_pull_request(&reply("OPEN", false, "MERGEABLE", "FAILURE", "main"), "a/b#1")
            .unwrap();
        assert_eq!(read.status.title, "Add watcher");
        assert_eq!(read.status.ci_state, "failing");
        assert_eq!(read.status.unresolved_comments, 2);
        assert!(!read.status.has_conflicts && !read.status.merged && !read.status.closed_without_merge);
        assert_eq!(read.base_ref, "main");
        assert_eq!(read.default_branch.as_deref(), Some("main"));
    }

    #[test]
    fn check_rollups_map_to_ci_states() {
        let ci = |rollup: &str| {
            parse_pull_request(&reply("OPEN", false, "MERGEABLE", rollup, "main"), "a/b#1")
                .unwrap()
                .status
                .ci_state
        };
        assert_eq!(ci("SUCCESS"), "passing");
        assert_eq!(ci("ERROR"), "failing");
        assert_eq!(ci("PENDING"), "pending");
        assert_eq!(ci("EXPECTED"), "pending");
        assert_eq!(ci(""), "");
    }

    #[test]
    fn conflicts_merged_and_closed_are_told_apart() {
        let conflicted =
            parse_pull_request(&reply("OPEN", false, "CONFLICTING", "SUCCESS", "main"), "a/b#1")
                .unwrap();
        assert!(conflicted.status.has_conflicts);
        let merged =
            parse_pull_request(&reply("MERGED", true, "UNKNOWN", "SUCCESS", "main"), "a/b#1")
                .unwrap();
        assert!(merged.status.merged && !merged.status.closed_without_merge);
        let closed =
            parse_pull_request(&reply("CLOSED", false, "UNKNOWN", "SUCCESS", "main"), "a/b#1")
                .unwrap();
        assert!(closed.status.closed_without_merge && !closed.status.merged);
    }

    #[test]
    fn a_missing_pull_request_is_not_found() {
        let printed = r#"{"data":{"repository":{"defaultBranchRef":{"name":"main"},"pullRequest":null}}}"#;
        assert!(matches!(
            parse_pull_request(printed, "a/b#9"),
            Err(PrStatusError::NotFound { repo_pr }) if repo_pr == "a/b#9"
        ));
        assert!(matches!(
            parse_pull_request(r#"{"data":{"repository":null}}"#, "a/b#9"),
            Err(PrStatusError::NotFound { .. })
        ));
    }

    #[test]
    fn text_that_is_not_json_is_unreadable() {
        assert!(matches!(
            parse_pull_request("not json", "a/b#1"),
            Err(PrStatusError::Unreadable { .. })
        ));
        assert!(matches!(
            parse_below_merged("not json"),
            Err(PrStatusError::Unreadable { .. })
        ));
    }

    #[test]
    fn the_pull_request_below_counts_only_when_a_merged_one_has_the_base_as_head() {
        let merged = r#"{"data":{"repository":{"pullRequests":{"nodes":[{"number":4}]}}}}"#;
        let none = r#"{"data":{"repository":{"pullRequests":{"nodes":[]}}}}"#;
        assert!(parse_below_merged(merged).unwrap());
        assert!(!parse_below_merged(none).unwrap());
    }

    #[test]
    fn stderr_tells_the_failures_apart() {
        assert!(matches!(
            classify("API rate limit exceeded, retry in 30 seconds", "a/b#1"),
            PrStatusError::RateLimited {
                wait_seconds: Some(30)
            }
        ));
        assert!(matches!(
            classify("secondary rate limit", "a/b#1"),
            PrStatusError::RateLimited { wait_seconds: None }
        ));
        assert!(matches!(
            classify("To get started with GitHub CLI, please run: gh auth login", "a/b#1"),
            PrStatusError::SignedOut { .. }
        ));
        assert!(matches!(
            classify("GraphQL: Could not resolve to a PullRequest with the number of 9.", "a/b#9"),
            PrStatusError::NotFound { .. }
        ));
        assert!(matches!(
            classify("error connecting to api.github.com", "a/b#1"),
            PrStatusError::Unreachable { .. }
        ));
    }

    #[test]
    fn a_missing_program_is_gh_missing() {
        let gh = GhPrStatus::with_program("/nonexistent/gh-for-test");
        assert!(matches!(
            run(&gh.program, &[], "a/b#1"),
            Err(PrStatusError::GhMissing { .. })
        ));
    }
}
