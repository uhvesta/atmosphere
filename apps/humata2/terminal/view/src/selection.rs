//! What a click or drag selects: cells, words or whole lines, chosen by
//! the click count as in `Ghostty` (`Surface.zig`, `mouseButtonCallback`),
//! with the same default word boundaries (`selection-word-chars`). A word
//! or line that wrapped at the right edge continues on the next row.

use terminal_grid::{CellWidth, Grid, ViewportPoint};

/// The characters that end a word, besides blanks: `Ghostty`'s default
/// `selection-word-chars`.
const BOUNDARIES: &str = "'\"│`|:;,()[]{}<>$";

/// How much one click selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Granularity {
    Cell,
    Word,
    Line,
}

impl Granularity {
    /// One click selects cells, two a word, three a line; a fourth starts
    /// over, as `Ghostty` and most terminals do.
    pub(crate) fn of_clicks(count: usize) -> Granularity {
        match count.saturating_sub(1) % 3 {
            1 => Granularity::Word,
            2 => Granularity::Line,
            _ => Granularity::Cell,
        }
    }
}

/// What kind of text a cell holds, for finding where a word ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Blank,
    /// A boundary character, which is a word on its own.
    Boundary,
    Word,
}

/// The selection a drag from `anchor` to `here` makes at `granularity`,
/// both ends included, in reading order.
pub(crate) fn extend(
    grid: &impl Grid,
    granularity: Granularity,
    anchor: ViewportPoint,
    here: ViewportPoint,
) -> (ViewportPoint, ViewportPoint) {
    let forward = (anchor.y, anchor.x) <= (here.y, here.x);
    let (first, last) = if forward { (anchor, here) } else { (here, anchor) };
    match granularity {
        Granularity::Cell => (first, last),
        Granularity::Word => (word(grid, first).0, word(grid, last).1),
        Granularity::Line => (line(grid, first.y).0, line(grid, last.y).1),
    }
}

/// The word under `at`: the run of cells of the same kind around it.
/// A boundary character is a word by itself.
pub(crate) fn word(grid: &impl Grid, at: ViewportPoint) -> (ViewportPoint, ViewportPoint) {
    let at = owner(grid, at);
    let class = class(grid, at);
    if class == Class::Boundary {
        return (at, at);
    }
    let mut start = at;
    while let Some(before) = step_back(grid, start) {
        if class_of(grid, before) != class {
            break;
        }
        start = before;
    }
    let mut end = at;
    while let Some(after) = step_forward(grid, end) {
        if class_of(grid, after) != class {
            break;
        }
        end = after;
    }
    (start, end)
}

/// The whole line through row `y`: up to the first row of a line that
/// wrapped onto it, and down to the last row it wrapped onto.
pub(crate) fn line(grid: &impl Grid, y: u16) -> (ViewportPoint, ViewportPoint) {
    let size = grid.size();
    let mut top = y;
    while top > 0 && wrapped(grid, top - 1) {
        top -= 1;
    }
    let mut bottom = y;
    while bottom + 1 < size.rows && wrapped(grid, bottom) {
        bottom += 1;
    }
    (
        ViewportPoint { x: 0, y: top },
        ViewportPoint {
            x: size.cols.saturating_sub(1),
            y: bottom,
        },
    )
}

fn wrapped(grid: &impl Grid, y: u16) -> bool {
    grid.row(y).is_some_and(|row| row.wrapped)
}

fn width(grid: &impl Grid, at: ViewportPoint) -> Option<CellWidth> {
    grid.row(at.y)?.cells.get(usize::from(at.x)).map(|cell| cell.width)
}

/// The cell holding the character at `at`: the wide cell for its spacer.
fn owner(grid: &impl Grid, at: ViewportPoint) -> ViewportPoint {
    if width(grid, at) == Some(CellWidth::SpacerTail) && at.x > 0 {
        ViewportPoint { x: at.x - 1, ..at }
    } else {
        at
    }
}

fn class(grid: &impl Grid, at: ViewportPoint) -> Class {
    let Some(cell) = grid.row(at.y).and_then(|row| row.cells.get(usize::from(at.x))) else {
        return Class::Blank;
    };
    let text = cell.text.as_str();
    if text.chars().all(char::is_whitespace) {
        Class::Blank
    } else if text.chars().count() == 1 && text.chars().all(|c| BOUNDARIES.contains(c)) {
        Class::Boundary
    } else {
        Class::Word
    }
}

