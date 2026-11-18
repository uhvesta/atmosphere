//! Agent-prepared reviews (RV-32, RV-34): a diff spec, an order for the
//! files, groups with titles, and notes on files, hunks and lines. The
//! order and notes stay attached while the changes move under them: a note
//! whose lines changed is kept and marked outdated, as PR drafts are.

use std::collections::{HashMap, HashSet};

use crate::checkout::Checkout;
use crate::error::{DiffError, PlanError};
use crate::model::{FileChunk, FileSummary, RepoDiff, Side};
use crate::spec::DiffSpec;
use crate::stack::Stack;

/// What `review_prepare` carries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewPlan {
    pub spec: DiffSpec,
    /// The files in the order to read them. Files of the diff left out
    /// come after, in git's order.
    pub order: Vec<FileRef>,
    /// Section titles over the order: each starts at a file of `order`
    /// and runs until the next group.
    pub groups: Vec<Group>,
    pub notes: Vec<Note>,
    /// The layers the change is told as (D800), if it is a stack.
    pub stack: Option<Stack>,
}

/// A file of one repo.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileRef {
    pub repo: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub title: String,
    pub note: String,
    pub first: FileRef,
}

/// An agent's note on a file, a hunk or lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub file: FileRef,
    pub target: NoteTarget,
    pub text: String,
    /// What the note was written against, recorded when the plan is
    /// prepared ([`Checkout::anchor_notes_blocking`]).
    pub anchor: Option<Anchor>,
    /// The title of the layer the note is on; `None` is the whole stack.
    pub layer: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteTarget {
    File,
    /// The hunk that covers `line` on `side`.
    Hunk { line: u32, side: Side },
    /// Lines `first..=last` on `side`.
    Lines { first: u32, last: u32, side: Side },
}

/// The content a line or hunk note was written against: the side's blob
/// and the lines it covered, so it can be found again after edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub blob: String,
    pub first: u32,
    pub last: u32,
    pub quoted: Vec<String>,
}

/// Where a note stands against the current diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteState {
    /// Its lines are as they were, at these line numbers (moved, if other
    /// lines were added above).
    Current { first: u32, last: u32 },
    /// Its lines changed: kept, shown at its old lines, marked outdated.
    Outdated,
    /// A file note, which nothing makes outdated while the file is shown.
    OnFile,
    /// Its file is no longer in the diff.
    Gone,
}

/// The review as the UI lays it out: sections in order, each file with
/// its notes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Arrangement {
    pub sections: Vec<Section>,
    /// Files of the order that the diff no longer has.
    pub gone: Vec<FileRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Section {
    /// `None` for files before the first group, and for the files the plan
    /// didn't order ("Other changes").
    pub title: Option<String>,
    pub note: String,
    pub files: Vec<FileRef>,
}

impl ReviewPlan {
    /// Checks the plan against the diff its spec gives: every ordered file
    /// and every note is on a file the diff shows, groups start at ordered
    /// files, and nothing is listed twice.
    ///
    /// # Errors
    ///
    /// The first [`PlanError`] found.
    pub fn check(&self, diffs: &[RepoDiff]) -> Result<(), PlanError> {
        let shown = shown_files(diffs);
        let mut seen = HashSet::new();
        for file in &self.order {
            if !shown.contains_key(file) {
                return Err(PlanError::NotInDiff {
                    repo: file.repo.clone(),
                    path: file.path.clone(),
                });
            }
            if !seen.insert(file) {
                return Err(PlanError::Duplicate {
                    repo: file.repo.clone(),
                    path: file.path.clone(),
                });
            }
        }
        for group in &self.groups {
            if !seen.contains(&group.first) {
                return Err(PlanError::GroupNotInOrder {
                    title: group.title.clone(),
                    repo: group.first.repo.clone(),
                    path: group.first.path.clone(),
                });
            }
        }
        for note in &self.notes {
            if !shown.contains_key(&note.file) {
                return Err(PlanError::NoteNotInDiff {
                    repo: note.file.repo.clone(),
                    path: note.file.path.clone(),
                });
            }
        }
        Ok(())
    }

