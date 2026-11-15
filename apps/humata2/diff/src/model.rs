//! What a diff view shows, as plain data: repos, their changed files, and
//! each file's hunks. Nothing here knows about GPUI, so a client can build a
//! [`Review`] from git's output, a fixture or a test without a window.

use thiserror::Error;

use crate::guide::Guide;
use crate::spec::{Base, DiffSpec};

/// Everything one diff view shows: the changes of one or more repos, and,
/// for a prepared review, the order and notes to show them with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Review {
    pub repos: Vec<RepoDiff>,
    /// An agent's order, groups and notes (RV-32); `None` lists files repo
    /// by repo.
    pub guide: Option<Guide>,
}

impl Review {
    /// Number of changed files across all repos.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.repos.iter().map(|repo| repo.files.len()).sum()
    }

    /// Added and removed lines across all repos.
    #[must_use]
    pub fn totals(&self) -> Totals {
        self.repos.iter().fold(Totals::default(), |sum, repo| sum + repo.totals())
    }

    /// Number of diff lines (context, added and removed) across all hunks.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.repos
            .iter()
            .flat_map(|repo| &repo.files)
            .flat_map(|file| &file.hunks)
            .map(|hunk| hunk.lines.len())
            .sum()
    }
}

/// One repo's changes against its own base.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoDiff {
    /// The repo's name as the session shows it, such as `atmosphere`.
    pub name: String,
    /// The branch the changes are on.
    pub branch: String,
    /// What the changes are compared against (RV-31). Every comment anchor
    /// in this repo carries it (RV-27).
    pub spec: DiffSpec,
    /// The commit the base resolved to, when known, for comment anchors.
    pub base_commit: Option<String>,
    /// Other bases the spec editor offers, such as earlier reviews
    /// (RV-30); the spec's own base is always offered.
    pub bases: Vec<Base>,
    pub files: Vec<FileDiff>,
}

impl RepoDiff {
    #[must_use]
    pub fn totals(&self) -> Totals {
        self.files.iter().fold(Totals::default(), |sum, file| sum + file.totals())
    }
}

/// How a file changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    #[default]
    Modified,
    Deleted,
    /// Moved from another path, possibly with changes.
    Renamed { from: String },
}

impl FileStatus {
    /// The word the file header shows.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed { .. } => "renamed",
        }
    }
}

/// One changed file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// The path inside the repo, with `/` separators.
    pub path: String,
    pub status: FileStatus,
    pub hunks: Vec<Hunk>,
    /// The whole file as it is now (the new side), one entry per line,
    /// without line endings. Full-file mode (RV-21) and unfolding unchanged
    /// regions (RV-18) need it; without it those regions only show their
    /// size.
    pub new_lines: Option<Vec<String>>,
    /// Whether the user marked the file viewed (RV-29).
    pub viewed: Viewed,
    /// Identifies the file's content as shown, such as its blob hash; the
    /// view hands it back when the user marks the file viewed, so what is
    /// recorded is what the user saw.
    pub content_id: Option<String>,
}

/// Whether the user has looked at a file already (RV-29).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Viewed {
    #[default]
    No,
    /// Marked viewed and unchanged since: the view folds it to its header.
    Yes,
    /// Marked viewed, then changed: the file's hunks are only the change
    /// since it was viewed, and the view says so. It counts as not viewed.
    ChangedSince,
}

impl FileDiff {
    #[must_use]
    pub fn totals(&self) -> Totals {
        let mut totals = Totals::default();
        for line in self.hunks.iter().flat_map(|hunk| &hunk.lines) {
            match line.kind {
                LineKind::Added => totals.added += 1,
                LineKind::Removed => totals.removed += 1,
                LineKind::Context => {}
            }
        }
        totals
    }

    /// The language the path's extension names, for highlighting.
    #[must_use]
    pub fn language(&self) -> Language {
        Language::from_path(&self.path)
    }
}

