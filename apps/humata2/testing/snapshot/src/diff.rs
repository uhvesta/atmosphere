//! A line diff, for showing how a snapshot or a state changed.

/// One line of a diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Line<'a> {
    Same(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

/// Above this many cells the longest-common-subsequence table is skipped and
/// the changed middle is shown as all removed, then all added. That keeps a
/// diff of two unrelated large texts from taking hundreds of megabytes.
const MAX_TABLE: usize = 4_000_000;

/// How `new` differs from `old`, line by line: the fewest lines removed and
/// added, in order.
pub(crate) fn lines<'a>(old: &'a str, new: &'a str) -> Vec<Line<'a>> {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    let removed = &old[prefix..old.len() - suffix];
    let added = &new[prefix..new.len() - suffix];

    let mut diff = Vec::with_capacity(old.len() + added.len());
    diff.extend(old[..prefix].iter().map(|line| Line::Same(line)));
    middle(removed, added, &mut diff);
    diff.extend(old[old.len() - suffix..].iter().map(|line| Line::Same(line)));
    diff
}

fn middle<'a>(old: &[&'a str], new: &[&'a str], diff: &mut Vec<Line<'a>>) {
    let (rows, columns) = (old.len(), new.len());
    if (rows + 1).saturating_mul(columns + 1) > MAX_TABLE {
        diff.extend(old.iter().map(|line| Line::Removed(line)));
        diff.extend(new.iter().map(|line| Line::Added(line)));
        return;
    }
    // common[i][j]: the longest common subsequence of old[i..] and new[j..].
    let width = columns + 1;
    let mut common = vec![0u32; (rows + 1) * width];
    for i in (0..rows).rev() {
        for j in (0..columns).rev() {
            common[i * width + j] = if old[i] == new[j] {
                common[(i + 1) * width + j + 1] + 1
            } else {
                common[(i + 1) * width + j].max(common[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < rows && j < columns {
        if old[i] == new[j] {
            diff.push(Line::Same(old[i]));
            i += 1;
            j += 1;
        } else if common[(i + 1) * width + j] >= common[i * width + j + 1] {
            diff.push(Line::Removed(old[i]));
            i += 1;
        } else {
            diff.push(Line::Added(new[j]));
            j += 1;
        }
    }
    diff.extend(old[i..].iter().map(|line| Line::Removed(line)));
    diff.extend(new[j..].iter().map(|line| Line::Added(line)));
}

/// `diff` as text: changed lines marked `-` and `+`, with `context`
/// unchanged lines around each change, and `...` where unchanged lines are
/// left out. Empty when nothing changed. Every line starts with `indent`.
pub(crate) fn render(diff: &[Line<'_>], context: usize, indent: &str) -> String {
    let changed: Vec<usize> = diff
        .iter()
        .enumerate()
        .filter(|(_, line)| !matches!(line, Line::Same(_)))
        .map(|(index, _)| index)
        .collect();
    let mut shown = vec![false; diff.len()];
    for &index in &changed {
        let start = index.saturating_sub(context);
        let end = (index + context + 1).min(diff.len());
        shown[start..end].iter_mut().for_each(|show| *show = true);
    }

    let mut text = String::new();
    let mut skipped = false;
    for (line, show) in diff.iter().zip(&shown) {
        if !show {
            skipped = true;
            continue;
        }
        if skipped {
            text.push_str(indent);
            text.push_str("  ...\n");
        }
        skipped = false;
        let (mark, content) = match line {
            Line::Same(content) => (' ', content),
            Line::Removed(content) => ('-', content),
            Line::Added(content) => ('+', content),
        };
        text.push_str(indent);
        text.push(mark);
        text.push(' ');
        text.push_str(content);
        text.push('\n');
    }
    if skipped && !text.is_empty() {
        text.push_str(indent);
        text.push_str("  ...\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn removed_and_added(diff: &[Line<'_>]) -> (Vec<String>, Vec<String>) {
        let mut removed = Vec::new();
        let mut added = Vec::new();
        for line in diff {
            match line {
                Line::Removed(content) => removed.push((*content).to_owned()),
                Line::Added(content) => added.push((*content).to_owned()),
                Line::Same(_) => {}
            }
        }
        (removed, added)
    }

    #[test]
    fn equal_texts_have_no_changes() {
        let diff = lines("a\nb\nc\n", "a\nb\nc\n");
        assert_eq!(removed_and_added(&diff), (vec![], vec![]));
        assert!(render(&diff, 2, "").is_empty());
    }

    #[test]
    fn a_changed_line_is_removed_and_added() {
        let diff = lines("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(
            removed_and_added(&diff),
            (vec!["b".to_owned()], vec!["B".to_owned()])
        );
    }

    #[test]
    fn an_inserted_line_leaves_the_rest_unchanged() {
        let diff = lines("a\nb\nc\nd\n", "a\nb\nx\nc\nd\n");
        assert_eq!(removed_and_added(&diff), (vec![], vec!["x".to_owned()]));
        assert_eq!(diff.len(), 5);
    }

    #[test]
    fn reordered_lines_keep_the_longest_common_run() {
        let diff = lines("1\n2\n3\n4\n", "2\n3\n4\n1\n");
        assert_eq!(
            removed_and_added(&diff),
            (vec!["1".to_owned()], vec!["1".to_owned()])
        );
    }

    #[test]
    fn rendering_leaves_out_lines_far_from_a_change() {
        let old = (0..20).map(|n| format!("{n}\n")).collect::<String>();
        let new = old.replace("10\n", "ten\n");
        let text = render(&lines(&old, &new), 1, "");
        let shown: Vec<&str> = text.lines().map(|line| line[2..].trim()).collect();
        assert!(shown.contains(&"10") && shown.contains(&"ten"));
        assert!(shown.contains(&"9") && shown.contains(&"11"));
        assert!(!shown.contains(&"0") && !shown.contains(&"19"));
    }
}
