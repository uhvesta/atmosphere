//! Finding a comment's quoted lines again in its file as it is now
//! (SM-7, SM-16, D190): still in place, moved unchanged, changed, or gone.

use std::cmp::Reverse;
use std::collections::HashMap;

/// Where a comment's quoted lines are in the file as it is now.
pub enum Placement {
    /// Still at the lines the anchor names.
    Same,
    /// Unchanged but at other lines (SM-7): 1-based inclusive.
    Moved { first: u32, last: u32 },
    /// Changed but still there (SM-16, D190): the window of lines that now
    /// holds most of the quoted words, 1-based inclusive, and what is at
    /// those lines.
    Changed {
        first: u32,
        last: u32,
        now: Vec<String>,
    },
    /// The passage is gone (SM-7, D190): fewer than half of its words are
    /// left in the best window, or the file is empty.
    Gone,
}

/// A line without its whitespace, so a change that only alters whitespace
/// does not count (SM-16), as `git diff -w` reads it.
fn squash(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The words of a line: maximal runs of alphanumeric characters, case kept.
fn words(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
}

/// How many of the quoted words, counted as a multiset, the window also has.
fn shared(want: &HashMap<&str, usize>, window: &[&str]) -> usize {
    let mut have: HashMap<&str, usize> = HashMap::new();
    for line in window {
        for word in words(line) {
            *have.entry(word).or_default() += 1;
        }
    }
    want.iter()
        .map(|(word, count)| (*count).min(have.get(word).copied().unwrap_or(0)))
        .sum()
}

fn line_number(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Finds `quoted` (the anchored lines as they were, at 1-based `first`) in
/// `lines`, the file's lines now.
///
/// Lines that match once squashed are Same or Moved. Otherwise the quote's
/// words are scored against every window of its length (the whole file if
/// it is shorter): the best window is Changed when it keeps at least half
/// the quote's words, and Gone when it doesn't. Ties go to the window
/// nearest the anchor, then the earlier one. A quote with no words is
/// Changed at the anchor's own lines when they are still in the file.
#[must_use]
pub fn place(quoted: &[String], first: u32, lines: &[&str]) -> Placement {
    let len = quoted.len();
    if len == 0 {
        return Placement::Same;
    }
    let want_lines: Vec<String> = quoted.iter().map(|line| squash(line)).collect();
    let now: Vec<String> = lines.iter().map(|line| squash(line)).collect();
    let at = |start: usize| {
        len <= now.len() - start.min(now.len()) && now[start..start + len] == want_lines[..]
    };
    let start = (first as usize).saturating_sub(1);
    if first != 0 && start <= now.len() && at(start) {
        return Placement::Same;
    }
    if len <= now.len() {
        let found = (0..=now.len() - len)
            .filter(|s| now[*s..*s + len] == want_lines[..])
            .min_by_key(|s| (s + 1).abs_diff(first as usize));
        if let Some(s) = found {
            return Placement::Moved {
                first: line_number(s + 1),
                last: line_number(s + len),
            };
        }
    }
    if lines.is_empty() {
        return Placement::Gone;
    }

    let mut want: HashMap<&str, usize> = HashMap::new();
    for line in quoted {
        for word in words(line) {
            *want.entry(word).or_default() += 1;
        }
    }
    let total: usize = want.values().sum();
    if total == 0 {
        let inside = first >= 1 && start + len <= lines.len();
        if !inside {
            return Placement::Gone;
        }
        return Placement::Changed {
            first,
            last: line_number(start + len),
            now: lines[start..start + len]
                .iter()
                .map(|line| (*line).to_string())
                .collect(),
        };
    }

    let width = len.min(lines.len());
    let best = (0..=lines.len() - width)
        .map(|s| (s, shared(&want, &lines[s..s + width])))
        .min_by_key(|&(s, score)| (Reverse(score), (s + 1).abs_diff(first as usize), s));
    let Some((s, score)) = best else {
        return Placement::Gone;
    };
    if 2 * score < total {
        return Placement::Gone;
    }
    Placement::Changed {
        first: line_number(s + 1),
        last: line_number(s + width),
        now: lines[s..s + width]
            .iter()
            .map(|line| (*line).to_string())
            .collect(),
    }
}