/// One hunk: a run of changed lines with the unchanged lines around them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hunk {
    /// First old line the hunk covers, counted from 1; 0 when the hunk
    /// covers no old lines (a new file).
    pub old_start: u32,
    /// First new line the hunk covers, counted from 1; 0 when the hunk
    /// covers no new lines (a deleted file).
    pub new_start: u32,
    /// The text after the second `@@`, usually the enclosing function.
    pub section: String,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    /// Number of old lines the hunk covers (context and removed).
    #[must_use]
    pub fn old_len(&self) -> u32 {
        count(self.lines.iter().filter(|line| line.kind != LineKind::Added))
    }

    /// Number of new lines the hunk covers (context and added).
    #[must_use]
    pub fn new_len(&self) -> u32 {
        count(self.lines.iter().filter(|line| line.kind != LineKind::Removed))
    }

    /// The hunk's header as git writes it, such as `@@ -3,7 +3,8 @@ fn main`.
    #[must_use]
    pub fn header(&self) -> String {
        let mut header = format!(
            "@@ -{},{} +{},{} @@",
            self.old_start,
            self.old_len(),
            self.new_start,
            self.new_len()
        );
        if !self.section.is_empty() {
            header.push(' ');
            header.push_str(&self.section);
        }
        header
    }

    /// The first old line after the hunk.
    pub(crate) fn old_end(&self) -> u32 {
        first_line(self.old_start, self.old_len()) + self.old_len()
    }

    /// The first new line after the hunk.
    pub(crate) fn new_end(&self) -> u32 {
        first_line(self.new_start, self.new_len()) + self.new_len()
    }

    /// The first new line the hunk covers, or, when it covers none, the new
    /// line after the place it is in.
    pub(crate) fn new_first(&self) -> u32 {
        first_line(self.new_start, self.new_len())
    }

    /// As [`Self::new_first`], on the old side.
    pub(crate) fn old_first(&self) -> u32 {
        first_line(self.old_start, self.old_len())
    }
}

/// Git writes the line before an empty range as its start; the range itself
/// starts on the next line.
fn first_line(start: u32, len: u32) -> u32 {
    if len == 0 { start + 1 } else { start.max(1) }
}

fn count<T>(items: impl Iterator<Item = T>) -> u32 {
    u32::try_from(items.count()).unwrap_or(u32::MAX)
}

/// One line of a hunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// The line's text without its `+`, `-` or space mark and without a line
    /// ending.
    pub text: String,
}

impl DiffLine {
    #[must_use]
    pub fn new(kind: LineKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }
}

/// Whether a line is unchanged, added or removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

impl LineKind {
    /// The mark a unified diff writes before the line.
    #[must_use]
    pub fn mark(self) -> char {
        match self {
            Self::Context => ' ',
            Self::Added => '+',
            Self::Removed => '-',
        }
    }
}

/// Added and removed line counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub added: usize,
    pub removed: usize,
}

impl std::ops::Add for Totals {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            added: self.added + other.added,
            removed: self.removed + other.removed,
        }
    }
}

/// The languages the built-in highlighter knows, picked by file extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    Python,
    Go,
    Sql,
    Toml,
    Terraform,
    Shell,
    Markdown,
    Plain,
}

impl Language {
    #[must_use]
    pub fn from_path(path: &str) -> Self {
        let name = path.rsplit('/').next().unwrap_or(path);
        let extension = name.rsplit_once('.').map_or("", |(_, extension)| extension);
        match extension {
            "rs" => Self::Rust,
            "ts" | "tsx" | "js" | "jsx" | "mjs" => Self::TypeScript,
            "py" => Self::Python,
            "go" => Self::Go,
            "sql" => Self::Sql,
            "toml" => Self::Toml,
            "tf" | "hcl" => Self::Terraform,
            "sh" | "bash" | "zsh" => Self::Shell,
            "md" => Self::Markdown,
            _ if name == "BUILD.bazel" || name == "MODULE.bazel" => Self::Python,
            _ => Self::Plain,
        }
    }
}

