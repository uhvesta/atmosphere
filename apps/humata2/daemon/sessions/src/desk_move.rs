//! The git side of moving a Desk session into a channel (DK-15..DK-17,
//! D305): read the Desk folder's repos for the plan, pick the branch the
//! session takes in the channel, and once the folder has landed point each
//! landed clone at the channel. The clone is the one that was copied, never
//! one made again from the channel's main clone, so every commit and every
//! uncommitted file comes along. The Desk folder itself is never changed
//! here.

use std::fs;
use std::path::{Path, PathBuf};

use crate::desk::{origin_url, owner_and_name};
use crate::error::{GitError, StartError};
use crate::git::{default_branch, git, remote_url};
use crate::names::Branch;
use crate::start::{RepoName, Starter};

/// Why reading a Desk folder's repos or pointing a landed clone at the
/// channel failed.
#[derive(Debug, thiserror::Error)]
pub enum DeskMoveError {
    #[error(
        "could not read the repository at {path}: {source}; check it is a git clone git can \
         open, or move it out of the Desk folder before moving the session"
    )]
    Read {
        path: PathBuf,
        #[source]
        source: GitError,
    },
    #[error(
        "could not {step} in the moved clone at {path}: {source}. The move is taken back and \
         the session stays on the Desk; fix what git says, then move the session again"
    )]
    Repoint {
        path: PathBuf,
        step: &'static str,
        #[source]
        source: GitError,
    },
}

/// One repository of a Desk folder, as the move finds it (DK-16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeskMoveRepo {
    /// From the clone's origin.
    pub owner: String,
    pub name: String,
    /// The clone, directly under the Desk folder.
    pub path: PathBuf,
    /// The commit HEAD is at: the session's branch starts here.
    pub head: String,
    /// The branch HEAD is on; `None` when detached.
    pub head_branch: Option<String>,
    /// Changed, staged and untracked files, which the move carries.
    pub uncommitted: u32,
}

/// The repositories directly under `folder`, sorted by owner and name, each
/// with its HEAD and how many files it has uncommitted. Runs git in each,
/// never fetching. A folder with no readable origin is not a repository the
/// channel can hold, so it comes along as plain files and is not listed.
pub fn desk_move_repos(folder: &Path) -> Result<Vec<DeskMoveRepo>, DeskMoveError> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Ok(Vec::new());
    };
    let mut repos = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir());
        if !is_dir || !path.join(".git").is_dir() {
            continue;
        }
        let named = fs::read_to_string(path.join(".git").join("config"))
            .ok()
            .and_then(|config| origin_url(&config))
            .and_then(|url| owner_and_name(&url));
        let Some((owner, name)) = named else {
            continue;
        };
        let read = |source| DeskMoveError::Read { path: path.clone(), source };
        let head = git(Some(&path), &["rev-parse", "HEAD"]).map_err(read)?.trim().to_owned();
        let head_branch = git(Some(&path), &["branch", "--show-current"]).map_err(read)?.trim().to_owned();
        let status = git(Some(&path), &["status", "--porcelain", "--untracked-files=all"]).map_err(read)?;
        let uncommitted = status.lines().filter(|line| !line.trim().is_empty()).count();
        repos.push(DeskMoveRepo {
            owner,
            name,
            head,
            head_branch: Some(head_branch).filter(|branch| !branch.is_empty()),
            uncommitted: u32::try_from(uncommitted).unwrap_or(u32::MAX),
            path,
        });
    }
    repos.sort_by(|a, b| (&a.owner, &a.name).cmp(&(&b.owner, &b.name)));
    Ok(repos)
}

/// Points the landed clone at `clone` at the channel: a new branch `branch`
/// from the commit HEAD is at, every uncommitted file kept and nothing
/// tracked; `origin` set to the channel's `remote`; and the local default
/// branch put back on origin's when it holds commits origin lacks, so the
/// session's work is on its own branch only. Done again on a clone already
/// on `branch`, it changes nothing more.
pub fn repoint_clone(clone: &Path, branch: &str, remote: &str) -> Result<(), DeskMoveError> {
    let failed = |step: &'static str| move |source| DeskMoveError::Repoint { path: clone.to_owned(), step, source };
    // Read before origin changes: the default is the old origin's HEAD.
    let default = default_branch(clone).ok().filter(|default| default != branch);
    let on = git(Some(clone), &["branch", "--show-current"]).map_err(failed("read the branch it is on"))?;
    if on.trim() != branch {
        git(Some(clone), &["checkout", "--no-track", "-b", branch]).map_err(failed("put it on the session's branch"))?;
    }
    git(Some(clone), &["remote", "set-url", "origin", remote]).map_err(failed("point origin at the channel's remote"))?;
    let Some(default) = default else {
        return Ok(());
    };
    let local = format!("refs/heads/{default}");
    let upstream = format!("refs/remotes/origin/{default}");
    let has = |name: &str| git(Some(clone), &["rev-parse", "--verify", "--quiet", name]).is_ok();
    if !has(&local) || !has(&upstream) {
        return Ok(());
    }
    let ahead = git(Some(clone), &["rev-list", "--count", &format!("{upstream}..{local}")])
        .map_err(failed("count the default branch's unpushed commits"))?;
    if ahead.trim() != "0" {
        git(Some(clone), &["branch", "-f", &default, &upstream]).map_err(failed("put the default branch back on origin's"))?;
    }
    Ok(())
}

