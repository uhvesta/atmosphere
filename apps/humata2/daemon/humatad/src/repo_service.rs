//! RepoService (repos.proto; RL-1 to RL-12, D470 to D479): the host's repo
//! library, the one repo search and the default search scope. On a machine
//! the service is [`OnMachine`], which sends its agents' searches and lists
//! to the host (RL-12, D568).
//!
//! The library is `humata2_repo_library`'s file, `<state>/repos.db`, opened
//! the first time a call needs it, so a file that can't be used fails the
//! calls that need it and never stops the daemon. The default scope is the
//! host's `<home>/config/repos.toml`, read on every search so a hand edit
//! counts at once and written atomically, keeping the user's comments.
//!
//! Who may call: searching, listing and reading the scope are global reads
//! (D278), open to agents as other global reads are. Removing a repo,
//! forgetting the library and saving a scope are the user's alone
//! (`Capability::SettingsChange`, user-only): no capability grants them to
//! an agent (RL-11, D479).
//!
//! GitHub trouble never fails a search (RL-10, D478): the answer carries
//! the library group and a [`pb::RepoSearchProblem`] saying what to do.
//! "Your repos" matches CH-3's listing locally; owners and All of GitHub
//! run one `gh api search/repositories`. An answer from GitHub is reused
//! for a minute for the same query and scope (D477).

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use humata2_access::{Call, Capability, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::StackConfig;
use humata2_repo_library::{
    Entry, GitHubQuery, Hit, Library, LibraryError, NameError, Place, RepoName, Scope, ScopeError,
    github_query, merge,
};
use humata2_store::{StoreError, write_atomic};
use pb::invalid_field::Problem;
use pb::repo_search_problem::Kind as ProblemKind;
use tonic::{Code, Request, Response, Status};

use crate::channel_service::gh_repos::{self, GhReposError};
use crate::channel_service::{blocking, failed, timestamp_ms};
use crate::guard::Guard;
use crate::identity::{self, Caller};
use crate::status;

mod on_machine;
pub(crate) use on_machine::OnMachine;

/// How long an answer from GitHub is reused (D477).
const CACHE_FOR: Duration = Duration::from_secs(60);
/// What one GitHub search asks for: the most it gives in a page.
const SEARCH_PAGE: &str = "per_page=100";
/// The fields of a found repo, one JSON object per line.
const SEARCH_JQ: &str = ".items[] | {full_name, description, private}";

/// What went wrong in a RepoService call, for the person who fixes it.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RepoServiceError {
    #[error("the repo library can't be used")]
    Library {
        #[source]
        source: LibraryError,
    },
    #[error("couldn't read the default search scope from {}", path.display())]
    ReadScope {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the default search scope in {} can't be used", path.display())]
    ScopeFile {
        path: PathBuf,
        #[source]
        source: ScopeError,
    },
    #[error("that scope can't be saved as the default")]
    ScopeRefused {
        #[source]
        source: ScopeError,
    },
    #[error("couldn't make the folder {} for the default search scope", path.display())]
    MakeFolder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't save the default search scope to {}", path.display())]
    WriteScope {
        path: PathBuf,
        #[source]
        source: StoreError,
    },
    #[error("{text:?} isn't a repo")]
    BadRepo {
        text: String,
        #[source]
        source: NameError,
    },
}

impl RepoServiceError {
    fn code(&self) -> Code {
        match self {
            Self::ScopeRefused { .. } | Self::BadRepo { .. } => Code::InvalidArgument,
            Self::Library { .. }
            | Self::ReadScope { .. }
            | Self::ScopeFile { .. }
            | Self::MakeFolder { .. }
            | Self::WriteScope { .. } => Code::FailedPrecondition,
        }
    }

    fn fix(&self) -> &'static str {
        match self {
            Self::Library { .. } => {
                "free some disk space or fix the folder's permissions, then try again; don't delete the repo library file"
            }
            Self::ReadScope { .. } | Self::MakeFolder { .. } | Self::WriteScope { .. } => {
                "fix the permissions of ~/.humata2/config on the host or free some disk space, then try again"
            }
            Self::ScopeFile { .. } => {
                "fix config/repos.toml on the host (or move it aside to go back to Your repos), then search again"
            }
            Self::ScopeRefused { .. } => {
                "save owners (orgs or users) as the default; All of GitHub applies to one search only"
            }
            Self::BadRepo { .. } => "write the repo as owner/name, such as acme/api",
        }
    }
}

