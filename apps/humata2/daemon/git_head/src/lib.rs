//! The branch a clone's `origin/HEAD` names.

use std::path::Path;
use std::process::Command;

/// The branch `origin/HEAD` names in the clone at `path`, as `origin/main`, or
/// `None` when it isn't set. `git remote set-head origin --auto` sets it.
#[must_use]
pub fn origin_head(path: &Path) -> Option<String> {
    origin_head_with(Path::new("git"), path)
}

/// As [`origin_head`], running the git at `program`.
#[must_use]
pub fn origin_head_with(program: &Path, path: &Path) -> Option<String> {
    let output = Command::new(program)
        .current_dir(path)
        .args(["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim_end().to_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git").current_dir(dir).args(args).status().unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn an_unset_origin_head_is_none() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        assert_eq!(origin_head(dir.path()), None);
    }

    #[test]
    fn a_set_origin_head_names_its_branch_with_the_remote() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(
            dir.path(),
            &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk"],
        );
        assert_eq!(origin_head(dir.path()).as_deref(), Some("origin/trunk"));
    }
}
