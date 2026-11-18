//! The one comment stack keeps on each pull request: the stack it is in and
//! a row for every revision uploaded. All its state is read back from the
//! comment itself.

use crate::gh::Comment;

/// The first line of the comment; it finds the comment again.
pub(crate) const MARKER: &str = "<!-- stack:history v1 -->";

/// GitHub's limit for a comment is 65,536 characters; oldest rows go first
/// to stay under this.
const MAX_LEN: usize = 60_000;

/// What a revision's Diff cell links to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Diff {
    /// The first revision: the whole change against its base. Shown `full`.
    Full,
    /// Same base as revision N: the change since N. Shown `since N`.
    Since(u32),
    /// A new base and the same patch. Shown `rebase`.
    Rebase,
    /// A new base and a different patch. Shown `rebase + changes`, the link
    /// on `changes` going to the whole change against the new base.
    RebaseAndChanges,
}

/// One table row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) rev: u32,
    /// Full shas.
    pub(crate) head: String,
    pub(crate) base: String,
    pub(crate) diff: Diff,
    /// `YYYY-MM-DD HH:MM`, UTC.
    pub(crate) pushed: String,
    pub(crate) summary: String,
}

/// One line of the stack tree: a pull request at a depth (0 for the root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StackLine {
    pub(crate) depth: usize,
    pub(crate) number: u64,
    pub(crate) title: String,
    /// The pull request this comment is on: shown bold with `← this PR`.
    pub(crate) current: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct History {
    pub(crate) rows: Vec<Row>,
}

/// The history comment among a pull request's comments: the first one
/// written by `login` that starts with `MARKER`.
pub(crate) fn find<'a>(comments: &'a [Comment], login: &str) -> Option<&'a Comment> {
    comments
        .iter()
        .find(|comment| comment.author == login && comment.body.trim_start().starts_with(MARKER))
}

/// Which Diff the next row gets, given the last row, the new base and
/// whether the new patch (base..head) equals the last row's patch.
pub(crate) fn classify(last: Option<&Row>, new_base: &str, same_patch: bool) -> Diff {
    match last {
        None => Diff::Full,
        Some(row) if row.base == new_base => Diff::Since(row.rev),
        Some(_) if same_patch => Diff::Rebase,
        Some(_) => Diff::RebaseAndChanges,
    }
}

impl History {
    /// Reads the rows back from a comment `render` wrote. Lines that aren't
    /// rows are ignored; a row is a table line whose first cell is a number,
    /// with the head and base shas taken from its `/commit/<40 hex>` links.
    pub(crate) fn parse(body: &str) -> History {
        History {
            rows: body.lines().filter_map(parse_row).collect(),
        }
    }

    /// The next revision number: 1 more than the last row's, or 1.
    pub(crate) fn next_rev(&self) -> u32 {
        self.rows.last().map_or(1, |row| row.rev + 1)
    }
}

