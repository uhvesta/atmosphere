//! A session's changes for review: as a diff over a range of its history,
//! file by file and hunk by hunk, and staged, unstaged or reverted by file
//! or by hunk.

use std::path::Path;

use crate::{Checkout, Repository, Snapshot, WorkspaceError, Workspaces, git, git_with_input};

/// Which changes to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Range {
    /// Everything since the session's copy was made.
    Session,
    /// In the copy, not staged yet.
    Unstaged,
    /// Staged in the copy, not committed yet.
    Staged,
    /// Since the copy's work left the branch: from where the two meet to the
    /// files in the copy now.
    Branch(String),
    /// What one turn changed, counting the session's first turn as 1.
    Turn(usize),
}

/// One file, by its repository's name in the session and its path there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRef {
    pub repository: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub range: Range,
    /// Unchanged lines shown around each change.
    pub context: u32,
    /// Only this file; with `whole_file`, every line of it, and its lines
    /// even when it didn't change.
    pub only: Option<FileRef>,
    pub whole_file: bool,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            range: Range::Session,
            context: 3,
            only: None,
            whole_file: false,
        }
    }
}

/// What a session changed in one repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changes {
    pub name: String,
    /// A unified diff; empty if nothing changed.
    pub patch: String,
    pub files: Vec<FileDiff>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    /// Asked for as a whole file, and the same at both ends of the range.
    Unchanged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    /// Where a renamed file was.
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    /// The file's contents at the end of the range, as git names them; none
    /// for a deleted file. The same whenever the file is.
    pub blob: Option<String>,
    pub hunks: Vec<Hunk>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    /// The line git shows after the hunk's range, such as the function it
    /// is in.
    pub header: String,
    pub lines: Vec<Line>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    /// Its number before the change; none for an added line.
    pub old: Option<u32>,
    /// Its number after; none for a removed line.
    pub new: Option<u32>,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Stages the copy's unstaged changes.
    Stage,
    /// Unstages staged changes, keeping them in the copy.
    Unstage,
    /// Throws away the copy's unstaged changes; a file git didn't know is
    /// removed.
    Revert,
}

/// A hunk, by where it starts in the diff the action works on: the unstaged
/// changes to stage or revert, the staged ones to unstage, each shown with
/// `context` lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HunkAt {
    pub old_start: u32,
    pub new_start: u32,
    pub context: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub file: FileRef,
    pub action: Action,
    /// Only this hunk; otherwise the whole file.
    pub hunk: Option<HunkAt>,
}

/// Lines enough for any file to be one hunk.
const WHOLE_FILE: u32 = 100_000_000;

impl Workspaces {
    /// The changes in `query`'s range in each repository, from the session's
    /// snapshots, oldest first, and its copies.
    pub async fn changes(
        &self,
        session: &str,
        repositories: &[Repository],
        checkpoints: &[Vec<Snapshot>],
        checkouts: &[Checkout],
        query: &Query,
    ) -> Result<Vec<Changes>, WorkspaceError> {
        if let Some(only) = &query.only
            && !repositories
                .iter()
                .any(|repository| repository.name == only.repository)
        {
            return Err(WorkspaceError::UnknownRepository {
                name: only.repository.clone(),
            });
        }
        if let Range::Turn(turn) = query.range
            && (turn == 0 || turn >= checkpoints.len())
        {
            return Err(WorkspaceError::UnknownTurn {
                turn,
                turns: checkpoints.len().saturating_sub(1),
            });
        }
        let mut all = Vec::with_capacity(repositories.len());
        for repository in repositories {
            let path = query
                .only
                .as_ref()
                .filter(|only| only.repository == repository.name)
                .map(|only| only.path.as_str());
            if query.only.is_some() && path.is_none() {
                continue;
            }
            let checkout = checkouts
                .iter()
                .find(|checkout| checkout.name == repository.name && checkout.path.is_dir());
            let Some((from, to, directory)) = self
                .ends(session, repository, checkpoints, checkout, &query.range)
                .await?
            else {
                all.push(Changes {
                    name: repository.name.clone(),
                    patch: String::new(),
                    files: Vec::new(),
                });
                continue;
            };
            let context = if query.whole_file && path.is_some() {
                WHOLE_FILE
            } else {
                query.context
            };
            let patch = diff(&directory, &from, &to, context, path).await?;
            let mut files = parse(&patch);
            if let Some(path) = path
                && query.whole_file
                && files.is_empty()
            {
                files.push(unchanged(&directory, &repository.name, &to, path).await?);
            }
            all.push(Changes {
                name: repository.name.clone(),
                patch,
                files,
            });
        }
        Ok(all)
    }

