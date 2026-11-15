//! An agent-prepared review (RV-32–RV-34): an order for the files, groups
//! with titles, and notes on files, hunks and lines.
//!
//! A [`Guide`] rides along with a [`crate::Review`]. The view lists the
//! files in the guide's order under its group titles and draws its notes in
//! place. Notes stay attached while the changes move under them: a note
//! on lines whose code no longer reads as it did when the note was written
//! is kept and marked outdated (RV-34), as drafts are.

use crate::anchor::{LineTarget, Side};

/// A file in a review, by repo name and path.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileRef {
    pub repo: String,
    pub path: String,
}

impl FileRef {
    #[must_use]
    pub fn new(repo: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            repo: repo.into(),
            path: path.into(),
        }
    }
}

/// Files shown together under a title, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub title: String,
    pub files: Vec<FileRef>,
}

/// What a note is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteTarget {
    /// A whole file; drawn under its header.
    File(FileRef),
    /// The hunk holding `line` on `side`; drawn under the hunk's header.
    Hunk { file: FileRef, side: Side, line: u32 },
    /// Lines; drawn under the last of them. `quoted` is their code when
    /// the note was written, one line each, without diff marks: when the
    /// lines no longer read so, or are gone, the note shows as outdated.
    Lines { target: LineTarget, quoted: String },
}

impl NoteTarget {
    #[must_use]
    pub fn file(&self) -> (&str, &str) {
        let file = match self {
            Self::File(file) | Self::Hunk { file, .. } => file,
            Self::Lines { target, .. } => {
                return (target.repo.as_str(), target.path.as_str());
            }
        };
        (file.repo.as_str(), file.path.as_str())
    }
}

/// A note from whoever prepared the review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub target: NoteTarget,
    /// Plain text; line breaks are kept.
    pub text: String,
}

/// The order, groups and notes of a prepared review.
///
/// Files the groups don't name are listed after them, under "Other
/// changes", in the review's own order; names of files the review doesn't
/// have are skipped, and a file named twice is listed the first time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Guide {
    /// Who prepared it, as the view credits it, such as `agent · Haiku`.
    pub author: String,
    pub groups: Vec<Group>,
    pub notes: Vec<Note>,
}
