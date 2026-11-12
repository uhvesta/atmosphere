//! A session's work landed on a branch of the person's naming, and its base
//! branch merged in. Both commit as the person git is set up for, not as
//! Humata, since what they make is shared.

use std::path::Path;

use super::{Checkout, WorkspaceError, git, git_as_person};

/// One checkout after landing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Landed {
    /// The checkout, now on the branch landed on.
    pub checkout: Checkout,
    /// The commit made of its changes; none if it had none, having been
    /// landed on the branch before.
    pub commit: Option<String>,
    /// The branch its work goes into, from [`base`].
    pub base: String,
}

/// What merging a base branch into a checkout did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Merged {
    /// The checkout had it all already.
    UpToDate,
    Merged,
    /// The merge stopped with these files conflicting, left marked in the
    /// worktree for whoever resolves them.
    Conflicts(Vec<String>),
}

/// Commits each checkout's changes on `branch`, with `message`. A checkout
/// with changes moves onto the branch first; one without is left out unless
/// it was landed on the branch before, so a repository the session didn't
/// change gets no branch. Nothing is changed unless every checkout can land.
pub async fn land(
    checkouts: &[Checkout],
    branch: &str,
    message: &str,
) -> Result<Vec<Landed>, WorkspaceError> {
    let Some(first) = checkouts.first() else {
        return Ok(Vec::new());
    };
    if git(&first.path, &["check-ref-format", "--branch", branch])
        .await
        .is_err()
    {
        return Err(WorkspaceError::BadBranch {
            branch: branch.to_owned(),
        });
    }
    let mut landing = Vec::with_capacity(checkouts.len());
    for checkout in checkouts {
        if !checkout.path.exists() {
            return Err(WorkspaceError::NoCopy {
                repository: checkout.name.clone(),
            });
        }
        let changed = !git(&checkout.path, &["status", "--porcelain"])
            .await?
            .trim()
            .is_empty();
        // The worktree's own branch, not the record's: a land that failed
        // after renaming it is then landed again on the same name.
        let current = branch_of(&checkout.path)
            .await
            .unwrap_or_else(|| checkout.branch.clone());
        if !changed && current != branch {
            continue;
        }
        if current != branch && has_branch(&checkout.path, branch).await {
            return Err(WorkspaceError::BranchTaken {
                branch: branch.to_owned(),
                repository: checkout.name.clone(),
            });
        }
        if changed {
            identity(&checkout.path).await?;
        }
        landing.push((checkout, current, changed, base(checkout).await?));
    }
    let mut landed = Vec::with_capacity(landing.len());
    for (checkout, current, changed, base) in landing {
        if current != branch {
            git(&checkout.path, &["branch", "-m", branch]).await?;
        }
        let commit = if changed {
            git(&checkout.path, &["add", "--all"]).await?;
            git_as_person(&checkout.path, &["commit", "--quiet", "-m", message]).await?;
            let head = git(&checkout.path, &["rev-parse", "HEAD"]).await?;
            Some(head.trim().to_owned())
        } else {
            None
        };
        landed.push(Landed {
            checkout: Checkout {
                branch: branch.to_owned(),
                ..checkout.clone()
            },
            commit,
            base,
        });
    }
    Ok(landed)
}

/// The branch `checkout`'s work goes into: the one its `origin` names as
/// its default, else the one its repository has checked out.
pub async fn base(checkout: &Checkout) -> Result<String, WorkspaceError> {
    if let Ok(named) = git(
        &checkout.path,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--verify",
            "--quiet",
            "origin/HEAD",
        ],
    )
    .await
        && let Some(branch) = named.trim().strip_prefix("origin/")
        && !branch.is_empty()
    {
        return Ok(branch.to_owned());
    }
    match git(&checkout.repository, &["symbolic-ref", "--short", "HEAD"]).await {
        Ok(branch) if !branch.trim().is_empty() => Ok(branch.trim().to_owned()),
        _ => Err(WorkspaceError::NoBase {
            repository: checkout.name.clone(),
        }),
    }
}

/// What to merge into `checkout` to bring its base up to date: origin's
/// copy of the base, as last fetched, if it has one; else the base.
pub async fn upstream(checkout: &Checkout) -> Result<String, WorkspaceError> {
    let base = base(checkout).await?;
    let remote = format!("refs/remotes/origin/{base}");
    if git(
        &checkout.path,
        &["rev-parse", "--verify", "--quiet", &remote],
    )
    .await
    .is_ok()
    {
        return Ok(format!("origin/{base}"));
    }
    Ok(base)
}

/// Merges `upstream`, a branch or a remote's, as `origin/main`, into
/// `checkout`, keeping changes it hasn't committed: they're put aside and
/// back, and if putting them back conflicts, those files are the conflicts.
pub async fn merge(checkout: &Checkout, upstream: &str) -> Result<Merged, WorkspaceError> {
    if !checkout.path.exists() {
        return Err(WorkspaceError::NoCopy {
            repository: checkout.name.clone(),
        });
    }
    identity(&checkout.path).await?;
    let before = git(&checkout.path, &["rev-parse", "HEAD"]).await?;
    let merged = git_as_person(
        &checkout.path,
        &["merge", "--autostash", "--no-edit", "--quiet", upstream],
    )
    .await;
    let conflicts = conflicts(&checkout.path).await?;
    if !conflicts.is_empty() {
        return Ok(Merged::Conflicts(conflicts));
    }
    merged?;
    let after = git(&checkout.path, &["rev-parse", "HEAD"]).await?;
    Ok(if before == after {
        Merged::UpToDate
    } else {
        Merged::Merged
    })
}

/// The branch the worktree at `directory` has checked out; None when it
/// can't be read or HEAD is detached.
pub async fn branch_of(directory: &Path) -> Option<String> {
    let branch = git(directory, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .ok()?;
    let branch = branch.trim();
    (!branch.is_empty()).then(|| branch.to_owned())
}

/// Deletes `branch` from the repository at `repository`; one already gone
/// is fine.
pub async fn delete_branch(repository: &Path, branch: &str) -> Result<(), WorkspaceError> {
    if !has_branch(repository, branch).await {
        return Ok(());
    }
    git(repository, &["branch", "-D", branch]).await?;
    Ok(())
}

async fn has_branch(directory: &Path, branch: &str) -> bool {
    git(
        directory,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .await
    .is_ok()
}

/// The files with unmerged changes in the worktree at `directory`.
async fn conflicts(directory: &Path) -> Result<Vec<String>, WorkspaceError> {
    let listed = git(directory, &["diff", "--name-only", "--diff-filter=U"]).await?;
    Ok(listed.lines().map(str::to_owned).collect())
}

/// Fails unless git knows who commits in `directory`, from its config or
/// humatad's variables.
async fn identity(directory: &Path) -> Result<(), WorkspaceError> {
    let set = |name| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    if set("GIT_AUTHOR_EMAIL") && set("GIT_AUTHOR_NAME") {
        return Ok(());
    }
    let email = git(directory, &["config", "user.email"]).await;
    let name = git(directory, &["config", "user.name"]).await;
    match (email, name) {
        (Ok(email), Ok(name)) if !email.trim().is_empty() && !name.trim().is_empty() => Ok(()),
        _ => Err(WorkspaceError::NoIdentity {
            directory: directory.to_owned(),
        }),
    }
}
