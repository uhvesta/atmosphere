//! A changed code file's editor (RV-17–RV-22): the diff and the file's
//! current text become rows, unchanged stretches fold unless they were
//! opened, and the changes can be listed and stepped through. Its settings
//! are kept per file and session in [`SessionView`].
use std::collections::BTreeSet;

use humata2_clients::model::{DiffLine, FileDiff, LineKind};
use serde::{Deserialize, Serialize};

use crate::SessionView;

/// How a changed file is shown (RV-17).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodeMode {
    /// Old and new numbers, removed and added lines interleaved.
    #[default]
    Inline,
    /// Removed lines beside the added lines after them.
    SideBySide,
    /// The whole file, with the changes inline and nothing folded (RV-21).
    FullFile,
}

/// One file's editor settings in a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeView {
    /// How the file is shown.
    #[serde(default)]
    pub mode: CodeMode,
    /// Whether Full file marks the changes inline (RV-22).
    #[serde(default = "default_changes")]
    pub changes: bool,
    /// The folds opened, by the new line number they start at (RV-18).
    #[serde(default)]
    pub unfolded: BTreeSet<u32>,
}

fn default_changes() -> bool {
    true
}

impl Default for CodeView {
    fn default() -> Self {
        Self {
            mode: CodeMode::Inline,
            changes: true,
            unfolded: BTreeSet::new(),
        }
    }
}

/// One row of the inline or full-file editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A line: a changed one, or an unchanged one of the file.
    Line(DiffLine),
    /// Unchanged lines that are folded: `count` lines from new line `start`.
    Fold {
        /// The first new line number folded.
        start: u32,
        /// How many lines are folded.
        count: u32,
    },
}

/// One row of the side-by-side editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SideRow {
    /// The old line on the left and the new line on the right; `None` for
    /// the side that has no line here.
    Pair {
        /// The line in the old file.
        old: Option<DiffLine>,
        /// The line in the new file.
        new: Option<DiffLine>,
    },
    /// Unchanged lines that are folded, as in [`Row::Fold`].
    Fold {
        /// The first new line number folded.
        start: u32,
        /// How many lines are folded.
        count: u32,
    },
}

/// One file's stored settings, with the repository and path that key it.
/// Its fields stay private; read and write through `SessionView::code` and
/// `SessionView::set_code`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeEntry {
    repo: String,
    path: String,
    view: CodeView,
}

impl SessionView {
    /// A file's editor settings; the defaults when none were picked.
    pub fn code(&self, repo: &str, path: &str) -> CodeView {
        self.code
            .iter()
            .find(|entry| entry.repo == repo && entry.path == path)
            .map(|entry| entry.view.clone())
            .unwrap_or_default()
    }

    /// Store a file's settings; the defaults remove the entry.
    pub fn set_code(&mut self, repo: &str, path: &str, view: CodeView) {
        self.code
            .retain(|entry| !(entry.repo == repo && entry.path == path));
        if view != CodeView::default() {
            self.code.push(CodeEntry {
                repo: repo.to_owned(),
                path: path.to_owned(),
                view,
            });
        }
    }
}

/// The rows of a file: its diff against the current text, in `view`'s mode.
/// With no diff, or Full file with the changes hidden, it is the plain
/// current file.
#[must_use]
pub fn rows(diff: Option<&FileDiff>, current: &str, view: &CodeView) -> Vec<Row> {
    let text: Vec<&str> = current.lines().collect();
    let Some(diff) = diff.filter(|_| view.mode != CodeMode::FullFile || view.changes) else {
        return plain(&text);
    };
    let full = view.mode == CodeMode::FullFile;
    let total = u32::try_from(text.len()).unwrap_or(u32::MAX);
    let mut rows = Vec::new();
    // The next new and old line numbers not yet shown.
    let (mut next_new, mut next_old) = (1u32, 1u32);
    for line in diff
        .lines
        .iter()
        .filter(|line| !matches!(line.kind, LineKind::Hunk | LineKind::Note))
    {
        match line.kind {
            LineKind::Context | LineKind::Added => {
                let new = line.new.unwrap_or(next_new);
                let count = new.saturating_sub(next_new);
                gap(&mut rows, &text, view, full, next_new, next_old, count);
                next_old += count;
                rows.push(Row::Line(line.clone()));
                next_new = new.saturating_add(1);
                if line.kind == LineKind::Context {
                    next_old = line.old.unwrap_or(next_old).saturating_add(1);
                }
            }
            LineKind::Removed => {
                let old = line.old.unwrap_or(next_old);
                let count = old.saturating_sub(next_old);
                gap(&mut rows, &text, view, full, next_new, next_old, count);
                next_new += count;
                rows.push(Row::Line(line.clone()));
                next_old = old.saturating_add(1);
            }
            LineKind::Hunk | LineKind::Note => {}
        }
    }
    let count = (total + 1).saturating_sub(next_new);
    gap(&mut rows, &text, view, full, next_new, next_old, count);
    rows
}

