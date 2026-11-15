//! A review's files as the patches gpui-component's diff parses.
//!
//! Each file becomes a Git patch of its own, named `repo/path` so files
//! from different repos never share a path. By default a patch holds only
//! the file's hunks; with [`Context::Full`] the unchanged lines between and
//! around them come from [`FileDiff::new_lines`], so the diff can fold and
//! unfold them itself (RV-18, RV-21).

use crate::model::{FileDiff, FileStatus, Hunk, LineKind};

/// How much of the file a patch holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Context {
    /// The hunks as the daemon sent them.
    Hunks,
    /// The whole file, as one hunk; the hunks alone when the file's text is
    /// missing or doesn't match them.
    Full,
}

/// The path a file goes by in the diff: its repo, then its path in the repo.
pub(crate) fn key(repo: &str, path: &str) -> String {
    format!("{repo}/{path}")
}

/// The file as a Git patch.
pub(crate) fn file_patch(repo: &str, file: &FileDiff, context: Context) -> String {
    let new = quote(&format!("b/{}", key(repo, &file.path)));
    let old_path = match &file.status {
        FileStatus::Renamed { from } => from.as_str(),
        _ => file.path.as_str(),
    };
    let old = quote(&format!("a/{}", key(repo, old_path)));
    let mut patch = format!("diff --git {old} {new}\n");
    match &file.status {
        FileStatus::Added => patch.push_str("new file mode 100644\n"),
        FileStatus::Deleted => patch.push_str("deleted file mode 100644\n"),
        FileStatus::Renamed { from } => {
            patch.push_str(&format!("rename from {}\n", quote(&key(repo, from))));
            patch.push_str(&format!("rename to {}\n", quote(&key(repo, &file.path))));
        }
        FileStatus::Modified => {}
    }
    let hunks: Vec<Hunk>;
    let hunks = match context {
        Context::Full => match full_hunk(file) {
            Some(full) => {
                hunks = vec![full];
                &hunks[..]
            }
            None => &file.hunks[..],
        },
        Context::Hunks => &file.hunks[..],
    };
    if hunks.is_empty() {
        return patch;
    }
    let from = if file.status == FileStatus::Added { "/dev/null".to_owned() } else { old };
    let to = if file.status == FileStatus::Deleted { "/dev/null".to_owned() } else { new };
    patch.push_str(&format!("--- {from}\n+++ {to}\n"));
    for hunk in hunks {
        patch.push_str(&hunk.header());
        patch.push('\n');
        for line in &hunk.lines {
            patch.push(line.kind.mark());
            patch.push_str(&line.text);
            patch.push('\n');
        }
    }
    patch
}

/// The file as it is now, without changes, for full-file mode with
/// changes hidden (RV-22); `None` without its text.
pub(crate) fn file_text(file: &FileDiff) -> Option<String> {
    let lines = file.new_lines.as_ref()?;
    let mut text = String::with_capacity(lines.iter().map(|line| line.len() + 1).sum());
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    Some(text)
}

/// The whole file as one hunk: the hunks with the unchanged lines between
/// and around them taken from the file's text. `None` when the text is
/// missing or the hunks don't fit it.
fn full_hunk(file: &FileDiff) -> Option<Hunk> {
    let text = file.new_lines.as_ref()?;
    if file.hunks.is_empty() || matches!(file.status, FileStatus::Added | FileStatus::Deleted) {
        return None;
    }
    let mut lines = Vec::with_capacity(text.len() + file.hunks.len() * 4);
    // The next new line, counted from 1, not yet in `lines`.
    let mut next = 1u32;
    for hunk in &file.hunks {
        let first = hunk.new_first();
        if first < next {
            return None;
        }
        for number in next..first {
            let line = text.get(index(number)?)?;
            lines.push(crate::model::DiffLine::new(LineKind::Context, line.clone()));
        }
        let mut number = first;
        for line in &hunk.lines {
            if line.kind != LineKind::Removed {
                if text.get(index(number)?)? != &line.text {
                    return None;
                }
                number += 1;
            }
            lines.push(line.clone());
        }
        next = number;
    }
    let last = u32::try_from(text.len()).ok()?;
    for number in next..=last {
        lines.push(crate::model::DiffLine::new(LineKind::Context, text.get(index(number)?)?.clone()));
    }
    Some(Hunk {
        old_start: 1,
        new_start: 1,
        section: String::new(),
        lines,
    })
}

fn index(number: u32) -> Option<usize> {
    usize::try_from(number.checked_sub(1)?).ok()
}