impl From<RepoServiceError> for Status {
    fn from(error: RepoServiceError) -> Status {
        match &error {
            RepoServiceError::BadRepo { .. } => status::invalid_field(
                "repo",
                Problem::BadFormat,
                status::chain(&error),
                error.fix(),
            ),
            _ => failed(error.code(), &error, error.fix(), None),
        }
    }
}

/// Why a run of `gh` for the search gave no answer.
#[derive(Debug, thiserror::Error)]
enum GhError {
    #[error("couldn't run {program}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} ended with {status}: {stderr}")]
    Exit {
        program: String,
        status: String,
        stderr: String,
    },
    #[error("the GitHub CLI printed a line this Humata can't read: {line}")]
    Unreadable {
        line: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the GitHub CLI printed a repo with no owner and name: {line}")]
    Name { line: String },
    #[error("the GitHub CLI printed no login for the signed-in user")]
    NoLogin,
    #[error("the background task that runs the GitHub CLI didn't finish")]
    Wait {
        #[source]
        source: Status,
    },
    #[error("the background task that lists your repos didn't finish")]
    ListingWait {
        #[source]
        source: tokio::task::JoinError,
    },
}

impl From<GhReposError> for GhError {
    fn from(error: GhReposError) -> Self {
        match error {
            GhReposError::Spawn { program, source } => GhError::Spawn { program, source },
            // A rate limit keeps its words in `stderr`, which is how it is told apart.
            GhReposError::Failed {
                program,
                status,
                stderr,
            }
            | GhReposError::RateLimited {
                program,
                status,
                stderr,
            } => GhError::Exit {
                program,
                status,
                stderr,
            },
            GhReposError::Parse { line, source } => GhError::Unreadable { line, source },
            GhReposError::Name { line } => GhError::Name { line },
            GhReposError::Join { source } => GhError::ListingWait { source },
        }
    }
}

/// A search's GitHub answer worth reusing.
#[derive(Debug)]
struct Fetched {
    hits: Vec<Hit>,
    unknown_owners: Vec<String>,
}

/// The GitHub group of one search: what it found, or why it didn't.
struct GithubGroup {
    hits: Vec<Hit>,
    unknown_owners: Vec<String>,
    problem: Option<pb::RepoSearchProblem>,
}

impl GithubGroup {
    fn nothing() -> Self {
        Self {
            hits: Vec::new(),
            unknown_owners: Vec::new(),
            problem: None,
        }
    }

    fn problem(problem: pb::RepoSearchProblem) -> Self {
        Self {
            problem: Some(problem),
            ..Self::nothing()
        }
    }

    fn of(fetched: &Fetched) -> Self {
        Self {
            hits: fetched.hits.clone(),
            unknown_owners: fetched.unknown_owners.clone(),
            problem: None,
        }
    }
}

/// What the service and its calls share.
struct Shared {
    /// `<state>/repos.db`.
    db_path: PathBuf,
    /// `<home>/config/repos.toml`.
    scope_path: PathBuf,
    library: OnceLock<Library>,
    guard: Arc<Guard>,
    /// GitHub's answers for a minute, by query and scope.
    cache: Mutex<HashMap<String, (Instant, Arc<Fetched>)>>,
    /// The `gh` login, once known.
    login: Mutex<Option<String>>,
}

/// The repo library, the default scope and the one repo search.
#[derive(Clone)]
pub(crate) struct RepoSvc {
    shared: Arc<Shared>,
}

impl RepoSvc {
    /// `db_path` is `<state>/repos.db` and `home` the humata home (the
    /// folder the state folder is in), whose `config/repos.toml` is the
    /// default scope. Opens nothing yet.
    pub(crate) fn new(db_path: PathBuf, home: &Path, guard: Arc<Guard>) -> Self {
        Self {
            shared: Arc::new(Shared {
                db_path,
                scope_path: StackConfig::host_path(home).with_file_name("repos.toml"),
                library: OnceLock::new(),
                guard,
                cache: Mutex::new(HashMap::new()),
                login: Mutex::new(None),
            }),
        }
    }