    /// Stages, unstages or reverts a file or one of its hunks in the
    /// session's copy.
    pub async fn edit(&self, checkouts: &[Checkout], edit: &Edit) -> Result<(), WorkspaceError> {
        let checkout = checkouts
            .iter()
            .find(|checkout| checkout.name == edit.file.repository)
            .ok_or_else(|| WorkspaceError::UnknownRepository {
                name: edit.file.repository.clone(),
            })?;
        if !checkout.path.is_dir() {
            return Err(WorkspaceError::NoCopy {
                repository: checkout.name.clone(),
            });
        }
        let directory = &checkout.path;
        let path = edit.file.path.as_str();
        let Some(hunk) = edit.hunk else {
            return edit_file(directory, path, edit.action).await;
        };
        // The diff the hunk is in, as the person saw it.
        let (from, to) = match edit.action {
            Action::Stage | Action::Revert => (
                index_tree(directory).await?,
                self.worktree(directory).await?,
            ),
            Action::Unstage => (head(directory).await?, index_tree(directory).await?),
        };
        let patch = diff(directory, &from, &to, hunk.context, Some(path)).await?;
        let one = hunk_patch(&patch, hunk).ok_or_else(|| WorkspaceError::ChangeGone {
            path: path.to_owned(),
        })?;
        let mut arguments = vec!["apply", "--recount"];
        match edit.action {
            Action::Stage => arguments.push("--cached"),
            Action::Unstage => arguments.extend(["--cached", "--reverse"]),
            Action::Revert => arguments.push("--reverse"),
        }
        if hunk.context == 0 {
            arguments.push("--unidiff-zero");
        }
        arguments.push("-");
        git_with_input(directory, &arguments, &one).await?;
        Ok(())
    }

    /// The commits a range runs between, and the folder to compare them in;
    /// none when the repository has no snapshot to start from.
    async fn ends(
        &self,
        session: &str,
        repository: &Repository,
        checkpoints: &[Vec<Snapshot>],
        checkout: Option<&Checkout>,
        range: &Range,
    ) -> Result<Option<(String, String, std::path::PathBuf)>, WorkspaceError> {
        let named = |snapshots: &[Snapshot]| {
            snapshots
                .iter()
                .find(|snapshot| snapshot.name == repository.name)
                .cloned()
        };
        let copy = || {
            checkout.ok_or_else(|| WorkspaceError::NoCopy {
                repository: repository.name.clone(),
            })
        };
        Ok(Some(match range {
            Range::Session => {
                let Some(from) = checkpoints.first().and_then(|first| named(first)) else {
                    return Ok(None);
                };
                let latest = checkpoints.last().and_then(|last| named(last));
                let to = match (checkout, latest) {
                    (Some(checkout), _) => self.capture(session, &checkout.path).await?.1,
                    (None, Some(latest)) => latest.commit,
                    (None, None) => from.commit.clone(),
                };
                let directory =
                    checkout.map_or_else(|| repository.path.clone(), |c| c.path.clone());
                (from.head, to, directory)
            }
            Range::Turn(turn) => {
                let (Some(from), Some(to)) =
                    (named(&checkpoints[turn - 1]), named(&checkpoints[*turn]))
                else {
                    return Ok(None);
                };
                (from.commit, to.commit, repository.path.clone())
            }
            Range::Unstaged => {
                let checkout = copy()?;
                let from = index_tree(&checkout.path).await?;
                (
                    from,
                    self.worktree(&checkout.path).await?,
                    checkout.path.clone(),
                )
            }
            Range::Staged => {
                let checkout = copy()?;
                let from = head(&checkout.path).await?;
                (
                    from,
                    index_tree(&checkout.path).await?,
                    checkout.path.clone(),
                )
            }
            Range::Branch(branch) => {
                let checkout = copy()?;
                let to = self.worktree(&checkout.path).await?;
                let from = git(&checkout.path, &["merge-base", branch, "HEAD"])
                    .await
                    .map_err(|_| WorkspaceError::UnknownBranch {
                        branch: branch.clone(),
                        repository: repository.name.clone(),
                    })?;
                (from.trim().to_owned(), to, checkout.path.clone())
            }
        }))
    }

