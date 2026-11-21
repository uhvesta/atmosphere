//! A1, PR-1, PR-3: what a pasted pull request is. The link
//! (`https://github.com/acme/api/pull/482`) or `owner/repo#n` is looked up
//! with the host's `gh pr view --json`: title, author, branches, head, size,
//! state and a summary of its checks.

use std::collections::{BTreeSet, VecDeque};
use std::fmt;

use crate::pb;
use serde_json::Value;
use tonic::Status;

use super::github;
use super::{PullRequestError, PullRequests};
use crate::ports::blocking;

/// The fields `gh pr view` is asked for.
const FIELDS: &str = "number,url,title,author,baseRefName,headRefName,headRefOid,changedFiles,additions,deletions,state,mergedAt,statusCheckRollup";

/// The fields `gh pr view` is asked for when detecting stack metadata.
const STACK_FIELDS: &str = "number,url,title,author,baseRefName,headRefName,headRefOid,changedFiles,additions,deletions,state,mergedAt,statusCheckRollup,body";

/// How a resolved pull request stack was discovered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StackSource {
    /// Native `gh stack` / `gh-stack` metadata block in the PR body.
    NativeGhStack,
    /// Open PRs in the same repository chained by `upper.base_branch == lower.head_branch`.
    BaseChain,
    /// Standalone PR (no connected stack found).
    Single,
}

impl StackSource {
    fn to_pb(self) -> pb::StackSource {
        match self {
            Self::NativeGhStack => pb::StackSource::NativeGhStack,
            Self::BaseChain => pb::StackSource::BaseChain,
            Self::Single => pb::StackSource::Single,
        }
    }
}

/// A resolved pull request stack, ordered bottom-to-top.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ResolvedStack {
    pub(super) layers: Vec<pb::PullRequestInfo>,
    pub(super) source: StackSource,
}

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
        stack_layers: Vec::new(),
        stack_source: pb::StackSource::Unspecified as i32,
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

/// Extracts positive PR numbers from a native `gh stack` / `gh-stack`
/// metadata block in a PR body, preserving order and deduplicating.
fn parse_gh_stack_numbers(body: &str) -> Option<Vec<u32>> {
    let lower = body.to_ascii_lowercase();
    let (marker_pos, inline_mode) = if let Some(pos) = lower.find("<!-- stack-history -->") {
        (pos + "<!-- stack-history -->".len(), false)
    } else if let Some(pos) = lower.find("<!-- gh-stack -->") {
        (pos + "<!-- gh-stack -->".len(), false)
    } else if let Some(pos) = lower.find("<!-- gh-stack:") {
        (pos + "<!-- gh-stack:".len(), true)
    } else {
        return None;
    };

    let rest = &body[marker_pos..];
    let section = if inline_mode {
        let end = rest.find("-->").unwrap_or(rest.len());
        &rest[..end]
    } else {
        let lower_rest = rest.to_ascii_lowercase();
        let end = [
            "<!-- /stack-history -->",
            "<!-- end-stack-history -->",
            "<!-- /gh-stack -->",
        ]
        .iter()
        .filter_map(|closing| lower_rest.find(closing))
        .min()
        .unwrap_or(rest.len());
        &rest[..end]
    };

    let mut numbers = Vec::new();
    let mut push_unique = |num: u32| {
        if num > 0 && !numbers.contains(&num) {
            numbers.push(num);
        }
    };

    if inline_mode {
        for token in section.split(|c: char| !c.is_ascii_digit()) {
            if let Ok(num) = token.parse::<u32>() {
                push_unique(num);
            }
        }
    } else {
        let bytes = section.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'#' {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
                if end > start
                    && let Ok(num) = section[start..end].parse::<u32>()
                {
                    push_unique(num);
                }
                i = end;
            } else if section[i..].starts_with("/pull/") {
                let start = i + "/pull/".len();
                let mut end = start;
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
                if end > start
                    && let Ok(num) = section[start..end].parse::<u32>()
                {
                    push_unique(num);
                }
                i = end;
            } else {
                i += 1;
            }
        }
    }

    (!numbers.is_empty()).then_some(numbers)
}

fn pr_number(info: &pb::PullRequestInfo) -> Option<u32> {
    info.pr.as_ref().map(|pr| pr.number).filter(|n| *n > 0)
}

fn same_repo(a: &pb::PullRequestInfo, b: &pb::PullRequestInfo) -> bool {
    match (
        a.pr.as_ref().and_then(|pr| pr.repo.as_ref()),
        b.pr.as_ref().and_then(|pr| pr.repo.as_ref()),
    ) {
        (Some(ra), Some(rb)) => ra.owner == rb.owner && ra.name == rb.name,
        _ => false,
    }
}

