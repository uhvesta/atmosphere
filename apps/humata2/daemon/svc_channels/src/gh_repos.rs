//! SearchGitHubRepos (CH-3): the repos the host's `gh` can see, matched
//! fuzzily on "owner/name" and paged. `gh` keeps the login; this module
//! never reads or logs a token. A listing is reused for a minute, so
//! typing in the search box doesn't ask GitHub again on every key.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::pb;
use humata2_rpc_kit::{invalid_field, rfc3339};
use tonic::{Code, Status};

use humata2_rpc_kit::failed;

/// Repos kept from one listing, in `gh`'s order (most recently pushed first).
const MAX_REPOS: usize = 1000;
/// Repos kept per owner (org or user) in the per-owner catalog cache.
const MAX_OWNER_REPOS: usize = 500;
/// How long a listing is reused before `gh` is asked again.
const CACHE_FOR: Duration = Duration::from_secs(60);
/// How long a per-owner repo catalog is reused before `gh` is asked again.
const OWNER_CACHE_FOR: Duration = Duration::from_secs(120);
const DEFAULT_PAGE: usize = 50;
/// Default page size for cached catalogs (`YourRepos` / `Owners`) so clients
/// can filter locally without extra round-trips.
pub const CATALOG_DEFAULT_PAGE: usize = 200;
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
    pub fn is_not_found(&self) -> bool {
        match self {
            Self::Failed { stderr, .. } => {
                let lower = stderr.to_ascii_lowercase();
                lower.contains("404") || lower.contains("not found")
            }
            _ => false,
        }
    }

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
#[derive(Debug, Clone)]
pub struct Repo {
    pub owner: String,
    pub name: String,
    pub description: String,
    pub is_private: bool,
    default_branch: String,
    pushed_at: Option<humata2_grpc_core::Timestamp>,
}

impl Repo {
    fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    fn pushed_at_key(&self) -> Option<(i64, i32)> {
        self.pushed_at.as_ref().map(|ts| (ts.seconds, ts.nanos))
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

/// Cached repo catalog for a single GitHub owner (org or user).
#[derive(Debug, Clone)]
pub struct OwnerCatalog {
    pub repos: Arc<Vec<Repo>>,
    /// True when the owner has more repos than `MAX_OWNER_REPOS`, so a local
    /// filter may miss repos and `search/repositories` should be consulted
    /// for non-short queries.
    pub truncated: bool,
}

/// Per-owner repo catalog cache with TTL eviction.
#[derive(Debug)]
pub struct OwnerCache {
    ttl: Duration,
    entries: Mutex<HashMap<String, (Instant, OwnerCatalog)>>,
}

impl OwnerCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, owner: &str) -> Option<OwnerCatalog> {
        self.get_at(owner, Instant::now())
    }

    pub fn put(&self, owner: &str, catalog: OwnerCatalog) {
        self.put_at(owner, catalog, Instant::now());
    }

    fn get_at(&self, owner: &str, now: Instant) -> Option<OwnerCatalog> {
        let key = owner.to_ascii_lowercase();
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries
            .get(&key)
            .filter(|(at, _)| now.saturating_duration_since(*at) < self.ttl)
            .map(|(_, catalog)| catalog.clone())
    }

    fn put_at(&self, owner: &str, catalog: OwnerCatalog, now: Instant) {
        let key = owner.to_ascii_lowercase();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|_, (at, _)| now.saturating_duration_since(*at) < self.ttl);
        entries.insert(key, (now, catalog));
    }
}

/// The last full listing and when it was read. Only a successful listing
/// is kept.
static LISTING: Mutex<Option<(Instant, Arc<Vec<Repo>>)>> = Mutex::new(None);
/// Per-owner repo catalogs cached for `OWNER_CACHE_FOR`.
static OWNER_CACHE: OnceLock<OwnerCache> = OnceLock::new();

