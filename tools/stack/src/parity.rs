//! `gh stack` parity commands: `init`, `add`, `up`, `down`, `top`, `bottom`,
//! `trunk`, `rebase`, `link`. (`view` delegates to `status`; `submit`, `push`,
//! and `sync` share `upload`'s engine.)

use std::path::Path;

use crate::cli::{AddArgs, InitArgs, LinkArgs, RebaseArgs, Target};
use crate::config::Config;
use crate::error::Error;
use crate::gh::{Gh, PrEdit};
use crate::git::Git;
use crate::import;
use crate::restack;
use crate::state::{Layer, StackState, topic_from_branch};

/// `stack init [--base/--trunk <branch>] [--remote <name>] [branches...]`
pub(crate) fn init(args: &InitArgs, dir: &Path) -> Result<(), Error> {
    let git = Git::new(dir);
    let config = Config::load()?;
    let remote = config.remote(&args.target);
    let trunk = config.local_trunk(&args.target, &git, &remote);

    let mut state = StackState::load(&git)?.unwrap_or_default();
    state.trunk = Some(trunk.clone());
    state.remote = Some(remote.clone());
    if state.source.is_none() {
        state.source = Some("local".to_owned());
    }

    if !args.branches.is_empty() {
        state.layers.clear();
        let mut prev_topic: Option<String> = None;
        let mut prev_branch = trunk.clone();
        for branch in &args.branches {
            if !git.branch_exists(branch) {
                git.checkout_branch(branch, Some(&prev_branch))?;
            }
            let topic = topic_from_branch(branch);
            state.layers.push(Layer {
                topic: topic.clone(),
                relative: prev_topic.clone(),
                branch: branch.clone(),
                base: prev_branch.clone(),
                pr_number: None,
                pr_url: None,
                title: None,
                author: None,
                foreign: false,
                remote_head: None,
                remote_base: None,
            });
            prev_topic = Some(topic);
            prev_branch = branch.clone();
        }
    } else if state.layers.is_empty()
        && let Some(current) = git.current_branch()?
        && current != trunk
    {
        let topic = topic_from_branch(&current);
        state.layers.push(Layer {
            topic,
            relative: None,
            branch: current,
            base: trunk.clone(),
            pr_number: None,
            pr_url: None,
            title: None,
            author: None,
            foreign: false,
            remote_head: None,
            remote_base: None,
        });
    }

    state.save(&git)?;
    println!(
        "Initialized stack on trunk `{trunk}` with {} layer(s).",
        state.layers.len()
    );
    Ok(())
}

/// `stack add <name> [--relative <topic>]`
pub(crate) fn add(args: &AddArgs, dir: &Path) -> Result<(), Error> {
    let git = Git::new(dir);
    let config = Config::load()?;
    let remote = config.remote(&args.target);
    let mut state = StackState::load(&git)?.unwrap_or_default();
    let trunk = state
        .trunk
        .clone()
        .unwrap_or_else(|| config.local_trunk(&args.target, &git, &remote));
    state.trunk = Some(trunk.clone());
    state.remote = Some(remote);

    let current = git.current_branch()?;
    let parent_info = match &args.relative {
        Some(rel) => state
            .layer_by_topic(rel)
            .or_else(|| state.layer_by_branch(rel)),
        None => current
            .as_deref()
            .and_then(|branch| state.layer_by_branch(branch))
            .or_else(|| state.layers.last()),
    }
    .map(|parent| (parent.topic.clone(), parent.branch.clone()));

    let (relative, base, start_point) = match &parent_info {
        Some((parent_topic, parent_branch)) => (
            Some(parent_topic.clone()),
            parent_branch.clone(),
            Some(parent_branch.clone()),
        ),
        None => (None, trunk.clone(), current.clone()),
    };

    let topic = topic_from_branch(&args.name);
    if !git.branch_exists(&args.name) {
        git.checkout_branch(&args.name, start_point.as_deref())?;
    } else {
        git.checkout(&args.name)?;
    }

    if let Some(existing) = state.layers.iter_mut().find(|l| l.branch == args.name || l.topic == topic) {
        existing.branch = args.name.clone();
        existing.relative = relative;
        existing.base = base.clone();
    } else {
        let insert_at = parent_info
            .as_ref()
            .and_then(|(parent_topic, _)| state.layers.iter().position(|l| l.topic == *parent_topic))
            .map(|i| i + 1)
            .unwrap_or(state.layers.len());
        state.layers.insert(
            insert_at,
            Layer {
                topic: topic.clone(),
                relative,
                branch: args.name.clone(),
                base: base.clone(),
                pr_number: None,
                pr_url: None,
                title: None,
                author: None,
                foreign: false,
                remote_head: None,
                remote_base: None,
            },
        );
    }

    state.save(&git)?;
    println!("Added layer `{}` (branch `{}`, base `{base}`).", topic, args.name);
    Ok(())
}

/// Direction for `up`, `down`, `top`, `bottom`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Nav {
    Up,
    Down,
    Top,
    Bottom,
}

