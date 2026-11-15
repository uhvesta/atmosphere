//! What a review is compared against (RV-28–RV-30), and reviews an agent
//! prepared with `review_prepare`: its order, groups and notes (RV-32,
//! RV-34), shown until the user reverts or keeps it (VW-8, VW-9).

use serde::{Deserialize, Serialize};

use super::agent::NoteId;
use super::anchor::LineRange;
use super::review::{FileKey, Review, Side};

/// What the review's changes are against, per session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Base {
    /// The session's own base (RV-1).
    #[default]
    Session,
    /// What the user saw when they marked review `n` reviewed (RV-28).
    Since(u32),
}

/// A review the user marked reviewed: a snapshot the daemon keeps as git
/// objects (RV-28), listed newest first by the history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviewed {
    pub number: u32,
    /// When, as the user reads it ("Tuesday 14:02").
    pub when: String,
}

/// A group of files an agent put together, with its title, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedGroup {
    pub title: String,
    pub files: Vec<FileKey>,
}

/// Where on a file a note is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteAt {
    pub side: Side,
    pub lines: LineRange,
    /// The lines it was written on, to follow them as the file changes.
    pub quoted: Vec<String>,
}

/// An agent's note on a file, or on lines of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedNote {
    pub file: FileKey,
    /// The lines, or none for the whole file.
    pub at: Option<NoteAt>,
    pub text: String,
    /// Its lines changed since; it's kept, under the file (RV-34).
    #[serde(default)]
    pub outdated: bool,
}

/// A review an agent prepared (RV-32): an order for the files, in groups
/// with titles, and notes. Files it didn't place follow, in a last group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedReview {
    /// The view's id, from the Journal (VW-10).
    pub id: String,
    pub title: String,
    /// Who prepared it ("Claude Code").
    pub agent: String,
    pub summary: String,
    pub groups: Vec<PreparedGroup>,
    pub notes: Vec<PreparedNote>,
}

/// How the pane was laid out: what Revert puts back. Viewed marks and
/// comments made meanwhile are the user's work and stay.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub collapsed: std::collections::BTreeSet<FileKey>,
    pub collapsed_repos: std::collections::BTreeSet<String>,
    pub filters: super::memory::Filters,
    pub rendered: std::collections::BTreeSet<FileKey>,
    pub current: Option<FileKey>,
    pub base: Base,
}

/// A prepared review being shown. `before` is what Revert restores; Keep
/// drops it, leaving the view as it is. One the review chat's agent made
/// has none: it's taken back with Undo instead (PR-24).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Showing {
    /// The view, its notes moved to the session's notes.
    pub view: PreparedReview,
    pub before: Option<Layout>,
    /// The notes it added, which Revert takes away.
    #[serde(default)]
    pub notes: Vec<NoteId>,
}

impl PreparedReview {
    /// The files of `review` in this order, grouped: each group's title (none
    /// for the last group of files it didn't place) and file indexes.
    pub fn order(&self, review: &Review) -> Vec<(Option<usize>, Vec<usize>)> {
        let mut placed = vec![false; review.files().len()];
        let mut out = Vec::with_capacity(self.groups.len() + 1);
        for (g, group) in self.groups.iter().enumerate() {
            let files: Vec<usize> = group
                .files
                .iter()
                .filter_map(|key| review.index(key))
                .filter(|&index| !std::mem::replace(&mut placed[index], true))
                .collect();
            out.push((Some(g), files));
        }
        let rest: Vec<usize> = (0..placed.len()).filter(|&index| !placed[index]).collect();
        if !rest.is_empty() {
            out.push((None, rest));
        }
        out
    }
}
