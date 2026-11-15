//! What the diff view lists, in order, and where its notes and folds go, as
//! plain data built from a [`Review`].
//!
//! gpui-component's diff lists files, each under a header. The view lists
//! more than files: a header per repo, and for a prepared review a heading
//! per group (RV-32). Each of those is an [`Entry`] that becomes one header
//! in the diff, so they scroll, virtualize and keep their place as files
//! do. Notes, folds and comments are annotations under a header or a line;
//! [`place_note`] and [`gaps`] say which. Everything here is tested without
//! a window.

use std::collections::HashSet;

use crate::anchor::{LineTarget, Side};
use crate::guide::{FileRef, NoteTarget};
use crate::model::{FileDiff, FileStatus, Hunk, LineKind, Review};
use crate::patch;

/// How a file's changes are laid out (RV-17).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DiffMode {
    /// Old and new line numbers, removed and added lines interleaved.
    #[default]
    Inline,
    /// Removed lines on the left beside the added lines.
    SideBySide,
    /// The whole current file with the changes in place (RV-21).
    FullFile,
}

impl DiffMode {
    pub const ALL: [Self; 3] = [Self::Inline, Self::SideBySide, Self::FullFile];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Inline => "Inline",
            Self::SideBySide => "Side by side",
            Self::FullFile => "Full file",
        }
    }
}

/// Identifies an annotation the embedder supplies, such as a comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AnnotationId(pub u64);

/// Something drawn under a line range, such as a comment, a draft or a
/// "not sent yet" note. The diff view places it under the last line of its
/// target in every mode, or under the file's header when that line isn't
/// shown, and asks the embedder to draw it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Annotation {
    pub id: AnnotationId,
    pub target: LineTarget,
}

/// One header in the diff: a repo's, a group's, or a file's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Entry {
    Repo { repo: usize },
    /// A group of a prepared review with how many files it lists; the
    /// group after the guide's last is "Other changes".
    Group { group: usize, files: usize },
    File { repo: usize, file: usize },
}

/// The path a repo's header goes by in the diff. No file's path starts
/// with a NUL, so headings and files never share one.
pub(crate) fn repo_path(name: &str) -> String {
    format!("\u{0}repo/{name}")
}

/// The path a group's heading goes by in the diff.
pub(crate) fn group_path(group: usize) -> String {
    format!("\u{0}group/{group}")
}

/// Whether a path in the diff is a heading rather than a file.
pub(crate) fn is_heading(path: &str) -> bool {
    path.starts_with('\u{0}')
}

impl Entry {
    /// The entry's path in the diff.
    pub(crate) fn path(self, review: &Review) -> Option<String> {
        Some(match self {
            Self::Repo { repo } => repo_path(&review.repos.get(repo)?.name),
            Self::Group { group, .. } => group_path(group),
            Self::File { repo, file } => {
                let repo = review.repos.get(repo)?;
                patch::key(&repo.name, &repo.files.get(file)?.path)
            }
        })
    }
}