    /// The library, for the flows that record a repo when it is added
    /// (RL-2). A clone shares the one file. Opens the file if no call has.
    #[allow(dead_code)]
    pub(crate) fn library(&self) -> Result<Library, RepoServiceError> {
        self.shared.library()
    }
}

impl Shared {
    fn library(&self) -> Result<Library, RepoServiceError> {
        if let Some(library) = self.library.get() {
            return Ok(library.clone());
        }
        let opened =
            Library::open(&self.db_path).map_err(|source| RepoServiceError::Library { source })?;
        // Another thread may have opened it first; either copy will do.
        Ok(self.library.get_or_init(|| opened).clone())
    }

    /// The library and the revision it was read at. Blocking.
    fn list(&self) -> Result<(Vec<Entry>, u64), RepoServiceError> {
        self.library()?
            .list()
            .map_err(|source| RepoServiceError::Library { source })
    }

    /// Anyone but a peer may read, as the rule for global reads says.
    fn may_read(&self, caller: &Caller) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        self.guard.require(
            caller,
            &Call::Use {
                capability: Capability::GlobalRead,
                target: Target::Global,
            },
        )
    }

    fn fresh(&self, key: &str) -> Option<Arc<Fetched>> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache
            .get(key)
            .filter(|(at, _)| at.elapsed() < CACHE_FOR)
            .map(|(_, fetched)| Arc::clone(fetched))
    }

    fn keep(&self, key: String, fetched: Fetched) {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.retain(|_, (at, _)| at.elapsed() < CACHE_FOR);
        cache.insert(key, (Instant::now(), Arc::new(fetched)));
    }

    /// The signed-in user's login, from `gh` once and then from memory.
    async fn login(&self) -> Result<String, GhError> {
        if let Some(login) = self
            .login
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return Ok(login);
        }
        let printed = gh(&["api", "user", "--jq", ".login"]).await?;
        let login = printed.trim().to_owned();
        if login.is_empty() {
            return Err(GhError::NoLogin);
        }
        *self.login.lock().unwrap_or_else(PoisonError::into_inner) = Some(login.clone());
        Ok(login)
    }

    /// The GitHub group of a search of `query` within `scope` (RL-6, RL-9,
    /// RL-10). Never fails: a problem is part of the group.
    async fn github(&self, query: &str, scope: &Scope) -> GithubGroup {
        let key = format!("{}\u{1f}{scope:?}", query.trim());
        if let Some(fetched) = self.fresh(&key) {
            return GithubGroup::of(&fetched);
        }
        // Whether a search goes out at all doesn't depend on who is signed
        // in, so it is settled before `gh` is asked anything.
        match github_query(query, scope, Some("me")) {
            GitHubQuery::Nothing => GithubGroup::nothing(),
            GitHubQuery::TooShort { needs } => GithubGroup::problem(too_short(needs)),
            GitHubQuery::Listing => match self.listing(query).await {
                Ok(hits) => GithubGroup {
                    hits,
                    ..GithubGroup::nothing()
                },
                Err(error) => GithubGroup::problem(problem_of(&error)),
            },
            GitHubQuery::Search(_) => self.search(query, scope, key).await,
        }
    }

    /// Your repos: CH-3's cached listing, matched locally.
    async fn listing(&self, query: &str) -> Result<Vec<Hit>, GhError> {
        let repos = gh_repos::listing().await?;
        Ok(gh_repos::rank(query.trim(), &repos)
            .into_iter()
            .filter_map(|repo| {
                Some(Hit {
                    repo: RepoName::parse(&format!("{}/{}", repo.owner, repo.name)).ok()?,
                    description: repo.description.clone(),
                    is_private: repo.is_private,
                })
            })
            .collect())
    }

    /// One repository search within owners or all of GitHub. An owner that
    /// GitHub doesn't know makes the whole search fail, so on that failure
    /// each owner is asked about and the search runs again without the
    /// unknown ones, which are reported (RL-10).
    async fn search(&self, query: &str, scope: &Scope, key: String) -> GithubGroup {
        let login = match scope {
            Scope::Owners {
                include_me: true, ..
            } => match self.login().await {
                Ok(login) => Some(login),
                Err(error) => return GithubGroup::problem(problem_of(&error)),
            },
            _ => None,
        };
        let mut scope = scope.clone();
        let mut unknown_owners = Vec::new();
        let mut asked_about_owners = false;
        loop {
            let GitHubQuery::Search(q) = github_query(query, &scope, login.as_deref()) else {
                // Every owner was unknown and nobody else is searched.
                return GithubGroup {
                    unknown_owners,
                    ..GithubGroup::nothing()
                };
            };
            let error = match search_with_gh(&q).await {
                Ok(hits) => {
                    self.keep(
                        key,
                        Fetched {
                            hits: hits.clone(),
                            unknown_owners: unknown_owners.clone(),
                        },
                    );
                    return GithubGroup {
                        hits,
                        unknown_owners,
                        problem: None,
                    };
                }
                Err(error) => error,
            };
            let (owners, include_me) = match &scope {
                Scope::Owners { owners, include_me } if !asked_about_owners && names_an_unknown_owner(&error) => {
                    (owners.clone(), *include_me)
                }
                _ => return GithubGroup::problem(problem_of(&error)),
            };
            asked_about_owners = true;
            let missing = unknown_among(&owners).await;
            if missing.is_empty() {
                return GithubGroup::problem(problem_of(&error));
            }
            let kept = owners
                .into_iter()
                .filter(|owner| !missing.iter().any(|m| m.eq_ignore_ascii_case(owner)))
                .collect();
            unknown_owners = missing;
            scope = Scope::Owners {
                owners: kept,
                include_me,
            };
        }
    }
}

