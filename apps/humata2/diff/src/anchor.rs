//! Where a comment points: a line range on one side of one file, with the
//! base it was compared against and the code it quotes (RV-27).

use std::fmt;

use crate::spec::DiffSpec;

/// Which version of a file a line number counts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    /// The base's version: removed and unchanged lines.
    Old,
    /// The current version: added and unchanged lines.
    New,
}

impl Side {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Old => "old",
            Self::New => "new",
        }
    }
}

/// Lines `start` to `end`, both included, counted from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    /// The range between two lines, in either order.
    #[must_use]
    pub fn between(a: u32, b: u32) -> Self {
        Self {
            start: a.min(b),
            end: a.max(b),
        }
    }

    #[must_use]
    pub fn single(line: u32) -> Self {
        Self {
            start: line,
            end: line,
        }
    }

    #[must_use]
    pub fn contains(self, line: u32) -> bool {
        (self.start..=self.end).contains(&line)
    }
}

impl fmt::Display for LineRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start == self.end {
            write!(f, "{}", self.start)
        } else {
            write!(f, "{}-{}", self.start, self.end)
        }
    }
}

/// Lines in one file of one repo, on one side.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LineTarget {
    /// The repo's name, as in [`crate::RepoDiff::name`].
    pub repo: String,
    /// The file's path inside the repo.
    pub path: String,
    pub side: Side,
    pub lines: LineRange,
}

impl fmt::Display for LineTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}:{}", self.repo, self.path, self.lines)?;
        if self.side == Side::Old {
            f.write_str(" (old)")?;
        }
        Ok(())
    }
}

/// What a new comment is about, as the diff view hands it out in
/// [`crate::DiffEvent::CommentRequested`]: the lines, the base they were
/// compared against, and the code they held, so the agent can find the
/// place even after the file has changed (RV-27).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CommentAnchor {
    pub target: LineTarget,
    /// What the lines were compared against: the repo's spec, as in
    /// [`crate::RepoDiff::spec`].
    pub spec: DiffSpec,
    /// The commit the spec's base resolved to, when the review said.
    pub base_commit: Option<String>,
    /// The selected lines' code on the target's side, one line each, in
    /// order, without diff marks.
    pub quoted: String,
}
