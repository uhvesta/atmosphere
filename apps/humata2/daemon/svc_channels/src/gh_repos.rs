//! SearchGitHubRepos (CH-3): the repos the host's `gh` can see, matched
//! fuzzily on "owner/name" and paged. `gh` keeps the login; this module
//! never reads or logs a token. A listing is reused for a minute, so
//! typing in the search box doesn't ask GitHub again on every key.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::{invalid_field, rfc3339};
use tonic::{Code, Status};

use humata2_rpc_kit::failed;

/// Repos kept from one listing, in `gh`'s order (most recently pushed first).
const MAX_REPOS: usize = 1000;
/// How long a listing is reused before `gh` is asked again.
const CACHE_FOR: Duration = Duration::from_secs(60);
const DEFAULT_PAGE: usize = 50;
const MAX_PAGE: usize = 1000;
const LIST_PATH: &str =
    "user/repos?per_page=100&sort=pushed&affiliation=owner,collaborator,organization_member";
const LIST_JQ: &str = ".[] | {full_name, description, private, default_branch, pushed_at}";

#[derive(Debug, thiserror::Error)]
pub enum GhReposError {
    #[error("couldn't run {program} to list your GitHub repos")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} couldn't list your GitHub repos ({status}): {stderr}")]
    Failed {
        program: String,
        status: String,
        stderr: String,
    },
    #[error("GitHub is limiting how often {program} may list your repos ({status}): {stderr}")]
    RateLimited {
        program: String,
        status: String,
        stderr: String,
    },
    #[error("the GitHub CLI printed a line this Humata can't read: {line}")]
    Parse {
        line: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the GitHub CLI printed a repo with no owner and name: {line}")]
    Name { line: String },
    #[error("the background task that lists your repos didn't finish")]
    Join {
        #[source]
        source: tokio::task::JoinError,
    },
}

impl GhReposError {
    fn code(&self) -> Code {
        match self {
            Self::Spawn { .. } | Self::Failed { .. } => Code::FailedPrecondition,
            Self::RateLimited { .. } => Code::ResourceExhausted,
            Self::Parse { .. } | Self::Name { .. } => Code::Internal,
            Self::Join { .. } => Code::Unavailable,
        }
    }

    fn fix(&self) -> &'static str {
        match self {
            Self::Spawn { .. } => {
                "install the GitHub CLI on this host (brew install gh) so it is on the PATH, then try again"
            }
            Self::Failed { .. } => {
                "run `gh auth login` in a terminal if gh is signed out, then try again"
            }
            Self::RateLimited { .. } => {
                "wait a few minutes for GitHub's limit to lift, then try again; signing in again does not lift it"
            }
            Self::Parse { .. } | Self::Name { .. } => {
                "update the GitHub CLI (brew upgrade gh) and try again"
            }
            Self::Join { .. } => "try again; if it keeps failing, restart humatad2",
        }
    }
}

/// What `gh` ending with a failure means: a rate limit (primary or
/// secondary) is told apart from a signed-out or otherwise broken `gh`.
fn gh_failure(program: String, status: String, stderr: String) -> GhReposError {
    if stderr.to_ascii_lowercase().contains("rate limit") {
        GhReposError::RateLimited {
            program,
            status,
            stderr,
        }
    } else {
        GhReposError::Failed {
            program,
            status,
            stderr,
        }
    }
}

/// One repo as `gh` listed it.
#[derive(Debug)]
pub struct Repo {
    pub owner: String,
    pub name: String,
    pub description: String,
    pub is_private: bool,
    default_branch: String,
    pushed_at: Option<humata2_grpc::Timestamp>,
}

impl Repo {
    fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    fn to_pb(&self) -> pb::GitHubRepo {
        pb::GitHubRepo {
            repo: Some(pb::RepoRef {
                owner: self.owner.clone(),
                name: self.name.clone(),
            }),
            description: self.description.clone(),
            is_private: self.is_private,
            default_branch: self.default_branch.clone(),
            pushed_at: self.pushed_at.clone(),
        }
    }
}

/// The last full listing and when it was read. Only a successful listing
/// is kept.
static LISTING: Mutex<Option<(Instant, Arc<Vec<Repo>>)>> = Mutex::new(None);

