//! `stack import`: fetches a remote stack (named by PR URL/number, a
//! `gh stack` stack, or a chain of PRs whose bases point at each other) and
//! materializes it in our local format: topics, relatives, order, PR numbers
//! and foreign-branch metadata. Re-importing updates the local stack in place
//! while preserving any interleaved local commits.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::json;
use stack_graph::{Pr as GraphPr, RepoStack};

use crate::cli::ImportArgs;
use crate::config::Config;
use crate::error::Error;
use crate::gh::{Gh, Pr};
use crate::git::Git;
use crate::plan::{Commit, Plan};
use crate::restack;
use crate::state::{Layer, StackState, topic_from_branch};
use crate::trailers::Message;

/// A resolved remote stack ready to fetch and materialize locally.
struct ResolvedStack {
    trunk: String,
    source: &'static str,
    github_stack_number: Option<u64>,
    /// Layers bottom-to-top, each with its parent PR number (None for root).
    entries: Vec<(Pr, Option<u64>)>,
}

/// What was preserved from a prior local branch when re-importing in place.
struct LocalCarry {
    /// Local-only topics (not part of the imported remote stack), in plan order.
    local_topics: Vec<(String, Option<String>, Vec<Commit>)>,
    /// Untagged local commits.
    untagged: Vec<Commit>,
    /// If an imported topic had its `Relative:` rewritten to a local topic,
    /// remember that mapping so interleaving survives re-import.
    rebased_relatives: BTreeMap<String, String>,
}

