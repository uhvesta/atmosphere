//! A session's own copy of its code.
//!
//! Each session that works in repositories gets a git worktree of each, on a
//! branch of its own, under one folder per session. After every turn each
//! worktree is saved as a hidden snapshot: a commit object that no branch
//! points to, kept alive by a ref under `refs/humata/<session>/<turn>/`. The
//! session's branch, its index and its history are never touched, so what to
//! commit stays the person's call. A fork starts a new worktree at the
//! snapshot's HEAD and puts the snapshot's files in it.

pub mod bundle;
mod error;
mod land;
mod review;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

pub use error::WorkspaceError;
pub use land::{
    Landed, Merged, base, branch_of, delete_branch, held_elsewhere, holds_only, land, merge,
    upstream,
};
pub use review::{
    Action, Changes, Edit, FileDiff, FileRef, FileStatus, Hunk, HunkAt, Line, LineKind, Query,
    Range,
};

/// A repository a session works in, as the project names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    /// Its folder's name in the session's copy; unique within a session.
    pub name: String,
    /// The repository's own folder on the host.
    pub path: PathBuf,
}

/// A session's worktree of one repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkout {
    pub name: String,
    /// The repository it is a worktree of.
    pub repository: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    /// The session that made it. A fork of the chat only works in its
    /// parent's checkouts, which are the parent's to remove.
    pub owner: String,
}

/// One repository's code at a point in a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    /// The commit the worktree was on.
    pub head: String,
    /// A commit, on no branch, of every file in the worktree that git
    /// doesn't ignore, with `head` as its parent.
    pub commit: String,
}

/// The sessions' copies of their repositories, under one folder.
#[derive(Debug)]
pub struct Workspaces {
    root: PathBuf,
}

/// Names the temporary index of each snapshot, so two at once don't share
/// one.
static INDEXES: AtomicU64 = AtomicU64::new(0);

impl Workspaces {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The folder a session works in: its one repository's worktree, or the
    /// folder holding a worktree of each.
    pub fn working_directory(&self, session: &str, repositories: &[Repository]) -> PathBuf {
        let folder = self.root.join(session);
        match repositories {
            [only] => folder.join(&only.name),
            _ => folder,
        }
    }

    /// Makes `session`'s worktree of each repository. One with a snapshot in
    /// `from` starts at its HEAD with its files; the others start at the
    /// repository's HEAD. A worktree left by an attempt that stopped part way
    /// is used again.
    pub async fn create(
        &self,
        session: &str,
        repositories: &[Repository],
        from: &[Snapshot],
    ) -> Result<Vec<Checkout>, WorkspaceError> {
        let folder = self.root.join(session);
        tokio::fs::create_dir_all(&folder).await.map_err(|source| {
            WorkspaceError::CreateDirectory {
                path: folder.clone(),
                source,
            }
        })?;
        let mut checkouts = Vec::with_capacity(repositories.len());
        for repository in repositories {
            let snapshot = from
                .iter()
                .find(|snapshot| snapshot.name == repository.name);
            checkouts.push(check_out(session, &folder, repository, snapshot).await?);
        }
        Ok(checkouts)
    }

    /// Saves each checkout as it is now, as the snapshot `turn` of
    /// `session`.
    pub async fn snapshot(
        &self,
        session: &str,
        turn: usize,
        checkouts: &[Checkout],
    ) -> Result<Vec<Snapshot>, WorkspaceError> {
        self.take(session, &turn.to_string(), checkouts).await
    }