/// Answers SearchGitHubRepos: the repos matching `request`'s query, one page
/// of them, most recently pushed first among equal matches.
pub async fn search(
    request: &pb::SearchGitHubReposRequest,
) -> Result<pb::SearchGitHubReposResponse, Status> {
    let repos = listing()
        .await
        .map_err(|error| failed(error.code(), &error, error.fix(), None))?;
    let ranked = rank(&request.query, &repos);
    let (start, size, next_page_token) = window(ranked.len(), request.page.as_ref())?;
    let repos = ranked
        .iter()
        .skip(start)
        .take(size)
        .map(|repo| repo.to_pb())
        .collect();
    Ok(pb::SearchGitHubReposResponse {
        repos,
        page: Some(pb::PageResponse { next_page_token }),
    })
}

/// The full listing: the cached one while it is fresh, else `gh` run on a
/// thread of its own.
pub async fn listing() -> Result<Arc<Vec<Repo>>, GhReposError> {
    if let Some(repos) = cached() {
        return Ok(repos);
    }
    let listed = tokio::task::spawn_blocking(list_with_gh)
        .await
        .map_err(|source| GhReposError::Join { source })?;
    let repos = Arc::new(listed?);
    *LISTING.lock().unwrap_or_else(PoisonError::into_inner) =
        Some((Instant::now(), Arc::clone(&repos)));
    Ok(repos)
}

fn cached() -> Option<Arc<Vec<Repo>>> {
    let listing = LISTING.lock().unwrap_or_else(PoisonError::into_inner);
    listing
        .as_ref()
        .filter(|(at, _)| at.elapsed() < CACHE_FOR)
        .map(|(_, repos)| Arc::clone(repos))
}

pub use humata2_daemon_runtime::tool_path::gh_program;

/// Lists the repos `gh` sees. Blocking: it runs `gh`, so it belongs on a
/// thread of its own.
fn list_with_gh() -> Result<Vec<Repo>, GhReposError> {
    let program = gh_program();
    let name = program.to_string_lossy().into_owned();
    let output = Command::new(&program)
        .args(["api", "--paginate", LIST_PATH, "--jq", LIST_JQ])
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|source| GhReposError::Spawn {
            program: name.clone(),
            source,
        })?;
    if !output.status.success() {
        return Err(gh_failure(
            name,
            output.status.to_string(),
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .take(MAX_REPOS)
        .map(parse_line)
        .collect()
}

/// One line of `gh`'s output: a JSON object with the repo's full name,
/// description, visibility, default branch and last push.
fn parse_line(line: &str) -> Result<Repo, GhReposError> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|source| GhReposError::Parse {
            line: line.to_owned(),
            source,
        })?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let full_name = text("full_name").unwrap_or_default();
    let (owner, name) = full_name
        .split_once('/')
        .filter(|(owner, name)| !owner.is_empty() && !name.is_empty())
        .ok_or_else(|| GhReposError::Name {
            line: line.to_owned(),
        })?;
    Ok(Repo {
        owner: owner.to_owned(),
        name: name.to_owned(),
        description: text("description").unwrap_or_default(),
        is_private: value
            .get("private")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        default_branch: text("default_branch").unwrap_or_default(),
        pushed_at: text("pushed_at").and_then(|at| rfc3339::parse(&at)),
    })
}

/// The repos `query` matches, whole matches first. A repo is a whole match
/// when the lowercased query is a substring of its lowercased "owner/name",
/// and a scattered match when the query's letters appear in it in order.
/// Both tiers keep `repos`' order. An empty query matches everything.
pub fn rank<'a>(query: &str, repos: &'a [Repo]) -> Vec<&'a Repo> {
    let query = query.to_lowercase();
    let mut whole = Vec::new();
    let mut scattered = Vec::new();
    for repo in repos {
        let name = repo.full_name().to_lowercase();
        if name.contains(query.as_str()) {
            whole.push(repo);
        } else if in_order(&query, &name) {
            scattered.push(repo);
        }
    }
    whole.extend(scattered);
    whole
}

/// Whether every character of `query` appears in `text`, in order.
fn in_order(query: &str, text: &str) -> bool {
    let mut rest = text.chars();
    query.chars().all(|wanted| rest.any(|c| c == wanted))
}

/// The part of a `total`-long list that `page` asks for: where it starts,
/// how many it holds, and the token of the page after it ("" on the last).
/// A token is the offset to start at; an empty one starts at the top.
pub fn window(
    total: usize,
    page: Option<&pb::PageRequest>,
) -> Result<(usize, usize, String), Status> {
    let (asked, token) = page.map_or((0, ""), |page| (page.page_size, page.page_token.as_str()));
    let size = match asked {
        0 => DEFAULT_PAGE,
        n => usize::try_from(n).unwrap_or(MAX_PAGE).min(MAX_PAGE),
    };
    let start = if token.is_empty() {
        0
    } else {
        token.parse::<usize>().map_err(|_| bad_token(token))?
    };
    let end = start.saturating_add(size);
    let next = if end < total {
        end.to_string()
    } else {
        String::new()
    };
    Ok((start, size, next))
}

