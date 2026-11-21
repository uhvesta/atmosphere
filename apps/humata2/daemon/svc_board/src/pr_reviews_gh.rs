//! The pull requests that ask for the user's review, read through the `gh`
//! CLI the host already uses for its repositories.

use std::ffi::OsString;
use std::process::{Command, Stdio};

use humata2_rpc_kit::blocking;
use serde::Deserialize;

use crate::ports::{FoundPr, ReviewFetchError, ReviewRequestsPort};

const QUERY: &str = r#"query {
  viewer { login }
  search(query: "is:open is:pr review-requested:@me archived:false", type: ISSUE, first: 50) {
    nodes {
      ... on PullRequest {
        number
        title
        url
        createdAt
        headRefName
        baseRefName
        author { login }
        repository { name owner { login } }
        reviewRequests(first: 50) {
          nodes {
            requestedReviewer {
              __typename
              ... on User { login }
              ... on Team { slug }
            }
          }
        }
        timelineItems(last: 1, itemTypes: [REVIEW_REQUESTED_EVENT]) {
          nodes { ... on ReviewRequestedEvent { createdAt } }
        }
      }
    }
  }
}"#;

/// Filtering options for GitHub review requests.
///
/// By default (`opted_in_teams` empty), only direct user review requests are
/// kept and team-only review requests are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewFilterOptions {
    /// Expected GitHub login for direct user requests; when `None`, uses
    /// `viewer.login` from the GraphQL reply if present, else any `User` node.
    pub viewer_login: Option<String>,
    /// Team slugs the user explicitly opted into reviewing.
    pub opted_in_teams: Vec<String>,
}

/// Reads review requests with `gh api graphql`.
pub struct GhReviewRequests {
    program: OsString,
    options: ReviewFilterOptions,
}

impl GhReviewRequests {
    /// Runs `program` (a path or a name on `PATH`) as the GitHub CLI.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            options: ReviewFilterOptions::default(),
        }
    }

    /// Configures team opt-in and viewer filtering for review requests.
    pub fn with_filter_options(mut self, options: ReviewFilterOptions) -> Self {
        self.options = options;
        self
    }
}

impl GhReviewRequests {
    /// The host's reader, or none for a test daemon whose rig gave it no
    /// stand-in `gh`: tests never reach the real GitHub.
    #[must_use]
    pub fn for_host() -> Option<Self> {
        (!humata2_rpc_kit::admin::is_admin_mode() || std::env::var_os("HUMATA2_GH").is_some())
            .then(Self::default)
    }
}

impl Default for GhReviewRequests {
    /// `gh`; in an admin test build, the one `HUMATA2_GH` names.
    fn default() -> Self {
        let program = if humata2_rpc_kit::admin::is_admin_mode()
            && let Some(program) = std::env::var_os("HUMATA2_GH")
        {
            program
        } else {
            OsString::from("gh")
        };
        Self {
            program,
            options: ReviewFilterOptions::default(),
        }
    }
}

#[tonic::async_trait]
impl ReviewRequestsPort for GhReviewRequests {
    async fn fetch(&self) -> Result<Vec<FoundPr>, ReviewFetchError> {
        let program = self.program.clone();
        let ran = blocking(move || run(&program)).await;
        let printed = match ran {
            Ok(done) => done?,
            Err(status) => {
                return Err(ReviewFetchError::Unreachable {
                    first_line: status.message().to_owned(),
                });
            }
        };
        parse_with_options(&printed, &self.options)
    }
}