pub(crate) fn run(args: &ImportArgs, dir: &Path) -> Result<(), Error> {
    let git = Git::new(dir);
    restack::refuse_if_in_progress(&git)?;
    if git.has_uncommitted_changes()? {
        return Err(Error::Usage {
            message: "the working tree has uncommitted changes to tracked files; commit or stash them before importing a stack".to_owned(),
            next: "git status".to_owned(),
        });
    }

    let config = Config::load()?;
    let gh = Gh::new(dir);
    let repo = gh.repo()?;
    let login = gh.login()?;
    let remote = config.remote(&args.target);
    let existing_state = StackState::load(&git)?;

    let resolved = resolve_remote_stack(&gh, &repo.name_with_owner, args.target_ref.as_deref())?;
    let trunk = args
        .target
        .trunk
        .clone()
        .or_else(|| config.trunk.value.clone())
        .unwrap_or_else(|| resolved.trunk.clone());

    // Fetch trunk and every layer branch from the remote.
    let mut refspecs = vec![format!("+refs/heads/{trunk}:refs/remotes/{remote}/{trunk}")];
    for (pr, _) in &resolved.entries {
        refspecs.push(format!(
            "+refs/heads/{}:refs/remotes/{remote}/{}",
            pr.head_ref_name, pr.head_ref_name
        ));
    }
    git.fetch(&remote, &refspecs)?;

    // Build topic names for each imported PR (deduplicated if two branches sanitize identically).
    let mut used_topics = BTreeSet::new();
    let mut pr_to_topic: BTreeMap<u64, String> = BTreeMap::new();
    for (pr, _) in &resolved.entries {
        let base_name = topic_from_branch(&pr.head_ref_name);
        let mut candidate = base_name.clone();
        if !used_topics.insert(candidate.clone()) {
            candidate = format!("{base_name}-{}", pr.number);
            used_topics.insert(candidate.clone());
        }
        pr_to_topic.insert(pr.number, candidate);
    }

    // If re-importing on top of an existing imported stack, capture any local
    // interleaved commits before rewriting the local work branch.
    let carry = capture_local_carry(&git, &remote, &trunk, existing_state.as_ref(), &used_topics)?;

    let trunk_sha = git.rev_parse(&format!("{remote}/{trunk}"))?;
    let mut layers: Vec<Layer> = Vec::with_capacity(resolved.entries.len());
    let mut topic_commits: BTreeMap<String, Vec<Commit>> = BTreeMap::new();

    for (pr, parent_pr) in &resolved.entries {
        let topic = pr_to_topic[&pr.number].clone();
        let default_relative = parent_pr.and_then(|num| pr_to_topic.get(&num).cloned());
        let relative = carry
            .as_ref()
            .and_then(|c| c.rebased_relatives.get(&topic).cloned())
            .or(default_relative);

        let head_ref = format!("{remote}/{}", pr.head_ref_name);
        let base_ref = format!("{remote}/{}", pr.base_ref_name);
        let head_sha = git.rev_parse(&head_ref)?;
        let base_sha = git.merge_base(&head_sha, &base_ref).unwrap_or_else(|_| trunk_sha.clone());
        let commits = git.commits(&base_sha, &head_sha)?;

        // Update local branch ref to match the remote branch so navigation
        // commands (`up`/`down`/`top`/`bottom`) work seamlessly.
        git.update_ref(&format!("refs/heads/{}", pr.head_ref_name), &head_sha)?;

        let foreign = !pr.author.is_empty() && pr.author != login;
        layers.push(Layer {
            topic: topic.clone(),
            relative,
            branch: pr.head_ref_name.clone(),
            base: pr.base_ref_name.clone(),
            pr_number: Some(pr.number),
            pr_url: Some(pr.url.clone()),
            title: Some(pr.title.clone()),
            author: (!pr.author.is_empty()).then(|| pr.author.clone()),
            foreign,
            remote_head: Some(head_sha),
            remote_base: Some(base_sha),
        });
        topic_commits.insert(topic, commits);
    }

    // Materialize the imported layers (and any preserved local interleaved
    // topics) linearly onto the local working branch.
    let work_branch = args
        .branch
        .clone()
        .or_else(|| existing_state.as_ref().and_then(|s| s.work_branch.clone()))
        .unwrap_or_else(|| {
            let root_topic = layers
                .first()
                .map(|l| l.topic.as_str())
                .unwrap_or("remote");
            format!("stack/import/{root_topic}")
        });

    let materialized_head = materialize_branch(
        &git,
        &trunk_sha,
        &layers,
        &topic_commits,
        carry.as_ref(),
    )?;

    git.checkout_branch(&work_branch, Some(&materialized_head))?;

    let state = StackState {
        trunk: Some(trunk.clone()),
        remote: Some(remote.clone()),
        source: Some(resolved.source.to_owned()),
        github_stack_number: resolved.github_stack_number,
        work_branch: Some(work_branch.clone()),
        layers,
    };
    state.save(&git)?;

    if args.json {
        let value = json!({
            "trunk": state.trunk,
            "remote": state.remote,
            "source": state.source,
            "github_stack_number": state.github_stack_number,
            "work_branch": state.work_branch,
            "layers": state.layers,
        });
        let text = serde_json::to_string_pretty(&value).map_err(|source| Error::Json { source })?;
        println!("{text}");
    } else {
        println!(
            "Imported {} layer(s) onto branch `{work_branch}` (trunk `{trunk}`, source {}).",
            state.layers.len(),
            resolved.source
        );
        for layer in &state.layers {
            let on = layer.relative.as_deref().unwrap_or(&trunk);
            let pr = layer
                .pr_number
                .map(|n| format!("#{n}"))
                .unwrap_or_default();
            let foreign = if layer.foreign { " [foreign]" } else { "" };
            println!(
                "  {}  {pr}  {} -> {on}{foreign}",
                layer.topic, layer.branch
            );
        }
        println!("Next: stack status");
    }
    Ok(())
}

