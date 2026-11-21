//! Link recognition for browser/mail URLs and plain `path:line` file
//! references, never shell commands.
use crate::{Grid, ViewportPoint};

/// Recognize a URL or plain `path:line` reference in visible cells, joining
/// soft-wrapped rows. Cell boundaries are retained so Unicode and wide
/// characters don't confuse hit testing.
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
    if is_url_link(token) || is_path_line_link(token) {
        Some(token.to_string())
    } else {
        None
    }
}

fn is_url_link(token: &str) -> bool {
    let Some(colon) = token.find(':') else {
        return false;
    };
    let scheme = &token[..colon];
    let allowed = scheme.eq_ignore_ascii_case("https")
        || scheme.eq_ignore_ascii_case("http")
        || scheme.eq_ignore_ascii_case("mailto");
    allowed && token.len() > colon + 1
}

/// Plain `path:line`, `path:line:col`, or `path:start-end` reference.
fn is_path_line_link(token: &str) -> bool {
    if token.contains("://") || token.chars().any(char::is_control) {
        return false;
    }
    let Some((head, tail)) = token.rsplit_once(':') else {
        return false;
    };
    if head.is_empty() || tail.is_empty() {
        return false;
    }
    let path = if is_digits(tail) {
        if let Some((head2, line)) = head.rsplit_once(':')
            && !head2.is_empty()
            && is_digits(line)
        {
            head2
        } else {
            head
        }
    } else if let Some((start, end)) = tail.split_once('-')
        && is_digits(start)
        && is_digits(end)
    {
        head
    } else {
        return false;
    };
    if path.is_empty() || path.contains(':') {
        return false;
    }
    if path.starts_with('/') {
        return path.split('/').any(|seg| !seg.is_empty());
    }
    if path.contains('/') {
        return path
            .split('/')
            .any(|seg| !seg.is_empty() && seg != "." && seg != "..");
    }
    let Some((stem, ext)) = path.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && stem.bytes().any(|b| b.is_ascii_alphanumeric())
        && (1..=10).contains(&ext.len())
        && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        && ext.bytes().any(|b| b.is_ascii_alphabetic())
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cell, Colors, Cursor, CursorStyle, Dirty, Row, Size, Span};

    struct TestGrid {
        rows: Vec<Row>,
        colors: Colors,
    }

    impl TestGrid {
        fn from_lines(lines: &[(&str, bool)]) -> Self {
            let cols = lines.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(1);
            let rows = lines
                .iter()
                .map(|(line, wrapped)| {
                    let mut cells: Vec<Cell> = line
                        .chars()
                        .map(|c| Cell {
                            text: c.to_string(),
                            ..Cell::default()
                        })
                        .collect();
                    cells.resize_with(cols, Cell::default);
                    Row {
                        cells,
                        wrapped: *wrapped,
                    }
                })
                .collect();
            TestGrid {
                rows,
                colors: Colors::default(),
            }
        }
    }

    impl Grid for TestGrid {
        fn sync(&mut self) {}
        fn size(&self) -> Size {
            Size {
                cols: self.rows.first().map_or(0, |r| r.cells.len() as u16),
                rows: self.rows.len() as u16,
            }
        }
        fn row(&self, y: u16) -> Option<&Row> {
            self.rows.get(usize::from(y))
        }
        fn cursor(&self) -> Cursor {
            Cursor::default()
        }
        fn cursor_style(&self) -> CursorStyle {
            CursorStyle::default()
        }
        fn colors(&self) -> &Colors {
            &self.colors
        }
        fn selection(&self, _: u16) -> Option<Span> {
            None
        }
        fn dirty(&self) -> Dirty {
            Dirty::Clean
        }
        fn is_row_dirty(&self, _: u16) -> bool {
            false
        }
        fn clear_dirty(&mut self) {}
    }

    #[test]
    fn detects_urls_and_plain_path_line_links() {
        let grid = TestGrid::from_lines(&[
            ("see https://example.com/a and src/main.rs:42:7", false),
            ("at /tmp/foo.rs:10 or Cargo.toml:12-18 not 12:34", false),
        ]);
        assert_eq!(
            plain_link(&grid, ViewportPoint { x: 8, y: 0 }).as_deref(),
            Some("https://example.com/a")
        );
        assert_eq!(
            plain_link(&grid, ViewportPoint { x: 35, y: 0 }).as_deref(),
            Some("src/main.rs:42:7")
        );
        assert_eq!(
            plain_link(&grid, ViewportPoint { x: 6, y: 1 }).as_deref(),
            Some("/tmp/foo.rs:10")
        );
        assert_eq!(
            plain_link(&grid, ViewportPoint { x: 25, y: 1 }).as_deref(),
            Some("Cargo.toml:12-18")
        );
        // Timestamps without path or extension are not links.
        assert_eq!(plain_link(&grid, ViewportPoint { x: 43, y: 1 }), None);
    }
}