/// The class of `at` as a word walk sees it: a spacer belongs with the
/// wide character before it, and the padding before a wide character that
/// wrapped belongs with the word it continues.
fn class_of(grid: &impl Grid, at: ViewportPoint) -> Class {
    match width(grid, at) {
        Some(CellWidth::SpacerTail) => class(grid, owner(grid, at)),
        Some(CellWidth::SpacerHead) if wrapped(grid, at.y) => {
            step_forward(grid, at).map_or(Class::Blank, |next| class(grid, next))
        }
        _ => class(grid, at),
    }
}

fn step_back(grid: &impl Grid, at: ViewportPoint) -> Option<ViewportPoint> {
    if at.x > 0 {
        Some(ViewportPoint { x: at.x - 1, ..at })
    } else if at.y > 0 && wrapped(grid, at.y - 1) {
        Some(ViewportPoint {
            x: grid.size().cols.saturating_sub(1),
            y: at.y - 1,
        })
    } else {
        None
    }
}

fn step_forward(grid: &impl Grid, at: ViewportPoint) -> Option<ViewportPoint> {
    let size = grid.size();
    if at.x + 1 < size.cols {
        Some(ViewportPoint { x: at.x + 1, ..at })
    } else if at.y + 1 < size.rows && wrapped(grid, at.y) {
        Some(ViewportPoint { x: 0, y: at.y + 1 })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use terminal_grid::{Cell, FakeGrid};

    use super::*;

    fn at(x: u16, y: u16) -> ViewportPoint {
        ViewportPoint { x, y }
    }

    fn grid(lines: &[&str]) -> FakeGrid {
        let cols = 12;
        let mut grid = FakeGrid::new(cols, u16::try_from(lines.len()).unwrap_or(u16::MAX));
        for (y, line) in lines.iter().enumerate() {
            grid.write(0, u16::try_from(y).unwrap_or(0), line, &Cell::default());
        }
        grid
    }

    #[::std::prelude::v1::test]
    fn clicks_cycle_through_cells_words_and_lines() {
        let counts: Vec<_> = (1..=4).map(Granularity::of_clicks).collect();
        assert_eq!(counts, [
            Granularity::Cell,
            Granularity::Word,
            Granularity::Line,
            Granularity::Cell,
        ]);
    }

    #[::std::prelude::v1::test]
    fn a_double_click_selects_the_word_up_to_blanks_and_boundaries() {
        let grid = grid(&["ls -la foo", "(src/main)"]);
        assert_eq!(word(&grid, at(4, 0)), (at(3, 0), at(5, 0)), "-la");
        assert_eq!(word(&grid, at(3, 1)), (at(1, 1), at(8, 1)), "src/main, parens excluded");
        assert_eq!(word(&grid, at(0, 1)), (at(0, 1), at(0, 1)), "a parenthesis alone");
    }

    #[::std::prelude::v1::test]
    fn a_word_that_wrapped_continues_on_the_next_row() {
        let mut grid = grid(&["echo abcdefg", "hij next"]);
        grid.set_wrapped(0, true);

        assert_eq!(word(&grid, at(1, 1)), (at(5, 0), at(2, 1)));
        assert_eq!(line(&grid, 1), (at(0, 0), at(11, 1)));
    }

    #[::std::prelude::v1::test]
    fn a_wide_character_is_selected_whole() {
        let grid = grid(&["日本語 text"]);
        // 日 takes columns 0-1, 本 2-3 and 語 4-5; clicking 本's spacer
        // selects all three.
        assert_eq!(word(&grid, at(3, 0)), (at(0, 0), at(5, 0)));
    }

    #[::std::prelude::v1::test]
    fn dragging_by_words_or_lines_extends_to_whole_units_either_way() {
        let grid = grid(&["one two six", "four five"]);
        assert_eq!(
            extend(&grid, Granularity::Word, at(5, 0), at(1, 1)),
            (at(4, 0), at(3, 1)),
            "two through four"
        );
        assert_eq!(
            extend(&grid, Granularity::Word, at(1, 1), at(5, 0)),
            (at(4, 0), at(3, 1)),
            "the same backwards"
        );
        assert_eq!(
            extend(&grid, Granularity::Line, at(3, 1), at(2, 0)),
            (at(0, 0), at(11, 1))
        );
        assert_eq!(extend(&grid, Granularity::Cell, at(3, 1), at(2, 0)), (at(2, 0), at(3, 1)));
    }
}