    /// The sections the UI shows: the order cut at each group, then every
    /// other file of the diff under no title.
    #[must_use]
    pub fn arrange(&self, diffs: &[RepoDiff]) -> Arrangement {
        let shown = shown_files(diffs);
        let starts: HashMap<&FileRef, &Group> =
            self.groups.iter().map(|group| (&group.first, group)).collect();
        let mut arrangement = Arrangement::default();
        let mut current = Section::default();
        let mut placed = HashSet::new();
        for file in &self.order {
            if !shown.contains_key(file) {
                arrangement.gone.push(file.clone());
                continue;
            }
            if !placed.insert(file.clone()) {
                continue;
            }
            if let Some(group) = starts.get(file) {
                if !current.files.is_empty() || current.title.is_some() {
                    arrangement.sections.push(std::mem::take(&mut current));
                }
                current.title = Some(group.title.clone());
                current.note = group.note.clone();
            }
            current.files.push(file.clone());
        }
        if !current.files.is_empty() {
            arrangement.sections.push(current);
        }
        let rest: Vec<FileRef> = diffs
            .iter()
            .flat_map(|diff| {
                diff.files.iter().map(|file| FileRef {
                    repo: diff.repo.clone(),
                    path: file.path.clone(),
                })
            })
            .filter(|file| !placed.contains(file))
            .collect();
        if !rest.is_empty() {
            arrangement.sections.push(Section {
                title: None,
                note: String::new(),
                files: rest,
            });
        }
        arrangement
    }
}

impl Checkout {
    /// Records, for each line and hunk note on a file of `diff`, the blob
    /// and the lines it covers, so later diffs can tell whether they
    /// changed (RV-34). `hunks` gives a file's hunks, for hunk notes.
    ///
    /// # Errors
    ///
    /// [`PlanError`] (as [`AnchorError::Plan`]) for a note outside its
    /// file; [`DiffError`] when git fails.
    pub fn anchor_notes_blocking(
        &self,
        diff: &RepoDiff,
        notes: &mut [Note],
        mut hunks: impl FnMut(&FileSummary) -> Result<FileChunk, DiffError>,
    ) -> Result<(), AnchorError> {
        for note in notes.iter_mut().filter(|note| note.file.repo == diff.repo) {
            let Some(file) = diff.file(&note.file.path) else {
                return Err(PlanError::NoteNotInDiff {
                    repo: note.file.repo.clone(),
                    path: note.file.path.clone(),
                }
                .into());
            };
            let (first, last, side) = match note.target {
                NoteTarget::File => continue,
                NoteTarget::Lines { first, last, side } => (first.min(last), first.max(last), side),
                NoteTarget::Hunk { line, side } => {
                    let chunk = hunks(file)?;
                    hunk_span(&chunk, line, side).ok_or_else(|| PlanError::NoHunkAt {
                        repo: note.file.repo.clone(),
                        path: note.file.path.clone(),
                        line,
                    })?
                }
            };
            let Some(blob) = file.blob(side) else {
                return Err(out_of_range(note, first, last, side, 0).into());
            };
            let lines = self.blob_lines(blob)?;
            let Some(quoted) = quote(&lines, first, last) else {
                return Err(out_of_range(note, first, last, side, lines.len()).into());
            };
            note.anchor = Some(Anchor {
                blob: blob.to_owned(),
                first,
                last,
                quoted,
            });
        }
        Ok(())
    }

    /// Where each note of `notes` on this repo stands against the current
    /// `diff` (RV-34). Notes on other repos come back as `None`.
    ///
    /// # Errors
    ///
    /// [`DiffError`] when git fails reading a file's content.
    pub fn note_states_blocking(
        &self,
        diff: &RepoDiff,
        notes: &[Note],
    ) -> Result<Vec<Option<NoteState>>, DiffError> {
        let mut contents: HashMap<String, Vec<String>> = HashMap::new();
        notes
            .iter()
            .map(|note| {
                if note.file.repo != diff.repo {
                    return Ok(None);
                }
                let Some(file) = diff.file(&note.file.path) else {
                    return Ok(Some(NoteState::Gone));
                };
                let (side, anchor) = match (&note.target, &note.anchor) {
                    (NoteTarget::File, _) => return Ok(Some(NoteState::OnFile)),
                    (NoteTarget::Lines { side, .. } | NoteTarget::Hunk { side, .. }, Some(anchor)) => {
                        (*side, anchor)
                    }
                    (_, None) => return Ok(Some(NoteState::Outdated)),
                };
                let Some(blob) = file.blob(side) else {
                    return Ok(Some(NoteState::Outdated));
                };
                if blob == anchor.blob {
                    return Ok(Some(NoteState::Current {
                        first: anchor.first,
                        last: anchor.last,
                    }));
                }
                if !contents.contains_key(blob) {
                    contents.insert(blob.to_owned(), self.blob_lines(blob)?);
                }
                let lines = contents.get(blob).map_or(&[][..], Vec::as_slice);
                Ok(Some(relocate(lines, anchor)))
            })
            .collect()
    }
}

