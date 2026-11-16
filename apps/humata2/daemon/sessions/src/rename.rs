//! Renaming a session's branch along with its title (FC-17, D196): the new
//! name follows the same rule as a start, the session's own branch doesn't
//! count as taken, and a branch that was ever pushed is left alone. The
//! session's folder keeps its name.

use std::path::PathBuf;

use crate::error::StartError;
use crate::git::git;
use crate::start::{RepoName, Starter, store_err};

/// One repository's branch to rename.
#[derive(Clone, Debug)]
struct RepoBranch {
    repo: RepoName,
    path: PathBuf,
    old: String,
}

/// The branches of a session and the name they would take.
#[derive(Clone, Debug)]
pub struct BranchRename {
    new: String,
    repos: Vec<RepoBranch>,
}

impl Starter {
    /// The rename `session` would get for `new_title`. Blocking: it runs git.
    pub fn plan_branch_rename(&self, session: &str, new_title: &str) -> Result<BranchRename, StartError> {
        let record = self
            .store
            .session(session)
            .map_err(store_err("read the session"))?
            .ok_or_else(|| StartError::NoSession { id: session.to_owned() })?;
        let detail = self
            .store
            .session_detail(session)
            .map_err(store_err("read the session"))?
            .ok_or_else(|| StartError::NoSession { id: session.to_owned() })?;
        let channel = self.channel_record(&record.channel)?;
        let repos: Vec<RepoName> = detail.repos.iter().map(|r| (r.owner.clone(), r.name.clone())).collect();
        let new = self.pick_excluding(&channel, &repos, new_title, Some(session))?.name;
        Ok(BranchRename {
            new,
            repos: detail
                .repos
                .into_iter()
                .map(|r| RepoBranch {
                    repo: (r.owner, r.name),
                    path: r.path,
                    old: r.branch,
                })
                .collect(),
        })
    }
}

impl BranchRename {
    /// The name every repository's branch takes.
    pub fn new_name(&self) -> &str {
        &self.new
    }

    /// Whether any branch has a different name now.
    pub fn changes(&self) -> bool {
        self.repos.iter().any(|r| r.old != self.new)
    }

    /// The `owner/name` of each repository whose branch was pushed: it has
    /// an upstream, or `origin` has a branch of that name.
    pub fn pushed(&self) -> Vec<String> {
        let mut pushed = Vec::new();
        for r in self.repos.iter().filter(|r| r.old != self.new) {
            let upstream = git(Some(&r.path), &["rev-parse", "--abbrev-ref", &format!("{}@{{upstream}}", r.old)]).is_ok();
            let remote = git(Some(&r.path), &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{}", r.old)]).is_ok();
            if upstream || remote {
                pushed.push(format!("{}/{}", r.repo.0, r.repo.1));
            }
        }
        pushed
    }

    /// Renames the branches; on a failure the ones already renamed go back.
    pub fn apply(&self) -> Result<(), StartError> {
        let mut done: Vec<&RepoBranch> = Vec::new();
        for r in self.repos.iter().filter(|r| r.old != self.new) {
            if let Err(source) = git(Some(&r.path), &["branch", "-m", &r.old, &self.new]) {
                self.roll_back(&done);
                return Err(StartError::Git { owner: r.repo.0.clone(), name: r.repo.1.clone(), source });
            }
            done.push(r);
        }
        Ok(())
    }

    /// Puts the branches back after the record couldn't be written.
    pub fn undo(&self) {
        let done: Vec<&RepoBranch> = self.repos.iter().filter(|r| r.old != self.new).collect();
        self.roll_back(&done);
    }

    fn roll_back(&self, done: &[&RepoBranch]) {
        for r in done {
            // Best effort: the failure being reported is the one that matters.
            let _ = git(Some(&r.path), &["branch", "-m", &self.new, &r.old]);
        }
    }
}