    /// Saves each checkout as it is now, part way through a turn, as the
    /// session's `saved`th such save. These are kept apart from the turns'
    /// snapshots, so what a turn changed is still from one turn's end to the
    /// next. None when no checkout changed since `last`, the code as last
    /// saved: nothing is committed then.
    pub async fn save(
        &self,
        session: &str,
        saved: usize,
        checkouts: &[Checkout],
        last: &[Snapshot],
    ) -> Result<Option<Vec<Snapshot>>, WorkspaceError> {
        let label = format!("saved/{saved}");
        let mut snapshots = Vec::with_capacity(checkouts.len());
        let mut changed = false;
        for (position, checkout) in checkouts.iter().enumerate() {
            let before = last.iter().find(|snapshot| snapshot.name == checkout.name);
            let (head, tree) = self.tree(&checkout.path).await?;
            if let Some(before) = before
                && before.head == head
                && git(
                    &checkout.path,
                    &["rev-parse", &format!("{}^{{tree}}", before.commit)],
                )
                .await
                .is_ok_and(|was| was.trim() == tree)
            {
                snapshots.push(before.clone());
                continue;
            }
            changed = true;
            let commit = commit(session, &checkout.path, &head, &tree).await?;
            let name = format!("refs/humata/{session}/{label}/{position}");
            git(&checkout.path, &["update-ref", &name, &commit]).await?;
            snapshots.push(Snapshot {
                name: checkout.name.clone(),
                head,
                commit,
            });
        }
        Ok(changed.then_some(snapshots))
    }

    /// Saves each checkout as it is now as the session's `rewind`th rewind,
    /// so undoing it can put the files back.
    pub async fn save_rewound(
        &self,
        session: &str,
        rewind: usize,
        checkouts: &[Checkout],
    ) -> Result<Vec<Snapshot>, WorkspaceError> {
        self.take(session, &format!("rewound/{rewind}"), checkouts)
            .await
    }

    /// Puts each checkout back as its snapshot in `to` has it: its branch at
    /// the snapshot's HEAD, and its files, with what the session had changed
    /// unstaged. Files made since go; ones git ignores stay. A checkout with
    /// no snapshot in `to` is left as it is.
    pub async fn restore(
        &self,
        checkouts: &[Checkout],
        to: &[Snapshot],
    ) -> Result<(), WorkspaceError> {
        for checkout in checkouts {
            let Some(snapshot) = to.iter().find(|snapshot| snapshot.name == checkout.name) else {
                continue;
            };
            let path = &checkout.path;
            git(path, &["reset", "--quiet", "--hard", &snapshot.head]).await?;
            git(path, &["clean", "--quiet", "--force", "-d"]).await?;
            git(path, &["read-tree", "-u", "--reset", &snapshot.commit]).await?;
            git(path, &["reset", "--quiet"]).await?;
        }
        Ok(())
    }

    async fn take(
        &self,
        session: &str,
        label: &str,
        checkouts: &[Checkout],
    ) -> Result<Vec<Snapshot>, WorkspaceError> {
        let mut snapshots = Vec::with_capacity(checkouts.len());
        for (position, checkout) in checkouts.iter().enumerate() {
            let (head, commit) = self.capture(session, &checkout.path).await?;
            // By position: a folder's name needn't be a valid ref's.
            let name = format!("refs/humata/{session}/{label}/{position}");
            git(&checkout.path, &["update-ref", &name, &commit]).await?;
            snapshots.push(Snapshot {
                name: checkout.name.clone(),
                head,
                commit,
            });
        }
        Ok(snapshots)
    }

    /// Removes the worktrees, with whatever is in them that wasn't
    /// snapshotted. Their branches and snapshots stay.
    pub async fn remove(&self, checkouts: &[Checkout]) -> Result<(), WorkspaceError> {
        for checkout in checkouts {
            if checkout.path.exists() {
                let path = checkout.path.to_string_lossy();
                git(
                    &checkout.repository,
                    &["worktree", "remove", "--force", &path],
                )
                .await?;
            } else {
                git(&checkout.repository, &["worktree", "prune"]).await?;
            }
            // The session's folder goes with its last worktree.
            if let Some(folder) = checkout.path.parent()
                && folder.starts_with(&self.root)
                && folder != self.root
            {
                let _ = tokio::fs::remove_dir(folder).await;
            }
        }
        Ok(())
    }

    /// Commits every file in the worktree at `path` that git doesn't ignore,
    /// through an index of its own; returns the HEAD and the commit.
    async fn capture(
        &self,
        session: &str,
        path: &Path,
    ) -> Result<(String, String), WorkspaceError> {
        let (head, tree) = self.tree(path).await?;
        let commit = commit(session, path, &head, &tree).await?;
        Ok((head, commit))
    }