/// Resolves a remote stack from a PR URL/number, a `gh stack` stack reference,
/// or a chain of PRs whose bases point at each other.
fn resolve_remote_stack(
    gh: &Gh,
    repo_name: &str,
    target: Option<&str>,
) -> Result<ResolvedStack, Error> {
    let mut open_prs = gh.open_prs()?;
    let native_stacks = gh.native_stack_view()?.unwrap_or_default();

    // Check if target explicitly names a native stack (`stack:<N>` or `gh-stack:<N>`).
    if let Some(raw) = target
        && let Some(stack_num) = parse_stack_ref(raw)
    {
        let native = native_stacks
            .iter()
            .find(|s| s.number == stack_num)
            .ok_or_else(|| Error::Usage {
                message: format!("no native GitHub stack #{stack_num} was found via `gh stack view --json`"),
                next: "stack import <pr-url-or-number>".to_owned(),
            })?;
        return build_from_native(gh, repo_name, native.number, &native.prs, &mut open_prs);
    }

    // If no target was passed, use the first native stack if one exists.
    let target_pr_num = match target {
        Some(raw) => parse_pr_target(raw)?,
        None => {
            if let Some(first) = native_stacks.first() {
                return build_from_native(gh, repo_name, first.number, &first.prs, &mut open_prs);
            }
            return Err(Error::Usage {
                message: "no PR URL, PR number, or `gh stack` stack was given".to_owned(),
                next: "stack import <pr-url-or-number>".to_owned(),
            });
        }
    };

    // Ensure the target PR is in `open_prs` (fetch via `gh pr view` if missing).
    if !open_prs.iter().any(|pr| pr.number == target_pr_num) {
        let pr = gh.pr_view(&target_pr_num.to_string())?;
        open_prs.push(pr);
    }

    // Prefer native GitHub stack if the target PR belongs to one.
    if let Some(native) = native_stacks
        .iter()
        .find(|s| s.prs.contains(&target_pr_num))
        && let Ok(resolved) =
            build_from_native(gh, repo_name, native.number, &native.prs, &mut open_prs)
    {
        return Ok(resolved);
    }

    // Otherwise resolve by PR base chain.
    let graph_prs: Vec<GraphPr> = open_prs.iter().map(to_graph_pr).collect();
    if let Some(repo_stack) = RepoStack::from_base_chain(repo_name, target_pr_num, &graph_prs) {
        let entries = repo_stack
            .entries()
            .iter()
            .filter_map(|node| {
                open_prs
                    .iter()
                    .find(|pr| pr.number == node.pr.number)
                    .cloned()
                    .map(|pr| (pr, node.parent))
            })
            .collect();
        return Ok(ResolvedStack {
            trunk: repo_stack.trunk().to_owned(),
            source: "base_chain",
            github_stack_number: None,
            entries,
        });
    }

    // Single standalone PR (not chained with any other open PR).
    let single = open_prs
        .into_iter()
        .find(|pr| pr.number == target_pr_num)
        .ok_or_else(|| Error::Usage {
            message: format!("pull request #{target_pr_num} could not be found"),
            next: "gh pr list".to_owned(),
        })?;
    let trunk = single.base_ref_name.clone();
    Ok(ResolvedStack {
        trunk,
        source: "pr",
        github_stack_number: None,
        entries: vec![(single, None)],
    })
}

fn build_from_native(
    gh: &Gh,
    repo_name: &str,
    stack_number: u64,
    members: &[u64],
    open_prs: &mut Vec<Pr>,
) -> Result<ResolvedStack, Error> {
    for &num in members {
        if !open_prs.iter().any(|pr| pr.number == num)
            && let Ok(pr) = gh.pr_view(&num.to_string())
        {
            open_prs.push(pr);
        }
    }
    let graph_prs: Vec<GraphPr> = open_prs.iter().map(to_graph_pr).collect();
    let repo_stack = RepoStack::native(repo_name, stack_number, members, &graph_prs).ok_or_else(
        || Error::Usage {
            message: format!("native stack #{stack_number} does not form a valid base chain"),
            next: "stack import <pr-url-or-number>".to_owned(),
        },
    )?;
    let entries = repo_stack
        .entries()
        .iter()
        .filter_map(|node| {
            open_prs
                .iter()
                .find(|pr| pr.number == node.pr.number)
                .cloned()
                .map(|pr| (pr, node.parent))
        })
        .collect();
    Ok(ResolvedStack {
        trunk: repo_stack.trunk().to_owned(),
        source: "native",
        github_stack_number: Some(stack_number),
        entries,
    })
}

