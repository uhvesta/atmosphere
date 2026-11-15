//! Line changes between two snapshot texts, kept whole so they can be
//! applied again: the start of a recording plus its changes gives the
//! text at every step.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// One run of changed lines: at line `at` of the old text (counting from
/// 0), the `removed` lines were replaced by the `added` ones.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    pub at: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
}

impl Hunk {
    pub(crate) fn weight(&self) -> usize {
        16 + self.removed.iter().chain(&self.added).map(|line| line.len() + 1).sum::<usize>()
    }
}

/// A change that doesn't fit the text it is applied to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mismatch {
    pub(crate) line: usize,
}

/// Above this many cells the longest-common-subsequence table is skipped
/// and the changed middle is replaced whole, so diffing two unrelated
/// large texts stays cheap.
const MAX_TABLE: usize = 4_000_000;

/// How `new` differs from `old`, line by line: the fewest lines removed and
/// added, grouped into hunks in order. Empty when they are the same.
pub(crate) fn diff(old: &str, new: &str) -> Vec<Hunk> {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    diff_lines(&old, &new)
}

pub(crate) fn diff_lines<S: AsRef<str>>(old: &[S], new: &[S]) -> Vec<Hunk> {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(a, b)| a.as_ref() == b.as_ref())
        .count();
    let room = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a.as_ref() == b.as_ref())
        .count();
    let old_middle = &old[prefix..old.len() - suffix];
    let new_middle = &new[prefix..new.len() - suffix];
    if old_middle.is_empty() && new_middle.is_empty() {
        return Vec::new();
    }

    let mut hunks = Vec::new();
    let mut open: Option<Hunk> = None;
    for op in ops(old_middle, new_middle) {
        match op {
            Op::Same => hunks.extend(open.take()),
            Op::Removed(i) => open
                .get_or_insert_with(|| Hunk {
                    at: prefix + i,
                    removed: Vec::new(),
                    added: Vec::new(),
                })
                .removed
                .push(old_middle[i].as_ref().to_owned()),
            Op::Added(i, at) => open
                .get_or_insert_with(|| Hunk {
                    at: prefix + at,
                    removed: Vec::new(),
                    added: Vec::new(),
                })
                .added
                .push(new_middle[i].as_ref().to_owned()),
        }
    }
    hunks.extend(open);
    hunks
}

/// A step of the edit script. `Added(j, i)` adds `new[j]` before `old[i]`.
enum Op {
    Same,
    Removed(usize),
    Added(usize, usize),
}

fn ops<S: AsRef<str>>(old: &[S], new: &[S]) -> Vec<Op> {
    let (rows, columns) = (old.len(), new.len());
    let mut ops = Vec::with_capacity(rows + columns);
    if (rows + 1).saturating_mul(columns + 1) > MAX_TABLE {
        ops.extend((0..rows).map(Op::Removed));
        ops.extend((0..columns).map(|j| Op::Added(j, rows)));
        return ops;
    }
    // common[i][j]: the longest common subsequence of old[i..] and new[j..].
    let width = columns + 1;
    let mut common = vec![0u32; (rows + 1) * width];
    for i in (0..rows).rev() {
        for j in (0..columns).rev() {
            common[i * width + j] = if old[i].as_ref() == new[j].as_ref() {
                common[(i + 1) * width + j + 1] + 1
            } else {
                common[(i + 1) * width + j].max(common[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < rows && j < columns {
        if old[i].as_ref() == new[j].as_ref() {
            ops.push(Op::Same);
            i += 1;
            j += 1;
        } else if common[(i + 1) * width + j] >= common[i * width + j + 1] {
            ops.push(Op::Removed(i));
            i += 1;
        } else {
            ops.push(Op::Added(j, i));
            j += 1;
        }
    }
    ops.extend((i..rows).map(Op::Removed));
    ops.extend((j..columns).map(|j| Op::Added(j, rows)));
    ops
}

/// Applies `hunks` to `lines` in place.
///
/// # Errors
///
/// [`Mismatch`] when a hunk's removed lines aren't where it says, so the
/// change was made against another text. `lines` may be partly changed.
pub(crate) fn apply(lines: &mut Vec<String>, hunks: &[Hunk]) -> Result<(), Mismatch> {
    // Each hunk counts lines in the old text, so apply from the last.
    let mut limit = usize::MAX;
    for hunk in hunks.iter().rev() {
        let end = hunk.at + hunk.removed.len();
        if end > lines.len() || end > limit || lines[hunk.at..end] != hunk.removed[..] {
            return Err(Mismatch { line: hunk.at + 1 });
        }
        limit = hunk.at;
        lines.splice(hunk.at..end, hunk.added.iter().cloned());
    }
    Ok(())
}

/// `text` as its lines, for [`apply`].
pub(crate) fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

/// `text` with each line ending in one newline, as [`join`] gives it back,
/// so a tree rebuilt from changes equals the tree it was recorded from.
pub(crate) fn normal(text: &str) -> String {
    join(&lines(text))
}

/// `lines` as one text, each line ending in a newline.
pub(crate) fn join(lines: &[String]) -> String {
    let mut text = String::with_capacity(lines.iter().map(|line| line.len() + 1).sum());
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// `hunks` as a readable diff: `@@ line N` before each hunk, then `- ` and
/// `+ ` lines, each line starting with `indent`.
pub(crate) fn render(hunks: &[Hunk], indent: &str) -> String {
    let mut text = String::new();
    for hunk in hunks {
        let _ = writeln!(text, "{indent}@@ line {}", hunk.at + 1);
        for line in &hunk.removed {
            let _ = writeln!(text, "{indent}- {line}");
        }
        for line in &hunk.added {
            let _ = writeln!(text, "{indent}+ {line}");
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(old: &str, new: &str) {
        let hunks = diff(old, new);
        let mut applied = lines(old);
        apply(&mut applied, &hunks).expect("a diff applies to the text it was made from");
        assert_eq!(applied, lines(new), "{old:?} -> {new:?} via {hunks:?}");
    }

    #[test]
    fn a_diff_applied_to_its_old_text_gives_the_new_one() {
        round_trip("a\nb\nc\n", "a\nb\nc\n");
        round_trip("a\nb\nc\n", "a\nB\nc\n");
        round_trip("a\nb\nc\nd\n", "a\nb\nx\nc\nd\n");
        round_trip("1\n2\n3\n4\n", "2\n3\n4\n1\n");
        round_trip("", "a\nb\n");
        round_trip("a\nb\n", "");
        round_trip("a\nb\nc\nd\ne\n", "x\nb\ny\nd\nz\nw\n");
    }

    #[test]
    fn equal_texts_have_no_hunks() {
        assert!(diff("a\nb\n", "a\nb\n").is_empty());
    }

    #[test]
    fn a_change_made_against_another_text_does_not_apply() {
        let hunks = diff("a\nb\nc\n", "a\nB\nc\n");
        let mut other = lines("a\nx\nc\n");
        assert!(apply(&mut other, &hunks).is_err());
    }
}
