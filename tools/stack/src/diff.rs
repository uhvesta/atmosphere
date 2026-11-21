//! `stack diff`: what changed between two uploaded revisions of a topic,
//! from their pins, with the base's own changes left out.

use std::collections::BTreeMap;
use std::path::Path;

use crate::cli::DiffArgs;
use crate::config::{Config, ConfigError};
use crate::git::{Git, GitError};
use crate::pins;
use crate::state::StackState;

#[derive(Debug, thiserror::Error)]
pub(crate) enum DiffError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error("could not read or fetch the pinned revisions of `{topic}`: {source}")]
    Git {
        topic: String,
        #[source]
        source: GitError,
    },

    #[error(
        "{remote} has no pinned revisions of `{topic}`{under}; pins are written by `stack upload` (pin_revisions in stack.toml), so revisions uploaded before pinning was on can't be compared",
        under = .prefix.as_ref().map(|prefix| format!(" under prefix `{prefix}`")).unwrap_or_default()
    )]
    NoPins {
        topic: String,
        prefix: Option<String>,
        remote: String,
    },

    #[error("`{topic}` has pins under more than one prefix ({}); say which with --prefix", .prefixes.join(", "))]
    AmbiguousPrefix { topic: String, prefixes: Vec<String> },

    #[error("`{given}` is not a revision; give the number from the history table, such as `3` or `r3`")]
    BadRevision { given: String },

    #[error("`{topic}` has no pinned revision r{rev}; the pinned ones are {}", list(.have))]
    UnknownRevision { topic: String, rev: u32, have: Vec<u32> },

    #[error("`{topic}` has only one pinned revision (r{rev}), so there is nothing to compare it with yet")]
    OnlyOne { topic: String, rev: u32 },
}

impl DiffError {
    pub(crate) fn exit_code(&self) -> u8 {
        match self {
            DiffError::Config(error) => error.exit_code(),
            DiffError::Git { .. } => 1,
            _ => 2,
        }
    }

    /// The command to run next.
    pub(crate) fn next_command(&self) -> String {
        match self {
            DiffError::Config(_) => "stack config show".to_owned(),
            DiffError::Git { .. } => "git fetch <remote>, then stack diff again".to_owned(),
            DiffError::NoPins { .. } | DiffError::OnlyOne { .. } => "stack upload (each upload that moves a topic pins a new revision)".to_owned(),
            DiffError::AmbiguousPrefix { topic, prefixes } => {
                format!("stack diff {topic} --prefix {}", prefixes.first().map_or("<prefix>", String::as_str))
            }
            DiffError::BadRevision { .. } => "stack diff <topic> <from-rev> <to-rev>".to_owned(),
            DiffError::UnknownRevision { topic, have, .. } => match have.as_slice() {
                [.., from, to] => format!("stack diff {topic} {from} {to}"),
                _ => format!("stack diff {topic}"),
            },
        }
    }
}

fn list(revs: &[u32]) -> String {
    revs.iter().map(|rev| format!("r{rev}")).collect::<Vec<_>>().join(", ")
}

pub(crate) fn run(args: &DiffArgs, dir: &Path) -> Result<(), crate::error::Error> {
    let config = Config::load().map_err(DiffError::from)?;
    let git = Git::new(dir);
    let remote = args.remote.clone().unwrap_or_else(|| config.remote.value.clone());
    let topic = args.topic.as_str();
    let git_error = |source| DiffError::Git {
        topic: topic.to_owned(),
        source,
    };

    let prefix = match args.prefix.clone().or_else(|| config.prefix.value.clone()) {
        Some(prefix) => Some(prefix),
        None => {
            let mut found = pins::prefixes_with(&git, &remote, topic).map_err(git_error)?;
            match found.len() {
                0 => None,
                1 => Some(found.remove(0)),
                _ => {
                    return Err(DiffError::AmbiguousPrefix {
                        topic: topic.to_owned(),
                        prefixes: found,
                    }
                    .into());
                }
            }
        }
    };

    // If no pins exist, fall back to diffing an imported layer directly.
    let Some(prefix) = prefix else {
        if let Ok(Some(state)) = StackState::load(&git)
            && let Some(layer) = state.layer_by_topic(topic)
            && let (Some(base), Some(head)) = (&layer.remote_base, &layer.remote_head)
        {
            let text = git.diff_range(base, head).map_err(git_error)?;
            println!("{topic}: {}..{} (imported layer)", &base[..8.min(base.len())], &head[..8.min(head.len())]);
            if text.trim().is_empty() {
                println!("No changes.");
            } else {
                println!("{text}");
            }
            return Ok(());
        }
        return Err(DiffError::NoPins {
            topic: topic.to_owned(),
            prefix: None,
            remote,
        }
        .into());
    };

    let pinned: BTreeMap<u32, String> = pins::on_remote(&git, &remote, &prefix, topic).map_err(git_error)?;
    let have: Vec<u32> = pinned.keys().copied().collect();
    let (from, to) = match (&args.from, &args.to) {
        (Some(from), Some(to)) => (revision(from)?, revision(to)?),
        (Some(from), None) => {
            let to = have.last().copied().unwrap_or_default();
            (revision(from)?, to)
        }
        _ => match have.as_slice() {
            [] => {
                return Err(DiffError::NoPins {
                    topic: topic.to_owned(),
                    prefix: Some(prefix),
                    remote,
                }
                .into());
            }
            [only] => {
                return Err(DiffError::OnlyOne {
                    topic: topic.to_owned(),
                    rev: *only,
                }
                .into());
            }
            [.., from, to] => (*from, *to),
        },
    };
    for rev in [from, to] {
        if !pinned.contains_key(&rev) {
            return Err(DiffError::UnknownRevision {
                topic: topic.to_owned(),
                rev,
                have,
            }
            .into());
        }
    }

    // Mirror the topic's pins locally, so range-diff can read them.
    let folder = pins::folder(&prefix, topic);
    git.fetch(&remote, &[format!("+{folder}*:{folder}*")]).map_err(git_error)?;
    let (old_base, old_head) = (pins::base_ref(&prefix, topic, from), pins::head_ref(&prefix, topic, from));
    let (new_base, new_head) = (pins::base_ref(&prefix, topic, to), pins::head_ref(&prefix, topic, to));
    let text = git
        .range_diff((old_base.as_str(), old_head.as_str()), (new_base.as_str(), new_head.as_str()))
        .map_err(git_error)?;
    println!("{topic}: r{from} -> r{to} (base changes left out)");
    if text.trim().is_empty() {
        println!("No commits on either side.");
    } else {
        println!("{text}");
    }
    Ok(())
}

/// A revision as the history table writes it: `3` or `r3`.
fn revision(given: &str) -> Result<u32, DiffError> {
    given
        .strip_prefix('r')
        .unwrap_or(given)
        .parse()
        .map_err(|_| DiffError::BadRevision {
            given: given.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::revision;

    #[test]
    fn a_revision_is_its_number_with_or_without_the_r() {
        assert_eq!(revision("3").ok(), Some(3));
        assert_eq!(revision("r12").ok(), Some(12));
        assert!(revision("head").is_err());
        assert!(revision("r").is_err());
    }
}
