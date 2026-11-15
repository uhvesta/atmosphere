//! What one session's review shows: the changed files across its repos, as
//! the daemon reports them (`humata.v1.FileDiff`).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// The session a review belongs to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One file in one repo of the session: the key everything per file is kept
/// under (viewed marks, collapsed files, comments).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FileKey {
    /// The repo's short name, as the session's clones name it.
    pub repo: String,
    /// The path inside the repo, with `/` separators.
    pub path: String,
}

impl FileKey {
    pub fn new(repo: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            repo: repo.into(),
            path: path.into(),
        }
    }
}

impl std::fmt::Display for FileKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.repo, self.path)
    }
}

/// Which side of a diff a line belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Side {
    /// The base: the line as it was.
    Old,
    /// The change: the line as it is now.
    New,
}

/// What a diff line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// One line of a hunk, with its number on each side it's on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

impl DiffLine {
    pub fn context(old: u32, new: u32, text: impl Into<String>) -> Self {
        Self {
            kind: LineKind::Context,
            old: Some(old),
            new: Some(new),
            text: text.into(),
        }
    }

    pub fn added(new: u32, text: impl Into<String>) -> Self {
        Self {
            kind: LineKind::Added,
            old: None,
            new: Some(new),
            text: text.into(),
        }
    }

    pub fn removed(old: u32, text: impl Into<String>) -> Self {
        Self {
            kind: LineKind::Removed,
            old: Some(old),
            new: None,
            text: text.into(),
        }
    }

    /// The line's number on `side`, if it's on that side.
    pub fn number(&self, side: Side) -> Option<u32> {
        match side {
            Side::Old => self.old,
            Side::New => self.new,
        }
    }

    /// The side a comment on this line is anchored to: the new side, unless
    /// the line was removed.
    pub fn side(&self) -> Side {
        match self.kind {
            LineKind::Removed => Side::Old,
            LineKind::Context | LineKind::Added => Side::New,
        }
    }
}

/// A run of changed lines and the context around them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// A text file's change: its hunks, and the whole of each side when the
/// daemon sends it (a markdown file's rendered view needs both).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextDiff {
    pub hunks: Vec<Hunk>,
    pub old_text: Option<Arc<str>>,
    pub new_text: Option<Arc<str>>,
}

impl TextDiff {
    /// Every hunk line, in order.
    pub fn lines(&self) -> impl Iterator<Item = &DiffLine> {
        self.hunks.iter().flat_map(|hunk| hunk.lines.iter())
    }

    /// The text of `side`, line by line, as `(number, text)` in order. The
    /// whole side when it was sent, otherwise only the lines the hunks show.
    pub fn side_lines(&self, side: Side) -> Vec<(u32, &str)> {
        let whole = match side {
            Side::Old => self.old_text.as_deref(),
            Side::New => self.new_text.as_deref(),
        };
        if let Some(text) = whole {
            return text
                .lines()
                .enumerate()
                .map(|(index, line)| (u32::try_from(index + 1).unwrap_or(u32::MAX), line))
                .collect();
        }
        let mut lines: Vec<(u32, &str)> = self
            .lines()
            .filter_map(|line| line.number(side).map(|number| (number, line.text.as_str())))
            .collect();
        lines.sort_by_key(|(number, _)| *number);
        lines.dedup_by_key(|(number, _)| *number);
        lines
    }

    pub fn added(&self) -> usize {
        self.lines()
            .filter(|line| line.kind == LineKind::Added)
            .count()
    }

    pub fn removed(&self) -> usize {
        self.lines()
            .filter(|line| line.kind == LineKind::Removed)
            .count()
    }
}

/// One side of an image change, as the daemon sends it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageBlob {
    /// The blob's hash; decoded images are cached by it.
    pub sha: String,
    pub bytes: Arc<[u8]>,
}

/// An image file's change. A side is missing when the image was added (no
/// old) or deleted (no new).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageChange {
    pub old: Option<ImageBlob>,
    pub new: Option<ImageBlob>,
}

/// What a changed file holds, which decides how it's shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// Lines, shown as a diff.
    Text(TextDiff),
    /// A markdown file: a source diff, or the rendered document with its
    /// changes marked.
    Markdown(TextDiff),
    /// An image: before and after, side by side, swiped or as an onion skin.
    Image(ImageChange),
    /// Bytes that aren't shown, only named.
    Binary { old_size: Option<u64>, new_size: Option<u64> },
}

/// How the file changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// One changed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub key: FileKey,
    pub change: ChangeKind,
    /// The revision the change is against: the base comments are made on.
    pub base: String,
    /// The current blob's hash. A file marked viewed is unviewed again when
    /// this changes.
    pub blob: String,
    pub content: Content,
}

impl FileChange {
    /// Lines added and removed; images and binaries count as none.
    pub fn counts(&self) -> (usize, usize) {
        match &self.content {
            Content::Text(diff) | Content::Markdown(diff) => (diff.added(), diff.removed()),
            Content::Image(_) | Content::Binary { .. } => (0, 0),
        }
    }

    /// The text diff, for text and markdown files.
    pub fn text(&self) -> Option<&TextDiff> {
        match &self.content {
            Content::Text(diff) | Content::Markdown(diff) => Some(diff),
            Content::Image(_) | Content::Binary { .. } => None,
        }
    }
}

/// A pull request a review session reviews (SPEC §10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    /// The repo of the session it's cloned in.
    pub repo: String,
    pub number: u64,
    pub title: String,
}

/// Everything one session's review shows, in the order files are listed:
/// by repo, then path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Review {
    files: Vec<FileChange>,
    pulls: Vec<PullRequest>,
}

impl Review {
    pub fn new(mut files: Vec<FileChange>) -> Self {
        files.sort_by(|a, b| a.key.cmp(&b.key));
        files.dedup_by(|a, b| a.key == b.key);
        Self {
            files,
            pulls: Vec::new(),
        }
    }

    /// The review of a PR review session: every line, hunk and file then
    /// offers Ask, and the PR has a question box (PR-19, PR-20).
    pub fn with_pulls(mut self, pulls: Vec<PullRequest>) -> Self {
        self.pulls = pulls;
        self
    }

    /// The PRs reviewed; none for a session's own changes.
    pub fn pulls(&self) -> &[PullRequest] {
        &self.pulls
    }

    pub fn files(&self) -> &[FileChange] {
        &self.files
    }

    pub fn file(&self, key: &FileKey) -> Option<&FileChange> {
        self.index(key).map(|index| &self.files[index])
    }

    pub fn index(&self, key: &FileKey) -> Option<usize> {
        self.files.binary_search_by(|file| file.key.cmp(key)).ok()
    }

    /// The repos the review covers, in order.
    pub fn repos(&self) -> Vec<&str> {
        let mut repos: Vec<&str> = self.files.iter().map(|file| file.key.repo.as_str()).collect();
        repos.dedup();
        repos
    }
}