/// Why a unified diff could not be read.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error(
        "line {line} of the diff is a hunk header that can't be read: {text:?}. Hunk headers look \
         like `@@ -3,7 +3,8 @@`; pass the output of `git diff` unchanged, without colours \
         (`--no-color`)"
    )]
    BadHunkHeader { line: usize, text: String },
    #[error(
        "line {line} of the diff is a change line outside any hunk: {text:?}. Each file's \
         changes must follow an `@@` header; pass the output of `git diff` unchanged"
    )]
    LineOutsideHunk { line: usize, text: String },
}

/// Reads git's unified diff output (`git diff --no-color`) into files.
///
/// File headers (`diff --git`, `---`, `+++`, `new file mode`, `deleted file
/// mode`, `rename from`) set each file's path and status; `\ No newline at
/// end of file` is skipped. The files have no `new_lines`; add them when the
/// whole file is at hand.
///
/// # Errors
///
/// [`ParseError`] when a hunk header can't be read or a change line comes
/// before any hunk.
pub fn parse_unified(text: &str) -> Result<Vec<FileDiff>, ParseError> {
    let mut files: Vec<FileDiff> = Vec::new();
    // Old and new lines the current hunk still has to cover.
    let mut remaining = (0_u32, 0_u32);
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        if remaining != (0, 0) {
            let kind = match raw.chars().next() {
                Some('+') => LineKind::Added,
                Some('-') => LineKind::Removed,
                Some(' ') | None => LineKind::Context,
                Some('\\') => continue,
                Some(_) => {
                    return Err(ParseError::LineOutsideHunk {
                        line: number,
                        text: raw.to_owned(),
                    });
                }
            };
            match kind {
                LineKind::Added => remaining.1 = remaining.1.saturating_sub(1),
                LineKind::Removed => remaining.0 = remaining.0.saturating_sub(1),
                LineKind::Context => {
                    remaining.0 = remaining.0.saturating_sub(1);
                    remaining.1 = remaining.1.saturating_sub(1);
                }
            }
            if let Some(hunk) = files.last_mut().and_then(|file| file.hunks.last_mut()) {
                hunk.lines.push(DiffLine::new(kind, raw.get(1..).unwrap_or_default()));
            }
            continue;
        }
        if let Some(rest) = raw.strip_prefix("diff --git ") {
            files.push(FileDiff {
                path: git_path(rest.rsplit_once(" b/").map_or(rest, |(_, new)| new)).to_owned(),
                ..FileDiff::default()
            });
        } else if raw.starts_with("@@") {
            let (hunk, lengths) = parse_header(raw).ok_or_else(|| ParseError::BadHunkHeader {
                line: number,
                text: raw.to_owned(),
            })?;
            let Some(file) = files.last_mut() else {
                return Err(ParseError::LineOutsideHunk {
                    line: number,
                    text: raw.to_owned(),
                });
            };
            file.hunks.push(hunk);
            remaining = lengths;
        } else if let Some(old) = raw.strip_prefix("--- ") {
            // A plain `diff -u` has no `diff --git` line: `---` starts a file.
            if files.last().is_none_or(|file| !file.hunks.is_empty()) {
                files.push(FileDiff::default());
            }
            if let Some(file) = files.last_mut() {
                if old == "/dev/null" {
                    file.status = FileStatus::Added;
                } else if file.path.is_empty() {
                    file.path = git_path(old).to_owned();
                }
            }
        } else if let Some(file) = files.last_mut() {
            if let Some(new) = raw.strip_prefix("+++ ") {
                if new == "/dev/null" {
                    file.status = FileStatus::Deleted;
                } else {
                    file.path = git_path(new).to_owned();
                }
            } else if raw.starts_with("new file mode") {
                file.status = FileStatus::Added;
            } else if raw.starts_with("deleted file mode") {
                file.status = FileStatus::Deleted;
            } else if let Some(from) = raw.strip_prefix("rename from ") {
                file.status = FileStatus::Renamed {
                    from: from.to_owned(),
                };
            }
        }
    }
    Ok(files)
}

