//! Pushing a session's work (MV-10, MV-12): the change totals of a clone
//! against its base, and committing and pushing a clone's branch. Blocking:
//! each function runs git.

use std::path::{Path, PathBuf};

use crate::error::GitError;
use crate::git::git;
use crate::work::RepoWork;

/// Lines added and removed across a clone's changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub added: u32,
    pub removed: u32,
}

/// The totals of `work`: committed, staged and unstaged changes against the
/// base the clone's default branch gives (see `repo_work`), with each
/// untracked file's lines counted as added.
pub fn totals(work: &RepoWork) -> Totals {
    work.files.iter().fold(Totals::default(), |sum, file| Totals {
        added: sum.added.saturating_add(file.added),
        removed: sum.removed.saturating_add(file.removed),
    })
}

/// Why a push step failed. Each names the clone it failed in; the git
/// message it carries says what git refused.
#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("could not stage the changes in {path}")]
    Stage {
        path: PathBuf,
        #[source]
        source: GitError,
    },
    #[error("could not commit the changes in {path}")]
    Commit {
        path: PathBuf,
        #[source]
        source: GitError,
    },
    #[error("could not push {branch} from {path} to origin")]
    Push {
        path: PathBuf,
        branch: String,
        #[source]
        source: GitError,
    },
}

/// Stages every change in `clone` and commits it with `message`. Nothing to
/// commit is not an error: it makes no commit and returns false.
pub fn commit_all(clone: &Path, message: &str) -> Result<bool, PushError> {
    let stage = |source| PushError::Stage {
        path: clone.to_path_buf(),
        source,
    };
    git(Some(clone), &["add", "-A"]).map_err(stage)?;
    let staged = git(Some(clone), &["status", "--porcelain"]).map_err(stage)?;
    if staged.trim().is_empty() {
        return Ok(false);
    }
    git(Some(clone), &["commit", "-m", message]).map_err(|source| PushError::Commit {
        path: clone.to_path_buf(),
        source,
    })?;
    Ok(true)
}

/// Pushes `branch` to origin and makes origin's branch its upstream.
/// When `clone` is on a detached `HEAD` (e.g. a concurrent PR review worktree),
/// pushes `HEAD:refs/heads/<branch>` cleanly without `--set-upstream`.
pub fn push_branch(clone: &Path, branch: &str) -> Result<(), PushError> {
    let err = |source| PushError::Push {
        path: clone.to_path_buf(),
        branch: branch.to_owned(),
        source,
    };
    if crate::worktree::is_detached_head(clone) {
        let refspec = format!("HEAD:refs/heads/{branch}");
        return git(Some(clone), &["push", "origin", &refspec])
            .map(|_| ())
            .map_err(err);
    }
    git(Some(clone), &["push", "--set-upstream", "origin", branch])
        .map(|_| ())
        .map_err(err)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::work::repo_work;
    use crate::work::tests::{origin_and_clone, run};

    /// Gives the clone its own identity, so commits don't depend on the
    /// machine's git configuration.
    fn identify(clone: &Path) {
        run(clone, &["config", "user.name", "t"]);
        run(clone, &["config", "user.email", "t@t"]);
    }

    fn head_subject(clone: &Path) -> String {
        git(Some(clone), &["log", "-1", "--format=%s"])
            .unwrap()
            .trim()
            .to_owned()
    }

    #[test]
    fn an_untracked_file_counts_its_lines_as_added() {
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        fs::write(clone.join("retry.rs"), "fn retry() {}\nfn again() {}\n").unwrap();
        let work = repo_work(&clone).unwrap();
        assert_eq!(
            totals(&work),
            Totals {
                added: 2,
                removed: 0
            }
        );
    }

    #[test]
    fn committed_and_unstaged_changes_both_count() {
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        identify(&clone);
        fs::write(clone.join("f"), "one\ntwo\n").unwrap();
        run(&clone, &["commit", "-am", "two"]);
        fs::write(clone.join("f"), "changed\n").unwrap();
        // Base has "one"; the tree now has "changed": one line each way.
        let work = repo_work(&clone).unwrap();
        assert_eq!(
            totals(&work),
            Totals {
                added: 1,
                removed: 1
            }
        );
    }

    #[test]
    fn a_clean_clone_has_no_totals() {
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        assert_eq!(totals(&repo_work(&clone).unwrap()), Totals::default());
    }

    #[test]
    fn commit_takes_every_change_and_leaves_the_tree_clean() {
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        identify(&clone);
        fs::write(clone.join("retry.rs"), "fn retry() {}\n").unwrap();
        fs::write(clone.join("f"), "changed\n").unwrap();
        assert!(commit_all(&clone, "Retry writes").unwrap());
        assert_eq!(head_subject(&clone), "Retry writes");
        assert_eq!(
            git(Some(&clone), &["status", "--porcelain"]).unwrap().trim(),
            ""
        );
    }

    #[test]
    fn commit_with_nothing_to_commit_is_not_an_error() {
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        identify(&clone);
        let before = git(Some(&clone), &["rev-parse", "HEAD"]).unwrap();
        assert!(!commit_all(&clone, "Nothing new").unwrap());
        assert_eq!(git(Some(&clone), &["rev-parse", "HEAD"]).unwrap(), before);
    }

    #[test]
    fn push_sends_the_branch_to_a_bare_remote_and_sets_upstream() {
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        identify(&clone);
        let bare = t.path().join("bare.git");
        run(t.path(), &["init", "--bare", "-b", "main", bare.to_str().unwrap()]);
        run(&clone, &["remote", "set-url", "origin", bare.to_str().unwrap()]);
        fs::write(clone.join("retry.rs"), "fn retry() {}\n").unwrap();
        commit_all(&clone, "Retry writes").unwrap();

        push_branch(&clone, "me/x").unwrap();

        let on_remote = git(
            Some(&bare),
            &["show", "me/x:retry.rs"],
        )
        .unwrap();
        assert_eq!(on_remote, "fn retry() {}\n");
        let upstream = git(
            Some(&clone),
            &["rev-parse", "--abbrev-ref", "me/x@{upstream}"],
        )
        .unwrap();
        assert_eq!(upstream.trim(), "origin/me/x");
    }

    #[test]
    fn push_to_a_missing_remote_is_an_error_naming_the_branch() {
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        run(&clone, &["remote", "set-url", "origin", t.path().join("gone.git").to_str().unwrap()]);
        match push_branch(&clone, "me/x") {
            Err(PushError::Push { branch, .. }) => assert_eq!(branch, "me/x"),
            other => panic!("a push to a missing remote should fail: {other:?}"),
        }
    }
}
