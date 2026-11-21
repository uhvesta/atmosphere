//! What every command reads first: the commits between the trunk and HEAD,
//! grouped into topics, and each topic rebuilt onto the one it builds on.

use std::collections::{BTreeMap, BTreeSet};

use crate::body::Template;
use crate::error::Error;
use crate::gh::Pr;
use crate::git::{Git, GitError};
use crate::plan::Plan;
use crate::state::{Layer, StackState};

/// A topic's rebuilt branch: `head` sits on `base`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tip {
    /// The relative topic's head, or the merge base with the trunk.
    pub(crate) base: String,
    pub(crate) head: String,
}

/// The branch a new topic is pushed to: `<prefix>/stack/<topic>`.
pub(crate) fn branch_name(prefix: &str, topic: &str) -> String {
    format!("{prefix}/stack/{topic}")
}

/// The prefix and topic of a branch named `<prefix>/stack/<topic>`, each one
/// part of the name; None for any other branch.
pub(crate) fn split_branch(branch: &str) -> Option<(&str, &str)> {
    let (prefix, topic) = branch.split_once("/stack/")?;
    let one_part = |part: &str| !part.is_empty() && !part.contains('/');
    (one_part(prefix) && one_part(topic)).then_some((prefix, topic))
}

/// Each topic's branch. GitHub records the head branch of every open pull
/// request, so a topic with one keeps that branch, whatever prefix it was
/// opened with; any other topic gets `<prefix>/stack/<topic>`.
pub(crate) struct Branches {
    prefix: String,
    /// The open pull request of each topic, from your open pull requests on
    /// stack branches. With several for one topic: the one on the current
    /// prefix, else the oldest.
    open: BTreeMap<String, Pr>,
    /// Imported or locally registered layers from `.git/stack/state.json`.
    layers: BTreeMap<String, Layer>,
}

impl Branches {
    #[cfg(test)]
    pub(crate) fn new(prefix: String, mine: Vec<Pr>) -> Branches {
        Self::with_state(prefix, mine, None)
    }

    pub(crate) fn with_state(prefix: String, mine: Vec<Pr>, state: Option<&StackState>) -> Branches {
        let mut open: BTreeMap<String, Pr> = BTreeMap::new();
        for pr in mine {
            let Some((pr_prefix, topic)) = split_branch(&pr.head_ref_name) else {
                continue;
            };
            let topic = topic.to_owned();
            let better = match open.get(&topic) {
                None => true,
                Some(kept) => {
                    let kept_current = split_branch(&kept.head_ref_name)
                        .is_some_and(|(kept_prefix, _)| kept_prefix == prefix);
                    let current = pr_prefix == prefix;
                    (current && !kept_current) || (current == kept_current && pr.number < kept.number)
                }
            };
            if better {
                open.insert(topic, pr);
            }
        }
        let layers = state
            .map(|state| {
                state
                    .layers
                    .iter()
                    .map(|layer| (layer.topic.clone(), layer.clone()))
                    .collect()
            })
            .unwrap_or_default();
        Branches { prefix, open, layers }
    }

    /// The prefix new topics use.
    pub(crate) fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The topic's branch: its imported layer's branch, else its open pull
    /// request's head, else `<prefix>/stack/<topic>`.
    pub(crate) fn branch(&self, topic: &str) -> String {
        if let Some(layer) = self.layers.get(topic) {
            return layer.branch.clone();
        }
        self.open
            .get(topic)
            .map_or_else(|| branch_name(&self.prefix, topic), |pr| pr.head_ref_name.clone())
    }

    /// Whether `topic` belongs to a foreign imported layer.
    pub(crate) fn is_foreign(&self, topic: &str) -> bool {
        self.layers.get(topic).is_some_and(|layer| layer.foreign)
    }

    /// The imported/registered layer for `topic`, if any.
    pub(crate) fn layer(&self, topic: &str) -> Option<&Layer> {
        self.layers.get(topic)
    }

    /// Every non-standard branch name recorded in imported state, for fetching
    /// and inspecting remote heads.
    pub(crate) fn extra_branches(&self) -> Vec<String> {
        self.layers
            .values()
            .map(|layer| layer.branch.clone())
            .collect()
    }

    /// The topic's open pull request, when it has one.
    pub(crate) fn open_pr(&self, topic: &str) -> Option<&Pr> {
        self.open.get(topic)
    }

    /// Every open pull request on a stack branch, one per topic.
    pub(crate) fn open_prs(&self) -> impl Iterator<Item = &Pr> {
        self.open.values()
    }

    /// The current prefix and every prefix an open pull request's branch
    /// uses, each once.
    pub(crate) fn prefixes(&self) -> BTreeSet<String> {
        let mut prefixes: BTreeSet<String> = self
            .open
            .values()
            .filter_map(|pr| split_branch(&pr.head_ref_name).map(|(prefix, _)| prefix.to_owned()))
            .collect();
        prefixes.insert(self.prefix.clone());
        prefixes
    }
}