    /// The HEAD of the worktree at `path`, and the tree of every file in it
    /// that git doesn't ignore, written through an index of its own.
    async fn tree(&self, path: &Path) -> Result<(String, String), WorkspaceError> {
        let head = git(path, &["rev-parse", "--verify", "HEAD"]).await?;
        let head = head.trim().to_owned();
        let index = self.root.join(format!(
            ".index-{}-{}",
            std::process::id(),
            INDEXES.fetch_add(1, Ordering::Relaxed)
        ));
        let captured = async {
            // Starting from the worktree's own index lets git skip the files
            // it already knows haven't changed.
            let own = git(path, &["rev-parse", "--git-path", "index"]).await?;
            let own = path.join(own.trim());
            if tokio::fs::copy(&own, &index).await.is_err() {
                git_with_index(path, &index, &["read-tree", "HEAD"]).await?;
            }
            git_with_index(path, &index, &["add", "--all"]).await?;
            let tree = git_with_index(path, &index, &["write-tree"]).await?;
            Ok((head, tree.trim().to_owned()))
        }
        .await;
        let _ = tokio::fs::remove_file(&index).await;
        captured
    }
}

/// Commits `tree` onto `head` in the repository at `path`, as a snapshot of
/// `session`.
async fn commit(
    session: &str,
    path: &Path,
    head: &str,
    tree: &str,
) -> Result<String, WorkspaceError> {
    let message = format!("Humata snapshot of session {session}");
    let commit = git(path, &["commit-tree", tree, "-p", head, "-m", &message]).await?;
    Ok(commit.trim().to_owned())
}

/// Deletes `session`'s snapshot refs and the branch its worktrees were made
/// on from the repository or mirror at `repository`, so git can drop what
/// only they kept. One that is gone, or holds neither, is fine.
pub async fn forget(repository: &Path, session: &str) -> Result<(), WorkspaceError> {
    if !repository.exists() {
        return Ok(());
    }
    let prefix = format!("refs/humata/{session}/");
    let listed = git(
        repository,
        &["for-each-ref", "--format=%(refname)", &prefix],
    )
    .await?;
    let deletes: String = listed
        .lines()
        .filter(|name| !name.is_empty())
        .map(|name| format!("delete {name}\n"))
        .collect();
    if !deletes.is_empty() {
        git_with_input(repository, &["update-ref", "--stdin"], &deletes).await?;
    }
    land::delete_branch(repository, &branch(session)).await
}

/// The branch a session's worktrees are on.
fn branch(session: &str) -> String {
    let short: String = session.chars().take(8).collect();
    format!("humata/{short}")
}