/// The review's headers in order. Without a guide's groups: each repo's
/// header, then its files. With them: every repo's header, then each group
/// that lists a file under its heading, then "Other changes". A collapsed
/// repo keeps its header and loses its files everywhere.
pub(crate) fn entries(review: &Review, collapsed: &HashSet<String>) -> Vec<Entry> {
    let mut entries = Vec::with_capacity(review.repos.len() + review.file_count());
    let groups = review.guide.as_ref().map(|guide| guide.groups.as_slice()).unwrap_or_default();
    let shown = |repo: usize| review.repos.get(repo).is_some_and(|r| !collapsed.contains(&r.name));
    for (repo_ix, repo) in review.repos.iter().enumerate() {
        entries.push(Entry::Repo { repo: repo_ix });
        if groups.is_empty() && shown(repo_ix) {
            entries.extend((0..repo.files.len()).map(|file| Entry::File { repo: repo_ix, file }));
        }
    }
    if groups.is_empty() {
        return entries;
    }
    let find = |file: &FileRef| -> Option<(usize, usize)> {
        let repo = review.repos.iter().position(|repo| repo.name == file.repo)?;
        let at = review.repos[repo].files.iter().position(|f| f.path == file.path)?;
        Some((repo, at))
    };
    let mut listed: HashSet<(usize, usize)> = HashSet::new();
    let mut lists: Vec<Vec<(usize, usize)>> = groups
        .iter()
        .map(|group| group.files.iter().filter_map(find).filter(|at| listed.insert(*at)).collect())
        .collect();
    lists.push(
        review
            .repos
            .iter()
            .enumerate()
            .flat_map(|(repo, r)| (0..r.files.len()).map(move |file| (repo, file)))
            .filter(|at| !listed.contains(at))
            .collect(),
    );
    for (group, files) in lists.iter().enumerate() {
        if files.is_empty() {
            continue;
        }
        entries.push(Entry::Group { group, files: files.len() });
        entries.extend(
            files
                .iter()
                .filter(|(repo, _)| shown(*repo))
                .map(|&(repo, file)| Entry::File { repo, file }),
        );
    }
    entries
}

/// Where an annotation goes: under a file's header, or under a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Place {
    File,
    Line { side: Side, line: u32 },
}

/// Whether the file's patch shows `line` on `side`: any line of the file
/// with its whole text, otherwise only the lines of its hunks.
pub(crate) fn shown(file: &FileDiff, whole: bool, side: Side, line: u32) -> bool {
    if whole {
        return line_text(file, side, line).is_some();
    }
    file.hunks.iter().any(|hunk| hunk_covers(hunk, side, line))
}

/// Where a comment on `target` goes in a file whose patch is whole or not.
pub(crate) fn place_lines(file: &FileDiff, whole: bool, target: &LineTarget) -> Place {
    if shown(file, whole, target.side, target.lines.end) {
        Place::Line {
            side: target.side,
            line: target.lines.end,
        }
    } else {
        Place::File
    }
}

/// Where a guide's note goes, and whether it is outdated: a note on a hunk
/// goes under the hunk's first change, a note on lines under the last of
/// them. A hunk that is gone, or lines that no longer read as quoted, make
/// the note outdated (RV-34); it then goes where its lines still are, or
/// under the file's header.
pub(crate) fn place_note(file: &FileDiff, whole: bool, target: &NoteTarget) -> (Place, bool) {
    match target {
        NoteTarget::File(_) => (Place::File, false),
        NoteTarget::Hunk { side, line, .. } => {
            let hunk = file.hunks.iter().find(|hunk| hunk_covers(hunk, *side, *line));
            match hunk.and_then(first_change) {
                Some((side, line)) => (Place::Line { side, line }, false),
                None => (Place::File, true),
            }
        }
        NoteTarget::Lines { target, quoted } => {
            let outdated = !reads_as(file, target, quoted);
            (place_lines(file, whole, target), outdated)
        }
    }
}

/// The hunk's first added or removed line, or its first line when it has
/// no change.
fn first_change(hunk: &Hunk) -> Option<(Side, u32)> {
    let (mut old, mut new) = (hunk.old_first(), hunk.new_first());
    let mut first = None;
    for line in &hunk.lines {
        let at = match line.kind {
            LineKind::Removed => (Side::Old, old),
            LineKind::Added | LineKind::Context => (Side::New, new),
        };
        first.get_or_insert(at);
        if line.kind != LineKind::Context {
            return Some(at);
        }
        if line.kind != LineKind::Added {
            old += 1;
        }
        if line.kind != LineKind::Removed {
            new += 1;
        }
    }
    first
}

/// The hunk's last line, where the unchanged lines after it fold.
fn last_line(hunk: &Hunk) -> Option<(Side, u32)> {
    let kind = hunk.lines.last()?.kind;
    Some(if kind == LineKind::Removed {
        (Side::Old, hunk.old_end().checked_sub(1)?)
    } else {
        (Side::New, hunk.new_end().checked_sub(1)?)
    })
}