/// The unchanged lines of a stretch: folded in Inline and Side by side, unless
/// the fold was opened, and shown as lines otherwise.
fn gap(
    rows: &mut Vec<Row>,
    text: &[&str],
    view: &CodeView,
    full: bool,
    new: u32,
    old: u32,
    count: u32,
) {
    if count == 0 {
        return;
    }
    if !full && !view.unfolded.contains(&new) {
        rows.push(Row::Fold { start: new, count });
        return;
    }
    for index in 0..count {
        let number = new + index;
        rows.push(Row::Line(DiffLine {
            kind: LineKind::Context,
            old: Some(old + index),
            new: Some(number),
            text: line_text(text, number),
        }));
    }
}

/// The text of new line `number` (1-based), empty past the file's end.
fn line_text(text: &[&str], number: u32) -> String {
    let index = usize::try_from(number.saturating_sub(1)).unwrap_or(usize::MAX);
    text.get(index).copied().unwrap_or("").to_owned()
}

/// The file as it is, every line unchanged.
fn plain(text: &[&str]) -> Vec<Row> {
    text.iter()
        .enumerate()
        .map(|(index, line)| {
            Row::Line(DiffLine {
                kind: LineKind::Context,
                old: None,
                new: Some(u32::try_from(index + 1).unwrap_or(u32::MAX)),
                text: (*line).to_owned(),
            })
        })
        .collect()
}

/// The row indices where each run of added or removed lines starts.
#[must_use]
pub fn changes(rows: &[Row]) -> Vec<usize> {
    run_starts(rows.iter().map(|row| {
        matches!(
            row,
            Row::Line(DiffLine {
                kind: LineKind::Added | LineKind::Removed,
                ..
            })
        )
    }))
}

/// The side-by-side rows of `rows`: a context line is the same line on both
/// sides; a run of removed lines is paired by position with the added lines
/// after it, and the rest of either run has an empty side.
#[must_use]
pub fn side_by_side(rows: &[Row]) -> Vec<SideRow> {
    let mut out = Vec::new();
    let mut rest = rows;
    while let Some((first, tail)) = rest.split_first() {
        match first {
            Row::Fold { start, count } => {
                out.push(SideRow::Fold {
                    start: *start,
                    count: *count,
                });
                rest = tail;
            }
            Row::Line(line) if line.kind == LineKind::Context => {
                out.push(SideRow::Pair {
                    old: Some(line.clone()),
                    new: Some(line.clone()),
                });
                rest = tail;
            }
            Row::Line(_) => {
                let (removed, after) = take_run(rest, LineKind::Removed);
                let (added, after) = take_run(after, LineKind::Added);
                for index in 0..removed.len().max(added.len()) {
                    out.push(SideRow::Pair {
                        old: removed.get(index).map(|line| (*line).clone()),
                        new: added.get(index).map(|line| (*line).clone()),
                    });
                }
                rest = after;
            }
        }
    }
    out
}

/// The indices of the side-by-side rows where each run of changed pairs
/// starts.
#[must_use]
pub fn side_changes(rows: &[SideRow]) -> Vec<usize> {
    run_starts(rows.iter().map(|row| match row {
        SideRow::Pair { old, new } => {
            let context = |line: &Option<DiffLine>| {
                line.as_ref()
                    .is_some_and(|line| line.kind == LineKind::Context)
            };
            !(context(old) && context(new))
        }
        SideRow::Fold { .. } => false,
    }))
}

/// The lines of the run of `kind` lines at the start of `rows`, and the rows
/// after them.
fn take_run(rows: &[Row], kind: LineKind) -> (Vec<&DiffLine>, &[Row]) {
    let end = rows
        .iter()
        .position(|row| !matches!(row, Row::Line(line) if line.kind == kind))
        .unwrap_or(rows.len());
    let (run, rest) = rows.split_at(end);
    let lines = run
        .iter()
        .filter_map(|row| match row {
            Row::Line(line) => Some(line),
            Row::Fold { .. } => None,
        })
        .collect();
    (lines, rest)
}

/// The index of each run of `true` values.
fn run_starts(flags: impl Iterator<Item = bool>) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut inside = false;
    for (index, flag) in flags.enumerate() {
        if flag && !inside {
            starts.push(index);
        }
        inside = flag;
    }
    starts
}