/// Runs `gh` with `args` on a thread of its own and returns what it printed.
async fn gh(args: &[&str]) -> Result<String, GhError> {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    match blocking(move || run_gh(&args)).await {
        Ok(done) => done,
        Err(source) => Err(GhError::Wait { source }),
    }
}

/// Blocking: runs the `gh` this host uses (`HUMATA2_GH` in an admin build).
fn run_gh(args: &[String]) -> Result<String, GhError> {
    let program = gh_repos::gh_program();
    let name = program.to_string_lossy().into_owned();
    let output = Command::new(&program)
        .args(args)
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|source| GhError::Spawn {
            program: name.clone(),
            source,
        })?;
    if !output.status.success() {
        return Err(GhError::Exit {
            program: name,
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The repos GitHub's repository search finds for `q`.
async fn search_with_gh(q: &str) -> Result<Vec<Hit>, GhError> {
    let q = format!("q={q}");
    let printed = gh(&[
        "api",
        "-X",
        "GET",
        "search/repositories",
        "-f",
        &q,
        "-f",
        SEARCH_PAGE,
        "--jq",
        SEARCH_JQ,
    ])
    .await?;
    hits_in(&printed)
}

/// The hits in `gh`'s lines of JSON. A repo whose name isn't a GitHub repo
/// name is left out.
fn hits_in(printed: &str) -> Result<Vec<Hit>, GhError> {
    let mut hits = Vec::new();
    for line in printed.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|source| GhError::Unreadable {
                line: line.to_owned(),
                source,
            })?;
        let text = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let full_name = text("full_name");
        if full_name.is_empty() {
            return Err(GhError::Name {
                line: line.to_owned(),
            });
        }
        let Ok(repo) = RepoName::parse(&full_name) else {
            continue;
        };
        hits.push(Hit {
            repo,
            description: text("description"),
            is_private: value
                .get("private")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        });
    }
    Ok(hits)
}

/// Whether a failed search says an owner in it isn't there or can't be
/// searched (GitHub answers 422 for it).
fn names_an_unknown_owner(error: &GhError) -> bool {
    match error {
        GhError::Exit { stderr, .. } => {
            let stderr = stderr.to_ascii_lowercase();
            stderr.contains("422")
                || stderr.contains("validation failed")
                || stderr.contains("cannot be searched")
        }
        _ => false,
    }
}

/// The owners among `owners` that GitHub has no user or org for.
async fn unknown_among(owners: &[String]) -> Vec<String> {
    let mut missing = Vec::new();
    for owner in owners {
        let path = format!("users/{owner}");
        if let Err(GhError::Exit { stderr, .. }) = gh(&["api", &path, "--jq", ".login"]).await {
            let stderr = stderr.to_ascii_lowercase();
            if stderr.contains("404") || stderr.contains("not found") {
                missing.push(owner.clone());
            }
        }
    }
    missing
}

/// The org logins in what `gh api user/orgs --jq .[].login` printed: one
/// per line, none twice (paging can repeat one), blank lines left out.
fn owners_in(printed: &str) -> Vec<String> {
    let mut owners: Vec<String> = Vec::new();
    for line in printed.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if !owners.iter().any(|seen| seen.eq_ignore_ascii_case(line)) {
            owners.push(line.to_owned());
        }
    }
    owners
}