fn owner_cache() -> &'static OwnerCache {
    OWNER_CACHE.get_or_init(|| OwnerCache::new(OWNER_CACHE_FOR))
}

/// Answers SearchGitHubRepos: the repos matching `request`'s query, one page
/// of them, most recently pushed first among equal matches.
pub async fn search(
    request: &pb::SearchGitHubReposRequest,
) -> Result<pb::SearchGitHubReposResponse, Status> {
    let repos = listing()
        .await
        .map_err(|error| failed(error.code(), &error, error.fix(), None))?;
    let ranked = rank(&request.query, &repos);
    let default_page = if request.query.trim().is_empty() {
        CATALOG_DEFAULT_PAGE
    } else {
        DEFAULT_PAGE
    };
    let (start, size, next_page_token) =
        window_with_default(ranked.len(), request.page.as_ref(), default_page)?;
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

/// The repo catalog for one GitHub owner (org or user): cached for 120s,
/// fetched via `orgs/{owner}/repos?per_page=100&sort=pushed` with fallback
/// to `users/{owner}/repos?per_page=100&sort=pushed`.
pub async fn owner_listing(owner: &str) -> Result<OwnerCatalog, GhReposError> {
    if let Some(catalog) = owner_cache().get(owner) {
        return Ok(catalog);
    }
    let owner_owned = owner.to_owned();
    let catalog = tokio::task::spawn_blocking(move || list_owner_with_gh(&owner_owned))
        .await
        .map_err(|source| GhReposError::Join { source })??;
    owner_cache().put(owner, catalog.clone());
    Ok(catalog)
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

/// Lists one owner's repos via `orgs/{owner}/repos`, falling back to
/// `users/{owner}/repos` on 404 (user account rather than org).
fn list_owner_with_gh(owner: &str) -> Result<OwnerCatalog, GhReposError> {
    let org_path = format!("orgs/{owner}/repos?per_page=100&sort=pushed");
    match list_path_with_gh(&org_path, MAX_OWNER_REPOS) {
        Ok(catalog) => Ok(catalog),
        Err(err) if err.is_not_found() => {
            let user_path = format!("users/{owner}/repos?per_page=100&sort=pushed");
            list_path_with_gh(&user_path, MAX_OWNER_REPOS)
        }
        Err(err) => Err(err),
    }
}

/// Runs `gh api --paginate <path> --jq <LIST_JQ>` and reads up to `max_repos`
/// repos. If `gh` emits more than `max_repos` lines, stops reading early and
/// marks `truncated = true` so huge orgs don't stall pagination.
fn list_path_with_gh(path: &str, max_repos: usize) -> Result<OwnerCatalog, GhReposError> {
    let program = gh_program();
    let name = program.to_string_lossy().into_owned();
    let mut child = Command::new(&program)
        .args(["api", "--paginate", path, "--jq", LIST_JQ])
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| GhReposError::Spawn {
            program: name.clone(),
            source,
        })?;

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut buf);
        buf
    });

    let mut repos = Vec::new();
    let mut truncated = false;
    let mut parse_err = None;
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        if repos.len() >= max_repos {
            truncated = true;
            break;
        }
        match parse_line(&line) {
            Ok(repo) => repos.push(repo),
            Err(err) => {
                parse_err = Some(err);
                break;
            }
        }
    }

    if let Some(err) = parse_err {
        let _ = child.kill();
        let _ = child.wait();
        let _ = stderr_thread.join();
        return Err(err);
    }

    if truncated {
        let _ = child.kill();
        let _ = child.wait();
        let _ = stderr_thread.join();
        return Ok(OwnerCatalog {
            repos: Arc::new(repos),
            truncated: true,
        });
    }

    let status = child.wait().map_err(|source| GhReposError::Spawn {
        program: name.clone(),
        source,
    })?;
    let stderr_text = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        return Err(gh_failure(
            name,
            status.to_string(),
            stderr_text.trim().to_owned(),
        ));
    }

    Ok(OwnerCatalog {
        repos: Arc::new(repos),
        truncated: false,
    })
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

