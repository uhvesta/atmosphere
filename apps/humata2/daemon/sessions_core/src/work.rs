//! What a session's clone holds that isn't on the remote yet (LD-2): the
//! summary archive keeps, and the check before a clone is freed.

use std::fs;
use std::path::Path;

use crate::error::GitError;
use crate::git::{default_branch, git};

/// A line count for one changed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// The work in a clone that a remote doesn't have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoWork {
    pub branch: String,
    /// Changed, staged and untracked files.
    pub dirty_files: u32,
    pub unpushed_commits: u32,
    /// The branch is not on origin at all.
    pub never_pushed: bool,
    /// Where origin has the branch, when it does.
    pub last_pushed_sha: Option<String>,
    /// The diff against where the branch left the default branch.
    pub files: Vec<FileChange>,
}

impl RepoWork {
    /// Anything that would be lost if the clone were deleted.
    pub fn has_work(&self) -> bool {
        self.dirty_files > 0 || self.unpushed_commits > 0
    }
}

fn count(value: &str) -> u32 {
    value.trim().parse().unwrap_or(0)
}

/// Reads the work in `clone`. Blocking: it runs git, and fetches origin
/// first (a fetch that fails leaves the last known state of origin).
pub fn repo_work(clone: &Path) -> Result<RepoWork, GitError> {
    let branch = git(Some(clone), &["symbolic-ref", "--short", "HEAD"])
        .map(|b| b.trim().to_owned())
        .unwrap_or_else(|_| "HEAD".to_owned());
    let dirty_files = git(
        Some(clone),
        &["status", "--porcelain", "--untracked-files=all"],
    )?
    .lines()
    .filter(|l| !l.trim().is_empty())
    .count() as u32;

    let _ = git(Some(clone), &["fetch", "--quiet", "--prune", "origin"]);
    let base = default_branch(clone)?;
    let base_ref = format!("origin/{base}");
    let remote_ref = format!("refs/remotes/origin/{branch}");
    let pushed = if branch == "HEAD" {
        None
    } else {
        git(Some(clone), &["rev-parse", "--verify", "--quiet", &remote_ref])
            .ok()
            .map(|s| s.trim().to_owned())
    };
    let (never_pushed, last_pushed_sha, unpushed_commits) = match pushed {
        Some(sha) => {
            let range = format!("origin/{branch}..HEAD");
            let n = git(Some(clone), &["rev-list", "--count", &range])?;
            (false, Some(sha), count(&n))
        }
        None => {
            let range = format!("{base_ref}..HEAD");
            let n = git(Some(clone), &["rev-list", "--count", &range])?;
            (true, None, count(&n))
        }
    };

    let fork_point = git(Some(clone), &["merge-base", &base_ref, "HEAD"])?;
    let mut files = numstat(clone, fork_point.trim())?;
    let others = git(
        Some(clone),
        &["ls-files", "-z", "--others", "--exclude-standard"],
    )?;
    for path in others.split('\0').filter(|p| !p.is_empty()) {
        files.push(FileChange {
            path: path.to_owned(),
            added: untracked_lines(&clone.join(path)),
            removed: 0,
        });
    }
    Ok(RepoWork {
        branch,
        dirty_files,
        unpushed_commits,
        never_pushed,
        last_pushed_sha,
        files,
    })
}

fn numstat(clone: &Path, base: &str) -> Result<Vec<FileChange>, GitError> {
    let out = git(
        Some(clone),
        &["diff", "--numstat", "--no-renames", "-z", base],
    )?;
    Ok(out
        .split('\0')
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let added = parts.next()?;
            let removed = parts.next()?;
            let path = parts.next()?;
            Some(FileChange {
                path: path.to_owned(),
                added: added.parse().unwrap_or(0),
                removed: removed.parse().unwrap_or(0),
            })
        })
        .collect())
}