async fn check_out(
    session: &str,
    folder: &Path,
    repository: &Repository,
    snapshot: Option<&Snapshot>,
) -> Result<Checkout, WorkspaceError> {
    let path = folder.join(&repository.name);
    let branch = branch(session);
    if let Err(error) = git(&repository.path, &["rev-parse", "--git-dir"]).await {
        return Err(match error {
            WorkspaceError::Git { .. } => WorkspaceError::NotARepository {
                path: repository.path.clone(),
            },
            error => error,
        });
    }
    let start = match snapshot {
        Some(snapshot) => snapshot.head.clone(),
        None => match git(&repository.path, &["rev-parse", "--verify", "HEAD"]).await {
            Ok(head) => head.trim().to_owned(),
            Err(WorkspaceError::Git { .. }) => {
                return Err(WorkspaceError::NoCommits {
                    path: repository.path.clone(),
                });
            }
            Err(error) => return Err(error),
        },
    };
    let target = path.to_string_lossy();
    let made = !path.exists();
    if !made {
        if !is_worktree_at(&path).await {
            return Err(WorkspaceError::InTheWay {
                path: path.clone(),
                repository: repository.name.clone(),
            });
        }
    } else {
        // A worktree whose folder was deleted (a cleared cache, say) stays
        // registered, and git won't add another at its path until it's
        // pruned.
        git(&repository.path, &["worktree", "prune"]).await?;
        let reference = format!("refs/heads/{branch}");
        let branched = git(
            &repository.path,
            &["rev-parse", "--verify", "--quiet", &reference],
        )
        .await
        .is_ok();
        if branched {
            git(&repository.path, &["worktree", "add", &target, &branch]).await?;
        } else {
            git(
                &repository.path,
                &["worktree", "add", "-b", &branch, &target, &start],
            )
            .await?;
        }
    }
    // A worktree that was already there keeps what's in it: its edits may
    // be newer than the snapshot.
    if let Some(snapshot) = snapshot.filter(|_| made) {
        // The snapshot's files, with the index left at HEAD, so what the
        // session had changed is still unstaged.
        let applied = async {
            git(&path, &["read-tree", "-u", "--reset", &snapshot.commit]).await?;
            git(&path, &["reset", "--quiet"]).await
        }
        .await;
        if let Err(error) = applied {
            // Not left half made, so trying again applies the snapshot.
            let _ = git(
                &repository.path,
                &["worktree", "remove", "--force", &target],
            )
            .await;
            return Err(error);
        }
    }
    Ok(Checkout {
        name: repository.name.clone(),
        repository: repository.path.clone(),
        path,
        branch,
        owner: session.to_owned(),
    })
}

/// Whether `path` is the top of a git worktree.
async fn is_worktree_at(path: &Path) -> bool {
    let Ok(top) = git(path, &["rev-parse", "--show-toplevel"]).await else {
        return false;
    };
    let top = PathBuf::from(top.trim());
    match (top.canonicalize(), path.canonicalize()) {
        (Ok(top), Ok(path)) => top == path,
        _ => false,
    }
}

async fn git(directory: &Path, arguments: &[&str]) -> Result<String, WorkspaceError> {
    run(directory, None, arguments).await
}

/// Runs git with `input` on its standard input.
async fn git_with_input(
    directory: &Path,
    arguments: &[&str],
    input: &str,
) -> Result<String, WorkspaceError> {
    run_with(directory, None, arguments, Some(input)).await
}

async fn git_with_index(
    directory: &Path,
    index: &Path,
    arguments: &[&str],
) -> Result<String, WorkspaceError> {
    run(directory, Some(index), arguments).await
}

async fn run(
    directory: &Path,
    index: Option<&Path>,
    arguments: &[&str],
) -> Result<String, WorkspaceError> {
    run_with(directory, index, arguments, None).await
}

async fn run_with(
    directory: &Path,
    index: Option<&Path>,
    arguments: &[&str],
    input: Option<&str>,
) -> Result<String, WorkspaceError> {
    spawn(directory, index, arguments, input, true).await
}

/// Runs git as the person it is set up for, for commits that are shared.
async fn git_as_person(directory: &Path, arguments: &[&str]) -> Result<String, WorkspaceError> {
    spawn(directory, None, arguments, None, false).await
}

async fn spawn(
    directory: &Path,
    index: Option<&Path>,
    arguments: &[&str],
    input: Option<&str>,
    as_humata: bool,
) -> Result<String, WorkspaceError> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0");
    if as_humata {
        // Snapshots are on no branch; they needn't be the person's.
        command
            .env("GIT_AUTHOR_NAME", "Humata")
            .env("GIT_AUTHOR_EMAIL", "humata@localhost")
            .env("GIT_COMMITTER_NAME", "Humata")
            .env("GIT_COMMITTER_EMAIL", "humata@localhost");
    }
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let mut child = command
        .spawn()
        .map_err(|source| WorkspaceError::GitMissing { source })?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        use tokio::io::AsyncWriteExt;
        // A git that stops reading early says why on its standard error.
        let _ = stdin.write_all(input.as_bytes()).await;
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|source| WorkspaceError::GitMissing { source })?;
    if !output.status.success() {
        return Err(WorkspaceError::Git {
            directory: directory.to_owned(),
            command: arguments.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests;