    /// A commit of the files in the copy at `path` now.
    async fn worktree(&self, path: &Path) -> Result<String, WorkspaceError> {
        Ok(self.capture("review", path).await?.1)
    }
}

async fn head(directory: &Path) -> Result<String, WorkspaceError> {
    Ok(git(directory, &["rev-parse", "--verify", "HEAD"])
        .await?
        .trim()
        .to_owned())
}

/// The tree of what's staged in the copy at `directory`.
async fn index_tree(directory: &Path) -> Result<String, WorkspaceError> {
    Ok(git(directory, &["write-tree"]).await?.trim().to_owned())
}

async fn diff(
    directory: &Path,
    from: &str,
    to: &str,
    context: u32,
    path: Option<&str>,
) -> Result<String, WorkspaceError> {
    let unified = format!("--unified={context}");
    let mut arguments = vec![
        "-c",
        "core.quotePath=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--full-index",
        "--find-renames",
        &unified,
        from,
        to,
    ];
    if let Some(path) = path {
        arguments.extend(["--", path]);
    }
    git(directory, &arguments).await
}

/// A file that's the same at both ends, as one hunk of every line.
async fn unchanged(
    directory: &Path,
    repository: &str,
    commit: &str,
    path: &str,
) -> Result<FileDiff, WorkspaceError> {
    let object = format!("{commit}:{path}");
    let blob = git(directory, &["rev-parse", "--verify", "--quiet", &object])
        .await
        .map_err(|_| WorkspaceError::NoSuchFile {
            path: path.to_owned(),
            repository: repository.to_owned(),
        })?;
    let text = git(directory, &["cat-file", "blob", &object]).await?;
    let lines: Vec<Line> = text
        .lines()
        .zip(1..)
        .map(|(text, number)| Line {
            kind: LineKind::Context,
            old: Some(number),
            new: Some(number),
            text: text.to_owned(),
        })
        .collect();
    let count = u32::try_from(lines.len()).unwrap_or(u32::MAX);
    Ok(FileDiff {
        path: path.to_owned(),
        old_path: None,
        status: FileStatus::Unchanged,
        binary: text.contains('\0'),
        blob: Some(blob.trim().to_owned()),
        hunks: vec![Hunk {
            old_start: 1,
            old_count: count,
            new_start: 1,
            new_count: count,
            header: String::new(),
            lines,
        }],
    })
}

async fn edit_file(directory: &Path, path: &str, action: Action) -> Result<(), WorkspaceError> {
    match action {
        Action::Stage => {
            git(directory, &["add", "--all", "--", path]).await?;
        }
        Action::Unstage => {
            git(directory, &["reset", "--quiet", "--", path]).await?;
        }
        Action::Revert => {
            let known = git(directory, &["ls-files", "--error-unmatch", "--", path])
                .await
                .is_ok();
            if known {
                git(directory, &["checkout", "--", path]).await?;
            } else {
                let file = directory.join(path);
                tokio::fs::remove_file(&file)
                    .await
                    .map_err(|source| WorkspaceError::Write { path: file, source })?;
            }
        }
    }
    Ok(())
}

/// One hunk of `patch`, with its file's header, as a patch of its own.
fn hunk_patch(patch: &str, at: HunkAt) -> Option<String> {
    let mut header = String::new();
    let mut found = String::new();
    let mut in_hunk = false;
    for line in patch.split_inclusive('\n') {
        if line.starts_with("@@") {
            if !found.is_empty() {
                break;
            }
            in_hunk = hunk_range(line)
                .is_some_and(|(old, _, new, _)| old == at.old_start && new == at.new_start);
            if in_hunk {
                found.push_str(line);
            }
        } else if in_hunk {
            found.push_str(line);
        } else if found.is_empty() && !line.starts_with("@@") && header_line(line) {
            header.push_str(line);
        }
    }
    (!found.is_empty()).then(|| header + &found)
}