/// Unchanged lines a patch of hunks leaves out (RV-18).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Gap {
    /// The first new line left out, counted from 1.
    pub new_start: u32,
    pub len: u32,
    /// Where the fold is drawn: under the file's header before the first
    /// hunk, otherwise under the last line before the gap.
    pub place: Place,
}

/// The unchanged lines before, between and after the file's hunks. The
/// lines after the last hunk are known only with the file's text.
pub(crate) fn gaps(file: &FileDiff) -> Vec<Gap> {
    if matches!(file.status, FileStatus::Added | FileStatus::Deleted) {
        return Vec::new();
    }
    let mut gaps = Vec::new();
    let mut next = 1u32;
    let mut place = Place::File;
    for hunk in &file.hunks {
        let first = hunk.new_first();
        if first > next {
            gaps.push(Gap {
                new_start: next,
                len: first - next,
                place,
            });
        }
        next = next.max(hunk.new_end());
        if let Some((side, line)) = last_line(hunk) {
            place = Place::Line { side, line };
        }
    }
    let total = file.new_lines.as_ref().and_then(|lines| u32::try_from(lines.len()).ok());
    if let Some(total) = total.filter(|total| *total >= next && !file.hunks.is_empty()) {
        gaps.push(Gap {
            new_start: next,
            len: total + 1 - next,
            place,
        });
    }
    gaps
}

fn hunk_covers(hunk: &Hunk, side: Side, line: u32) -> bool {
    let (first, end) = match side {
        Side::Old => (hunk.old_first(), hunk.old_end()),
        Side::New => (hunk.new_first(), hunk.new_end()),
    };
    (first..end).contains(&line)
}

/// Whether the target's lines still read `quoted`.
fn reads_as(file: &FileDiff, target: &LineTarget, quoted: &str) -> bool {
    let mut expected = quoted.lines();
    for number in target.lines.start..=target.lines.end {
        match (line_text(file, target.side, number), expected.next()) {
            (Some(now), Some(then)) if now == then => {}
            _ => return false,
        }
    }
    expected.next().is_none()
}

/// The text of line `number` on `side`: from the hunks, or between them
/// from the file's text; `None` when neither has it.
pub(crate) fn line_text(file: &FileDiff, side: Side, number: u32) -> Option<&str> {
    let pick = |old: u32, new: u32| match side {
        Side::Old => old,
        Side::New => new,
    };
    // How far new line numbers are ahead of old ones in the unchanged
    // region being walked.
    let mut shift: i64 = 0;
    for hunk in &file.hunks {
        let first = pick(hunk.old_first(), hunk.new_first());
        if number < first {
            break;
        }
        let (mut old, mut new) = (hunk.old_first(), hunk.new_first());
        for line in &hunk.lines {
            let numbers = match line.kind {
                LineKind::Context => (Some(old), Some(new)),
                LineKind::Removed => (Some(old), None),
                LineKind::Added => (None, Some(new)),
            };
            if pick(numbers.0.unwrap_or(0), numbers.1.unwrap_or(0)) == number {
                return Some(&line.text);
            }
            if line.kind != LineKind::Added {
                old += 1;
            }
            if line.kind != LineKind::Removed {
                new += 1;
            }
        }
        shift = i64::from(hunk.new_end()) - i64::from(hunk.old_end());
    }
    let new = match side {
        Side::New => i64::from(number),
        Side::Old => i64::from(number) + shift,
    };
    let index = usize::try_from(new.checked_sub(1)?).ok()?;
    file.new_lines.as_ref()?.get(index).map(String::as_str)
}

