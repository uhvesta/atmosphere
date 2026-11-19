//! Bringing a session's clones back after they were freed (LD-4): from the
//! branch on the remote when it was pushed, else from the default branch.

use std::path::Path;

use humata2_store::{SessionDetail, SessionRecord};

use crate::error::{GitError, StartError};
use crate::git::{default_branch, git, in_partial, remote_url, text};
use crate::start::{Starter, blocking};

/// Where a restored clone's branch came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoredFrom {
    /// The branch was on the remote.
    Branch,
    /// It never was, so it starts again from the default branch.
    Main,
}

/// Clones `main_clone` into `dest` through `<dest>.partial`, points origin at
/// `remote_url`, and checks out `branch` from the remote's copy when it has
/// one, else as a new branch from the default branch. A failure leaves
/// nothing behind. Blocking.
pub fn restore_clone(
    main_clone: &Path,
    remote_url: &str,
    dest: &Path,
    branch: &str,
) -> Result<RestoredFrom, GitError> {
    let base = default_branch(main_clone)?;
    let mut from = RestoredFrom::Main;
    in_partial(dest, |tmp| {
        git(None, &["clone", "--", text(main_clone)?, text(tmp)?])?;
        git(Some(tmp), &["remote", "set-url", "origin", remote_url])?;
        git(Some(tmp), &["fetch", "--quiet", "--prune", "origin"])?;
        let remote_ref = format!("refs/remotes/origin/{branch}");
        let pushed = git(Some(tmp), &["rev-parse", "--verify", "--quiet", &remote_ref]).is_ok();
        let start = if pushed {
            from = RestoredFrom::Branch;
            format!("origin/{branch}")
        } else {
            format!("origin/{base}")
        };
        git(Some(tmp), &["checkout", "--no-track", "-B", branch, &start])?;
        Ok(())
    })?;
    Ok(from)
}

impl Starter {
    /// Clones each repository of `detail` that has no clone in the session
    /// folder, on its branch (LD-4). Returns where each branch came from.
    pub async fn reclone(
        &self,
        session: &SessionRecord,
        detail: &SessionDetail,
    ) -> Result<Vec<(String, RestoredFrom)>, StartError> {
        let this = self.clone();
        let channel_id = session.channel.clone();
        let channel = blocking(move || this.channel_record(&channel_id)).await??;
        let mut restored = Vec::new();
        for repo in &detail.repos {
            if repo.path.join(".git").exists() {
                continue;
            }
            let git_err = |source| StartError::Git {
                owner: repo.owner.clone(),
                name: repo.name.clone(),
                source,
            };
            let main = self.layout.main_clone(&channel.folder, &repo.name);
            let url =
                remote_url(&self.remote_template, &repo.owner, &repo.name).map_err(git_err)?;
            let (path, branch) = (repo.path.clone(), repo.branch.clone());
            let from = blocking(move || restore_clone(&main, &url, &path, &branch))
                .await?
                .map_err(git_err)?;
            restored.push((repo.name.clone(), from));
        }
        Ok(restored)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::layout::partial;
    use crate::work::tests::{origin_and_clone, run};

    fn url(p: &Path) -> String {
        format!("file://{}", p.display())
    }

    fn head(repo: &Path) -> String {
        git(Some(repo), &["symbolic-ref", "--short", "HEAD"])
            .unwrap()
            .trim()
            .to_owned()
    }

    #[test]
    fn pushed_branch_comes_back_with_its_commits() {
        let t = tempfile::tempdir().unwrap();
        let (origin, clone) = origin_and_clone(t.path(), "me/x");
        fs::write(clone.join("g"), "g\n").unwrap();
        run(&clone, &["add", "g"]);
        run(&clone, &["commit", "-m", "two"]);
        run(&clone, &["push", "origin", "me/x"]);
        let dest = t.path().join("sessions/s/api");
        let from = restore_clone(&clone, &url(&origin), &dest, "me/x").unwrap();
        assert_eq!(from, RestoredFrom::Branch);
        assert_eq!(head(&dest), "me/x");
        assert!(dest.join("g").is_file());
        assert!(!partial(&dest).exists());
    }

    #[test]
    fn never_pushed_branch_starts_from_main() {
        let t = tempfile::tempdir().unwrap();
        let (origin, clone) = origin_and_clone(t.path(), "me/x");
        let dest = t.path().join("sessions/s/api");
        let from = restore_clone(&clone, &url(&origin), &dest, "me/y").unwrap();
        assert_eq!(from, RestoredFrom::Main);
        assert_eq!(head(&dest), "me/y");
        assert!(dest.join("f").is_file());
    }

    #[test]
    fn unreachable_remote_leaves_nothing() {
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        let dest = t.path().join("sessions/s/api");
        let bad = url(&t.path().join("missing"));
        assert!(restore_clone(&clone, &bad, &dest, "me/x").is_err());
        assert!(!dest.exists());
        assert!(!partial(&dest).exists());
    }
}