/// The comment: MARKER, the stack (`**Stack** (base `<trunk>`)` then one
/// `- #N title` line per StackLine, indented two spaces per depth, the
/// current one as `- **#N title** ← this PR`; left out when the stack has
/// one pull request), a blank line, then the table
/// `| Rev | Head | Base | Diff | Pushed (UTC) | Summary |`. Head and base
/// cells are `` [`<8 hex>`](<repo_url>/commit/<sha>) ``; Diff links go to
/// `<repo_url>/compare/<from>...<to>`. `|` in a summary is escaped as `\|`.
/// Oldest rows are dropped until the comment fits MAX_LEN.
///
/// A `rebase` or `rebase + changes` cell also names the command that shows
/// the change between that revision and the one before it, base changes left
/// out: `` `stack diff <topic> <rev - 1> <rev>` ``.
pub(crate) fn render(
    repo_url: &str,
    trunk: &str,
    topic: &str,
    stack: &[StackLine],
    history: &History,
) -> String {
    let mut head_lines = vec![MARKER.to_string()];
    if stack.len() > 1 {
        head_lines.push(format!("**Stack** (base `{trunk}`)"));
        head_lines.extend(stack.iter().map(stack_line));
    }
    head_lines.push(String::new());
    head_lines.push("| Rev | Head | Base | Diff | Pushed (UTC) | Summary |".to_string());
    head_lines.push("| --- | --- | --- | --- | --- | --- |".to_string());

    // Every Since link is computed against the full history, before any drop.
    let row_lines: Vec<String> = history
        .rows
        .iter()
        .map(|row| render_row(repo_url, topic, &history.rows, row))
        .collect();

    // Each line is written with a newline, so it costs its length plus one.
    // Dropping the oldest rows one at a time gives the same text as rendering
    // again after each drop, without rendering the table again each time.
    let cost = |line: &String| line.len() + 1;
    let mut total: usize = head_lines.iter().map(cost).sum::<usize>()
        + row_lines.iter().map(cost).sum::<usize>();
    let mut dropped = 0;
    for line in &row_lines {
        if total <= MAX_LEN || row_lines.len().saturating_sub(dropped) <= 1 {
            break;
        }
        total = total.saturating_sub(cost(line));
        dropped += 1;
    }

    head_lines
        .iter()
        .map(String::as_str)
        .chain(row_lines.iter().skip(dropped).map(String::as_str))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// One tree line: two spaces per depth, the current pull request in bold.
pub(crate) fn stack_line(line: &StackLine) -> String {
    let indent = "  ".repeat(line.depth);
    if line.current {
        format!("{indent}- **#{} {}** ← this PR", line.number, line.title)
    } else {
        format!("{indent}- #{} {}", line.number, line.title)
    }
}

/// One table row. `all` is the full history, for the Since links.
fn render_row(repo_url: &str, topic: &str, all: &[Row], row: &Row) -> String {
    let compare = |from: &str, to: &str| format!("{repo_url}/compare/{from}...{to}");
    let previous = all
        .iter()
        .filter(|other| other.rev < row.rev)
        .map(|other| other.rev)
        .max();
    let range_diff = previous.map_or_else(String::new, |previous| {
        format!(" · `stack diff {topic} {previous} {}`", row.rev)
    });
    let diff = match row.diff {
        Diff::Full => format!("[full]({})", compare(&row.base, &row.head)),
        Diff::Since(rev) => {
            let from = all
                .iter()
                .find(|other| other.rev == rev)
                .map_or(row.base.as_str(), |other| other.head.as_str());
            format!("[since {rev}]({})", compare(from, &row.head))
        }
        Diff::Rebase => format!("[rebase]({}){range_diff}", compare(&row.base, &row.head)),
        Diff::RebaseAndChanges => {
            format!("rebase + [changes]({}){range_diff}", compare(&row.base, &row.head))
        }
    };
    let cells = [
        row.rev.to_string(),
        format!("[`{}`]({repo_url}/commit/{})", short_sha(&row.head), row.head),
        format!("[`{}`]({repo_url}/commit/{})", short_sha(&row.base), row.base),
        diff,
        row.pushed.clone(),
        escape_summary(&row.summary),
    ];
    let padded: Vec<String> = cells.iter().map(|cell| format!(" {cell} ")).collect();
    format!("|{}|", padded.join("|"))
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

/// Newlines become spaces and `|` becomes `\|`, so a summary stays in its cell.
fn escape_summary(summary: &str) -> String {
    summary
        .chars()
        .map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c })
        .collect::<String>()
        .replace('|', "\\|")
}

/// Reads one table row; `None` for any line that is not a complete row.
fn parse_row(line: &str) -> Option<Row> {
    let line = line.trim();
    if !line.starts_with('|') {
        return None;
    }
    // The line starts with `|`, so the first piece is empty and goes.
    let mut cells: Vec<String> = split_cells(line).into_iter().skip(1).collect();
    if cells.last().is_some_and(String::is_empty) {
        cells.pop();
    }
    let rev = cells.first()?.trim().parse::<u32>().ok()?;
    let head = commit_sha(cells.get(1)?)?;
    let base = commit_sha(cells.get(2)?)?;
    let diff = parse_diff(cells.get(3)?)?;
    let pushed = cells.get(4)?.trim().to_string();
    let summary = cells.get(5)?.trim().replace("\\|", "|");
    Some(Row {
        rev,
        head,
        base,
        diff,
        pushed,
        summary,
    })
}