fn run(program: &OsString) -> Result<String, ReviewFetchError> {
    let output = Command::new(program)
        .args(["api", "graphql", "-f"])
        .arg(format!("query={QUERY}"))
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|source| ReviewFetchError::GhMissing {
            program: program.to_string_lossy().into_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(classify(String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// What `gh`'s stderr says went wrong.
fn classify(stderr: &str) -> ReviewFetchError {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("rate limit") {
        ReviewFetchError::RateLimited {
            wait_seconds: wait_seconds(stderr),
        }
    } else if ["auth login", "not logged", "authentication", "http 401", "bad credentials"]
        .iter()
        .any(|sign| lower.contains(sign))
    {
        ReviewFetchError::SignedOut {
            stderr: stderr.to_owned(),
        }
    } else {
        ReviewFetchError::Unreachable {
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
struct Reply {
    data: Option<Data>,
}

#[derive(Deserialize)]
struct Data {
    viewer: Option<Login>,
    search: Search,
}

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    nodes: Vec<Option<Node>>,
}

#[derive(Deserialize, Default)]
struct Node {
    number: Option<u32>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default, rename = "createdAt")]
    created_at: String,
    #[serde(default, rename = "headRefName")]
    head_ref: String,
    #[serde(default, rename = "baseRefName")]
    base_ref: String,
    author: Option<Login>,
    repository: Option<Repository>,
    #[serde(default, rename = "reviewRequests")]
    review_requests: Option<ReviewRequests>,
    #[serde(default, rename = "timelineItems")]
    timeline: Timeline,
}

#[derive(Deserialize, Default)]
struct ReviewRequests {
    #[serde(default)]
    nodes: Vec<Option<ReviewRequestNode>>,
}

#[derive(Deserialize)]
struct ReviewRequestNode {
    #[serde(rename = "requestedReviewer")]
    requested_reviewer: Option<RequestedReviewer>,
}

#[derive(Deserialize)]
struct RequestedReviewer {
    #[serde(default, rename = "__typename")]
    typename: String,
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    slug: Option<String>,
}

#[derive(Deserialize)]
struct Login {
    #[serde(default)]
    login: String,
}

#[derive(Deserialize)]
struct Repository {
    #[serde(default)]
    name: String,
    owner: Login,
}

#[derive(Deserialize, Default)]
struct Timeline {
    #[serde(default)]
    nodes: Vec<Option<Requested>>,
}

#[derive(Deserialize)]
struct Requested {
    #[serde(default, rename = "createdAt")]
    created_at: String,
}

/// The pull requests in `gh`'s printed reply, using default direct-only
/// filtering.
#[cfg(test)]
fn parse(printed: &str) -> Result<Vec<FoundPr>, ReviewFetchError> {
    parse_with_options(printed, &ReviewFilterOptions::default())
}

fn matches_review_filter(
    requests: Option<&ReviewRequests>,
    viewer_login: Option<&str>,
    opted_in_teams: &[String],
) -> bool {
    let Some(requests) = requests else {
        // Older payloads without `reviewRequests` remain accepted.
        return true;
    };
    let reviewers: Vec<&RequestedReviewer> = requests
        .nodes
        .iter()
        .flatten()
        .filter_map(|node| node.requested_reviewer.as_ref())
        .collect();
    if reviewers.is_empty() {
        return true;
    }
    reviewers.iter().any(|reviewer| {
        let is_user = reviewer.typename == "User"
            || (reviewer.typename.is_empty() && reviewer.login.is_some());
        if is_user {
            return match (viewer_login, reviewer.login.as_deref()) {
                (Some(viewer), Some(login)) if !viewer.is_empty() => {
                    login.eq_ignore_ascii_case(viewer)
                }
                _ => true,
            };
        }
        let is_team = reviewer.typename == "Team"
            || (reviewer.typename.is_empty() && reviewer.slug.is_some());
        if is_team {
            return reviewer.slug.as_deref().is_some_and(|slug| {
                opted_in_teams
                    .iter()
                    .any(|opted| opted.eq_ignore_ascii_case(slug))
            });
        }
        false
    })
}

/// The pull requests in `gh`'s printed reply, filtered by direct user requests
/// and any explicitly opted-in teams.
pub fn parse_with_options(
    printed: &str,
    options: &ReviewFilterOptions,
) -> Result<Vec<FoundPr>, ReviewFetchError> {
    let reply: Reply = serde_json::from_str(printed)
        .map_err(|source| ReviewFetchError::Unreadable { source })?;
    let (viewer_login, nodes) = match reply.data {
        Some(data) => (
            options
                .viewer_login
                .as_deref()
                .map(str::to_owned)
                .or_else(|| data.viewer.map(|v| v.login)),
            data.search.nodes,
        ),
        None => (options.viewer_login.clone(), Vec::new()),
    };
    Ok(nodes
        .into_iter()
        .flatten()
        .filter(|node| {
            matches_review_filter(
                node.review_requests.as_ref(),
                viewer_login.as_deref(),
                &options.opted_in_teams,
            )
        })
        .filter_map(|node| {
            let number = node.number?;
            let repository = node.repository?;
            let asked = node
                .timeline
                .nodes
                .iter()
                .flatten()
                .find_map(|event| unix_ms(&event.created_at))
                .or_else(|| unix_ms(&node.created_at))
                .unwrap_or(0);
            Some(FoundPr {
                owner: repository.owner.login,
                name: repository.name,
                number,
                title: node.title,
                author: node.author.map(|author| author.login).unwrap_or_default(),
                url: node.url,
                head_ref: node.head_ref,
                base_ref: node.base_ref,
                requested_unix_ms: asked,
            })
        })
        .collect())
}

/// Unix milliseconds of an RFC 3339 UTC time like `2026-11-17T10:20:30Z`.
fn unix_ms(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let mut day = date.split('-');
    let year: i64 = day.next()?.parse().ok()?;
    let month: i64 = day.next()?.parse().ok()?;
    let of_month: i64 = day.next()?.parse().ok()?;
    let time = time.trim_end_matches('Z');
    let time = time.split(['+', '.']).next()?;
    let mut clock = time.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&of_month) {
        return None;
    }
    // Days from the civil date (proleptic Gregorian).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted = (month + 9) % 12;
    let day_of_year = (153 * shifted + 2) / 5 + of_month - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = r#"{"data":{"search":{"nodes":[
      {"number":483,"title":"Ledger reads behind a flag","url":"https://github.com/acme/api/pull/483",
       "createdAt":"2026-11-10T00:00:00Z","headRefName":"ledger-flag","baseRefName":"main",
       "author":{"login":"dana-k"},"repository":{"name":"api","owner":{"login":"acme"}},
       "timelineItems":{"nodes":[{"createdAt":"2026-11-17T10:20:30Z"}]}},
      {"number":9,"title":"No request event","url":"u","createdAt":"1970-01-02T00:00:00Z",
       "headRefName":"h","baseRefName":"main","author":null,
       "repository":{"name":"web","owner":{"login":"acme"}},"timelineItems":{"nodes":[]}},
      {}
    ]}}}"#;

    #[test]
    fn a_reply_becomes_pull_requests_with_the_request_time() {
        let found = parse(REPLY).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].owner, "acme");
        assert_eq!(found[0].name, "api");
        assert_eq!(found[0].number, 483);
        assert_eq!(found[0].author, "dana-k");
        assert_eq!(found[0].base_ref, "main");
        // 2026-11-17T10:20:30Z
        assert_eq!(found[0].requested_unix_ms, 1_794_910_830_000);
    }

    #[test]
    fn without_a_request_event_the_time_is_when_the_pull_request_opened() {
        let found = parse(REPLY).unwrap();
        assert_eq!(found[1].requested_unix_ms, 86_400_000);
        assert_eq!(found[1].author, "");
    }

    #[test]
    fn text_that_is_not_json_is_unreadable() {
        assert!(matches!(
            parse("not json"),
            Err(ReviewFetchError::Unreadable { .. })
        ));
    }

    #[test]
    fn stderr_tells_rate_limit_signed_out_and_the_rest_apart() {
        assert!(matches!(
            classify("API rate limit exceeded, retry in 30 seconds"),
            ReviewFetchError::RateLimited {
                wait_seconds: Some(30)
            }
        ));
        assert!(matches!(
            classify("To get started with GitHub CLI, please run: gh auth login"),
            ReviewFetchError::SignedOut { .. }
        ));
        assert!(matches!(
            classify("error connecting to api.github.com"),
            ReviewFetchError::Unreachable { .. }
        ));
    }

    #[test]
    fn a_missing_program_is_gh_missing() {
        let gh = GhReviewRequests::with_program("/nonexistent/gh-for-test");
        let ran = run(&gh.program);
        assert!(matches!(ran, Err(ReviewFetchError::GhMissing { .. })));
    }

    const TEAM_AND_DIRECT_REPLY: &str = r#"{"data":{"viewer":{"login":"alice"},"search":{"nodes":[
      {"number":101,"title":"Direct request","url":"https://github.com/acme/api/pull/101",
       "createdAt":"2026-11-10T00:00:00Z","headRefName":"direct","baseRefName":"main",
       "author":{"login":"bob"},"repository":{"name":"api","owner":{"login":"acme"}},
       "reviewRequests":{"nodes":[{"requestedReviewer":{"__typename":"User","login":"alice"}}]},
       "timelineItems":{"nodes":[]}},
      {"number":102,"title":"Team-only request","url":"https://github.com/acme/api/pull/102",
       "createdAt":"2026-11-10T00:00:00Z","headRefName":"team-only","baseRefName":"main",
       "author":{"login":"carol"},"repository":{"name":"api","owner":{"login":"acme"}},
       "reviewRequests":{"nodes":[{"requestedReviewer":{"__typename":"Team","slug":"payments"}}]},
       "timelineItems":{"nodes":[]}},
      {"number":103,"title":"Other user request","url":"https://github.com/acme/api/pull/103",
       "createdAt":"2026-11-10T00:00:00Z","headRefName":"other","baseRefName":"main",
       "author":{"login":"dave"},"repository":{"name":"api","owner":{"login":"acme"}},
       "reviewRequests":{"nodes":[{"requestedReviewer":{"__typename":"User","login":"eve"}}]},
       "timelineItems":{"nodes":[]}}
    ]}}}"#;

    #[test]
    fn direct_review_requests_are_kept_and_team_only_requests_are_ignored_by_default() {
        let found = parse_with_options(TEAM_AND_DIRECT_REPLY, &ReviewFilterOptions::default())
            .unwrap();
        let numbers: Vec<u32> = found.iter().map(|pr| pr.number).collect();
        assert_eq!(numbers, vec![101]);
    }

    #[test]
    fn opted_in_team_review_requests_are_included() {
        let opts = ReviewFilterOptions {
            viewer_login: None,
            opted_in_teams: vec!["payments".to_owned()],
        };
        let found = parse_with_options(TEAM_AND_DIRECT_REPLY, &opts).unwrap();
        let numbers: Vec<u32> = found.iter().map(|pr| pr.number).collect();
        assert_eq!(numbers, vec![101, 102]);
    }
}
