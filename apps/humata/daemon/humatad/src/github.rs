//! The few parts of GitHub's REST API that pull requests and issues need.
//! Its calls block, so each runs on a thread of its own.

use std::io::Read;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// The API the Mac talks to unless `HUMATA_GITHUB_API` names another.
pub const API: &str = "https://api.github.com";

/// The variables a GitHub token is read from, in order.
pub const TOKENS: [&str; 2] = ["GH_TOKEN", "GITHUB_TOKEN"];

/// Where the GitHub command line is looked for besides `PATH`, which is
/// short for a login agent.
const GH: [&str; 3] = ["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"];

/// How long the GitHub command line's token is used before it's asked for
/// again; one GitHub refuses is asked for again at once.
const TOKEN_KEPT: Duration = Duration::from_secs(5 * 60);

/// How long the GitHub command line not being signed in is believed, so a
/// `gh auth login` soon counts.
const NO_TOKEN_KEPT: Duration = Duration::from_secs(15);

/// How long `gh auth token` may take, as it may wait on the keychain.
const TOKEN_WAIT: Duration = Duration::from_secs(10);

/// The most pages of a list read, 100 each.
const PAGES: usize = 10;

/// The GitHub command line's token as last asked for, kept only in memory.
static CLI_TOKEN: TokenCache = TokenCache::new();

/// The github.com token the GitHub command line signed in with, if it did.
/// It's asked for again after a few minutes, or at once after GitHub
/// refused it, so a new `gh auth login` counts soon; it's kept only in
/// memory.
pub async fn cli_token() -> Option<String> {
    if let Some(kept) = CLI_TOKEN.get(Instant::now()) {
        return kept;
    }
    let token = token_from(&GH.map(std::path::Path::new)).await;
    CLI_TOKEN.keep(Instant::now(), token.clone());
    token
}

/// A token, or that there was none, and when it was asked for.
struct TokenCache(Mutex<Option<(Instant, Option<String>)>>);

impl TokenCache {
    const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// What was kept, unless it's too old at `now`.
    fn get(&self, now: Instant) -> Option<Option<String>> {
        let kept = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let (asked, token) = kept.as_ref()?;
        let lasts = if token.is_some() {
            TOKEN_KEPT
        } else {
            NO_TOKEN_KEPT
        };
        (now.saturating_duration_since(*asked) < lasts).then(|| token.clone())
    }

    fn keep(&self, asked: Instant, token: Option<String>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some((asked, token));
    }

    fn forget(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The token the first of `candidates` that runs and is signed in prints.
async fn token_from(candidates: &[&std::path::Path]) -> Option<String> {
    for gh in candidates {
        let run = tokio::process::Command::new(gh)
            .args(["auth", "token", "--hostname", "github.com"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output();
        let Ok(Ok(output)) = tokio::time::timeout(TOKEN_WAIT, run).await else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let Ok(token) = String::from_utf8(output.stdout) else {
            continue;
        };
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_owned());
        }
    }
    None
}

/// The most of a reply that is read.
const MOST: u64 = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    #[error(
        "there's no GitHub token on this Mac, so Humata can't open or follow pull requests. Sign in to the GitHub command line with `gh auth login`, which Humata uses when it's there, or in Hosts select This Mac and add GH_TOKEN, a token that can read and write pull requests, contents and issues of these repositories. Don't put a token in a project file: it would be committed"
    )]
    NoToken,

    #[error(
        "{url} isn't a GitHub repository, so it can't have pull requests. Push its branch yourself, or set the repository's origin to its GitHub URL with `git remote set-url origin`"
    )]
    NotOnGitHub { url: String },

    #[error(
        "couldn't reach GitHub at {url}; check this Mac's connection. If GitHub is down, Humata tries again in a few minutes"
    )]
    Unreachable {
        url: String,
        #[source]
        source: Box<ureq::Error>,
    },

    #[error("couldn't read GitHub's reply from {url}; try again")]
    Read {
        url: String,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "GitHub refused the token for {url} (it said: {message}). The token may have expired or lack access to this repository. If it's the GitHub command line's, sign in again with `gh auth login`; if it's This Mac's GH_TOKEN, make a new one with read and write access to pull requests, contents and issues, and replace it in Hosts"
    )]
    Unauthorized { url: String, message: String },

    #[error(
        "GitHub's rate limit is used up for this token; wait {wait_minutes} minutes and try again. A new token doesn't help: the limit is per account"
    )]
    RateLimited { wait_minutes: u64 },

    #[error(
        "GitHub has no {url}: the repository or pull request may have moved, or the token can't see it"
    )]
    NotFound { url: String },

    #[error("GitHub refused {what} (status {status}): {message}")]
    Refused {
        what: String,
        status: u16,
        message: String,
    },

    #[error(
        "GitHub's reply from {url} wasn't what Humata expects; try again, and if it persists, report it"
    )]
    Decode {
        url: String,
        #[source]
        source: serde_json::Error,
    },
}