impl Starter {
    /// The branch a Desk session moved into `channel` takes: `typed` when
    /// given and free, else the channel's prefix and the session's `title`
    /// as a slug, the first name free in the channel's main clones, its
    /// remotes and its other sessions (FC-11..FC-14). Repos the channel
    /// doesn't have cloned yet (the move adds them) are left out of the
    /// check.
    pub fn desk_move_branch(
        &self,
        channel: &str,
        repos: &[RepoName],
        title: &str,
        typed: Option<&str>,
    ) -> Result<Branch, StartError> {
        let channel = self.channel_record(channel)?;
        let cloned: Vec<RepoName> = repos
            .iter()
            .filter(|(_, name)| self.layout.main_clone(&channel.folder, name).exists())
            .cloned()
            .collect();
        self.pick_fork(&channel, &cloned, title, typed)
    }

    /// The remote a clone of `owner/name` moved into the channel whose
    /// folder is `channel_folder` points at: the channel's main clone's
    /// origin, else the one the remote template names.
    pub fn desk_move_remote(&self, channel_folder: &str, owner: &str, name: &str) -> Result<String, GitError> {
        let main = self.layout.main_clone(channel_folder, name);
        if main.join(".git").exists()
            && let Ok(url) = git(Some(&main), &["remote", "get-url", "origin"])
            && !url.trim().is_empty()
        {
            return Ok(url.trim().to_owned());
        }
        remote_url(&self.remote_template, owner, name)
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::work::tests::run;

    /// A clone `<t>/desk/api` of `<t>/remotes/acme/api`, which has one commit
    /// on `main`.
    fn desk_clone(t: &Path) -> PathBuf {
        let origin = t.join("remotes/acme/api");
        fs::create_dir_all(&origin).unwrap();
        run(&origin, &["init", "-b", "main"]);
        fs::write(origin.join("README"), "acme/api\n").unwrap();
        run(&origin, &["add", "README"]);
        run(&origin, &["commit", "-m", "one"]);
        fs::create_dir_all(t.join("desk")).unwrap();
        let clone = t.join("desk/api");
        run(t, &["clone", origin.to_str().unwrap(), clone.to_str().unwrap()]);
        clone
    }

    fn git_out(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git").current_dir(dir).args(args).output().unwrap();
        assert!(output.status.success(), "git {args:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn a_desk_folders_repos_are_read_with_their_head_and_uncommitted_files() {
        let t = tempfile::tempdir().unwrap();
        let clone = desk_clone(t.path());
        fs::create_dir_all(t.path().join("desk/notes")).unwrap();
        fs::write(clone.join("wip.txt"), "wip\n").unwrap();
        fs::write(clone.join("README"), "changed\n").unwrap();

        let repos = desk_move_repos(&t.path().join("desk")).unwrap();

        assert_eq!(
            repos,
            vec![DeskMoveRepo {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
                path: clone.clone(),
                head: git_out(&clone, &["rev-parse", "HEAD"]),
                head_branch: Some("main".to_owned()),
                uncommitted: 2,
            }]
        );
    }

    #[test]
    fn a_repointed_clone_is_on_its_branch_at_its_commit_with_its_files_and_the_channels_origin() {
        let t = tempfile::tempdir().unwrap();
        let clone = desk_clone(t.path());
        fs::write(clone.join("wip.txt"), "wip\n").unwrap();
        run(&clone, &["commit", "--allow-empty", "-m", "unpushed"]);
        let head = git_out(&clone, &["rev-parse", "HEAD"]);
        let pushed = git_out(&clone, &["rev-parse", "origin/main"]);

        repoint_clone(&clone, "me/plan-q4", "https://example.invalid/acme/api.git").unwrap();

        assert_eq!(git_out(&clone, &["branch", "--show-current"]), "me/plan-q4");
        assert_eq!(git_out(&clone, &["rev-parse", "HEAD"]), head);
        assert_eq!(fs::read_to_string(clone.join("wip.txt")).unwrap(), "wip\n");
        assert_eq!(git_out(&clone, &["remote", "get-url", "origin"]), "https://example.invalid/acme/api.git");
        assert_eq!(git_out(&clone, &["rev-parse", "main"]), pushed);
        let upstream = Command::new("git")
            .current_dir(&clone)
            .args(["rev-parse", "--abbrev-ref", "me/plan-q4@{upstream}"])
            .output()
            .unwrap();
        assert!(!upstream.status.success(), "the new branch tracks nothing");
    }

    #[test]
    fn repointing_onto_a_branch_the_clone_already_has_is_refused_and_leaves_it_on_its_branch() {
        let t = tempfile::tempdir().unwrap();
        let clone = desk_clone(t.path());
        run(&clone, &["branch", "me/plan-q4"]);
        run(&clone, &["switch", "-c", "other"]);

        let refused = repoint_clone(&clone, "me/plan-q4", "https://example.invalid/acme/api.git");

        assert!(matches!(refused, Err(DeskMoveError::Repoint { .. })), "{refused:?}");
        assert_eq!(git_out(&clone, &["branch", "--show-current"]), "other");
    }
}