fn git_path(path: &str) -> &str {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
}

/// A hunk header's hunk (without lines) and its old and new lengths.
fn parse_header(line: &str) -> Option<(Hunk, (u32, u32))> {
    let rest = line.strip_prefix("@@ ")?;
    let (ranges, section) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(' ')?;
    let (old_start, old_len) = range(old.strip_prefix('-')?)?;
    let (new_start, new_len) = range(new.strip_prefix('+')?)?;
    let hunk = Hunk {
        old_start,
        new_start,
        section: section.trim_start().to_owned(),
        lines: Vec::new(),
    };
    Some((hunk, (old_len, new_len)))
}

/// `start,length`, or `start` alone for a length of one.
fn range(range: &str) -> Option<(u32, u32)> {
    match range.split_once(',') {
        Some((start, length)) => Some((start.parse().ok()?, length.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/src/ledger.rs b/src/ledger.rs
index 1111111..2222222 100644
--- a/src/ledger.rs
+++ b/src/ledger.rs
@@ -38,3 +38,4 @@ fn post(
     let account = repo.find(id)?;
-    let cents = amount * 100;
+    let cents = amount.round_half_even(2)?;
+    audit.record(&account, cents);
     Ok(())
diff --git a/notes.md b/notes.md
new file mode 100644
--- /dev/null
+++ b/notes.md
@@ -0,0 +1,2 @@
+# Notes
+one
\\ No newline at end of file
";

    #[test]
    fn a_git_diff_reads_into_files_with_their_hunks_and_status() {
        let files = parse_unified(DIFF).expect("the diff is well formed");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/ledger.rs");
        assert_eq!(files[0].status, FileStatus::Modified);
        let hunk = &files[0].hunks[0];
        assert_eq!((hunk.old_start, hunk.new_start), (38, 38));
        assert_eq!((hunk.old_len(), hunk.new_len()), (3, 4));
        assert_eq!(hunk.section, "fn post(");
        assert_eq!(files[0].totals(), Totals { added: 2, removed: 1 });
        assert_eq!(files[1].path, "notes.md");
        assert_eq!(files[1].status, FileStatus::Added);
        assert_eq!(files[1].hunks[0].lines.len(), 2);
    }

    #[test]
    fn a_hunk_header_writes_back_as_git_writes_it() {
        let files = parse_unified(DIFF).expect("the diff is well formed");
        let hunk = &files[0].hunks[0];
        let (parsed, lengths) = parse_header(&hunk.header()).expect("the header reads back");
        assert_eq!((parsed.old_start, parsed.new_start), (hunk.old_start, hunk.new_start));
        assert_eq!(parsed.section, hunk.section);
        assert_eq!(lengths, (hunk.old_len(), hunk.new_len()));
    }

    #[test]
    fn a_broken_hunk_header_is_an_error_naming_its_line() {
        let error = parse_unified("diff --git a/x b/x\n@@ -a +b @@\n").expect_err("bad header");
        assert!(matches!(error, ParseError::BadHunkHeader { line: 2, .. }));
    }

    #[test]
    fn an_empty_range_starts_after_the_line_git_names() {
        let hunk = Hunk {
            old_start: 0,
            new_start: 1,
            section: String::new(),
            lines: vec![DiffLine::new(LineKind::Added, "x")],
        };
        assert_eq!(hunk.old_first(), 1);
        assert_eq!(hunk.old_end(), 1);
        assert_eq!(hunk.new_first(), 1);
        assert_eq!(hunk.new_end(), 2);
    }

    #[test]
    fn languages_follow_the_extension() {
        assert_eq!(Language::from_path("a/b/lib.rs"), Language::Rust);
        assert_eq!(Language::from_path("web/app.tsx"), Language::TypeScript);
        assert_eq!(Language::from_path("BUILD.bazel"), Language::Python);
        assert_eq!(Language::from_path("README"), Language::Plain);
    }
}