fn to_graph_pr(pr: &Pr) -> GraphPr {
    GraphPr {
        number: pr.number,
        head: pr.head_ref_name.clone(),
        base: pr.base_ref_name.clone(),
        title: pr.title.clone(),
        from_fork: false,
    }
}

/// Parses `stack:<N>` or `gh-stack:<N>`.
pub(crate) fn parse_stack_ref(raw: &str) -> Option<u64> {
    let rest = raw
        .strip_prefix("stack:")
        .or_else(|| raw.strip_prefix("gh-stack:"))?;
    rest.trim().parse::<u64>().ok()
}

/// Parses a PR number (`42`, `#42`) or GitHub PR URL (`https://.../pull/42`).
pub(crate) fn parse_pr_target(raw: &str) -> Result<u64, Error> {
    let trimmed = raw.trim().trim_end_matches('/');
    if let Some(num) = trimmed
        .strip_prefix('#')
        .unwrap_or(trimmed)
        .parse::<u64>()
        .ok()
    {
        return Ok(num);
    }
    if let Some((_, tail)) = trimmed.rsplit_once("/pull/")
        && let Some(num_str) = tail.split('/').next()
        && let Ok(num) = num_str.parse::<u64>()
    {
        return Ok(num);
    }
    Err(Error::Usage {
        message: format!(
            "`{raw}` is not a PR URL, PR number, or `stack:<N>` reference"
        ),
        next: "stack import <pr-url-or-number>".to_owned(),
    })
}

/// Captures any local-only (interleaved) commits on the current branch before
/// re-importing a remote stack in place.
fn capture_local_carry(
    git: &Git,
    remote: &str,
    trunk: &str,
    existing: Option<&StackState>,
    new_imported_topics: &BTreeSet<String>,
) -> Result<Option<LocalCarry>, Error> {
    let Some(existing) = existing else {
        return Ok(None);
    };
    let old_imported: BTreeSet<String> = existing.layers.iter().map(|l| l.topic.clone()).collect();
    let Ok(merge_base) = git.merge_base("HEAD", &format!("{remote}/{trunk}")) else {
        return Ok(None);
    };
    let Ok(commits) = git.commits(&merge_base, "HEAD") else {
        return Ok(None);
    };
    let Ok(plan) = Plan::build(commits) else {
        return Ok(None);
    };
    let mut local_topics = Vec::new();
    let mut rebased_relatives = BTreeMap::new();
    for topic in plan.topics {
        if !old_imported.contains(&topic.name) && !new_imported_topics.contains(&topic.name) {
            local_topics.push((topic.name, topic.relative, topic.commits));
        } else if let Some(rel) = topic.relative
            && !old_imported.contains(&rel)
            && !new_imported_topics.contains(&rel)
        {
            rebased_relatives.insert(topic.name, rel);
        }
    }
    if local_topics.is_empty() && plan.untagged.is_empty() && rebased_relatives.is_empty() {
        return Ok(None);
    }
    Ok(Some(LocalCarry {
        local_topics,
        untagged: plan.untagged,
        rebased_relatives,
    }))
}