/// The remote's `<prefix>/stack/*` branches and their shas, for each prefix.
pub(crate) fn remote_heads(
    git: &Git,
    remote: &str,
    prefixes: &BTreeSet<String>,
) -> Result<BTreeMap<String, String>, GitError> {
    let mut heads = BTreeMap::new();
    for prefix in prefixes {
        heads.extend(git.remote_heads(remote, &format!("{prefix}/stack/"))?);
    }
    Ok(heads)
}

/// The merge base of HEAD and `<remote>/<trunk>` (the ref as last fetched),
/// and the plan of the commits after it.
pub(crate) fn read(git: &Git, remote: &str, trunk: &str) -> Result<(String, Plan), Error> {
    let merge_base = git.merge_base("HEAD", &format!("{remote}/{trunk}"))?;
    let commits = git.commits(&merge_base, "HEAD")?;
    let plan = Plan::build(commits)?;
    Ok((merge_base, plan))
}

/// Rebuilds each topic of `plan` in order: its commits go onto its
/// relative's new head, or onto `merge_base` when it has no relative or the
/// relative is in `merged`. A commit whose parent already is the head so far
/// is kept as it is; any other is copied with `Git::copy_onto`. Topics in
/// `merged` get no tip.
pub(crate) fn rebuild(
    git: &Git,
    plan: &Plan,
    merge_base: &str,
    merged: &BTreeSet<String>,
) -> Result<BTreeMap<String, Tip>, Error> {
    let mut tips: BTreeMap<String, Tip> = BTreeMap::new();
    for topic in &plan.topics {
        if merged.contains(&topic.name) {
            continue;
        }
        let base = topic
            .relative
            .as_deref()
            .filter(|relative| !merged.contains(*relative))
            .and_then(|relative| tips.get(relative))
            .map_or_else(|| merge_base.to_string(), |tip| tip.head.clone());

        let mut head = base.clone();
        for commit in &topic.commits {
            head = if commit.parent == head {
                commit.sha.clone()
            } else {
                git.copy_onto(commit, &head)?
            };
        }
        tips.insert(topic.name.clone(), Tip { base, head });
    }
    Ok(tips)
}

/// Checks that the first commit of each topic has the template's required
/// sections. Topics in `skip` (e.g. foreign imported layers) are skipped.
pub(crate) fn require_sections(
    template: &Template,
    plan: &Plan,
    skip: &BTreeSet<String>,
) -> Result<(), Error> {
    for topic in &plan.topics {
        if skip.contains(&topic.name) {
            continue;
        }
        let Some(commit) = topic.commits.first() else {
            continue;
        };
        let missing = template.missing_required(&commit.message);
        if !missing.is_empty() {
            return Err(Error::MissingSections {
                topic: topic.name.clone(),
                commit: commit.sha.chars().take(8).collect(),
                sections: missing.join(", "),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Branches, split_branch};
    use crate::gh::{Pr, PrState};

    fn pr(number: u64, head: &str) -> Pr {
        Pr {
            number,
            url: format!("https://github.com/o/r/pull/{number}"),
            state: PrState::Open,
            head_ref_name: head.to_owned(),
            base_ref_name: "main".to_owned(),
            title: format!("pr {number}"),
            body: String::new(),
            is_draft: false,
            review_decision: None,
            author: String::new(),
        }
    }

    #[test]
    fn split_branch_takes_one_part_on_each_side() {
        assert_eq!(split_branch("foo/stack/codec"), Some(("foo", "codec")));
        assert_eq!(split_branch("a/b/stack/codec"), None);
        assert_eq!(split_branch("foo/stack/a/b"), None);
        assert_eq!(split_branch("/stack/codec"), None);
        assert_eq!(split_branch("feature/codec"), None);
    }

    #[test]
    fn a_topic_with_an_open_pr_keeps_its_branch_and_a_new_one_uses_the_prefix() {
        let branches = Branches::new("foo".to_owned(), vec![pr(1, "alice/stack/codec"), pr(2, "fix-typo")]);
        assert_eq!(branches.branch("codec"), "alice/stack/codec");
        assert_eq!(branches.branch("journal"), "foo/stack/journal");
        assert_eq!(branches.open_pr("codec").map(|pr| pr.number), Some(1));
        assert!(branches.open_pr("fix-typo").is_none());
        let prefixes: Vec<String> = branches.prefixes().into_iter().collect();
        assert_eq!(prefixes, ["alice", "foo"]);
    }

    #[test]
    fn with_two_open_prs_for_a_topic_the_current_prefix_wins_else_the_oldest() {
        let mine = vec![pr(5, "bob/stack/codec"), pr(9, "foo/stack/codec"), pr(3, "carol/stack/codec")];
        assert_eq!(Branches::new("foo".to_owned(), mine.clone()).branch("codec"), "foo/stack/codec");
        assert_eq!(Branches::new("zed".to_owned(), mine).branch("codec"), "carol/stack/codec");
    }
}