fn problem(kind: ProblemKind, message: String) -> pb::RepoSearchProblem {
    pb::RepoSearchProblem {
        kind: kind as i32,
        message,
        retry_at: None,
    }
}

fn too_short(needs: usize) -> pb::RepoSearchProblem {
    problem(
        ProblemKind::TooShort,
        format!("Type at least {needs} characters to search GitHub."),
    )
}

/// What a failed run of `gh` means for the search, and what to do (RL-10).
fn problem_of(error: &GhError) -> pb::RepoSearchProblem {
    match error {
        GhError::Spawn { program, source } => problem(
            ProblemKind::GhMissing,
            format!(
                "GitHub search needs the GitHub CLI on this host, and Humata couldn't run {program} ({source}). Install it on the host (brew install gh), then search again. If gh already runs in your terminal, put its folder on PATH in your login profile (~/.zprofile): Humata's daemon reads your login shell's PATH when it starts. Your library still shows."
            ),
        ),
        GhError::Exit { stderr, .. } => {
            let lower = stderr.to_ascii_lowercase();
            if lower.contains("rate limit") {
                let wait = wait_seconds(stderr);
                let when = match wait {
                    Some(seconds) => format!("{seconds} seconds"),
                    None => "a few minutes".to_owned(),
                };
                pb::RepoSearchProblem {
                    kind: ProblemKind::RateLimited as i32,
                    message: format!(
                        "GitHub is limiting searches. Wait {when}, then search again; signing in again doesn't lift the limit. Your library still shows."
                    ),
                    retry_at: wait.map(|seconds| {
                        timestamp_ms(
                            now_ms().saturating_add(
                                i64::try_from(seconds).unwrap_or(0).saturating_mul(1000),
                            ),
                        )
                    }),
                }
            } else if signed_out(&lower) {
                problem(
                    ProblemKind::GhSignedOut,
                    "The GitHub CLI on this host is signed out, so GitHub search is off. In a terminal on this host, run `gh auth login`, then search again. Your library still shows.".to_owned(),
                )
            } else {
                unreachable_problem(stderr)
            }
        }
        other => problem(
            ProblemKind::GithubUnreachable,
            format!(
                "GitHub search got an answer this Humata can't read ({}). Update the GitHub CLI on the host (brew upgrade gh), then search again. Your library still shows.",
                status::chain(other)
            ),
        ),
    }
}

fn unreachable_problem(stderr: &str) -> pb::RepoSearchProblem {
    let first = stderr.lines().next().unwrap_or_default();
    problem(
        ProblemKind::GithubUnreachable,
        format!(
            "This host can't reach GitHub right now ({first}). Nothing on this machine fixes it: GitHub search returns when the host is online. Your library still shows."
        ),
    )
}

/// Whether `gh`'s lowercased stderr says it has no usable login.
fn signed_out(lower: &str) -> bool {
    ["auth login", "not logged", "authentication", "http 401", "bad credentials"]
        .iter()
        .any(|sign| lower.contains(sign))
}

/// The wait GitHub names in seconds ("retry in 30 seconds", "after 2
/// minutes"), if its message does.
fn wait_seconds(stderr: &str) -> Option<u64> {
    let words: Vec<&str> = stderr.split_whitespace().collect();
    words.windows(2).find_map(|pair| {
        let count: u64 = pair[0].parse().ok()?;
        let unit = pair[1].trim_end_matches(['.', ',', ')']);
        match unit {
            "second" | "seconds" => Some(count),
            "minute" | "minutes" => Some(count.saturating_mul(60)),
            _ => None,
        }
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Reads the default scope: Your repos when the file isn't there.
fn read_scope(path: &Path) -> Result<Scope, RepoServiceError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Scope::YourRepos),
        Err(source) => {
            return Err(RepoServiceError::ReadScope {
                path: path.to_owned(),
                source,
            });
        }
    };
    Scope::read(&text).map_err(|source| RepoServiceError::ScopeFile {
        path: path.to_owned(),
        source,
    })
}

