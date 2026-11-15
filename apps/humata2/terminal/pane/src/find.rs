//! Find in a terminal's history: plain text or a regular expression,
//! either case or exact case, over every row of the scrollback and the
//! screen. A line the terminal wrapped is searched as one line, so a match
//! can run from the end of one row into the next.

use regex::{Regex, RegexBuilder};
use terminal_grid::Row;
use terminal_view::{HistoryPoint, Mark};

use crate::error::FindError;

/// What the find bar looks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    /// Off: letters match in either case.
    pub case_sensitive: bool,
    /// On: `text` is a regular expression (Rust `regex` syntax).
    pub regex: bool,
}

/// A query ready to run.
#[derive(Debug, Clone)]
pub(crate) struct Finder {
    regex: Regex,
}

/// The most matches kept for one query. More than this are not shown,
/// and the find bar says the count is a lower bound.
pub(crate) const MATCH_LIMIT: usize = 10_000;

impl Finder {
    /// `None` when there is nothing to look for.
    pub(crate) fn new(query: &Query) -> Result<Option<Finder>, FindError> {
        if query.text.is_empty() {
            return Ok(None);
        }
        let pattern = if query.regex {
            query.text.clone()
        } else {
            regex::escape(&query.text)
        };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!query.case_sensitive)
            .build()
            .map_err(|source| FindError::Pattern {
                pattern: query.text.clone(),
                source,
            })?;
        Ok(Some(Finder { regex }))
    }

    /// Every match in rows `0..total`, oldest first, read with `row`. A row
    /// `row` can't give is taken as blank. Matches that are empty (a
    /// pattern such as `x*`) are skipped; at most [`MATCH_LIMIT`] are kept.
    pub(crate) fn search(&self, total: u64, mut row: impl FnMut(u64) -> Option<Row>) -> Vec<Mark> {
        let mut found = Vec::new();
        let mut line = Line::default();
        for y in 0..total {
            let read = row(y);
            let wrapped = read.as_ref().is_some_and(|row| row.wrapped);
            if let Some(read) = &read {
                line.push_row(y, read);
            }
            if !wrapped || y + 1 == total {
                self.search_line(&line, &mut found);
                if found.len() >= MATCH_LIMIT {
                    found.truncate(MATCH_LIMIT);
                    break;
                }
                line.clear();
            }
        }
        found
    }

    fn search_line(&self, line: &Line, found: &mut Vec<Mark>) {
        for m in self.regex.find_iter(line.text.trim_end_matches(' ')) {
            if m.is_empty() {
                continue;
            }
            let (Some(start), Some(last)) = (line.cell_at(m.start()), line.cell_at(m.end() - 1)) else {
                continue;
            };
            found.push(Mark {
                start: start.point,
                end: HistoryPoint {
                    row: last.point.row,
                    col: last.point.col + u16::from(last.columns.max(1)) - 1,
                },
                current: false,
            });
        }
    }
}

/// Where a piece of a line's text came from.
#[derive(Debug, Clone, Copy)]
struct Piece {
    /// The byte offset of the cell's text in the line.
    at: usize,
    point: HistoryPoint,
    columns: u8,
}

/// The text of rows joined across wraps, and the cell each byte came from.
#[derive(Debug, Default)]
struct Line {
    text: String,
    pieces: Vec<Piece>,
}

impl Line {
    fn push_row(&mut self, y: u64, row: &Row) {
        for (x, cell) in row.cells.iter().enumerate() {
            let columns = cell.columns();
            if columns == 0 {
                // The second half of a wide character, or the gap a wide
                // character left at the end of a row: no text of its own.
                continue;
            }
            let col = u16::try_from(x).unwrap_or(u16::MAX);
            self.pieces.push(Piece {
                at: self.text.len(),
                point: HistoryPoint { row: y, col },
                columns,
            });
            // A blank cell reads as a space, so "a b" finds "a" and "b" a
            // cell apart.
            self.text.push_str(if cell.text.is_empty() { " " } else { &cell.text });
        }
    }

    /// The cell holding byte `at` of the text.
    fn cell_at(&self, at: usize) -> Option<Piece> {
        let index = self.pieces.partition_point(|piece| piece.at <= at);
        index.checked_sub(1).map(|index| self.pieces[index])
    }