/// Lines of a text file; 0 for a binary or unreadable one.
fn untracked_lines(path: &Path) -> u32 {
    let Ok(bytes) = fs::read(path) else {
        return 0;
    };
    if bytes.contains(&0) {
        return 0;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count();
    let open_line = usize::from(!bytes.is_empty() && bytes.last() != Some(&b'\n'));
    u32::try_from(newlines + open_line).unwrap_or(u32::MAX)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::process::Command;

    use super::*;

    pub(crate) fn run(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?}");
    }

    /// A bare-enough origin with one commit on main, and a clone of it on
    /// `branch`.
    pub(crate) fn origin_and_clone(root: &Path, branch: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let origin = root.join("origin");
        fs::create_dir_all(&origin).unwrap();
        run(&origin, &["init", "-b", "main"]);
        fs::write(origin.join("f"), "one\n").unwrap();
        run(&origin, &["add", "f"]);
        run(&origin, &["commit", "-m", "one"]);
        let clone = root.join("clone");
        run(
            root,
            &["clone", origin.to_str().unwrap(), clone.to_str().unwrap()],
        );
        run(&clone, &["checkout", "--no-track", "-b", branch, "origin/main"]);
        (origin, clone)
    }

    #[test]
    fn clean_clone_has_no_work() {
        let t = tempfile::tempdir().unwrap();
        let (_o, clone) = origin_and_clone(t.path(), "me/x");
        let work = repo_work(&clone).unwrap();
        assert!(!work.has_work());
        assert!(work.never_pushed);
        assert_eq!(work.branch, "me/x");
        assert!(work.files.is_empty());
    }

    #[test]
    fn dirty_files_count_untracked_too() {
        let t = tempfile::tempdir().unwrap();
        let (_o, clone) = origin_and_clone(t.path(), "me/x");
        fs::write(clone.join("f"), "one\ntwo\n").unwrap();
        fs::write(clone.join("new"), "a\nb\nc").unwrap();
        fs::write(clone.join("bin"), [0u8, 1, 2]).unwrap();
        let work = repo_work(&clone).unwrap();
        assert_eq!(work.dirty_files, 3);
        assert!(work.has_work());
        let change = |p: &str| work.files.iter().find(|f| f.path == p).unwrap().clone();
        assert_eq!((change("f").added, change("f").removed), (1, 0));
        assert_eq!(change("new").added, 3);
        assert_eq!(change("bin").added, 0);
    }

    #[test]
    fn unpushed_commits_on_a_never_pushed_branch_count_from_main() {
        let t = tempfile::tempdir().unwrap();
        let (_o, clone) = origin_and_clone(t.path(), "me/x");
        fs::write(clone.join("g"), "g\n").unwrap();
        run(&clone, &["add", "g"]);
        run(&clone, &["commit", "-m", "two"]);
        let work = repo_work(&clone).unwrap();
        assert!(work.never_pushed);
        assert_eq!(work.unpushed_commits, 1);
        assert_eq!(work.dirty_files, 0);
        assert!(work.has_work());
        assert_eq!(work.files.len(), 1);
    }

    #[test]
    fn pushed_branch_counts_only_what_came_after() {
        let t = tempfile::tempdir().unwrap();
        let (_o, clone) = origin_and_clone(t.path(), "me/x");
        fs::write(clone.join("g"), "g\n").unwrap();
        run(&clone, &["add", "g"]);
        run(&clone, &["commit", "-m", "two"]);
        run(&clone, &["push", "origin", "me/x"]);
        let pushed = work_sha(&clone);
        fs::write(clone.join("h"), "h\n").unwrap();
        run(&clone, &["add", "h"]);
        run(&clone, &["commit", "-m", "three"]);
        let work = repo_work(&clone).unwrap();
        assert!(!work.never_pushed);
        assert_eq!(work.unpushed_commits, 1);
        assert_eq!(work.last_pushed_sha.as_deref(), Some(pushed.as_str()));
        // The diff still runs from where the branch left main.
        assert_eq!(work.files.len(), 2);
    }

    fn work_sha(clone: &Path) -> String {
        git(Some(clone), &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_owned()
    }
}
