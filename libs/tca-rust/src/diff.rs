//! Line diffs of two values as Debug shows them, for test failures.

/// Lines kept around each change.
const CONTEXT: usize = 2;

/// Past this many lines differing on each side, both are shown whole
/// instead of matched line by line.
const MATCH_LIMIT: usize = 2_000;

/// `expected` and `actual` line by line: `-` lines only expected, `+`
/// lines only actual, with a little of what's the same around them.
pub(crate) fn diff(expected: &str, actual: &str) -> String {
    let old: Vec<&str> = expected.lines().collect();
    let new: Vec<&str> = actual.lines().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old_middle = &old[prefix..old.len() - suffix];
    let new_middle = &new[prefix..new.len() - suffix];

    let mut lines: Vec<(char, &str)> = Vec::new();
    for line in &old[prefix.saturating_sub(CONTEXT)..prefix] {
        lines.push((' ', line));
    }
    if old_middle.len() > MATCH_LIMIT || new_middle.len() > MATCH_LIMIT {
        lines.extend(old_middle.iter().map(|line| ('-', *line)));
        lines.extend(new_middle.iter().map(|line| ('+', *line)));
    } else {
        lines.extend(matched(old_middle, new_middle));
    }
    let after = old.len() - suffix;
    for line in &old[after..(after + CONTEXT).min(old.len())] {
        lines.push((' ', line));
    }

    let mut shown = String::new();
    for (mark, line) in lines {
        shown.push(mark);
        shown.push(' ');
        shown.push_str(line);
        shown.push('\n');
    }
    shown
}

/// The two runs of lines matched by their longest common subsequence.
fn matched<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(char, &'a str)> {
    // `longest[i][j]`: the longest common run of old[i..] and new[j..].
    let mut longest = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            longest[i][j] = if old[i] == new[j] {
                longest[i + 1][j + 1] + 1
            } else {
                longest[i + 1][j].max(longest[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut lines = Vec::new();
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            lines.push((' ', old[i]));
            i += 1;
            j += 1;
        } else if j < new.len() && (i == old.len() || longest[i][j + 1] >= longest[i + 1][j]) {
            lines.push(('+', new[j]));
            j += 1;
        } else {
            lines.push(('-', old[i]));
            i += 1;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(diff: &str, mark: char) -> Vec<String> {
        diff.lines()
            .filter(|line| line.starts_with(mark))
            .map(|line| line[2..].to_owned())
            .collect()
    }

    #[test]
    fn only_the_lines_that_differ_are_marked() {
        let expected = "a\nb\nc\nd\ne\nf\ng";
        let actual = "a\nb\nc\nD\ne\nf\ng\nh";
        let shown = diff(expected, actual);
        assert_eq!(marked(&shown, '-'), ["d"]);
        assert_eq!(marked(&shown, '+'), ["D", "h"]);
    }

    #[test]
    fn equal_text_has_no_changes() {
        let shown = diff("a\nb", "a\nb");
        assert!(marked(&shown, '-').is_empty() && marked(&shown, '+').is_empty());
    }
}
