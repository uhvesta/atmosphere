//! Where a comment points: a line range on one side of one file at one base,
//! with the text it quotes, so it can follow the code when the code moves.

use serde::{Deserialize, Serialize};

use super::review::{FileKey, Side};

/// Lines `first..=last`, numbered from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LineRange {
    pub first: u32,
    pub last: u32,
}

impl LineRange {
    /// The lines from `a` to `b`, in either order.
    pub fn new(a: u32, b: u32) -> Self {
        Self {
            first: a.min(b).max(1),
            last: a.max(b).max(1),
        }
    }

    pub fn line(line: u32) -> Self {
        Self::new(line, line)
    }

    /// The range of an anchor on a whole file, such as an image or a
    /// question about the file: no line, so nothing to follow or quote.
    pub const FILE: Self = Self { first: 0, last: 0 };

    pub fn is_file(&self) -> bool {
        *self == Self::FILE
    }

    pub fn len(&self) -> u32 {
        self.last - self.first + 1
    }

    pub fn contains(&self, line: u32) -> bool {
        (self.first..=self.last).contains(&line)
    }

    pub fn overlaps(&self, other: &LineRange) -> bool {
        self.first <= other.last && other.first <= self.last
    }
}

/// A comment made on a markdown file's rendered view: what was selected
/// there. Its source lines are the anchor's line range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedSpan {
    /// The selected text as it reads rendered.
    pub text: String,
}

/// Where a comment points.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub file: FileKey,
    pub side: Side,
    pub lines: LineRange,
    /// The revision the commented code is from.
    pub base: String,
    /// The anchored lines' text when the comment was made, one per line.
    pub quoted: Vec<String>,
    /// Set when the comment was made on the rendered view of a markdown file.
    pub rendered: Option<RenderedSpan>,
}

/// Where an anchor is now that the file changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    /// The quoted lines are still where they were.
    Unchanged,
    /// The quoted lines are intact but moved to `lines`.
    Moved { lines: LineRange },
    /// The quoted lines are gone or changed: the comment is outdated.
    Outdated,
}

/// Lines compare equal when they differ only in whitespace.
fn same(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

/// Finds where `quoted` sits in `lines`, the `(number, text)` lines of the
/// anchored side. A block that's still at `at` stays there; otherwise the
/// copy nearest to `at` wins. Lines must run without gaps to match.
pub fn place(quoted: &[String], at: LineRange, lines: &[(u32, &str)]) -> Placement {
    if at.is_file() {
        return Placement::Unchanged;
    }
    if quoted.is_empty() || lines.is_empty() {
        return Placement::Outdated;
    }
    let matches_at = |start: usize| -> bool {
        let Some(window) = lines.get(start..start + quoted.len()) else {
            return false;
        };
        window
            .windows(2)
            .all(|pair| pair[1].0 == pair[0].0 + 1)
            && window
                .iter()
                .zip(quoted)
                .all(|((_, text), quote)| same(text, quote))
    };
    let mut best: Option<(u32, u32)> = None;
    for start in 0..lines.len() {
        if !matches_at(start) {
            continue;
        }
        let first = lines[start].0;
        if first == at.first {
            return Placement::Unchanged;
        }
        let distance = first.abs_diff(at.first);
        if best.is_none_or(|(_, nearest)| distance < nearest) {
            best = Some((first, distance));
        }
    }
    match best {
        Some((first, _)) => Placement::Moved {
            lines: LineRange::new(first, first + at.len() - 1),
        },
        None => Placement::Outdated,
    }
}

/// The quote for `range` from the `(number, text)` lines of a side.
pub fn quote(lines: &[(u32, &str)], range: LineRange) -> Vec<String> {
    lines
        .iter()
        .filter(|(number, _)| range.contains(*number))
        .map(|(_, text)| (*text).to_owned())
        .collect()
}
