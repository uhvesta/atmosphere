//! `stack status`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::json;

use crate::cli::StatusArgs;
use crate::config::Config;
use crate::error::Error;
use crate::gh::{Gh, Pr, PrState};
use crate::git::Git;
use crate::local::{self, Branches};
use crate::restack;

/// One topic's status: its branch, the base its pull request targets, and
/// whether its branch on the remote is behind the rebuilt head.
struct Row<'a> {
    topic: &'a str,
    /// The topic it builds on, when that one is still open; None on the trunk.
    parent: Option<&'a str>,
    /// How many topics down from the trunk: 0 on the trunk.
    depth: usize,
    branch: String,
    base: String,
    pr: Option<&'a Pr>,
    needs_upload: bool,
}

pub(crate) fn run(args: &StatusArgs, dir: &Path) -> Result<(), Error> {
    let config = Config::load()?;
    let git = Git::new(dir);
    let in_progress = restack::in_progress(&git)?;
    let gh = Gh::new(dir);
    let repo = gh.repo()?;
    let login = gh.login()?;
    let prefix = config.prefix(&git, None, &login)?;
    let trunk = config.trunk(&args.target, repo.default_branch);
    let remote = config.remote(&args.target);
    let remote = remote.as_str();
    git.fetch(remote, &[format!("+refs/heads/{trunk}:refs/remotes/{remote}/{trunk}")])?;
    let (merge_base, plan) = local::read(&git, remote, &trunk)?;

    let mine = gh.my_open_prs()?;
    let branches = Branches::new(prefix, mine.clone());
    let mut prs: BTreeMap<&str, Pr> = BTreeMap::new();
    for topic in &plan.topics {
        let found = match branches.open_pr(&topic.name) {
            Some(pr) => Some(pr.clone()),
            None => gh.pr_for_head(&branches.branch(&topic.name))?,
        };
        if let Some(pr) = found {
            prs.insert(topic.name.as_str(), pr);
        }
    }
    let merged: BTreeSet<String> = prs
        .iter()
        .filter(|(_, pr)| pr.state == PrState::Merged)
        .map(|(name, _)| name.to_string())
        .collect();
    let tips = local::rebuild(&git, &plan, &merge_base, &merged)?;
    let heads = local::remote_heads(&git, remote, &branches.prefixes())?;

    let mut depths: BTreeMap<&str, usize> = BTreeMap::new();
    let rows: Vec<Row<'_>> = plan
        .topics
        .iter()
        .map(|topic| {
            let name = topic.name.as_str();
            let branch = branches.branch(name);
            let parent = topic.relative.as_deref().filter(|relative| !merged.contains(*relative));
            let depth = parent.map_or(0, |parent| depths.get(parent).map_or(1, |depth| depth + 1));
            depths.insert(name, depth);
            let base = match parent {
                Some(relative) => branches.branch(relative),
                None => trunk.clone(),
            };
            let needs_upload = !merged.contains(name)
                && match tips.get(name) {
                    Some(tip) => heads.get(&branch) != Some(&tip.head),
                    None => true,
                };
            Row {
                topic: name,
                parent,
                depth,
                branch,
                base,
                pr: prs.get(name),
                needs_upload,
            }
        })
        .collect();

    let names: BTreeSet<&str> = plan.topics.iter().map(|topic| topic.name.as_str()).collect();
    let orphaned: Vec<Pr> = mine
        .into_iter()
        .filter(|pr| local::split_branch(&pr.head_ref_name).is_some_and(|(_, topic)| !names.contains(topic)))
        .collect();

    if args.json {
        let topics: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| {
                json!({
                    "topic": row.topic,
                    "parent": row.parent,
                    "depth": row.depth,
                    "branch": row.branch,
                    "base": row.base,
                    "number": row.pr.map(|pr| pr.number),
                    "url": row.pr.map(|pr| pr.url.as_str()),
                    "state": row.pr.map(|pr| state_name(pr.state)),
                    "needs_upload": row.needs_upload,
                })
            })
            .collect();
        let orphans: Vec<serde_json::Value> = orphaned
            .iter()
            .map(|pr| json!({"branch": pr.head_ref_name, "number": pr.number, "url": pr.url}))
            .collect();
        let value = json!({
            "trunk": trunk,
            "topics": topics,
            "orphaned": orphans,
            "restack": in_progress.as_ref().map(restack::Progress::to_json),
        });
        let text = serde_json::to_string_pretty(&value).map_err(|source| Error::Json { source })?;
        println!("{text}");
        return Ok(());
    }

    if let Some(progress) = &in_progress {
        println!("{}\n", progress.line());
    }
    if rows.is_empty() {
        println!("No commits with a Topic trailer between {trunk} and HEAD.");
    } else {
        println!("{trunk}");
    }
    for row in &rows {
        println!("{}{}", "   ".repeat(row.depth), status_line(row));
    }
    if !orphaned.is_empty() {
        println!("Open stack PRs no topic on this branch uses:");
        for pr in &orphaned {
            println!("  #{} {} {}", pr.number, pr.head_ref_name, pr.url);
        }
        println!("  Close them on GitHub if they are no longer needed; stack never closes PRs.");
    }
    Ok(())
}

/// One topic's text line, with or without a pull request.
fn status_line(row: &Row<'_>) -> String {
    let base = &row.base;
    match row.pr {
        Some(pr) => {
            let upload = if row.needs_upload {
                "needs upload"
            } else {
                "up to date"
            };
            format!(
                "└─ {}  #{} {}  base {base}  {upload}",
                row.topic,
                pr.number,
                state_name(pr.state)
            )
        }
        None => format!("└─ {}  no PR yet  base {base}  needs upload", row.topic),
    }
}

/// The state word `stack status` prints for a pull request.
fn state_name(state: PrState) -> &'static str {
    match state {
        PrState::Open => "open",
        PrState::Closed => "closed",
        PrState::Merged => "merged",
    }
}