/// Materializes imported layers and any preserved local topics in topological
/// order onto `trunk_sha`, returning the resulting tip commit sha.
fn materialize_branch(
    git: &Git,
    trunk_sha: &str,
    layers: &[Layer],
    topic_commits: &BTreeMap<String, Vec<Commit>>,
    carry: Option<&LocalCarry>,
) -> Result<String, Error> {
    // Build a unified list of (topic_name, relative, commits, is_imported)
    // and topologically order them so interleaved local topics sit in the right place.
    struct Node {
        name: String,
        relative: Option<String>,
        commits: Vec<Commit>,
        is_imported: bool,
    }
    let mut nodes: Vec<Node> = layers
        .iter()
        .map(|layer| Node {
            name: layer.topic.clone(),
            relative: layer.relative.clone(),
            commits: topic_commits.get(&layer.topic).cloned().unwrap_or_default(),
            is_imported: true,
        })
        .collect();
    if let Some(carry) = carry {
        for (name, relative, commits) in &carry.local_topics {
            nodes.push(Node {
                name: name.clone(),
                relative: relative.clone(),
                commits: commits.clone(),
                is_imported: false,
            });
        }
    }

    let mut ordered: Vec<Node> = Vec::with_capacity(nodes.len());
    let mut placed: BTreeSet<String> = BTreeSet::new();
    while !nodes.is_empty() {
        let idx = nodes
            .iter()
            .position(|node| match node.relative.as_deref() {
                None => true,
                Some(rel) => placed.contains(rel),
            })
            .unwrap_or(0);
        let node = nodes.remove(idx);
        placed.insert(node.name.clone());
        ordered.push(node);
    }

    let mut head = trunk_sha.to_owned();
    for node in &ordered {
        for commit in &node.commits {
            if node.is_imported {
                let msg = with_stack_trailers(&commit.message, &node.name, node.relative.as_deref());
                head = git.copy_onto_with_message(commit, &head, &msg)?;
            } else {
                head = git.copy_onto(commit, &head)?;
            }
        }
    }
    if let Some(carry) = carry {
        for commit in &carry.untagged {
            head = git.copy_onto(commit, &head)?;
        }
    }
    Ok(head)
}

/// Rewrites a commit message to include `Topic: <topic>` and optional
/// `Relative: <relative>` in its trailer paragraph, preserving subject, body
/// and any other trailers.
pub(crate) fn with_stack_trailers(
    message: &Message,
    topic: &str,
    relative: Option<&str>,
) -> String {
    let mut out = message.subject.clone();
    if !message.body.is_empty() {
        out.push_str("\n\n");
        out.push_str(&message.body);
    }
    let mut trailers: Vec<String> = message
        .trailers
        .iter()
        .filter(|t| t.key != "topic" && t.key != "relative")
        .map(|t| format!("{}: {}", capitalize_key(&t.key), t.value))
        .collect();
    trailers.push(format!("Topic: {topic}"));
    if let Some(rel) = relative {
        trailers.push(format!("Relative: {rel}"));
    }
    out.push_str("\n\n");
    out.push_str(&trailers.join("\n"));
    out.push('\n');
    out
}

fn capitalize_key(key: &str) -> String {
    key.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pr_target_accepts_number_hash_and_url() {
        assert_eq!(parse_pr_target("42").ok(), Some(42));
        assert_eq!(parse_pr_target("#42").ok(), Some(42));
        assert_eq!(
            parse_pr_target("https://github.com/o/r/pull/42").ok(),
            Some(42)
        );
        assert_eq!(
            parse_pr_target("https://github.com/o/r/pull/42/files").ok(),
            Some(42)
        );
        assert!(parse_pr_target("not-a-pr").is_err());
    }

    #[test]
    fn parse_stack_ref_accepts_stack_prefix() {
        assert_eq!(parse_stack_ref("stack:7"), Some(7));
        assert_eq!(parse_stack_ref("gh-stack:12"), Some(12));
        assert_eq!(parse_stack_ref("42"), None);
    }

    #[test]
    fn with_stack_trailers_sets_topic_and_relative_cleanly() {
        let msg = Message::parse("Add auth\n\nSome details.\n\nSigned-off-by: Bob\nTopic: old");
        let rendered = with_stack_trailers(&msg, "auth", Some("base"));
        let reparsed = Message::parse(&rendered);
        assert_eq!(reparsed.subject, "Add auth");
        assert_eq!(reparsed.body, "Some details.");
        assert_eq!(reparsed.values("topic").collect::<Vec<_>>(), ["auth"]);
        assert_eq!(reparsed.values("relative").collect::<Vec<_>>(), ["base"]);
        assert_eq!(
            reparsed.values("signed-off-by").collect::<Vec<_>>(),
            ["Bob"]
        );
    }
}