/// `stack up`, `stack down`, `stack top`, `stack bottom`.
pub(crate) fn navigate(nav: Nav, dir: &Path) -> Result<(), Error> {
    let git = Git::new(dir);
    let state = StackState::load(&git)?.ok_or_else(|| Error::Usage {
        message: "no stack is initialized or imported in this repository".to_owned(),
        next: "stack init (or `stack import <pr>`)".to_owned(),
    })?;
    if state.layers.is_empty() {
        return Err(Error::Usage {
            message: "this stack has no layers yet".to_owned(),
            next: "stack add <branch>".to_owned(),
        });
    }
    let current = git.current_branch()?;
    let idx = current
        .as_deref()
        .and_then(|branch| state.layers.iter().position(|l| l.branch == branch));

    let target_branch = match nav {
        Nav::Top => state.layers.last().map(|l| l.branch.clone()),
        Nav::Bottom => state.layers.first().map(|l| l.branch.clone()),
        Nav::Up => match idx {
            Some(i) if i + 1 < state.layers.len() => Some(state.layers[i + 1].branch.clone()),
            Some(_) => {
                return Err(Error::Usage {
                    message: "already at the top layer of the stack".to_owned(),
                    next: "stack view".to_owned(),
                });
            }
            None => state.layers.first().map(|l| l.branch.clone()),
        },
        Nav::Down => match idx {
            Some(i) if i > 0 => Some(state.layers[i - 1].branch.clone()),
            Some(0) => state.trunk.clone(),
            _ => {
                return Err(Error::Usage {
                    message: "already at the bottom of the stack".to_owned(),
                    next: "stack trunk".to_owned(),
                });
            }
        },
    };

    let Some(target) = target_branch else {
        return Err(Error::Usage {
            message: "no target branch found in stack".to_owned(),
            next: "stack view".to_owned(),
        });
    };
    git.checkout(&target)?;
    println!("Checked out `{target}`.");
    Ok(())
}

/// `stack trunk [--base/--trunk <branch>] [--remote <name>]`
pub(crate) fn trunk(target: &Target, dir: &Path) -> Result<(), Error> {
    let git = Git::new(dir);
    let config = Config::load()?;
    let remote = config.remote(target);
    let trunk = target
        .trunk
        .clone()
        .or_else(|| StackState::load(&git).ok().flatten().and_then(|s| s.trunk))
        .unwrap_or_else(|| config.local_trunk(target, &git, &remote));
    git.checkout(&trunk)?;
    println!("Checked out trunk `{trunk}`.");
    Ok(())
}

/// `stack rebase [--continue | --skip | --abort] [--onto <ref>]`
pub(crate) fn rebase(args: &RebaseArgs, dir: &Path) -> Result<(), Error> {
    let git = Git::new(dir);
    // If a `stack restack` is in progress, route --continue / --skip / --abort to it.
    if restack::in_progress(&git)?.is_some() {
        if args.continue_rebase {
            return Ok(restack::run(
                &crate::cli::RestackCommand::Continue(crate::cli::ReplayArgs {
                    step: false,
                    exec: None,
                }),
                dir,
            )?);
        }
        if args.skip {
            return Ok(restack::run(
                &crate::cli::RestackCommand::Skip(crate::cli::ReplayArgs {
                    step: false,
                    exec: None,
                }),
                dir,
            )?);
        }
        if args.abort {
            return Ok(restack::run(&crate::cli::RestackCommand::Abort, dir)?);
        }
    }

    if args.continue_rebase || args.skip || args.abort {
        let flag = if args.continue_rebase {
            "--continue"
        } else if args.skip {
            "--skip"
        } else {
            "--abort"
        };
        let output = git.rebase(&[flag], None)?;
        if !output.status.success() {
            return Err(Error::Usage {
                message: format!(
                    "`git rebase {flag}` failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                next: "git status".to_owned(),
            });
        }
        println!("Rebase `{flag}` finished.");
        return Ok(());
    }

    if git.has_uncommitted_changes()? {
        return Err(Error::Usage {
            message: "the working tree has uncommitted changes; commit or stash them before rebasing".to_owned(),
            next: "git status".to_owned(),
        });
    }

    let config = Config::load()?;
    let remote = config.remote(&args.target);
    let trunk = config.local_trunk(&args.target, &git, &remote);
    let onto = args
        .onto
        .clone()
        .unwrap_or_else(|| format!("{remote}/{trunk}"));

    let output = git.rebase(&[onto.as_str()], None)?;
    if !output.status.success() {
        return Err(Error::Usage {
            message: format!(
                "`git rebase {onto}` stopped: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            next: "resolve conflicts, `git add`, then `stack rebase --continue` (or `stack rebase --abort`)".to_owned(),
        });
    }
    println!("Rebased onto `{onto}`.");
    Ok(())
}

/// `stack link --base <trunk> <pr>...`
pub(crate) fn link(args: &LinkArgs, dir: &Path) -> Result<(), Error> {
    if args.prs.len() < 2 {
        return Err(Error::Usage {
            message: "`stack link` needs at least two pull requests (bottom to top)".to_owned(),
            next: "stack link --base main <pr1> <pr2>".to_owned(),
        });
    }
    let config = Config::load()?;
    let gh = Gh::new(dir);
    let repo = gh.repo()?;
    let trunk = config.trunk(&args.target, repo.default_branch);

    let mut prs = Vec::with_capacity(args.prs.len());
    for raw in &args.prs {
        let num = import::parse_pr_target(raw)?;
        prs.push(gh.pr_view(&num.to_string())?);
    }

    // Ensure PR base branches chain bottom-to-top.
    let mut expected_base = trunk.clone();
    for pr in &prs {
        if pr.base_ref_name != expected_base {
            gh.edit_pr(&PrEdit {
                number: pr.number,
                base: Some(expected_base.as_str()),
                title: None,
                body_file: None,
                add_reviewers: &[],
                add_labels: &[],
            })?;
        }
        expected_base = pr.head_ref_name.clone();
    }

    let urls: Vec<String> = prs.iter().map(|pr| pr.url.clone()).collect();
    if gh.has_native_stacks() {
        gh.link_stack(&trunk, &urls)?;
        println!(
            "Linked {} pull requests as a native GitHub stack on `{trunk}`.",
            prs.len()
        );
    } else {
        println!(
            "Chained {} pull requests by base branch on `{trunk}`.",
            prs.len()
        );
    }
    Ok(())
}