/// Changed lines as runs, the way the diff counts changes for "2 of 5":
/// each run of added and removed lines in a hunk is one change.
#[cfg(test)]
pub(crate) fn change_count(file: &FileDiff) -> usize {
    file.hunks
        .iter()
        .map(|hunk| {
            let mut runs = 0;
            let mut in_run = false;
            for line in &hunk.lines {
                let changed = line.kind != LineKind::Context;
                if changed && !in_run {
                    runs += 1;
                }
                in_run = changed;
            }
            runs
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anchor::LineRange;
    use crate::fixture;
    use crate::model::DiffLine;

    fn hunk(old_start: u32, new_start: u32, lines: &[(LineKind, &str)]) -> Hunk {
        Hunk {
            old_start,
            new_start,
            section: String::new(),
            lines: lines.iter().map(|(kind, text)| DiffLine::new(*kind, *text)).collect(),
        }
    }

    /// `a`..`k`, with `c` changed to `C` and `new` added after `h`.
    fn file(with_text: bool) -> FileDiff {
        use LineKind::{Added, Context as Same, Removed};
        FileDiff {
            path: "src/lib.rs".into(),
            hunks: vec![
                hunk(2, 2, &[(Same, "b"), (Removed, "c"), (Added, "C"), (Same, "d")]),
                hunk(8, 8, &[(Same, "h"), (Added, "new"), (Same, "i")]),
            ],
            new_lines: with_text.then(|| {
                ["a", "b", "C", "d", "e", "f", "g", "h", "new", "i", "j", "k"]
                    .map(String::from)
                    .to_vec()
            }),
            ..FileDiff::default()
        }
    }

    fn names(review: &Review, entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match *entry {
                Entry::Repo { repo } => format!("repo {}", review.repos[repo].name),
                Entry::Group { group, files } => format!("group {group} of {files}"),
                Entry::File { repo, file } => {
                    let repo = &review.repos[repo];
                    format!("file {}/{}", repo.name, repo.files[file].path)
                }
            })
            .collect()
    }

    #[test]
    fn each_repo_lists_its_files_under_its_header() {
        let review = fixture::small();
        let names = names(&review, &entries(&review, &HashSet::new()));
        let repos: Vec<_> = names.iter().filter(|name| name.starts_with("repo")).collect();
        assert_eq!(repos.len(), review.repos.len());
        assert_eq!(names.len(), review.repos.len() + review.file_count());
        let daemon = names.iter().position(|name| name == "repo humata-daemon");
        let client = names.iter().position(|name| name == "file humata-daemon/src/client.go");
        assert!(daemon < client, "{names:?}");
    }

    #[test]
    fn a_prepared_review_lists_files_in_its_groups_then_the_rest() {
        let review = fixture::prepared();
        assert_eq!(
            names(&review, &entries(&review, &HashSet::new())),
            [
                "repo atmosphere",
                "repo humata-daemon",
                "group 0 of 1",
                "file humata-daemon/src/client.go",
                // The missing file is skipped and the ledger, named twice,
                // is listed once.
                "group 1 of 2",
                "file atmosphere/src/ledger.rs",
                "file atmosphere/docs/ROUNDING.md",
                "group 2 of 1",
                "file humata-daemon/assets/icon.png",
            ]
        );
    }

    #[test]
    fn a_collapsed_repo_keeps_its_header_and_hides_its_files_everywhere() {
        for review in [fixture::small(), fixture::prepared()] {
            let collapsed = HashSet::from(["atmosphere".to_owned()]);
            let names = names(&review, &entries(&review, &collapsed));
            assert!(names.iter().any(|name| name == "repo atmosphere"));
            assert!(!names.iter().any(|name| name.starts_with("file atmosphere/")));
            assert!(names.iter().any(|name| name == "file humata-daemon/src/client.go"));
        }
    }

    #[test]
    fn headings_never_share_a_path_with_a_file() {
        let review = fixture::prepared();
        let paths: Vec<String> = entries(&review, &HashSet::new())
            .iter()
            .filter_map(|entry| entry.path(&review))
            .collect();
        let unique: HashSet<&String> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len());
        for (entry, path) in entries(&review, &HashSet::new()).iter().zip(&paths) {
            assert_eq!(is_heading(path), !matches!(entry, Entry::File { .. }), "{path:?}");
        }
    }

    #[test]
    fn gaps_fold_the_lines_between_and_around_hunks() {
        let with_text = gaps(&file(true));
        assert_eq!(
            with_text,
            [
                Gap { new_start: 1, len: 1, place: Place::File },
                Gap {
                    new_start: 5,
                    len: 3,
                    place: Place::Line { side: Side::New, line: 4 },
                },
                Gap {
                    new_start: 11,
                    len: 2,
                    place: Place::Line { side: Side::New, line: 10 },
                },
            ]
        );
        // Without the text, what follows the last hunk is unknown.
        assert_eq!(gaps(&file(false)), with_text[..2]);
    }

    #[test]
    fn added_and_deleted_files_have_no_gaps() {
        let added = FileDiff {
            status: FileStatus::Added,
            hunks: vec![hunk(0, 1, &[(LineKind::Added, "x")])],
            ..FileDiff::default()
        };
        assert!(gaps(&added).is_empty());
    }

    #[test]
    fn a_hunk_note_goes_under_its_first_change() {
        let target = NoteTarget::Hunk {
            file: FileRef::new("api", "src/lib.rs"),
            side: Side::New,
            line: 9,
        };
        let (place, outdated) = place_note(&file(false), false, &target);
        assert_eq!(place, Place::Line { side: Side::New, line: 9 });
        assert!(!outdated);
        let first = NoteTarget::Hunk {
            file: FileRef::new("api", "src/lib.rs"),
            side: Side::New,
            line: 2,
        };
        // The first hunk's first change is the removed `c`, old line 3.
        assert_eq!(place_note(&file(false), false, &first).0, Place::Line { side: Side::Old, line: 3 });
    }

    #[test]
    fn a_note_on_a_hunk_that_is_gone_is_outdated_under_the_header() {
        let target = NoteTarget::Hunk {
            file: FileRef::new("api", "src/lib.rs"),
            side: Side::New,
            line: 6,
        };
        assert_eq!(place_note(&file(true), true, &target), (Place::File, true));
    }

    #[test]
    fn a_note_on_lines_is_outdated_once_they_read_differently() {
        let target = |start, end| LineTarget {
            repo: "api".into(),
            path: "src/lib.rs".into(),
            side: Side::New,
            lines: LineRange { start, end },
        };
        let note = |start, end, quoted: &str| NoteTarget::Lines {
            target: target(start, end),
            quoted: quoted.into(),
        };
        let place = Place::Line { side: Side::New, line: 4 };
        assert_eq!(place_note(&file(false), false, &note(3, 4, "C\nd")), (place, false));
        assert_eq!(place_note(&file(false), false, &note(3, 4, "c\nd")), (place, true));
        // Line 6 is in a gap: shown with the whole file, under the header
        // without it.
        let whole = place_note(&file(true), true, &note(6, 6, "f"));
        assert_eq!(whole, (Place::Line { side: Side::New, line: 6 }, false));
        assert_eq!(place_note(&file(true), false, &note(6, 6, "f")), (Place::File, false));
    }

    #[test]
    fn line_text_reads_either_side_inside_and_outside_hunks() {
        let file = file(true);
        assert_eq!(line_text(&file, Side::Old, 3), Some("c"));
        assert_eq!(line_text(&file, Side::New, 3), Some("C"));
        assert_eq!(line_text(&file, Side::New, 6), Some("f"));
        // After the added line, old numbers are one behind.
        assert_eq!(line_text(&file, Side::Old, 10), Some("j"));
        assert_eq!(line_text(&file, Side::New, 13), None);
    }

    #[test]
    fn each_run_of_changed_lines_is_one_change() {
        assert_eq!(change_count(&file(false)), 2);
    }
}