/// Detects a base-chained stack in the same repository containing `target`,
/// returning layers ordered bottom-to-top with cycle protection.
fn detect_base_chain(
    target: &pb::PullRequestInfo,
    repo_prs: &[pb::PullRequestInfo],
) -> Vec<pb::PullRequestInfo> {
    let mut pool: Vec<&pb::PullRequestInfo> = repo_prs
        .iter()
        .filter(|pr| same_repo(target, pr))
        .collect();
    if let Some(target_num) = pr_number(target)
        && !pool.iter().any(|pr| pr_number(pr) == Some(target_num))
    {
        pool.push(target);
    }

    // Walk down from target to the root of the chain, guarding against cycles.
    let mut visited_down = BTreeSet::new();
    let mut root = target;
    if let Some(num) = pr_number(root) {
        visited_down.insert(num);
    }
    while let Some(parent) = pool.iter().copied().find(|candidate| {
        let Some(num) = pr_number(candidate) else {
            return false;
        };
        !visited_down.contains(&num)
            && !candidate.head_branch.is_empty()
            && candidate.head_branch == root.base_branch
    }) {
        if let Some(num) = pr_number(parent) {
            visited_down.insert(num);
        }
        root = parent;
    }

    // Walk up from root in bottom-to-top BFS order, guarding against cycles.
    let mut visited_up = BTreeSet::new();
    let mut queue = VecDeque::new();
    let mut ordered = Vec::new();
    if let Some(num) = pr_number(root) {
        visited_up.insert(num);
    }
    queue.push_back(root);
    while let Some(current) = queue.pop_front() {
        ordered.push(current.clone());
        let mut children: Vec<&pb::PullRequestInfo> = pool
            .iter()
            .copied()
            .filter(|candidate| {
                let Some(num) = pr_number(candidate) else {
                    return false;
                };
                !visited_up.contains(&num)
                    && !current.head_branch.is_empty()
                    && candidate.base_branch == current.head_branch
            })
            .collect();
        children.sort_by_key(|pr| pr_number(pr).unwrap_or(u32::MAX));
        for child in children {
            if let Some(num) = pr_number(child) {
                visited_up.insert(num);
            }
            queue.push_back(child);
        }
    }

    ordered
}

/// Resolves a stack from already-fetched PR data:
/// 1. Native `gh stack` metadata in `target_body` (if >= 2 layers).
/// 2. Base-chained PRs in `open_repo_prs` (if >= 2 layers).
/// 3. Fallback to `StackSource::Single`.
fn resolve_stack_from_candidates(
    target: pb::PullRequestInfo,
    target_body: &str,
    native_layers: Option<Vec<pb::PullRequestInfo>>,
    open_repo_prs: &[pb::PullRequestInfo],
) -> ResolvedStack {
    if let Some(layers) = native_layers
        && layers.len() > 1
    {
        return ResolvedStack {
            layers,
            source: StackSource::NativeGhStack,
        };
    }
    if parse_gh_stack_numbers(target_body).is_some() {
        // Metadata was present but didn't resolve >1 layers; fall through to base chain.
    }
    let chain = detect_base_chain(&target, open_repo_prs);
    if chain.len() > 1 {
        return ResolvedStack {
            layers: chain,
            source: StackSource::BaseChain,
        };
    }
    ResolvedStack {
        layers: vec![target],
        source: StackSource::Single,
    }
}

/// Asks gh for `target` including its `body`, returning `(info, body)`. Blocking.
fn fetch_with_body(target: &PrTarget) -> Result<(pb::PullRequestInfo, String), PullRequestError> {
    let (number, repo) = (target.number.to_string(), target.slug());
    let printed = github::gh_json(&[
        "pr",
        "view",
        number.as_str(),
        "--repo",
        repo.as_str(),
        "--json",
        STACK_FIELDS,
    ])
    .map_err(|source| PullRequestError::Github {
        target: target.clone(),
        source,
    })?;
    let info = info_of(target, &printed)?;
    let body = printed
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok((info, body))
}

/// Lists open pull requests in `target`'s repository. Blocking.
fn fetch_open_repo_prs(target: &PrTarget) -> Result<Vec<pb::PullRequestInfo>, PullRequestError> {
    let repo = target.slug();
    let printed = github::gh_json(&[
        "pr",
        "list",
        "--repo",
        repo.as_str(),
        "--state",
        "open",
        "--limit",
        "100",
        "--json",
        FIELDS,
    ])
    .map_err(|source| PullRequestError::Github {
        target: target.clone(),
        source,
    })?;
    let mut prs = Vec::new();
    for item in printed.as_array().into_iter().flatten() {
        let Some(number) = item
            .get("number")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
        else {
            continue;
        };
        let item_target = PrTarget {
            owner: target.owner.clone(),
            name: target.name.clone(),
            number,
        };
        if let Ok(info) = info_of(&item_target, item) {
            prs.push(info);
        }
    }
    Ok(prs)
}

