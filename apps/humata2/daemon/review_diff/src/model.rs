//! A computed diff as plain data: per repo, its files with their content
//! hashes, and per file, hunks of lines in chunks.
//!
//! It mirrors `humata2_diff_view`'s model (apps/humata2/diff/src/model.rs)
//! so the UI needs little glue; the mapping is in [`crate`]'s docs.

/// The two trees a repo's diff compares. Asking for a file's hunks with
/// them shows exactly the state the file list was computed from, even
/// after the working tree moved on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Trees {
    pub old: String,
    pub new: String,
}

/// One repo's changes under its [`crate::RepoSpec`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoDiff {
    pub repo: String,
    /// The branch HEAD is on; `None` when HEAD is detached.
    pub branch: Option<String>,
    /// HEAD's commit; `None` before the first commit.
    pub head: Option<String>,
    /// What the spec compares, such as `origin/main...HEAD + staged +
    /// unstaged + untracked` (RV-31's "Comparing: …", RV-27's base).
    pub label: String,
    pub trees: Trees,
    pub files: Vec<FileSummary>,
}

impl RepoDiff {
    #[must_use]
    pub fn totals(&self) -> (u64, u64) {
        self.files.iter().fold((0, 0), |(added, removed), file| {
            (added + u64::from(file.added), removed + u64::from(file.removed))
        })
    }

    /// The file at `path` (its new path, for a rename).
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&FileSummary> {
        self.files.iter().find(|file| file.path == path)
    }
}

/// How a file changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    /// Moved from `from`; `similarity` is git's score, 0–100.
    Renamed { from: String, similarity: u8 },
}

/// One changed file, without its hunks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSummary {
    /// The path inside the repo (the new path for a rename), `/`-separated.
    pub path: String,
    pub status: FileStatus,
    pub added: u32,
    pub removed: u32,
    pub binary: bool,
    /// The old side's blob id; `None` for an added file.
    pub old_blob: Option<String>,
    /// The new side's blob id: the file's content hash, which per-file
    /// Viewed records (RV-29). `None` for a deleted file.
    pub new_blob: Option<String>,
    /// The old side's git mode as its octal digits read as a number (100644);
    /// 0 for an added file.
    pub old_mode: u32,
    /// The new side's mode; 0 for a deleted file.
    pub new_mode: u32,
}

impl FileSummary {
    /// The path on the old side.
    #[must_use]
    pub fn old_path(&self) -> &str {
        match &self.status {
            FileStatus::Renamed { from, .. } => from,
            _ => &self.path,
        }
    }

    /// The content hash on `side`.
    #[must_use]
    pub fn blob(&self, side: Side) -> Option<&str> {
        match side {
            Side::Old => self.old_blob.as_deref(),
            Side::New => self.new_blob.as_deref(),
        }
    }
}

/// The old or new side of a diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Old,
    New,
}

impl Side {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Old => "old",
            Self::New => "new",
        }
    }
}

/// Where in a file's diff a chunk starts: a hunk, and a line inside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cursor {
    pub hunk: u32,
    pub line: u32,
}

/// How much one chunk of a file's diff may hold (TR-11: 1 MiB at most).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub max_lines: usize,
    pub max_bytes: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_lines: 5_000,
            max_bytes: 1 << 20,
        }
    }
}

/// Part of one file's hunks, from a [`Cursor`] until the [`Budget`] runs
/// out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileChunk {
    pub hunks: Vec<Hunk>,
    /// git says the file is binary; there are no hunks.
    pub binary: bool,
    /// Where the next chunk starts; `None` when this one ends the file.
    pub next: Option<Cursor>,
}

/// A run of changed lines with the unchanged lines around them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hunk {
    /// Its index among the file's hunks.
    pub index: u32,
    /// The hunk's first old and new line as git's header gives them (0
    /// for an empty side), for the whole hunk even when this chunk holds
    /// only part of it.
    pub old_start: u32,
    pub new_start: u32,
    /// The text after the header's second `@@`, usually the function.
    pub section: String,
    /// True when these lines continue a hunk the previous chunk started:
    /// append them to it.
    pub continued: bool,
    pub lines: Vec<DiffLine>,
}

/// One line of a hunk, with its line numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// Its line on the old side; `None` for an added line.
    pub old_line: Option<u32>,
    /// Its line on the new side; `None` for a removed line.
    pub new_line: Option<u32>,
    /// The text without its mark and line ending.
    pub text: String,
    /// git marks this line "\ No newline at end of file".
    pub no_newline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// A reviewed snapshot (RV-28): what the user saw of one repo when they
/// pressed Mark reviewed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub n: u32,
    /// The commit under `refs/humata/reviewed/<session>/<n>`; its tree is
    /// the files as shown, its parent HEAD at the time.
    pub commit: String,
    pub tree: String,
    /// HEAD when the snapshot was taken, if there was a commit.
    pub head: Option<String>,
    /// Seconds since the Unix epoch.
    pub taken_at: i64,
    /// Kept beyond the last 20 (RV-30).
    pub pinned: bool,
}

/// What a file marked Viewed at some content hash shows now (RV-29).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Viewed {
    /// Same content: it stays folded as viewed.
    Unchanged,
    /// The content changed: only the change since it was viewed. Viewed
    /// is cleared.
    Changed(FileChunk),
    /// The viewed content is no longer in the repository (git pruned it),
    /// so show the file's whole diff and clear Viewed.
    Forgotten,
}