/// Whether `line` belongs to a file's header, before its hunks.
fn header_line(line: &str) -> bool {
    !matches!(line.as_bytes().first(), Some(b' ' | b'+' | b'-' | b'\\'))
        || line.starts_with("---")
        || line.starts_with("+++")
}

/// `@@ -a,b +c,d @@`'s numbers; a count left out is 1.
fn hunk_range(line: &str) -> Option<(u32, u32, u32, u32)> {
    let inner = line.strip_prefix("@@ ")?;
    let (ranges, _) = inner.split_once(" @@")?;
    let (old, new) = ranges.split_once(' ')?;
    let side = |text: &str, sign: char| -> Option<(u32, u32)> {
        let text = text.strip_prefix(sign)?;
        match text.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            None => Some((text.parse().ok()?, 1)),
        }
    };
    let (old_start, old_count) = side(old, '-')?;
    let (new_start, new_count) = side(new, '+')?;
    Some((old_start, old_count, new_start, new_count))
}

/// The files of a unified diff from `git diff --full-index`.
pub(crate) fn parse(patch: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut old_line = 0;
    let mut new_line = 0;
    for line in patch.lines() {
        if let Some(names) = line.strip_prefix("diff --git ") {
            files.push(FileDiff {
                path: same_path(names).unwrap_or_default(),
                old_path: None,
                status: FileStatus::Modified,
                binary: false,
                blob: None,
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if let Some(hunk) = file.hunks.last_mut()
            && let Some(kind) = match line.as_bytes().first() {
                Some(b' ') => Some(LineKind::Context),
                Some(b'+') => Some(LineKind::Added),
                Some(b'-') => Some(LineKind::Removed),
                _ => None,
            }
        {
            let (old, new) = match kind {
                LineKind::Context => (Some(old_line), Some(new_line)),
                LineKind::Added => (None, Some(new_line)),
                LineKind::Removed => (Some(old_line), None),
            };
            if old.is_some() {
                old_line += 1;
            }
            if new.is_some() {
                new_line += 1;
            }
            hunk.lines.push(Line {
                kind,
                old,
                new,
                text: line[1..].to_owned(),
            });
            continue;
        }
        if line.starts_with("@@") {
            if let Some((old_start, old_count, new_start, new_count)) = hunk_range(line) {
                old_line = old_start;
                new_line = new_start;
                let header = line
                    .splitn(3, "@@")
                    .nth(2)
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_owned();
                file.hunks.push(Hunk {
                    old_start,
                    old_count,
                    new_start,
                    new_count,
                    header,
                    lines: Vec::new(),
                });
            }
        } else if line.starts_with("new file mode") {
            file.status = FileStatus::Added;
        } else if line.starts_with("deleted file mode") {
            file.status = FileStatus::Deleted;
        } else if let Some(from) = line.strip_prefix("rename from ") {
            file.status = FileStatus::Renamed;
            file.old_path = Some(from.to_owned());
        } else if let Some(to) = line.strip_prefix("rename to ") {
            file.path = to.to_owned();
        } else if let Some(hashes) = line.strip_prefix("index ") {
            let hashes = hashes.split(' ').next().unwrap_or_default();
            file.blob = hashes
                .split_once("..")
                .map(|(_, new)| new.to_owned())
                .filter(|new| new.bytes().any(|byte| byte != b'0'));
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
            file.binary = true;
        } else if let Some(to) = line.strip_prefix("+++ b/") {
            file.path = to.to_owned();
        } else if let Some(from) = line.strip_prefix("--- a/")
            && file.path.is_empty()
        {
            file.path = from.to_owned();
        }
    }
    files
}

/// `P` from `a/P b/P`, which is how git names a file that wasn't renamed.
fn same_path(names: &str) -> Option<String> {
    let half = names.len().checked_sub(1)? / 2;
    let (a, b) = (names.get(..half)?, names.get(half + 1..)?);
    let a = a.strip_prefix("a/")?;
    let b = b.strip_prefix("b/")?;
    (a == b).then(|| a.to_owned())
}