/// A path as Git writes it in a header: as is, or in double quotes with C
/// escapes when it holds a quote, a backslash or a control character.
fn quote(path: &str) -> String {
    let plain = !path.starts_with('"')
        && path.chars().all(|ch| ch != '"' && ch != '\\' && !ch.is_control());
    if plain {
        return path.to_owned();
    }
    let mut quoted = String::with_capacity(path.len() + 2);
    quoted.push('"');
    for ch in path.chars() {
        match ch {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\t' => quoted.push_str("\\t"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            ch if ch.is_control() => {
                let mut buffer = [0; 4];
                for byte in ch.encode_utf8(&mut buffer).bytes() {
                    quoted.push_str(&format!("\\{byte:03o}"));
                }
            }
            ch => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::diff::{DiffFile, DiffFileStatus};

    use super::*;
    use crate::model::{DiffLine, parse_unified};

    fn hunk(old_start: u32, new_start: u32, lines: &[(LineKind, &str)]) -> Hunk {
        Hunk {
            old_start,
            new_start,
            section: String::new(),
            lines: lines.iter().map(|(kind, text)| DiffLine::new(*kind, *text)).collect(),
        }
    }

    fn parse(patch: &str) -> DiffFile {
        let mut files = DiffFile::parse(patch).expect("the patch parses");
        assert_eq!(files.len(), 1, "one file per patch");
        files.remove(0)
    }

    /// The old and new line numbers the patch holds.
    fn numbers(patch: &str) -> (Vec<u32>, Vec<u32>) {
        let files = parse_unified(patch).expect("the patch reads back");
        let (mut old, mut new) = (Vec::new(), Vec::new());
        for hunk in files.iter().flat_map(|file| &file.hunks) {
            let (mut at_old, mut at_new) = (hunk.old_first(), hunk.new_first());
            for line in &hunk.lines {
                if line.kind != LineKind::Added {
                    old.push(at_old);
                    at_old += 1;
                }
                if line.kind != LineKind::Removed {
                    new.push(at_new);
                    at_new += 1;
                }
            }
        }
        (old, new)
    }

    fn modified() -> FileDiff {
        use LineKind::{Added, Context as Same, Removed};
        FileDiff {
            path: "src/lib.rs".into(),
            hunks: vec![
                hunk(2, 2, &[(Same, "b"), (Removed, "c"), (Added, "C"), (Same, "d")]),
                hunk(8, 8, &[(Same, "h"), (Added, "new"), (Same, "i")]),
            ],
            new_lines: Some(
                ["a", "b", "C", "d", "e", "f", "g", "h", "new", "i", "j"]
                    .map(String::from)
                    .to_vec(),
            ),
            ..FileDiff::default()
        }
    }

    #[test]
    fn a_file_goes_by_its_repo_and_path() {
        let file = parse(&file_patch("api", &modified(), Context::Hunks));
        assert_eq!(file.path().as_str(), "api/src/lib.rs");
        assert_eq!((file.additions(), file.deletions()), (2, 1));
        assert_eq!(file.status(), DiffFileStatus::Modified);
    }

    #[test]
    fn hunks_hold_only_their_lines() {
        let patch = file_patch("api", &modified(), Context::Hunks);
        parse(&patch);
        let (_, new) = numbers(&patch);
        assert_eq!(new, [2, 3, 4, 8, 9, 10]);
    }

    #[test]
    fn full_context_holds_every_line_with_old_numbers_shifted() {
        let patch = file_patch("api", &modified(), Context::Full);
        let file = parse(&patch);
        assert_eq!((file.additions(), file.deletions()), (2, 1));
        let (old, new) = numbers(&patch);
        assert_eq!(new, (1..=11).collect::<Vec<_>>());
        // One line was added before old line 9 (`i`): the old file has 10.
        assert_eq!(old, (1..=10).collect::<Vec<_>>());
    }

    #[test]
    fn full_context_falls_back_to_hunks_when_the_text_disagrees() {
        let mut stale = modified();
        if let Some(lines) = stale.new_lines.as_mut() {
            lines[2] = "something else".into();
        }
        let hunks = file_patch("api", &modified(), Context::Hunks);
        assert_eq!(file_patch("api", &stale, Context::Full), hunks);
        let mut missing = modified();
        missing.new_lines = None;
        assert_eq!(file_patch("api", &missing, Context::Full), hunks);
    }

    #[test]
    fn added_deleted_and_renamed_files_keep_their_status() {
        let added = FileDiff {
            path: "new.rs".into(),
            status: FileStatus::Added,
            hunks: vec![hunk(0, 1, &[(LineKind::Added, "fn main() {}")])],
            ..FileDiff::default()
        };
        assert_eq!(parse(&file_patch("r", &added, Context::Full)).status(), DiffFileStatus::Added);
        let deleted = FileDiff {
            path: "old.rs".into(),
            status: FileStatus::Deleted,
            hunks: vec![hunk(1, 0, &[(LineKind::Removed, "gone")])],
            ..FileDiff::default()
        };
        let parsed = parse(&file_patch("r", &deleted, Context::Hunks));
        assert_eq!(parsed.status(), DiffFileStatus::Deleted);
        assert_eq!(parsed.path().as_str(), "r/old.rs");
        let renamed = FileDiff {
            path: "to.rs".into(),
            status: FileStatus::Renamed { from: "from.rs".into() },
            ..FileDiff::default()
        };
        let parsed = parse(&file_patch("r", &renamed, Context::Hunks));
        assert_eq!(parsed.status(), DiffFileStatus::Renamed);
        assert_eq!(parsed.path().as_str(), "r/to.rs");
        assert_eq!(parsed.original_path().map(|path| path.as_str()), Some("r/from.rs"));
    }

    #[test]
    fn odd_paths_survive_the_header() {
        for path in ["with space.rs", "tab\there.rs", "quote\"d.rs", "back\\slash.rs", "ünï.rs"] {
            let file = FileDiff {
                path: path.into(),
                hunks: vec![hunk(1, 1, &[(LineKind::Removed, "a"), (LineKind::Added, "b")])],
                ..FileDiff::default()
            };
            let parsed = parse(&file_patch("repo", &file, Context::Hunks));
            assert_eq!(parsed.path().as_str(), format!("repo/{path}"));
        }
    }

    #[test]
    fn file_text_is_every_line() {
        let text = file_text(&modified()).expect("the file has text");
        assert_eq!(text.lines().count(), 11);
        let file = DiffFile::unchanged("api/src/lib.rs", &text);
        assert_eq!(file.status(), DiffFileStatus::Unchanged);
    }
}