    fn clear(&mut self) {
        self.text.clear();
        self.pieces.clear();
    }
}

#[cfg(test)]
mod tests {
    use terminal_grid::{Cell, CellWidth, Row};
    use terminal_view::HistoryPoint;

    use super::{Finder, Query};

    fn row(text: &str, cols: usize, wrapped: bool) -> Row {
        let mut cells: Vec<Cell> = Vec::new();
        for c in text.chars() {
            let wide = matches!(c, '日' | '本');
            cells.push(Cell {
                text: c.to_string(),
                width: if wide { CellWidth::Wide } else { CellWidth::Narrow },
                ..Cell::default()
            });
            if wide {
                cells.push(Cell {
                    width: CellWidth::SpacerTail,
                    ..Cell::default()
                });
            }
        }
        cells.resize_with(cols, Cell::default);
        Row { cells, wrapped }
    }

    fn find(query: Query, rows: &[Row]) -> Vec<((u64, u16), (u64, u16))> {
        let finder = Finder::new(&query).expect("valid").expect("not empty");
        finder
            .search(rows.len() as u64, |y| rows.get(y as usize).cloned())
            .into_iter()
            .map(|mark| ((mark.start.row, mark.start.col), (mark.end.row, mark.end.col)))
            .collect()
    }

    fn plain(text: &str) -> Query {
        Query {
            text: text.to_owned(),
            ..Query::default()
        }
    }

    #[test]
    fn plain_text_matches_either_case_unless_asked() {
        let rows = [row("Error error ERROR", 20, false)];
        assert_eq!(find(plain("error"), &rows).len(), 3);
        let exact = Query {
            case_sensitive: true,
            ..plain("error")
        };
        assert_eq!(find(exact, &rows), vec![((0, 6), (0, 10))]);
    }

    #[test]
    fn regex_characters_are_literal_in_plain_text() {
        let rows = [row("a.b axb", 10, false)];
        assert_eq!(find(plain("a.b"), &rows), vec![((0, 0), (0, 2))]);
        let pattern = Query {
            regex: true,
            ..plain("a.b")
        };
        assert_eq!(find(pattern, &rows).len(), 2);
    }

    #[test]
    fn a_match_runs_across_a_wrapped_row_but_not_a_new_line() {
        let wrapped = [row("hello wor", 9, true), row("ld again", 9, false)];
        assert_eq!(find(plain("world"), &wrapped), vec![((0, 6), (1, 1))]);
        let separate = [row("hello wor", 9, false), row("ld again", 9, false)];
        assert!(find(plain("world"), &separate).is_empty());
    }

    #[test]
    fn a_wide_character_match_covers_both_its_columns() {
        let rows = [row("x日本y", 8, false)];
        assert_eq!(find(plain("日本"), &rows), vec![((0, 1), (0, 4))]);
        assert_eq!(find(plain("y"), &rows), vec![((0, 5), (0, 5))]);
    }

    #[test]
    fn trailing_blanks_and_empty_matches_find_nothing() {
        let rows = [row("ab", 10, false)];
        let blanks = Query {
            regex: true,
            ..plain(r"\s+$")
        };
        assert!(Finder::new(&blanks).expect("valid").is_some());
        assert!(find(blanks, &rows).is_empty());
        let empty = Query {
            regex: true,
            ..plain("z*")
        };
        assert!(find(empty, &rows).is_empty());
    }

    #[test]
    fn an_invalid_pattern_is_an_error_and_empty_text_finds_nothing() {
        let broken = Query {
            regex: true,
            ..plain("(unclosed")
        };
        assert!(Finder::new(&broken).is_err());
        assert!(Finder::new(&plain("")).expect("valid").is_none());
    }

    #[test]
    fn rows_that_cannot_be_read_end_the_line_they_are_in() {
        let finder = Finder::new(&plain("ok")).expect("valid").expect("not empty");
        let rows = [row("ok", 4, false), row("ok", 4, false)];
        let found = finder.search(3, |y| (y != 1).then(|| rows[(y / 2) as usize].clone()));
        assert_eq!(
            found.iter().map(|mark| mark.start).collect::<Vec<_>>(),
            vec![HistoryPoint { row: 0, col: 0 }, HistoryPoint { row: 2, col: 0 }]
        );
    }
}