/// Resolves the stack containing `target` via `gh`:
/// native `gh stack` metadata -> base-chained open PRs -> single PR fallback.
/// Blocking.
pub(super) fn resolve_stack(target: &PrTarget) -> Result<ResolvedStack, PullRequestError> {
    let (target_info, body) = fetch_with_body(target)?;
    let native_layers = if let Some(numbers) = parse_gh_stack_numbers(&body)
        && numbers.len() > 1
        && numbers.contains(&target.number)
    {
        let mut layers = Vec::with_capacity(numbers.len());
        for num in numbers {
            if num == target.number {
                layers.push(target_info.clone());
            } else {
                let layer_target = PrTarget {
                    owner: target.owner.clone(),
                    name: target.name.clone(),
                    number: num,
                };
                layers.push(fetch(&layer_target)?);
            }
        }
        Some(layers)
    } else {
        None
    };

    if let Some(layers) = native_layers {
        return Ok(resolve_stack_from_candidates(
            target_info,
            &body,
            Some(layers),
            &[],
        ));
    }

    let open_prs = fetch_open_repo_prs(target).unwrap_or_default();
    Ok(resolve_stack_from_candidates(
        target_info,
        &body,
        None,
        &open_prs,
    ))
}

fn apply_resolved_stack(target: &PrTarget, resolved: ResolvedStack) -> pb::PullRequestInfo {
    let mut info = resolved
        .layers
        .iter()
        .find(|layer| pr_number(layer) == Some(target.number))
        .cloned()
        .or_else(|| resolved.layers.first().cloned())
        .unwrap_or_default();
    info.stack_source = resolved.source.to_pb() as i32;
    if resolved.layers.len() > 1 {
        info.stack_layers = resolved.layers;
    } else {
        info.stack_layers.clear();
    }
    info
}

fn resolve_with_stack(target: &PrTarget) -> Result<pb::PullRequestInfo, PullRequestError> {
    let resolved = match resolve_stack(target) {
        Ok(resolved) => resolved,
        Err(_) => {
            let single = fetch(target)?;
            ResolvedStack {
                layers: vec![single],
                source: StackSource::Single,
            }
        }
    };
    Ok(apply_resolved_stack(target, resolved))
}