/// Match quality tier for [`rank`], ordered from best (lowest) to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchTier {
    /// Exact match on `name` or `owner/name`.
    Exact,
    /// Prefix match on `name`.
    RepoPrefix,
    /// Prefix match on `owner/name`.
    FullNamePrefix,
    /// Match starting immediately after `-`, `_`, or `/`.
    WordBoundary,
    /// Contiguous substring match anywhere in `owner/name`.
    Substring,
    /// Fuzzy subsequence (all characters appear in order).
    FuzzySubsequence,
}

/// The repos `query` matches, ranked by:
/// exact match > repo-name prefix > `owner/name` prefix > word-boundary match
/// (`-`/`_`/`/`) > substring > fuzzy subsequence, tie-breaking by `pushed_at`
/// descending and then original order. An empty query matches everything.
pub fn rank<'a>(query: &str, repos: &'a [Repo]) -> Vec<&'a Repo> {
    let query = query.trim().to_lowercase();
    let mut scored: Vec<(MatchTier, usize, &'a Repo)> = if query.is_empty() {
        repos
            .iter()
            .enumerate()
            .map(|(idx, repo)| (MatchTier::Exact, idx, repo))
            .collect()
    } else {
        repos
            .iter()
            .enumerate()
            .filter_map(|(idx, repo)| {
                let tier = match_tier(&query, repo)?;
                Some((tier, idx, repo))
            })
            .collect()
    };
    scored.sort_by(|(tier_a, idx_a, repo_a), (tier_b, idx_b, repo_b)| {
        tier_a
            .cmp(tier_b)
            .then_with(|| repo_b.pushed_at_key().cmp(&repo_a.pushed_at_key()))
            .then_with(|| idx_a.cmp(idx_b))
    });
    scored.into_iter().map(|(_, _, repo)| repo).collect()
}

fn match_tier(query: &str, repo: &Repo) -> Option<MatchTier> {
    let name = repo.name.to_lowercase();
    let full = repo.full_name().to_lowercase();
    if name == query || full == query {
        Some(MatchTier::Exact)
    } else if name.starts_with(query) {
        Some(MatchTier::RepoPrefix)
    } else if full.starts_with(query) {
        Some(MatchTier::FullNamePrefix)
    } else if has_word_boundary_match(&full, query) {
        Some(MatchTier::WordBoundary)
    } else if full.contains(query) {
        Some(MatchTier::Substring)
    } else if in_order(query, &full) {
        Some(MatchTier::FuzzySubsequence)
    } else {
        None
    }
}

/// Whether `query` appears in `text` immediately after a `-`, `_`, or `/`
/// word boundary separator.
fn has_word_boundary_match(text: &str, query: &str) -> bool {
    for (idx, ch) in text.char_indices() {
        if matches!(ch, '-' | '_' | '/') {
            let next = idx + ch.len_utf8();
            if text[next..].starts_with(query) {
                return true;
            }
        }
    }
    false
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
    window_with_default(total, page, DEFAULT_PAGE)
}