/// Why notes couldn't be anchored.
#[derive(Debug, thiserror::Error)]
pub enum AnchorError {
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Diff(#[from] DiffError),
}

/// The anchored lines in `lines`, nearest to where they were; outdated
/// when they are nowhere.
fn relocate(lines: &[String], anchor: &Anchor) -> NoteState {
    let len = anchor.quoted.len();
    if len == 0 || len > lines.len() {
        return NoteState::Outdated;
    }
    let was = anchor.first as usize;
    let found = (0..=lines.len() - len)
        .filter(|start| lines[*start..*start + len] == anchor.quoted[..])
        .min_by_key(|start| (start + 1).abs_diff(was));
    match found {
        Some(start) => {
            let first = u32::try_from(start + 1).unwrap_or(u32::MAX);
            let last = u32::try_from(start + len).unwrap_or(u32::MAX);
            NoteState::Current { first, last }
        }
        None => NoteState::Outdated,
    }
}

/// The span on `side` of the hunk covering `line`.
fn hunk_span(chunk: &FileChunk, line: u32, side: Side) -> Option<(u32, u32, Side)> {
    chunk.hunks.iter().find_map(|hunk| {
        let numbers: Vec<u32> = hunk
            .lines
            .iter()
            .filter_map(|diff_line| match side {
                Side::Old => diff_line.old_line,
                Side::New => diff_line.new_line,
            })
            .collect();
        let first = *numbers.first()?;
        let last = *numbers.last()?;
        (first..=last).contains(&line).then_some((first, last, side))
    })
}

fn quote(lines: &[String], first: u32, last: u32) -> Option<Vec<String>> {
    let start = usize::try_from(first).ok()?.checked_sub(1)?;
    let end = usize::try_from(last).ok()?;
    lines.get(start..end).map(<[String]>::to_vec)
}

fn out_of_range(note: &Note, first: u32, last: u32, side: Side, lines: usize) -> PlanError {
    PlanError::NoteOutOfRange {
        repo: note.file.repo.clone(),
        path: note.file.path.clone(),
        first,
        last,
        side: side.name(),
        lines,
    }
}

fn shown_files(diffs: &[RepoDiff]) -> HashMap<FileRef, &FileSummary> {
    diffs
        .iter()
        .flat_map(|diff| {
            diff.files.iter().map(|file| {
                (
                    FileRef {
                        repo: diff.repo.clone(),
                        path: file.path.clone(),
                    },
                    file,
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    fn anchor(first: u32, quoted: &str) -> Anchor {
        let quoted = lines(quoted);
        Anchor {
            blob: "old".to_owned(),
            first,
            last: first + u32::try_from(quoted.len()).unwrap_or(0) - 1,
            quoted,
        }
    }

    #[test]
    fn a_note_whose_lines_moved_down_follows_them() {
        let now = lines("new\na\nb\nc\n");
        assert_eq!(
            relocate(&now, &anchor(2, "b\nc")),
            NoteState::Current { first: 3, last: 4 }
        );
    }

    #[test]
    fn a_note_whose_lines_changed_is_outdated() {
        let now = lines("a\nB\nc\n");
        assert_eq!(relocate(&now, &anchor(2, "b")), NoteState::Outdated);
    }

    #[test]
    fn of_two_copies_the_nearest_is_kept() {
        let now = lines("x\nx\ny\nx\n");
        assert_eq!(
            relocate(&now, &anchor(4, "x")),
            NoteState::Current { first: 4, last: 4 }
        );
    }
}