/// Saves `scope` as the default, keeping the file's other lines, and gives
/// the scope the file now holds. Written beside the file and renamed over
/// it, so a cut never leaves half a file.
fn write_scope(path: &Path, scope: &Scope) -> Result<Scope, RepoServiceError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(RepoServiceError::ReadScope {
                path: path.to_owned(),
                source,
            });
        }
    };
    let next = scope.write_into(&text).map_err(|source| match &source {
        ScopeError::Toml { .. } => RepoServiceError::ScopeFile {
            path: path.to_owned(),
            source,
        },
        _ => RepoServiceError::ScopeRefused { source },
    })?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|source| RepoServiceError::MakeFolder {
            path: folder.to_owned(),
            source,
        })?;
    }
    write_atomic(path, next.as_bytes()).map_err(|source| RepoServiceError::WriteScope {
        path: path.to_owned(),
        source,
    })?;
    Scope::read(&next).map_err(|source| RepoServiceError::ScopeFile {
        path: path.to_owned(),
        source,
    })
}

/// The scope a request names. An owner that isn't a GitHub name is refused.
fn scope_from(scope: &pb::RepoScope) -> Result<Scope, Status> {
    use pb::repo_scope::Kind;
    match &scope.kind {
        None => Err(status::missing("scope.kind")),
        Some(Kind::YourRepos(_)) => Ok(Scope::YourRepos),
        Some(Kind::AllOfGithub(_)) => Ok(Scope::AllOfGitHub),
        Some(Kind::Owners(owners)) => {
            let scope = Scope::Owners {
                owners: owners.owners.clone(),
                include_me: owners.include_me,
            };
            // Writing into an empty file checks every owner's name.
            scope
                .write_into("")
                .map_err(|source| Status::from(RepoServiceError::ScopeRefused { source }))?;
            Ok(scope)
        }
    }
}

fn scope_pb(scope: &Scope) -> pb::RepoScope {
    use pb::repo_scope::Kind;
    let kind = match scope {
        Scope::YourRepos => Kind::YourRepos(true),
        Scope::AllOfGitHub => Kind::AllOfGithub(true),
        Scope::Owners { owners, include_me } => Kind::Owners(pb::RepoOwners {
            owners: owners.clone(),
            include_me: *include_me,
        }),
    };
    pb::RepoScope { kind: Some(kind) }
}

fn repo_ref(repo: &RepoName) -> pb::RepoRef {
    pb::RepoRef {
        owner: repo.owner().to_owned(),
        name: repo.name().to_owned(),
    }
}

fn library_repo(entry: &Entry) -> pb::LibraryRepo {
    let (kind, channel) = match &entry.last_used_in {
        Place::Desk => (pb::RepoPlaceKind::Desk, None),
        Place::Channel(id) => (
            pb::RepoPlaceKind::Channel,
            Some(pb::ChannelId { value: id.clone() }),
        ),
    };
    pb::LibraryRepo {
        repo: Some(repo_ref(&entry.repo)),
        remote: entry.remote.clone(),
        default_branch: entry.default_branch.clone(),
        added_at: Some(timestamp_ms(entry.added_at)),
        last_used_at: Some(timestamp_ms(entry.last_used_at)),
        last_used_kind: kind as i32,
        last_used_channel: channel,
    }
}

fn found_repo(hit: &Hit) -> pb::FoundRepo {
    pb::FoundRepo {
        repo: Some(repo_ref(&hit.repo)),
        description: hit.description.clone(),
        is_private: hit.is_private,
    }
}

fn repo_of(repo: Option<&pb::RepoRef>) -> Result<RepoName, Status> {
    let repo = repo.ok_or_else(|| status::missing("repo"))?;
    let text = format!("{}/{}", repo.owner, repo.name);
    match RepoName::parse(&text) {
        Ok(name) => Ok(name),
        Err(source) => Err(Status::from(RepoServiceError::BadRepo { text, source })),
    }
}