fn bad_token(token: &str) -> Status {
    invalid_field(
        "page_token",
        pb::invalid_field::Problem::BadFormat,
        format!("the page token {token:?} isn't one this list gives out"),
        "send the next_page_token from the last page, or an empty page_token for the first page",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(full_name: &str) -> Repo {
        let (owner, name) = full_name
            .split_once('/')
            .expect("a test repo has an owner and a name");
        Repo {
            owner: owner.to_owned(),
            name: name.to_owned(),
            description: String::new(),
            is_private: false,
            default_branch: "main".to_owned(),
            pushed_at: None,
        }
    }

    fn names(repos: &[&Repo]) -> Vec<String> {
        repos.iter().map(|repo| repo.full_name()).collect()
    }

    fn page(size: u32, token: &str) -> pb::PageRequest {
        pb::PageRequest {
            page_size: size,
            page_token: token.to_owned(),
        }
    }

    #[test]
    fn an_empty_query_keeps_every_repo_in_gh_order() {
        let repos = [repo("acme/ledger"), repo("acme/api"), repo("octo/notes")];
        assert_eq!(
            names(&rank("", &repos)),
            ["acme/ledger", "acme/api", "octo/notes"]
        );
    }

    #[test]
    fn a_whole_match_ranks_before_a_scattered_one_and_case_is_ignored() {
        let repos = [repo("acme/docs"), repo("octo/notes")];
        assert_eq!(names(&rank("ES", &repos)), ["octo/notes", "acme/docs"]);
    }

    #[test]
    fn letters_that_are_out_of_order_or_absent_do_not_match() {
        let repos = [repo("acme/ledger"), repo("acme/api")];
        assert_eq!(names(&rank("LDGR", &repos)), ["acme/ledger"]);
        assert!(rank("gl", &repos).is_empty());
        assert!(rank("zz", &repos).is_empty());
    }

    #[test]
    fn a_missing_or_zero_page_size_is_fifty_and_a_huge_one_is_capped() {
        assert_eq!(window(120, None).ok(), Some((0, 50, "50".to_owned())));
        assert_eq!(
            window(120, Some(&page(0, ""))).ok(),
            Some((0, 50, "50".to_owned()))
        );
        assert_eq!(
            window(5_000, Some(&page(5_000, ""))).ok(),
            Some((0, 1000, "1000".to_owned()))
        );
    }

    #[test]
    fn a_token_is_the_offset_and_the_last_page_has_none() {
        assert_eq!(
            window(7, Some(&page(3, "3"))).ok(),
            Some((3, 3, "6".to_owned()))
        );
        assert_eq!(
            window(7, Some(&page(3, "6"))).ok(),
            Some((6, 3, String::new()))
        );
        assert_eq!(
            window(7, Some(&page(3, "99"))).ok(),
            Some((99, 3, String::new()))
        );
    }

    #[test]
    fn a_token_that_is_not_a_number_is_refused_as_invalid() {
        let refused = window(7, Some(&page(3, "next"))).expect_err("a bad token is refused");
        assert_eq!(refused.code(), Code::InvalidArgument);
    }

    fn gh_ended(stderr: &str) -> GhReposError {
        gh_failure("gh".to_owned(), "exit status: 1".to_owned(), stderr.to_owned())
    }

    #[test]
    fn a_rate_limited_gh_is_a_wait_and_not_a_sign_in() {
        let limited = gh_ended("gh: API rate limit exceeded for user ID 42. (HTTP 403)");
        assert_eq!(limited.code(), Code::ResourceExhausted);
        assert!(!limited.fix().contains("auth login"), "{}", limited.fix());

        let secondary = gh_ended("gh: You have exceeded a secondary rate limit. (HTTP 403)");
        assert_eq!(secondary.code(), Code::ResourceExhausted);
    }

    #[test]
    fn a_signed_out_gh_is_still_told_to_sign_in() {
        let signed_out = gh_ended("To get started with GitHub CLI, please run:  gh auth login");
        assert_eq!(signed_out.code(), Code::FailedPrecondition);
        assert!(signed_out.fix().contains("auth login"), "{}", signed_out.fix());
    }
}
