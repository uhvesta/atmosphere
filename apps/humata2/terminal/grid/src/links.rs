//! Link recognition is deliberately limited to browser/mail URLs, never shell commands.
use crate::{Grid, ViewportPoint};

/// Recognize a URL in visible cells, joining soft-wrapped rows. Cell boundaries
/// are retained so Unicode and wide characters don't confuse hit testing.
pub fn plain_link(grid: &(impl Grid + ?Sized), at: ViewportPoint) -> Option<String> {
    let row = grid.row(at.y)?;
    if usize::from(at.x) >= row.cells.len() {
        return None;
    }
    let mut first = at.y;
    while first > 0 && grid.row(first - 1).is_some_and(|r| r.wrapped) {
        first -= 1;
    }
    let mut text = String::new();
    let mut hit = 0;
    for y in first..grid.size().rows {
        let row = grid.row(y)?;
        for (x, cell) in row.cells.iter().enumerate() {
            if y == at.y && x == usize::from(at.x) {
                hit = text.len();
            }
            if cell.columns() > 0 {
                text.push_str(if cell.text.is_empty() {
                    " "
                } else {
                    &cell.text
                });
            }
            if text.len() > 16 * 1024 {
                return None;
            }
        }
        if !row.wrapped {
            break;
        }
    }
    let delimit = |c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`');
    let begin = text[..hit].rfind(delimit).map_or(0, |i| {
        i + text[i..].chars().next().map_or(0, char::len_utf8)
    });
    let end = text[hit..].find(delimit).map_or(text.len(), |i| hit + i);
    let mut token = &text[begin..end];
    token = token.trim_start_matches(['(', '[', '{']);
    token = token.trim_end_matches(['.', ',', ';', ':', '!']);
    // Keep balanced URL parentheses (e.g. encyclopedia URLs), trim prose closers.
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
        while token.ends_with(close)
            && token.chars().filter(|c| *c == close).count()
                > token.chars().filter(|c| *c == open).count()
        {
            token = &token[..token.len() - 1];
        }
    }
    let start = token.as_ptr() as usize - text.as_ptr() as usize;
    if hit < start || hit >= start + token.len() {
        return None;
    }
    let colon = token.find(':')?;
    let scheme = &token[..colon];
    let allowed = scheme.eq_ignore_ascii_case("https")
        || scheme.eq_ignore_ascii_case("http")
        || scheme.eq_ignore_ascii_case("mailto");
    (allowed && token.len() > colon + 1).then(|| token.to_string())
}
