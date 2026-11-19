//! File importance (D802): the agent's key and skim marks, the daemon's
//! own skims (lockfiles, generated files, pure renames), and the tier each
//! served file ends up with.
//!
//! The daemon decides from names, attributes and blob ids and never reads
//! file content. Its marks are worked out when files are served and are
//! never stored.

use std::collections::{HashMap, HashSet};

use thiserror::Error;

use crate::checkout::Checkout;
use crate::error::DiffError;
use crate::model::{FileStatus, FileSummary, RepoDiff};
use crate::plan::{FileRef, Importance, Tier};

/// Lockfiles that are skimmed by name, in any folder.
const LOCKFILES: [&str; 8] = [
    "Cargo.lock",
    "cargo-bazel-lock.json",
    "MODULE.bazel.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "go.sum",
    "Podfile.lock",
];

/// Why an importance list is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ImportanceError {
    #[error("{repo}/{path} is in the importance list twice; give each file one tier")]
    Duplicate { repo: String, path: String },
}

/// Refuses a list that names a file twice.
///
/// # Errors
///
/// [`ImportanceError::Duplicate`] for the first repeated file.
pub fn check_importance(list: &[Importance]) -> Result<(), ImportanceError> {
    let mut seen = HashSet::new();
    for entry in list {
        if !seen.insert(&entry.file) {
            return Err(ImportanceError::Duplicate {
                repo: entry.file.repo.clone(),
                path: entry.file.path.clone(),
            });
        }
    }
    Ok(())
}

/// The final tier of a file: the agent's tier wins either way (so normal
/// un-skims), else the daemon's skim, else normal.
#[must_use]
pub fn final_tier(agent: Option<Tier>, auto_skim: bool) -> Tier {
    match agent {
        Some(tier) => tier,
        None if auto_skim => Tier::Skim,
        None => Tier::Normal,
    }
}

/// The files of `files` the daemon skims by itself, as far as names and
/// blob ids tell: lockfiles, pure renames, and the paths in `generated`.
#[must_use]
pub fn auto_skims(files: &[FileSummary], generated: &HashSet<String>) -> HashSet<String> {
    files
        .iter()
        .filter(|file| {
            is_lockfile(&file.path) || is_pure_rename(file) || generated.contains(&file.path)
        })
        .map(|file| file.path.clone())
        .collect()
}

/// The tiers of `diff`'s files, from the agent's list and the daemon's
/// skims, with `generated` the paths marked generated. Normal files are
/// left out: a path missing from the map is normal.
#[must_use]
pub fn tiers_of(
    diff: &RepoDiff,
    generated: &HashSet<String>,
    agent: &[Importance],
) -> HashMap<String, Tier> {
    let agent: HashMap<&str, Tier> = agent
        .iter()
        .filter(|entry| entry.file.repo == diff.repo)
        .map(|entry| (entry.file.path.as_str(), entry.tier))
        .collect();
    let auto = auto_skims(&diff.files, generated);
    diff.files
        .iter()
        .filter_map(|file| {
            let tier = final_tier(agent.get(file.path.as_str()).copied(), auto.contains(&file.path));
            (tier != Tier::Normal).then(|| (file.path.clone(), tier))
        })
        .collect()
}

/// [`tiers_of`] with the generated paths asked of git: one `check-attr`
/// call for the repo.
///
/// # Errors
///
/// [`DiffError`] when git fails.
pub fn tiers_blocking(
    checkout: &Checkout,
    diff: &RepoDiff,
    agent: &[Importance],
) -> Result<HashMap<String, Tier>, DiffError> {
    let paths: Vec<&str> = diff.files.iter().map(|file| file.path.as_str()).collect();
    let generated = checkout.generated_paths_blocking(&paths)?;
    Ok(tiers_of(diff, &generated, agent))
}

/// A diff's tiers keyed by [`FileRef`], for [`crate::ReviewPlan::arrange_with`].
#[must_use]
pub fn file_tiers(repo: &str, tiers: &HashMap<String, Tier>) -> HashMap<FileRef, Tier> {
    tiers
        .iter()
        .map(|(path, tier)| {
            (
                FileRef {
                    repo: repo.to_owned(),
                    path: path.clone(),
                },
                *tier,
            )
        })
        .collect()
}

impl Checkout {
    /// The paths among `paths` that `.gitattributes` marks
    /// `linguist-generated` (set, or `true`), in one git call.
    ///
    /// # Errors
    ///
    /// [`DiffError`] when git fails.
    pub fn generated_paths_blocking(&self, paths: &[&str]) -> Result<HashSet<String>, DiffError> {
        if paths.is_empty() {
            return Ok(HashSet::new());
        }
        let mut input = Vec::new();
        for path in paths {
            input.extend_from_slice(path.as_bytes());
            input.push(0);
        }
        let out = self
            .git
            .run_with_input(["check-attr", "-z", "--stdin", "linguist-generated"], &input)?;
        let text = String::from_utf8_lossy(&out);
        let fields: Vec<&str> = text.split('\0').collect();
        Ok(fields
            .chunks_exact(3)
            .filter(|triple| matches!(triple[2], "set" | "true"))
            .map(|triple| triple[0].to_owned())
            .collect())
    }
}

fn is_lockfile(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    LOCKFILES.contains(&name)
}

fn is_pure_rename(file: &FileSummary) -> bool {
    let FileStatus::Renamed { similarity, .. } = &file.status else {
        return false;
    };
    let same_blob = file.old_blob.is_some() && file.old_blob == file.new_blob;
    same_blob || (*similarity == 100 && file.added == 0 && file.removed == 0)
}