impl PullRequests {
    /// A1: the pull request `request` names, as GitHub shows it now.
    pub(super) async fn resolve(
        &self,
        request: pb::ResolvePullRequestRequest,
    ) -> Result<pb::PullRequestInfo, Status> {
        let target = target_of(request.r#ref.as_ref()).map_err(|error| error.status())?;
        blocking(move || resolve_with_stack(&target).map_err(|error| error.status())).await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr_info(
        owner: &str,
        name: &str,
        number: u32,
        base_branch: &str,
        head_branch: &str,
    ) -> pb::PullRequestInfo {
        pb::PullRequestInfo {
            pr: Some(pb::PullRequestRef {
                repo: Some(pb::RepoRef {
                    owner: owner.to_owned(),
                    name: name.to_owned(),
                }),
                number,
                ..pb::PullRequestRef::default()
            }),
            title: format!("PR #{number}"),
            author: "alice".to_owned(),
            base_branch: base_branch.to_owned(),
            head_branch: head_branch.to_owned(),
            head_sha: format!("sha{number}"),
            files: 1,
            added: 10,
            removed: 2,
            state: pb::PullRequestState::Open as i32,
            checks: String::new(),
            stack_layers: Vec::new(),
            stack_source: pb::StackSource::Unspecified as i32,
        }
    }

    fn layer_numbers(stack: &ResolvedStack) -> Vec<u32> {
        stack
            .layers
            .iter()
            .filter_map(pr_number)
            .collect()
    }

    #[test]
    fn parses_native_gh_stack_history_block() {
        let body = "\
Summary of changes.

<!-- stack-history -->
- #101 Base layer
- **#102** Middle layer (this PR)
- https://github.com/acme/api/pull/103 Top layer
<!-- /stack-history -->
";
        assert_eq!(parse_gh_stack_numbers(body), Some(vec![101, 102, 103]));
    }

    #[test]
    fn parses_native_gh_stack_inline_comment() {
        let body = "Intro\n<!-- gh-stack: 10, 11, 12 -->\nDetails";
        assert_eq!(parse_gh_stack_numbers(body), Some(vec![10, 11, 12]));
    }

    #[test]
    fn native_gh_stack_metadata_produces_native_source() {
        let pr1 = pr_info("acme", "api", 101, "main", "feat-1");
        let pr2 = pr_info("acme", "api", 102, "feat-1", "feat-2");
        let pr3 = pr_info("acme", "api", 103, "feat-2", "feat-3");
        let body = "<!-- stack-history -->\n- #101\n- #102\n- #103\n<!-- /stack-history -->";

        let resolved = resolve_stack_from_candidates(
            pr2.clone(),
            body,
            Some(vec![pr1, pr2, pr3]),
            &[],
        );
        assert_eq!(resolved.source, StackSource::NativeGhStack);
        assert_eq!(layer_numbers(&resolved), vec![101, 102, 103]);
    }

    #[test]
    fn base_chain_detects_stack_from_bottom_middle_or_top() {
        let pr1 = pr_info("acme", "api", 101, "main", "feat-1");
        let pr2 = pr_info("acme", "api", 102, "feat-1", "feat-2");
        let pr3 = pr_info("acme", "api", 103, "feat-2", "feat-3");
        let open = vec![pr1.clone(), pr2.clone(), pr3.clone()];

        for target in [&pr1, &pr2, &pr3] {
            let resolved = resolve_stack_from_candidates(target.clone(), "", None, &open);
            assert_eq!(resolved.source, StackSource::BaseChain);
            assert_eq!(layer_numbers(&resolved), vec![101, 102, 103]);
        }
    }

    #[test]
    fn disconnected_prs_in_same_repo_are_excluded() {
        let pr1 = pr_info("acme", "api", 101, "main", "feat-1");
        let pr2 = pr_info("acme", "api", 102, "feat-1", "feat-2");
        let unrelated = pr_info("acme", "api", 200, "main", "other-work");
        let other_repo = pr_info("acme", "web", 300, "feat-2", "web-child");
        let open = vec![pr1.clone(), pr2.clone(), unrelated, other_repo];

        let resolved = resolve_stack_from_candidates(pr1, "", None, &open);
        assert_eq!(resolved.source, StackSource::BaseChain);
        assert_eq!(layer_numbers(&resolved), vec![101, 102]);
    }

    #[test]
    fn standalone_pr_falls_back_to_single() {
        let standalone = pr_info("acme", "api", 50, "main", "solo");
        let other = pr_info("acme", "api", 51, "main", "other");
        let open = vec![standalone.clone(), other];

        let resolved = resolve_stack_from_candidates(standalone, "", None, &open);
        assert_eq!(resolved.source, StackSource::Single);
        assert_eq!(layer_numbers(&resolved), vec![50]);
    }

    #[test]
    fn base_chain_is_cycle_safe() {
        let a = pr_info("acme", "api", 1, "branch-b", "branch-a");
        let b = pr_info("acme", "api", 2, "branch-a", "branch-b");
        let open = vec![a.clone(), b];

        let resolved = resolve_stack_from_candidates(a, "", None, &open);
        assert_eq!(resolved.source, StackSource::BaseChain);
        assert_eq!(resolved.layers.len(), 2);
    }

    #[test]
    fn apply_resolved_stack_populates_proto_source_and_layers() {
        let target = PrTarget {
            owner: "acme".to_owned(),
            name: "api".to_owned(),
            number: 102,
        };
        let pr1 = pr_info("acme", "api", 101, "main", "feat-1");
        let pr2 = pr_info("acme", "api", 102, "feat-1", "feat-2");
        let pr3 = pr_info("acme", "api", 103, "feat-2", "feat-3");

        let stacked = apply_resolved_stack(
            &target,
            ResolvedStack {
                layers: vec![pr1.clone(), pr2.clone(), pr3.clone()],
                source: StackSource::NativeGhStack,
            },
        );
        assert_eq!(pr_number(&stacked), Some(102));
        assert_eq!(stacked.stack_source, pb::StackSource::NativeGhStack as i32);
        assert_eq!(stacked.stack_layers.len(), 3);
        assert_eq!(
            stacked
                .stack_layers
                .iter()
                .filter_map(pr_number)
                .collect::<Vec<_>>(),
            vec![101, 102, 103]
        );

        let chained = apply_resolved_stack(
            &target,
            ResolvedStack {
                layers: vec![pr1, pr2.clone()],
                source: StackSource::BaseChain,
            },
        );
        assert_eq!(chained.stack_source, pb::StackSource::BaseChain as i32);
        assert_eq!(chained.stack_layers.len(), 2);

        let single = apply_resolved_stack(
            &target,
            ResolvedStack {
                layers: vec![pr2],
                source: StackSource::Single,
            },
        );
        assert_eq!(single.stack_source, pb::StackSource::Single as i32);
        assert!(single.stack_layers.is_empty());
    }
}
