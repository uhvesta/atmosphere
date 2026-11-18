//! The one place the pull request review runs `gh` (D283). It is the host's
//! own `gh`, signed in as the user: Humata never reads, stores or passes a
//! token (no `--with-token`, no `GH_TOKEN`), so what gh can't do is the
//! person's to fix with `gh auth login`. Every failure of gh is a
//! [`GithubError`], and [`status_of`] says it as the API's error with the fix.

use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_grpc::humata::v1 as pb;
use serde_json::Value;
use tonic::{Code, Status};

use crate::session_service::gh_program;
use crate::status;

/// What running `gh` for a pull request went wrong with.
#[derive(Debug, thiserror::Error)]
pub(super) enum GithubError {
    #[error("couldn't run the GitHub CLI `{program}`")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("couldn't hand its input to `gh {command}`")]
    Input {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("couldn't wait for `gh {command}` to finish")]
    Wait {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("`gh {command}` failed ({status}): {stderr}")]
    Failed {
        command: String,
        status: String,
        /// The exit code, if gh exited rather than being killed.
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    #[error("`gh {command}` printed something that isn't JSON")]
    NotJson {
        command: String,
        #[source]
        source: serde_json::Error,
    },
}

impl GithubError {
    fn stderr(&self) -> &str {
        match self {
            Self::Failed { stderr, .. } => stderr,
            _ => "",
        }
    }

    /// gh isn't signed in (it exits 4 and says to `gh auth login`).
    fn signed_out(&self) -> bool {
        match self {
            Self::Failed { code, stderr, .. } => {
                *code == Some(4) || stderr.contains("gh auth login") || stderr.contains("not logged in")
            }
            _ => false,
        }
    }

    fn rate_limited(&self) -> bool {
        self.stderr().to_ascii_lowercase().contains("rate limit")
    }

    /// The repository isn't one the signed-in account can see.
    fn repo_not_visible(&self) -> bool {
        self.stderr().contains("Could not resolve to a Repository")
    }

    /// The repository is visible but has no such pull request.
    fn pull_request_missing(&self) -> bool {
        self.stderr().contains("Could not resolve to a PullRequest")
    }