/// Splits a table line on each `|` not preceded by a backslash. Escaped
/// pipes stay in their piece, as written.
fn split_cells(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut prev = None;
    for ch in line.chars() {
        if ch == '|' && prev != Some('\\') {
            cells.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
        prev = Some(ch);
    }
    cells.push(current);
    cells
}

/// The 40-hex sha in a `](…/commit/<sha>)` link.
fn commit_sha(cell: &str) -> Option<String> {
    let rest = cell.split_once("/commit/")?.1;
    let sha = rest.get(..40)?;
    let closes = rest.get(40..)?.starts_with(')');
    (closes && sha.bytes().all(|b| b.is_ascii_hexdigit())).then(|| sha.to_string())
}

fn parse_diff(cell: &str) -> Option<Diff> {
    let text = strip_links(cell);
    // A rebase cell ends with ` · `stack diff …``; the kind is before it.
    let kind = text.split(" · ").next().unwrap_or_default();
    match kind.trim() {
        "full" => Some(Diff::Full),
        "rebase" => Some(Diff::Rebase),
        "rebase + changes" => Some(Diff::RebaseAndChanges),
        other => other
            .strip_prefix("since ")?
            .trim()
            .parse::<u32>()
            .ok()
            .map(Diff::Since),
    }
}

/// Replaces each `[text](url)` with its text.
fn strip_links(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('[') {
        out.push_str(before);
        let link = after.split_once("](").and_then(|(label, tail)| {
            tail.split_once(')').map(|(_, remainder)| (label, remainder))
        });
        match link {
            Some((label, remainder)) => {
                out.push_str(label);
                rest = remainder;
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::Comment;

    const REPO_URL: &str = "https://github.com/example/repo";

    fn sha(c: char) -> String {
        c.to_string().repeat(40)
    }

    fn row(rev: u32, head: char, base: char, diff: Diff, summary: &str) -> Row {
        Row {
            rev,
            head: sha(head),
            base: sha(base),
            diff,
            pushed: "2026-11-16 13:00".to_string(),
            summary: summary.to_string(),
        }
    }

    fn stack_entry(depth: usize, number: u64, title: &str, current: bool) -> StackLine {
        StackLine {
            depth,
            number,
            title: title.to_string(),
            current,
        }
    }

    fn history_of(rows: Vec<Row>) -> History {
        History { rows }
    }

    #[test]
    fn first_revision_is_a_full_diff() {
        assert_eq!(classify(None, &sha('a'), true), Diff::Full);
    }

    #[test]
    fn same_base_diffs_since_the_last_revision() {
        let last = row(3, 'c', 'a', Diff::Full, "");
        assert_eq!(classify(Some(&last), &sha('a'), false), Diff::Since(3));
    }

    #[test]
    fn new_base_with_the_same_patch_is_a_rebase() {
        let last = row(1, 'c', 'a', Diff::Full, "");
        assert_eq!(classify(Some(&last), &sha('b'), true), Diff::Rebase);
    }

    #[test]
    fn new_base_with_a_new_patch_is_a_rebase_and_changes() {
        let last = row(1, 'c', 'a', Diff::Full, "");
        assert_eq!(
            classify(Some(&last), &sha('b'), false),
            Diff::RebaseAndChanges
        );
    }

    #[test]
    fn parse_reads_back_the_rows_render_wrote() {
        let history = history_of(vec![
            row(1, 'a', 'b', Diff::Full, "add the codec"),
            row(2, 'c', 'b', Diff::Since(1), "escape a | pipe"),
            row(3, 'd', 'e', Diff::RebaseAndChanges, "rebased"),
        ]);
        let text = render(REPO_URL, "main", "codec", &[], &history);
        assert!(text.contains("escape a \\| pipe"));
        assert_eq!(History::parse(&text).rows, history.rows);
    }

    #[test]
    fn a_rebase_row_names_its_range_diff_and_still_parses() {
        let history = history_of(vec![
            row(1, 'a', 'b', Diff::Full, ""),
            row(2, 'c', 'd', Diff::RebaseAndChanges, ""),
        ]);
        let text = render(REPO_URL, "main", "codec", &[], &history);
        assert!(text.contains("`stack diff codec 1 2`"), "{text}");
        assert_eq!(History::parse(&text).rows, history.rows);
    }

    #[test]
    fn single_pull_request_stack_has_no_stack_block() {
        let history = history_of(vec![row(1, 'a', 'b', Diff::Full, "")]);
        let stack = [stack_entry(0, 1, "add the codec", true)];
        let text = render(REPO_URL, "main", "codec", &stack, &history);
        assert!(!text.contains("**Stack**"));
        assert!(text.contains("| Rev | Head | Base | Diff | Pushed (UTC) | Summary |"));
    }

    #[test]
    fn two_pull_request_stack_marks_the_current_one() {
        let history = history_of(vec![row(1, 'a', 'b', Diff::Full, "")]);
        let stack = [
            stack_entry(0, 1, "add the codec", false),
            stack_entry(1, 2, "compress the journal", true),
        ];
        let text = render(REPO_URL, "main", "codec", &stack, &history);
        assert!(text.contains("**Stack** (base `main`)"));
        assert!(text.lines().any(|line| line == "- #1 add the codec"));
        let marked: Vec<&str> = text
            .lines()
            .filter(|line| line.contains("← this PR"))
            .collect();
        assert_eq!(marked.len(), 1);
        assert!(marked[0].contains("**#2 compress the journal**"));
    }

    #[test]
    fn next_revision_follows_the_last_parsed_row() {
        let history = history_of(vec![
            row(1, 'a', 'b', Diff::Full, ""),
            row(2, 'c', 'b', Diff::Since(1), ""),
        ]);
        let parsed = History::parse(&render(REPO_URL, "main", "codec", &[], &history));
        assert_eq!(parsed.next_rev(), 3);
        assert_eq!(History::default().next_rev(), 1);
    }

    #[test]
    fn find_returns_the_marked_comment_written_by_the_login() {
        let body = render(
            REPO_URL,
            "main",
            "codec",
            &[],
            &history_of(vec![row(1, 'a', 'b', Diff::Full, "")]),
        );
        let comments = vec![
            Comment {
                id: 1,
                body: body.clone(),
                author: "someone-else".to_string(),
            },
            Comment {
                id: 2,
                body: "a comment without the marker".to_string(),
                author: "me".to_string(),
            },
            Comment {
                id: 3,
                body,
                author: "me".to_string(),
            },
        ];
        assert_eq!(find(&comments, "me").map(|found| found.id), Some(3));
        assert!(find(&comments, "nobody").is_none());
    }

    #[test]
    fn oversized_history_drops_the_oldest_rows_to_fit() {
        let rows: Vec<Row> = (1..=2000u32)
            .map(|rev| Row {
                rev,
                head: format!("{rev:040x}"),
                base: sha('b'),
                diff: if rev == 1 {
                    Diff::Full
                } else {
                    Diff::Since(rev - 1)
                },
                pushed: "2026-11-16 13:00".to_string(),
                summary: format!("revision {rev} {}", "a long summary ".repeat(8).trim_end()),
            })
            .collect();
        let history = history_of(rows);

        let text = render(REPO_URL, "main", "codec", &[], &history);
        assert!(text.len() <= MAX_LEN);

        let parsed = History::parse(&text);
        assert!(!parsed.rows.is_empty());
        assert_eq!(parsed.rows.last(), history.rows.last());
        assert!(history.rows.ends_with(&parsed.rows));
    }

    #[test]
    fn parse_skips_lines_that_are_not_complete_rows() {
        let kept = row(1, 'a', 'b', Diff::Full, "first");
        let a = sha('a');
        let body = format!(
            "Some prose.\n\
             | Rev | Head | Base | Diff | Pushed (UTC) | Summary |\n\
             | --- | --- | --- | --- | --- | --- |\n\
             | 2 | no commit link | x | full | now | y |\n\
             | 3 | [`aaaaaaaa`]({REPO_URL}/commit/{a}) | [`aaaaaaaa`]({REPO_URL}/commit/{a}) | sideways | now | y |\n\
             {}\n\
             - a list item\n",
            render(REPO_URL, "main", "codec", &[], &history_of(vec![kept.clone()])),
        );
        assert_eq!(History::parse(&body).rows, vec![kept]);
    }
}
