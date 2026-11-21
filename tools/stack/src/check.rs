//! `stack check`: the plan and its checks, locally. Never fetches or calls
//! GitHub.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Value, json};

use crate::body::Template;
use crate::cli::CheckArgs;
use crate::config::Config;
use crate::error::Error;
use crate::git::Git;
use crate::local;
use crate::plan::{Commit, Plan};
use crate::state::StackState;

/// Reads the topics between the trunk (as last fetched) and HEAD, checks the
/// first commit of each has the template's sections, and rebuilds each topic
/// to find conflicts. Prints the plan as text, or as JSON with `--json`.
pub(crate) fn run(args: &CheckArgs, dir: &Path) -> Result<(), Error> {
    let config = Config::load()?;
    let git = Git::new(dir);
    let state = StackState::load(&git)?;
    let remote = config.remote(&args.target);
    let trunk = config.local_trunk(&args.target, &git, &remote);

    let (merge_base, plan) = local::read(&git, &remote, &trunk)?;
    let template = Template::load(&config)?;
    let foreign = state
        .as_ref()
        .map(StackState::foreign_topics)
        .unwrap_or_default();
    local::require_sections(&template, &plan, &foreign)?;
    local::rebuild(&git, &plan, &merge_base, &BTreeSet::new())?;

    if args.json {
        print_json(&trunk, &plan)
    } else {
        print_text(&trunk, &plan);
        Ok(())
    }
}

fn print_text(trunk: &str, plan: &Plan) {
    if plan.topics.is_empty() {
        println!("No commits with a Topic trailer between {trunk} and HEAD.");
    }
    for topic in &plan.topics {
        let on = topic.relative.as_deref().unwrap_or(trunk);
        println!("{}  (on {on})", topic.name);
        for commit in &topic.commits {
            println!("  - {}", commit.message.subject);
        }
    }
    if !plan.untagged.is_empty() {
        println!("Stays local (no Topic):");
        for commit in &plan.untagged {
            println!("  - {}", commit.message.subject);
        }
    }
}

fn print_json(trunk: &str, plan: &Plan) -> Result<(), Error> {
    let topics: Vec<Value> = plan
        .topics
        .iter()
        .map(|topic| {
            json!({
                "topic": topic.name,
                "relative": topic.relative,
                "commits": commits_json(&topic.commits),
            })
        })
        .collect();
    let value = json!({
        "trunk": trunk,
        "topics": topics,
        "untagged": commits_json(&plan.untagged),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|source| Error::Json { source })?
    );
    Ok(())
}

fn commits_json(commits: &[Commit]) -> Vec<Value> {
    commits
        .iter()
        .map(|commit| json!({ "sha": commit.sha, "subject": commit.message.subject }))
        .collect()
}