    /// The HTTP status gh reports, such as 422 for a line GitHub refused.
    pub(super) fn http_status(&self) -> Option<u32> {
        let Self::Failed { stdout, stderr, .. } = self else {
            return None;
        };
        number_after(stderr, "HTTP ").or_else(|| number_after(stdout, "\"status\":\""))
    }
}

/// The digits that follow `marker` in `text`.
fn number_after(text: &str, marker: &str) -> Option<u32> {
    let rest = text.split_once(marker)?.1;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Runs `gh` with `args`, `input` on its standard input if there is any, and
/// returns what it printed. Blocking.
pub(super) fn gh(args: &[&str], input: Option<&[u8]>) -> Result<String, GithubError> {
    let program = gh_program();
    let command = args.join(" ");
    let mut child = Command::new(&program)
        .args(args)
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| GithubError::Spawn {
            program: program.to_string_lossy().into_owned(),
            source,
        })?;
    // Written on a thread of its own, so a long input never waits on gh's
    // output and gh's output never waits on the input.
    let writer = match (input, child.stdin.take()) {
        (Some(bytes), Some(mut stdin)) => {
            let bytes = bytes.to_vec();
            Some(std::thread::spawn(move || stdin.write_all(&bytes)))
        }
        _ => None,
    };
    let output = child
        .wait_with_output()
        .map_err(|source| GithubError::Wait {
            command: command.clone(),
            source,
        })?;
    let wrote = match writer {
        Some(handle) => handle
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("the thread writing gh's input panicked"))),
        None => Ok(()),
    };
    if !output.status.success() {
        return Err(GithubError::Failed {
            command,
            status: output.status.to_string(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    wrote.map_err(|source| GithubError::Input {
        command: command.clone(),
        source,
    })?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Runs `gh` and reads what it printed as JSON. Blocking.
pub(super) fn gh_json(args: &[&str]) -> Result<Value, GithubError> {
    let printed = gh(args, None)?;
    serde_json::from_str(&printed).map_err(|source| GithubError::NotJson {
        command: args.join(" "),
        source,
    })
}

/// When the rate limit that stopped gh resets, in Unix seconds, from `gh api
/// rate_limit`: the soonest reset of a resource that has nothing left, else
/// the overall one.
fn rate_limit_reset() -> Option<i64> {
    let limits = gh_json(&["api", "rate_limit"]).ok()?;
    let spent = limits
        .get("resources")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|resources| resources.values())
        .filter(|resource| resource.get("remaining").and_then(Value::as_i64) == Some(0))
        .filter_map(|resource| resource.get("reset").and_then(Value::as_i64))
        .min();
    spent.or_else(|| limits.pointer("/rate/reset").and_then(Value::as_i64))
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}

/// What a failure of `gh` says to the caller, and how to fix it. `repo` and
/// `number` name the pull request being asked about, when there is one.
pub(super) fn status_of(error: &GithubError, repo: Option<&pb::RepoRef>, number: Option<u32>) -> Status {
    use pb::git_hub_cli_problem::Problem;
    let message = status::chain(error);
    let cli = |problem: Problem, fix: String| {
        status::failure(
            Code::FailedPrecondition,
            status::error(
                message.clone(),
                fix,
                pb::error::Kind::GithubCli(pb::GitHubCliProblem {
                    problem: problem as i32,
                    http_status: error.http_status().unwrap_or(0),
                    ..pb::GitHubCliProblem::default()
                }),
            ),
        )
    };
    let slug = repo.map_or_else(String::new, |repo| format!("{}/{}", repo.owner, repo.name));
    match error {
        GithubError::Spawn { .. } => cli(
            Problem::NotInstalled,
            "install the GitHub CLI on this Mac (`brew install gh`) and sign in with `gh auth login`, then try again; Humata uses gh's own login and never asks for a token".to_owned(),
        ),
        error if error.signed_out() => cli(
            Problem::NotSignedIn,
            "run `gh auth login` on this Mac, then try again; Humata uses gh's own login and never asks for a token, so signing in anywhere else won't help".to_owned(),
        ),
        error if error.rate_limited() => {
            let reset = rate_limit_reset();
            let fix = match reset {
                Some(at) => {
                    let minutes = (at.saturating_sub(now_seconds()).max(0).saturating_add(59) / 60).max(1);
                    format!(
                        "wait about {minutes} minute(s), until GitHub resets the limit, then try again; a different token doesn't help, since the limit is the signed-in account's"
                    )
                }
                None => "wait a few minutes for GitHub to reset the limit, then try again; a different token doesn't help, since the limit is the signed-in account's".to_owned(),
            };
            status::failure(
                Code::ResourceExhausted,
                status::error(
                    message.clone(),
                    fix,
                    pb::error::Kind::RateLimited(pb::RateLimited {
                        service: pb::rate_limited::Service::Github as i32,
                        resets_at: reset.map(|seconds| humata2_grpc::Timestamp { seconds, nanos: 0 }),
                    }),
                ),
            )
        }
        error if error.repo_not_visible() => status::failure(
            Code::NotFound,
            status::error(
                message.clone(),
                format!(
                    "the GitHub account gh is signed in as can't see {}: check the owner and the name; for a private repository sign in as an account that can read it (`gh auth status` shows which one is signed in, `gh auth login` changes it); an organization with single sign-on also needs gh's login authorized for it",
                    if slug.is_empty() { "this repository" } else { slug.as_str() }
                ),
                pb::error::Kind::RepoNotVisible(pb::RepoNotVisible { repo: repo.cloned() }),
            ),
        ),
        error if error.pull_request_missing() => {
            let id = match number {
                Some(number) => format!("{slug}#{number}"),
                None => slug.clone(),
            };
            status::not_found(
                pb::EntityKind::PullRequest,
                &id,
                message.clone(),
                "check the pull request's number and repository in the link; it may have been deleted, or belong to another repository",
            )
        }
        error if error.http_status() == Some(422) => cli(
            Problem::ApiError,
            "GitHub refused the review as it was sent, most often because a drafted line isn't part of the pull request's diff (the lines changed since, or the pull request has new commits): update the clone, drop or re-attach the drafts it names, and submit again; the drafts are kept, nothing was lost".to_owned(),
        ),
        _ => cli(
            Problem::ApiError,
            "read the GitHub CLI's message, fix what it names, then try again; if it names a missing scope, run `gh auth refresh` with that scope".to_owned(),
        ),
    }
}

/// The login `gh` is signed in as. Blocking.
pub(super) fn signed_in_login() -> Result<String, GithubError> {
    Ok(gh(&["api", "user", "--jq", ".login"], None)?.trim().to_owned())
}

/// `POST repos/<owner>/<name>/pulls/<number>/reviews` with `body` (JSON) on
/// standard input; what GitHub answered. Blocking.
pub(super) fn post_review(owner: &str, name: &str, number: u32, body: &[u8]) -> Result<Value, GithubError> {
    let path = format!("repos/{owner}/{name}/pulls/{number}/reviews");
    let args = ["api", "--method", "POST", path.as_str(), "--input", "-"];
    let printed = gh(&args, Some(body))?;
    serde_json::from_str(&printed).map_err(|source| GithubError::NotJson {
        command: args.join(" "),
        source,
    })
}

/// Every review of the pull request, oldest first. Blocking.
pub(super) fn list_reviews(owner: &str, name: &str, number: u32) -> Result<Vec<Value>, GithubError> {
    let path = format!("repos/{owner}/{name}/pulls/{number}/reviews");
    Ok(listed(gh_json(&["api", "--paginate", "--slurp", path.as_str()])?))
}

/// The line comments of one review. Blocking.
pub(super) fn review_comments(owner: &str, name: &str, number: u32, review: u64) -> Result<Vec<Value>, GithubError> {
    let path = format!("repos/{owner}/{name}/pulls/{number}/reviews/{review}/comments");
    Ok(listed(gh_json(&["api", "--paginate", "--slurp", path.as_str()])?))
}

/// The items of a listing, whether gh printed one array or, with `--slurp`,
/// one array per page.
fn listed(printed: Value) -> Vec<Value> {
    match printed {
        Value::Array(items) => items
            .into_iter()
            .flat_map(|item| match item {
                Value::Array(page) => page,
                other => vec![other],
            })
            .collect(),
        _ => Vec::new(),
    }
}