/// A repository on GitHub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

impl std::fmt::Display for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl Repo {
    /// The repository a remote's `url` names. On github.com it is the
    /// owner and name after the host; with another API, standing in for
    /// GitHub, the last two parts of any URL or path.
    pub fn of(url: &str, other_api: bool) -> Option<Self> {
        let url = url.trim().trim_end_matches('/');
        let url = url.strip_suffix(".git").unwrap_or(url);
        let path = if let Some(rest) = url.strip_prefix("git@github.com:") {
            rest
        } else if let Some(rest) = ["https://", "http://", "ssh://git@", "git://"]
            .iter()
            .find_map(|scheme| url.strip_prefix(scheme))
            .and_then(|rest| rest.strip_prefix("github.com/"))
        {
            rest
        } else if other_api {
            url
        } else {
            return None;
        };
        let mut parts = path.rsplit(['/', ':']);
        let name = parts.next().filter(|part| !part.is_empty())?;
        let owner = parts.next().filter(|part| !part.is_empty())?;
        if !other_api && parts.next().is_some() {
            return None;
        }
        Some(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Pull {
    pub number: u64,
    pub html_url: String,
    pub state: String,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub merged_at: Option<String>,
    pub head: Head,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Head {
    pub sha: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct User {
    pub login: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Review {
    pub user: Option<User>,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize)]
struct CheckRuns {
    check_runs: Vec<CheckRun>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CheckRun {
    pub status: String,
    pub conclusion: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReviewComment {
    pub id: u64,
    #[serde(default)]
    pub in_reply_to_id: Option<u64>,
    pub path: String,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub original_line: Option<u32>,
    pub body: String,
    pub user: Option<User>,
    #[serde(default)]
    pub diff_hunk: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    /// Set on a pull request, which the issues list includes.
    #[serde(default)]
    pub pull_request: Option<Value>,
}

/// GitHub's API, as one token.
#[derive(Clone)]
pub struct GitHub {
    api: String,
    token: String,
    http: ureq::Agent,
}

impl GitHub {
    pub fn new(api: &str, token: &str) -> Self {
        let http = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        Self {
            api: api.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
            http,
        }
    }

    pub async fn open_pull(
        &self,
        repo: &Repo,
        title: &str,
        head: &str,
        base: &str,
        body: &str,
    ) -> Result<Pull, GitHubError> {
        let fields = json!({ "title": title, "head": head, "base": base, "body": body });
        self.send(
            "POST",
            &format!("/repos/{repo}/pulls"),
            Some(fields),
            "opening a pull request",
        )
        .await
    }

    pub async fn set_body(
        &self,
        repo: &Repo,
        number: u64,
        body: &str,
    ) -> Result<Pull, GitHubError> {
        let fields = json!({ "body": body });
        self.send(
            "PATCH",
            &format!("/repos/{repo}/pulls/{number}"),
            Some(fields),
            "updating a pull request's description",
        )
        .await
    }

    pub async fn pull(&self, repo: &Repo, number: u64) -> Result<Pull, GitHubError> {
        self.get(&format!("/repos/{repo}/pulls/{number}")).await
    }

    /// The pull request, unless it's as it was when GitHub tagged it `etag`.
    /// A review or review comment changes it too; its checks don't.
    pub async fn pull_since(
        &self,
        repo: &Repo,
        number: u64,
        etag: &str,
    ) -> Result<Fetched<Pull>, GitHubError> {
        let url = format!("{}/repos/{repo}/pulls/{number}", self.api);
        let answer = self.answer(url.clone(), etag).await?;
        let Some(text) = answer.text else {
            return Ok(Fetched::Unchanged);
        };
        Ok(Fetched::Changed {
            value: decode(&url, &text)?,
            etag: answer.etag,
        })
    }

    /// The pull request from `branch`, if one is open.
    pub async fn open_pull_from(
        &self,
        repo: &Repo,
        branch: &str,
    ) -> Result<Option<Pull>, GitHubError> {
        let pulls: Vec<Pull> = self
            .get(&format!(
                "/repos/{repo}/pulls?state=open&head={}:{branch}",
                repo.owner
            ))
            .await?;
        Ok(pulls.into_iter().next())
    }

    pub async fn reviews(&self, repo: &Repo, number: u64) -> Result<Vec<Review>, GitHubError> {
        self.all(&format!(
            "/repos/{repo}/pulls/{number}/reviews?per_page=100"
        ))
        .await
    }

    /// The check runs of commit `sha`, unless they're as they were when
    /// GitHub tagged them `etag`.
    pub async fn checks_since(
        &self,
        repo: &Repo,
        sha: &str,
        etag: &str,
    ) -> Result<Fetched<Vec<CheckRun>>, GitHubError> {
        self.pages(
            &format!("/repos/{repo}/commits/{sha}/check-runs?per_page=100"),
            etag,
            |runs: CheckRuns| runs.check_runs,
        )
        .await
    }

    pub async fn review_comments(
        &self,
        repo: &Repo,
        number: u64,
    ) -> Result<Vec<ReviewComment>, GitHubError> {
        self.all(&format!(
            "/repos/{repo}/pulls/{number}/comments?per_page=100"
        ))
        .await
    }

    /// The open issues, not pull requests, newest first.
    pub async fn issues(&self, repo: &Repo) -> Result<Vec<Issue>, GitHubError> {
        let issues: Vec<Issue> = self
            .all(&format!("/repos/{repo}/issues?state=open&per_page=100"))
            .await?;
        Ok(issues
            .into_iter()
            .filter(|issue| issue.pull_request.is_none())
            .collect())
    }

    async fn get<T: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
    ) -> Result<T, GitHubError> {
        self.send("GET", path, None, "reading").await
    }

    /// Every page of the list at `path`, up to [`PAGES`].
    async fn all<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>, GitHubError> {
        match self.pages(path, "", |page: Vec<T>| page).await? {
            Fetched::Changed { value, .. } => Ok(value),
            // Only a request with an ETag is answered unchanged.
            Fetched::Unchanged => Ok(Vec::new()),
        }
    }

    /// Every page of the list at `path`, following GitHub's `next` links up
    /// to [`PAGES`], each page's items taken by `items`; unchanged if the
    /// first page is as GitHub tagged it `etag`. Its ETag is kept only when
    /// the list fits one page, since it says nothing of the others.
    async fn pages<P, T>(
        &self,
        path: &str,
        etag: &str,
        items: fn(P) -> Vec<T>,
    ) -> Result<Fetched<Vec<T>>, GitHubError>
    where
        P: DeserializeOwned,
    {
        let mut url = format!("{}{path}", self.api);
        let mut all = Vec::new();
        for page in 0..PAGES {
            let answer = self
                .answer(url.clone(), if page == 0 { etag } else { "" })
                .await?;
            let Some(text) = answer.text else {
                return Ok(Fetched::Unchanged);
            };
            all.extend(items(decode(&url, &text)?));
            // The token goes only to the API it's for.
            match answer.next.filter(|next| self.serves(next)) {
                Some(next) => url = next,
                None => {
                    return Ok(Fetched::Changed {
                        value: all,
                        etag: answer.etag.filter(|_| page == 0),
                    });
                }
            }
        }
        Ok(Fetched::Changed {
            value: all,
            etag: None,
        })
    }

    /// Whether `url` is on this API.
    fn serves(&self, url: &str) -> bool {
        url.strip_prefix(&self.api)
            .is_some_and(|rest| rest.starts_with('/'))
    }

    async fn send<T: DeserializeOwned + Send + 'static>(
        &self,
        method: &'static str,
        path: &str,
        body: Option<Value>,
        what: &str,
    ) -> Result<T, GitHubError> {
        let github = self.clone();
        let url = format!("{}{path}", self.api);
        let what = what.to_owned();
        tokio::task::spawn_blocking(move || {
            let answer = github.call(method, &url, body, &what, "")?;
            decode(&url, answer.text.as_deref().unwrap_or_default())
        })
        .await
        .unwrap_or_else(|_| Err(stopped()))
    }

    /// GitHub's answer to reading `url`, sent `If-None-Match: etag` unless
    /// it's empty.
    async fn answer(&self, url: String, etag: &str) -> Result<Answer, GitHubError> {
        let github = self.clone();
        let etag = etag.to_owned();
        tokio::task::spawn_blocking(move || github.call("GET", &url, None, "reading", &etag))
            .await
            .unwrap_or_else(|_| Err(stopped()))
    }

    fn call(
        &self,
        method: &str,
        url: &str,
        body: Option<Value>,
        what: &str,
        etag: &str,
    ) -> Result<Answer, GitHubError> {
        let unreachable = |source| GitHubError::Unreachable {
            url: url.to_owned(),
            source: Box::new(source),
        };
        let authorization = format!("Bearer {}", self.token);
        let response = match (method, body) {
            ("GET", _) => {
                let request = self
                    .http
                    .get(url)
                    .header("Authorization", &authorization)
                    .header("Accept", "application/vnd.github+json")
                    .header("User-Agent", "humata")
                    .header("X-GitHub-Api-Version", "2022-11-28");
                if etag.is_empty() {
                    request.call()
                } else {
                    request.header("If-None-Match", etag).call()
                }
            }
            (method, body) => {
                let request = if method == "PATCH" {
                    self.http.patch(url)
                } else {
                    self.http.post(url)
                };
                request
                    .header("Authorization", &authorization)
                    .header("Accept", "application/vnd.github+json")
                    .header("User-Agent", "humata")
                    .header("X-GitHub-Api-Version", "2022-11-28")
                    .header("Content-Type", "application/json")
                    .send(body.unwrap_or(Value::Null).to_string())
            }
        }
        .map_err(unreachable)?;
        let status = response.status().as_u16();
        let remaining = header(&response, "x-ratelimit-remaining");
        let reset = header(&response, "x-ratelimit-reset");
        let tag = header(&response, "etag");
        let next = header(&response, "link").and_then(|link| next_link(&link));
        if status == 304 {
            return Ok(Answer {
                text: None,
                etag: tag,
                next: None,
            });
        }
        let mut text = String::new();
        response
            .into_body()
            .into_reader()
            .take(MOST)
            .read_to_string(&mut text)
            .map_err(|source| GitHubError::Read {
                url: url.to_owned(),
                source,
            })?;
        match status {
            200..=299 => Ok(Answer {
                text: Some(text),
                etag: tag,
                next,
            }),
            401 => {
                // The GitHub command line's token is asked for again, in
                // case the person signed in since.
                CLI_TOKEN.forget();
                Err(GitHubError::Unauthorized {
                    url: url.to_owned(),
                    message: message(&text),
                })
            }
            403 | 429 if remaining.as_deref() == Some("0") || status == 429 => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or_default();
                let reset = reset
                    .and_then(|reset| reset.parse::<u64>().ok())
                    .unwrap_or(now + 60);
                Err(GitHubError::RateLimited {
                    wait_minutes: reset.saturating_sub(now).div_ceil(60).max(1),
                })
            }
            404 => Err(GitHubError::NotFound {
                url: url.to_owned(),
            }),
            status => Err(GitHubError::Refused {
                what: what.to_owned(),
                status,
                message: message(&text),
            }),
        }
    }
}

/// What reading something again found.
pub enum Fetched<T> {
    /// It changed, or wasn't read before; `etag` tags it, if GitHub did.
    Changed { value: T, etag: Option<String> },
    /// It's as it was.
    Unchanged,
}

/// A reply GitHub gave.
struct Answer {
    /// Its body; none when it said nothing changed.
    text: Option<String>,
    etag: Option<String>,
    /// The next page of a list.
    next: Option<String>,
}

fn decode<T: DeserializeOwned>(url: &str, text: &str) -> Result<T, GitHubError> {
    serde_json::from_str(text).map_err(|source| GitHubError::Decode {
        url: url.to_owned(),
        source,
    })
}

/// A call whose thread ended before it did.
fn stopped() -> GitHubError {
    GitHubError::Refused {
        what: "the request".to_owned(),
        status: 0,
        message: "it stopped part way".to_owned(),
    }
}

/// The `rel="next"` URL of a `Link` header, if there's one.
fn next_link(link: &str) -> Option<String> {
    link.split(',').find_map(|part| {
        let (url, params) = part.split_once(';')?;
        let url = url.trim().strip_prefix('<')?.strip_suffix('>')?;
        params
            .split(';')
            .any(|param| {
                param
                    .trim()
                    .strip_prefix("rel=")
                    .is_some_and(|rel| rel.trim_matches('"').split(' ').any(|rel| rel == "next"))
            })
            .then(|| url.to_owned())
    })
}

fn header(response: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// What GitHub said was wrong, from its error reply.
fn message(text: &str) -> String {
    let said: Option<Value> = serde_json::from_str(text).ok();
    let mut message = said
        .as_ref()
        .and_then(|said| said["message"].as_str())
        .unwrap_or(text)
        .trim()
        .to_owned();
    if let Some(errors) = said.as_ref().and_then(|said| said["errors"].as_array()) {
        for error in errors {
            if let Some(detail) = error["message"].as_str() {
                message.push_str("; ");
                message.push_str(detail);
            }
        }
    }
    message.truncate(1024);
    message
}

/// The last review of each reviewer, which is where they stand.
pub fn standing(reviews: &[Review]) -> Vec<(String, String)> {
    let mut last: Vec<(String, String)> = Vec::new();
    for review in reviews {
        let Some(user) = &review.user else { continue };
        // A comment alone doesn't change where someone stands.
        if review.state == "COMMENTED" || review.state == "PENDING" {
            if !last.iter().any(|(login, _)| *login == user.login) {
                last.push((user.login.clone(), review.state.clone()));
            }
            continue;
        }
        match last.iter_mut().find(|(login, _)| *login == user.login) {
            Some((_, state)) => *state = review.state.clone(),
            None => last.push((user.login.clone(), review.state.clone())),
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for `gh` at `dir/name` that runs `body` as a shell script.
    fn stub_gh(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn the_token_comes_from_the_first_gh_that_runs_and_is_signed_in() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let signed_out = stub_gh(dir.path(), "signed-out", "exit 1");
        let empty = stub_gh(dir.path(), "empty", "exit 0");
        let signed_in = stub_gh(
            dir.path(),
            "signed-in",
            r#"[ "$*" = "auth token --hostname github.com" ] && echo "  gho_stub  " || exit 2"#,
        );
        let other = stub_gh(dir.path(), "other", "echo gho_other");

        assert_eq!(
            token_from(&[&missing, &signed_out, &empty, &signed_in, &other]).await,
            Some("gho_stub".to_owned())
        );
        assert_eq!(token_from(&[&missing, &signed_out, &empty]).await, None);
    }

    #[test]
    fn a_token_is_kept_a_while_and_its_absence_only_briefly() {
        let cache = TokenCache::new();
        let asked = Instant::now();
        assert_eq!(cache.get(asked), None);

        cache.keep(asked, Some("gho_kept".to_owned()));
        assert_eq!(
            cache.get(asked + Duration::from_secs(60)),
            Some(Some("gho_kept".to_owned()))
        );
        assert_eq!(cache.get(asked + TOKEN_KEPT), None);

        cache.keep(asked, None);
        assert_eq!(cache.get(asked + Duration::from_secs(1)), Some(None));
        assert_eq!(cache.get(asked + NO_TOKEN_KEPT), None);

        cache.keep(asked, Some("gho_kept".to_owned()));
        cache.forget();
        assert_eq!(cache.get(asked), None);
    }

    #[test]
    fn a_list_goes_on_at_its_next_link() {
        assert_eq!(
            next_link(
                r#"<https://api.github.com/repositories/1/issues?page=2>; rel="next", <https://api.github.com/repositories/1/issues?page=5>; rel="last""#
            )
            .as_deref(),
            Some("https://api.github.com/repositories/1/issues?page=2")
        );
        assert_eq!(
            next_link(
                r#"<https://api.github.com/repositories/1/issues?page=1>; rel="prev", <https://api.github.com/repositories/1/issues?page=1>; rel="first""#
            ),
            None
        );
        assert_eq!(next_link(""), None);
    }

    #[test]
    fn the_token_is_sent_only_to_its_own_api() {
        let github = GitHub::new("https://api.github.com/", "token");
        assert!(github.serves("https://api.github.com/repositories/1/issues?page=2"));
        assert!(!github.serves("https://api.github.com.evil.test/issues"));
        assert!(!github.serves("https://elsewhere.test/repositories/1/issues"));
    }

    #[test]
    fn a_remote_names_its_repository_on_github_and_any_path_for_another_api() {
        let repo = |owner: &str, name: &str| {
            Some(Repo {
                owner: owner.to_owned(),
                name: name.to_owned(),
            })
        };
        for url in [
            "https://github.com/o/app.git",
            "https://github.com/o/app/",
            "git@github.com:o/app.git",
            "ssh://git@github.com/o/app",
        ] {
            assert_eq!(Repo::of(url, false), repo("o", "app"), "{url}");
        }
        assert_eq!(Repo::of("https://gitlab.com/o/app.git", false), None);
        assert_eq!(Repo::of("/srv/git/o/app.git", false), None);
        assert_eq!(Repo::of("/srv/git/o/app.git", true), repo("o", "app"));
        assert_eq!(Repo::of("app", true), None);
    }

    #[test]
    fn a_reviewer_stands_where_their_last_approval_or_request_put_them() {
        let review = |login: &str, state: &str| Review {
            user: Some(User {
                login: login.to_owned(),
            }),
            state: state.to_owned(),
        };
        let reviews = [
            review("ana", "CHANGES_REQUESTED"),
            review("ben", "COMMENTED"),
            review("ana", "COMMENTED"),
            review("ana", "APPROVED"),
        ];
        assert_eq!(
            standing(&reviews),
            vec![
                ("ana".to_owned(), "APPROVED".to_owned()),
                ("ben".to_owned(), "COMMENTED".to_owned())
            ]
        );
    }
}