/// Refuses anyone but the user: no capability lets an agent or a script do it.
fn user_only(caller: &Caller, what: &str) -> Result<(), Status> {
    if matches!(caller, Caller::Peer) {
        return Err(status::refused(&Refusal::PeerCannotCall));
    }
    if matches!(caller, Caller::User(_)) {
        return Ok(());
    }
    Err(status::user_only_call(
        Capability::SettingsChange,
        what,
        "ask the user to do it in Settings, under General, Repositories",
    ))
}

fn listed(entries: &[Entry], revision: u64) -> pb::ListRepoLibraryResponse {
    pb::ListRepoLibraryResponse {
        repos: entries.iter().map(library_repo).collect(),
        revision: Some(pb::Revision { value: revision }),
    }
}

#[tonic::async_trait]
impl pb::repo_service_server::RepoService for RepoSvc {
    /// RL-5, RL-6: the library group, then the GitHub group within the
    /// scope, one page of it.
    async fn search_repos(
        &self,
        request: Request<pb::SearchReposRequest>,
    ) -> Result<Response<pb::SearchReposResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.shared.may_read(&caller)?;
        let request = request.into_inner();
        let scope = match &request.scope {
            Some(scope) => scope_from(scope)?,
            None => {
                let path = self.shared.scope_path.clone();
                blocking(move || read_scope(&path)).await??
            }
        };
        let shared = self.shared.clone();
        let (entries, _) = blocking(move || shared.list()).await??;
        let group = self.shared.github(&request.query, &scope).await;
        let found = merge(&request.query, &entries, &group.hits);
        let (start, size, next_page_token) =
            gh_repos::window(found.github.len(), request.page.as_ref())?;
        Ok(Response::new(pb::SearchReposResponse {
            library: found.library.iter().map(library_repo).collect(),
            github: found
                .github
                .iter()
                .skip(start)
                .take(size)
                .map(found_repo)
                .collect(),
            scope: Some(scope_pb(&scope)),
            problem: group.problem,
            unknown_owners: group.unknown_owners,
            page: Some(pb::PageResponse { next_page_token }),
        }))
    }

    /// RL-8: the orgs the host's `gh` user belongs to. A global read. A `gh`
    /// that cannot list them is a problem in the answer (RL-10).
    async fn list_repo_owners(
        &self,
        request: Request<pb::ListRepoOwnersRequest>,
    ) -> Result<Response<pb::ListRepoOwnersResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.shared.may_read(&caller)?;
        let answer = match gh(&["api", "user/orgs", "--paginate", "--jq", ".[].login"]).await {
            Ok(printed) => pb::ListRepoOwnersResponse {
                owners: owners_in(&printed),
                problem: None,
            },
            Err(error) => pb::ListRepoOwnersResponse {
                owners: Vec::new(),
                problem: Some(problem_of(&error)),
            },
        };
        Ok(Response::new(answer))
    }

    /// RL-4: the library, most recently used first.
    async fn list_repo_library(
        &self,
        request: Request<pb::ListRepoLibraryRequest>,
    ) -> Result<Response<pb::ListRepoLibraryResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.shared.may_read(&caller)?;
        let shared = self.shared.clone();
        let (entries, revision) = blocking(move || shared.list()).await??;
        Ok(Response::new(listed(&entries, revision)))
    }

    /// RL-4: takes one repo out of the library. User-only. A repo that
    /// isn't there changes nothing and is no error.
    async fn remove_from_repo_library(
        &self,
        request: Request<pb::RemoveFromRepoLibraryRequest>,
    ) -> Result<Response<pb::ListRepoLibraryResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller, "removing a repo from the library")?;
        let repo = repo_of(request.into_inner().repo.as_ref())?;
        let shared = self.shared.clone();
        let (entries, revision) = blocking(move || -> Result<_, RepoServiceError> {
            shared
                .library()?
                .remove(&repo)
                .map_err(|source| RepoServiceError::Library { source })?;
            shared.list()
        })
        .await??;
        Ok(Response::new(listed(&entries, revision)))
    }

    /// RL-4: empties the library and says how many repos it forgot.
    /// User-only.
    async fn forget_repo_library(
        &self,
        request: Request<pb::ForgetRepoLibraryRequest>,
    ) -> Result<Response<pb::ForgetRepoLibraryResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller, "forgetting the repo library")?;
        let shared = self.shared.clone();
        let forgotten = blocking(move || -> Result<usize, RepoServiceError> {
            shared
                .library()?
                .forget_all()
                .map_err(|source| RepoServiceError::Library { source })
        })
        .await??;
        Ok(Response::new(pb::ForgetRepoLibraryResponse {
            forgotten: u32::try_from(forgotten).unwrap_or(u32::MAX),
        }))
    }

    /// RL-7: the default scope in config/repos.toml.
    async fn get_repo_scope(
        &self,
        request: Request<pb::GetRepoScopeRequest>,
    ) -> Result<Response<pb::RepoScope>, Status> {
        let caller = identity::caller(&request)?;
        self.shared.may_read(&caller)?;
        let path = self.shared.scope_path.clone();
        let scope = blocking(move || read_scope(&path)).await??;
        Ok(Response::new(scope_pb(&scope)))
    }

    /// RL-8 Save as default. User-only. All of GitHub is refused: it widens
    /// one search only.
    async fn update_repo_scope(
        &self,
        request: Request<pb::UpdateRepoScopeRequest>,
    ) -> Result<Response<pb::RepoScope>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller, "saving the default repo search scope")?;
        let request = request.into_inner();
        let scope = scope_from(
            request
                .scope
                .as_ref()
                .ok_or_else(|| status::missing("scope"))?,
        )?;
        let path = self.shared.scope_path.clone();
        let saved = blocking(move || write_scope(&path, &scope)).await??;
        Ok(Response::new(scope_pb(&saved)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit(stderr: &str) -> GhError {
        GhError::Exit {
            program: "gh".to_owned(),
            status: "exit status: 1".to_owned(),
            stderr: stderr.to_owned(),
        }
    }

    fn kind_of(error: &GhError) -> i32 {
        problem_of(error).kind
    }

    #[test]
    fn a_rate_limit_a_signed_out_gh_and_a_network_failure_are_told_apart() {
        assert_eq!(
            kind_of(&exit("gh: API rate limit exceeded for user ID 42. (HTTP 403)")),
            ProblemKind::RateLimited as i32
        );
        assert_eq!(
            kind_of(&exit("To get started with GitHub CLI, please run:  gh auth login")),
            ProblemKind::GhSignedOut as i32
        );
        assert_eq!(
            kind_of(&exit("error connecting to api.github.com")),
            ProblemKind::GithubUnreachable as i32
        );
    }

    #[test]
    fn a_gh_that_cannot_be_run_is_missing() {
        let error = GhError::Spawn {
            program: "gh".to_owned(),
            source: std::io::Error::from(ErrorKind::NotFound),
        };
        assert_eq!(kind_of(&error), ProblemKind::GhMissing as i32);
    }

    #[test]
    fn a_wait_github_names_is_read_in_seconds() {
        assert_eq!(wait_seconds("rate limited, retry in 30 seconds."), Some(30));
        assert_eq!(wait_seconds("try again after 2 minutes"), Some(120));
        assert_eq!(wait_seconds("API rate limit exceeded (HTTP 403)"), None);
    }

    #[test]
    fn a_rate_limit_with_a_named_wait_carries_when_it_lifts() {
        let problem = problem_of(&exit("API rate limit exceeded, retry in 30 seconds"));
        assert!(problem.retry_at.is_some());
        let problem = problem_of(&exit("API rate limit exceeded"));
        assert!(problem.retry_at.is_none());
    }

    #[test]
    fn found_repos_keep_their_description_and_privacy_and_skip_odd_names() {
        let printed = concat!(
            "{\"full_name\":\"acme/ledger\",\"description\":\"The ledger\",\"private\":true}\n",
            "{\"full_name\":\"acme/api\",\"description\":null,\"private\":false}\n",
            "{\"full_name\":\"not a repo\",\"description\":null,\"private\":false}\n",
        );
        let hits = hits_in(printed).expect("the lines are read");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].repo.to_string(), "acme/ledger");
        assert_eq!(hits[0].description, "The ledger");
        assert!(hits[0].is_private);
        assert_eq!(hits[1].description, "");
        assert!(!hits[1].is_private);
    }
}
