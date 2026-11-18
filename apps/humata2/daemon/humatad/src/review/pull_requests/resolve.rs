//! A1, PR-1, PR-3: what a pasted pull request is. The link
//! (`https://github.com/acme/api/pull/482`) or `owner/repo#n` is looked up
//! with the host's `gh pr view --json`: title, author, branches, head, size,
//! state and a summary of its checks.

use std::fmt;

use humata2_grpc::humata::v1 as pb;
use serde_json::Value;
use tonic::Status;

use super::github;
use super::{PullRequestError, PullRequests};
use crate::channel_service::blocking;

/// The fields `gh pr view` is asked for.
const FIELDS: &str = "number,url,title,author,baseRefName,headRefName,headRefOid,changedFiles,additions,deletions,state,mergedAt,statusCheckRollup";

/// One pull request, named by repository and number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PrTarget {
    pub(super) owner: String,
    pub(super) name: String,
    pub(super) number: u32,
}

impl fmt::Display for PrTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}#{}", self.owner, self.name, self.number)
    }
}

impl PrTarget {
    pub(super) fn repo(&self) -> pb::RepoRef {
        pb::RepoRef {
            owner: self.owner.clone(),
            name: self.name.clone(),
        }
    }

    fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    fn reference(&self) -> pb::PullRequestRef {
        pb::PullRequestRef {
            repo: Some(self.repo()),
            number: self.number,
            ..pb::PullRequestRef::default()
        }
    }
}

/// A repository owner or name as GitHub allows them.
fn plain(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// `https://github.com/acme/api/pull/482`, with or without the scheme,
/// `www.`, and anything after the number (`/files`, `?x`, `#y`).
fn from_url(given: &str) -> Option<PrTarget> {
    let text = given.trim();
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .unwrap_or(text);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let mut parts = rest.strip_prefix("github.com/")?.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if parts.next()? != "pull" {
        return None;
    }
    let digits: String = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let number: u32 = digits.parse().ok().filter(|number| *number > 0)?;
    (plain(owner) && plain(name)).then(|| PrTarget {
        owner: owner.to_owned(),
        name: name.to_owned(),
        number,
    })
}

fn target_of(
    reference: Option<&pb::resolve_pull_request_request::Ref>,
) -> Result<PrTarget, PullRequestError> {
    use pb::resolve_pull_request_request::Ref;
    match reference {
        None => Err(PullRequestError::Missing { field: "ref" }),
        Some(Ref::Url(given)) => from_url(given).ok_or_else(|| PullRequestError::Reference {
            given: given.clone(),
        }),
        Some(Ref::Pr(pr)) => {
            let repo = pr.repo.clone().unwrap_or_default();
            if plain(&repo.owner) && plain(&repo.name) && pr.number > 0 {
                Ok(PrTarget {
                    owner: repo.owner,
                    name: repo.name,
                    number: pr.number,
                })
            } else {
                Err(PullRequestError::Reference {
                    given: format!("{}/{}#{}", repo.owner, repo.name, pr.number),
                })
            }
        }
    }
}

/// "3 passed, 1 failing, 2 running" from gh's `statusCheckRollup`; empty
/// when the pull request has no checks.
fn checks_summary(rollup: Option<&Value>) -> String {
    let (mut passed, mut failing, mut running) = (0_u32, 0_u32, 0_u32);
    for check in rollup.and_then(Value::as_array).into_iter().flatten() {
        // A check run has a conclusion once it is done; a status context
        // has only a state.
        let outcome = ["conclusion", "state", "status"]
            .iter()
            .filter_map(|key| check.get(*key).and_then(Value::as_str))
            .find(|value| !value.is_empty())
            .unwrap_or_default()
            .to_ascii_uppercase();
        match outcome.as_str() {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => passed += 1,
            "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => {
                failing += 1;
            }
            _ => running += 1,
        }
    }
    [(passed, "passed"), (failing, "failing"), (running, "running")]
        .iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, what)| format!("{count} {what}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The pull request as `printed`, gh's JSON for `target`, shows it.
fn info_of(target: &PrTarget, printed: &Value) -> Result<pb::PullRequestInfo, PullRequestError> {
    let text = |field: &'static str| {
        printed
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| PullRequestError::Answer {
                target: target.clone(),
                field,
            })
    };
    let count = |field: &str| {
        printed
            .get(field)
            .and_then(Value::as_u64)
            .map_or(0, |count| u32::try_from(count).unwrap_or(u32::MAX))
    };
    let merged = printed.get("mergedAt").is_some_and(|merged| !merged.is_null());
    let state = match (printed.get("state").and_then(Value::as_str), merged) {
        (_, true) | (Some("MERGED"), _) => pb::PullRequestState::Merged,
        (Some("OPEN"), _) => pb::PullRequestState::Open,
        (Some("CLOSED"), _) => pb::PullRequestState::Closed,
        _ => pb::PullRequestState::Unspecified,
    };
    Ok(pb::PullRequestInfo {
        pr: Some(target.reference()),
        title: text("title")?,
        author: printed
            .pointer("/author/login")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        base_branch: text("baseRefName")?,
        head_branch: text("headRefName")?,
        head_sha: text("headRefOid")?,
        files: count("changedFiles"),
        added: count("additions"),
        removed: count("deletions"),
        state: state as i32,
        checks: checks_summary(printed.get("statusCheckRollup")),
    })
}

/// Asks gh for `target`. Blocking.
fn fetch(target: &PrTarget) -> Result<pb::PullRequestInfo, PullRequestError> {
    let (number, repo) = (target.number.to_string(), target.slug());
    let printed = github::gh_json(&[
        "pr",
        "view",
        number.as_str(),
        "--repo",
        repo.as_str(),
        "--json",
        FIELDS,
    ])
    .map_err(|source| PullRequestError::Github {
        target: target.clone(),
        source,
    })?;
    info_of(target, &printed)
}

impl PullRequests {
    /// A1: the pull request `request` names, as GitHub shows it now.
    pub(super) async fn resolve(
        &self,
        request: pb::ResolvePullRequestRequest,
    ) -> Result<pb::PullRequestInfo, Status> {
        let target = target_of(request.r#ref.as_ref()).map_err(|error| error.status())?;
        blocking(move || fetch(&target).map_err(|error| error.status())).await?
    }
}
