//! Local stack metadata stored under `<git-dir>/stack/state.json`.
//!
//! Records imported remote stacks (topics, relatives, remote branch names,
//! PR numbers, authorship/foreign status) as well as stacks initialized or
//! navigated with `stack init`, `stack add`, `stack up`/`down`/`top`/`bottom`.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::git::Git;
use crate::local;

/// One layer of a local or imported stack, ordered bottom-to-top.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Layer {
    pub(crate) topic: String,
    #[serde(default)]
    pub(crate) relative: Option<String>,
    pub(crate) branch: String,
    pub(crate) base: String,
    #[serde(default)]
    pub(crate) pr_number: Option<u64>,
    #[serde(default)]
    pub(crate) pr_url: Option<String>,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) author: Option<String>,
    #[serde(default)]
    pub(crate) foreign: bool,
    #[serde(default)]
    pub(crate) remote_head: Option<String>,
    #[serde(default)]
    pub(crate) remote_base: Option<String>,
}

/// Persistent stack metadata stored in `<git-dir>/stack/state.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StackState {
    #[serde(default)]
    pub(crate) trunk: Option<String>,
    #[serde(default)]
    pub(crate) remote: Option<String>,
    /// `"native"`, `"base_chain"`, `"pr"`, or `"local"`.
    #[serde(default)]
    pub(crate) source: Option<String>,
    #[serde(default)]
    pub(crate) github_stack_number: Option<u64>,
    /// The local branch where imported layers are materialized linearly.
    #[serde(default)]
    pub(crate) work_branch: Option<String>,
    #[serde(default)]
    pub(crate) layers: Vec<Layer>,
}

impl StackState {
    fn path(git: &Git) -> Result<PathBuf, Error> {
        Ok(git.git_path("stack/state.json")?)
    }

    /// Reads `<git-dir>/stack/state.json`; returns `Ok(None)` if absent.
    pub(crate) fn load(git: &Git) -> Result<Option<StackState>, Error> {
        let path = Self::path(git)?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(Error::TempFile { path, source }),
        };
        let state = serde_json::from_str(&text).map_err(|source| Error::Json { source })?;
        Ok(Some(state))
    }

    /// Writes `<git-dir>/stack/state.json`, creating parent directories.
    pub(crate) fn save(&self, git: &Git) -> Result<(), Error> {
        let path = Self::path(git)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::TempFile {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|source| Error::Json { source })?;
        fs::write(&path, text).map_err(|source| Error::TempFile { path, source })?;
        Ok(())
    }

    pub(crate) fn layer_by_topic(&self, topic: &str) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.topic == topic)
    }

    pub(crate) fn layer_by_branch(&self, branch: &str) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.branch == branch)
    }

    pub(crate) fn foreign_topics(&self) -> BTreeSet<String> {
        self.layers
            .iter()
            .filter(|layer| layer.foreign)
            .map(|layer| layer.topic.clone())
            .collect()
    }
}

/// Turns any branch name into a valid topic name (`a-z`, `0-9`, `.`, `_`, `-`).
/// A `<prefix>/stack/<topic>` branch yields `<topic>` when valid.
pub(crate) fn topic_from_branch(branch: &str) -> String {
    if let Some((_, topic)) = local::split_branch(branch)
        && is_valid_topic(topic)
    {
        return topic.to_owned();
    }
    let mut out = String::with_capacity(branch.len());
    let mut last_dash = false;
    for ch in branch.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
            out.push(c);
            last_dash = c == '-';
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_owned();
    if trimmed.is_empty() {
        "layer".to_owned()
    } else {
        trimmed
    }
}

fn is_valid_topic(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::topic_from_branch;

    #[test]
    fn topic_from_branch_extracts_stack_topic_or_sanitizes() {
        assert_eq!(topic_from_branch("bob/stack/auth-layer"), "auth-layer");
        assert_eq!(topic_from_branch("Feature/Auth Layer!"), "feature-auth-layer");
        assert_eq!(topic_from_branch("tasks/api"), "tasks-api");
        assert_eq!(topic_from_branch("///"), "layer");
    }
}