/// Like [`window`], but uses `default_page` when `page` is absent or has
/// `page_size == 0`. Requested sizes are honored up to `MAX_PAGE`.
pub fn window_with_default(
    total: usize,
    page: Option<&pb::PageRequest>,
    default_page: usize,
) -> Result<(usize, usize, String), Status> {
    let (asked, token) = page.map_or((0, ""), |page| (page.page_size, page.page_token.as_str()));
    let size = match asked {
        0 => default_page.min(MAX_PAGE),
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

    fn repo_pushed(full_name: &str, seconds: i64) -> Repo {
        let mut r = repo(full_name);
        r.pushed_at = Some(humata2_grpc_core::Timestamp { seconds, nanos: 0 });
        r
    }

    #[test]
    fn rank_prioritizes_exact_repo_prefix_full_prefix_word_boundary_substring_fuzzy() {
        let repos = [
            repo("org/a-p-i-tools"),   // fuzzy subsequence ("api")
            repo("org/rapid"),         // substring ("api")
            repo("org/my-api-server"), // word boundary after '-' ("api")
            repo("org/my_api_client"), // word boundary after '_' ("api")
            repo("api-team/docs"),     // owner/name prefix ("api")
            repo("other/api-gateway"), // repo-name prefix ("api")
            repo("z/api"),             // exact repo-name match ("api")
        ];
        assert_eq!(
            names(&rank("api", &repos)),
            [
                "z/api",
                "other/api-gateway",
                "api-team/docs",
                "org/my-api-server",
                "org/my_api_client",
                "org/rapid",
                "org/a-p-i-tools",
            ]
        );
    }

    #[test]
    fn rank_exact_full_name_beats_full_name_prefix() {
        let repos = [repo("acme/api-v2"), repo("acme/api")];
        assert_eq!(names(&rank("acme/api", &repos)), ["acme/api", "acme/api-v2"]);
    }

    #[test]
    fn rank_tie_breaks_by_pushed_at_descending_then_original_order() {
        let repos = [
            repo_pushed("acme/api-old", 1_000),
            repo_pushed("acme/api-new", 3_000),
            repo_pushed("acme/api-mid-a", 2_000),
            repo_pushed("acme/api-mid-b", 2_000),
            repo("acme/api-none"),
        ];
        assert_eq!(
            names(&rank("api", &repos)),
            [
                "acme/api-new",
                "acme/api-mid-a",
                "acme/api-mid-b",
                "acme/api-old",
                "acme/api-none",
            ]
        );
    }

    #[test]
    fn owner_cache_is_case_insensitive_and_respects_ttl_and_truncated_flag() {
        let cache = OwnerCache::new(Duration::from_secs(120));
        let t0 = Instant::now();
        let catalog = OwnerCatalog {
            repos: Arc::new(vec![repo("Acme/ledger"), repo("Acme/api")]),
            truncated: true,
        };
        cache.put_at("Acme", catalog, t0);

        let hit = cache
            .get_at("acme", t0 + Duration::from_secs(60))
            .expect("fresh entry hits case-insensitively");
        assert_eq!(hit.repos.len(), 2);
        assert!(hit.truncated);

        assert!(
            cache.get_at("acme", t0 + Duration::from_secs(120)).is_none(),
            "entry expires at TTL"
        );

        // Inserting a new entry evicts expired ones.
        cache.put_at(
            "octo",
            OwnerCatalog {
                repos: Arc::new(vec![repo("octo/notes")]),
                truncated: false,
            },
            t0 + Duration::from_secs(121),
        );
        assert!(cache.get_at("acme", t0 + Duration::from_secs(121)).is_none());
        let octo = cache
            .get_at("OCTO", t0 + Duration::from_secs(121))
            .expect("new entry is fresh");
        assert!(!octo.truncated);
        assert_eq!(octo.repos.len(), 1);
    }

    #[test]
    fn not_found_error_is_detected_for_org_to_user_fallback() {
        assert!(gh_ended("gh: Not Found (HTTP 404)").is_not_found());
        assert!(!gh_ended("gh: API rate limit exceeded (HTTP 403)").is_not_found());
        assert!(!gh_ended("To get started with GitHub CLI, please run:  gh auth login").is_not_found());
    }

    #[test]
    fn window_with_default_uses_catalog_default_and_honors_large_page_size() {
        assert_eq!(
            window_with_default(500, None, CATALOG_DEFAULT_PAGE).ok(),
            Some((0, 200, "200".to_owned()))
        );
        assert_eq!(
            window_with_default(600, Some(&page(500, "")), CATALOG_DEFAULT_PAGE).ok(),
            Some((0, 500, "500".to_owned()))
        );
    }
}
