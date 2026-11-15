//! Where and how the cursor is drawn, after `Ghostty`'s
//! `renderer/cursor.zig` (`style`) and `renderer/generic.zig`
//! (`addCursor`): hidden while the program hides it or a blink has it
//! off, an outlined block while the view isn't focused, two cells wide
//! over a wide character, and a block shows the cell's text in the
//! background color so it stays readable.

use terminal_grid::{CellWidth, CursorShape, Grid, Rgb};

use crate::frame::{Style, cell_colors};

/// The cursor as drawn this frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CursorPlan {
    pub col: u16,
    pub row: u16,
    /// 2 over a wide character, else 1.
    pub cols: u16,
    pub shape: CursorShape,
    pub color: Rgb,
    /// The text under a filled block and the color to draw it in.
    pub text: Option<(String, Rgb)>,
}

/// Plans the cursor of `grid`, or `None` when nothing is drawn. `blink_on`
/// is whether a blinking cursor is in its visible half; `cursor_text`
/// overrides the color of the text under a block.
pub(crate) fn plan_cursor<G: Grid + ?Sized>(
    grid: &G,
    focused: bool,
    blink_on: bool,
    cursor_text: Option<Rgb>,
    style: &Style,
) -> Option<CursorPlan> {
    let cursor = grid.cursor();
    if !cursor.visible {
        return None;
    }
    let cursor_style = grid.cursor_style();
    // `Ghostty` keeps the cursor still and outlined while unfocused, so
    // the user sees where typing would go.
    let shape = if focused {
        cursor_style.shape
    } else {
        CursorShape::BlockHollow
    };
    if focused && cursor_style.blinking && !blink_on {
        return None;
    }

    let row = grid.row(cursor.y)?;
    let mut col = cursor.x.min(grid.size().cols.saturating_sub(1));
    let mut cell = row.cells.get(usize::from(col))?;
    // On the tail of a wide character, cover the character itself.
    if cell.width == CellWidth::SpacerTail && col > 0 {
        col -= 1;
        cell = row.cells.get(usize::from(col))?;
    }
    let cols = if cell.width == CellWidth::Wide { 2 } else { 1 };

    let selected = grid.selection(cursor.y).is_some_and(|span| span.contains(col));
    let colors = grid.colors();
    let drawn = cell_colors(cell, selected, colors, style);
    let color = colors.cursor.unwrap_or(drawn.fg);
    let text = (shape == CursorShape::Block && !cell.text.is_empty() && !cell.invisible)
        .then(|| (cell.text.clone(), cursor_text.unwrap_or(drawn.bg)));

    Some(CursorPlan {
        col,
        row: cursor.y,
        cols,
        shape,
        color,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminal_grid::{Cell, Colors, Cursor, CursorStyle, FakeGrid};

    const STYLE: Style = Style {
        selection_background: None,
        selection_foreground: None,
        bold_is_bright: false,
        faint_opacity: 0.5,
    };

    fn at(grid: &mut FakeGrid, x: u16, y: u16) {
        grid.set_cursor(Cursor {
            x,
            y,
            visible: true,
        });
    }

    #[::std::prelude::v1::test]
    fn a_block_shows_the_text_under_it_in_the_background_color() {
        let mut grid = FakeGrid::new(4, 2);
        grid.write(0, 1, "ab", &Cell::default());
        at(&mut grid, 1, 1);
        let colors = Colors::default();

        let plan = plan_cursor(&grid, true, true, None, &STYLE);

        assert_eq!(
            plan,
            Some(CursorPlan {
                col: 1,
                row: 1,
                cols: 1,
                shape: CursorShape::Block,
                color: colors.foreground,
                text: Some(("b".to_owned(), colors.background)),
            })
        );
    }

    #[::std::prelude::v1::test]
    fn the_cursor_covers_both_cells_of_a_wide_character() {
        let mut grid = FakeGrid::new(4, 1);
        grid.write(0, 0, "日", &Cell::default());
        for x in [0, 1] {
            at(&mut grid, x, 0);
            let plan = plan_cursor(&grid, true, true, None, &STYLE);
            assert_eq!(plan.as_ref().map(|plan| (plan.col, plan.cols)), Some((0, 2)));
        }
    }

    #[::std::prelude::v1::test]
    fn the_cursor_at_the_right_edge_stays_in_the_grid() {
        let mut grid = FakeGrid::new(3, 1);
        at(&mut grid, 3, 0);
        let plan = plan_cursor(&grid, true, true, None, &STYLE);
        assert_eq!(plan.map(|plan| plan.col), Some(2));
    }

    #[::std::prelude::v1::test]
    fn an_unfocused_cursor_is_an_outline_that_doesnt_blink() {
        let mut grid = FakeGrid::new(3, 1);
        grid.set_cursor_style(CursorStyle {
            shape: terminal_grid::CursorShape::Bar,
            blinking: true,
        });
        at(&mut grid, 0, 0);
        assert_eq!(plan_cursor(&grid, true, false, None, &STYLE), None);
        let plan = plan_cursor(&grid, false, false, None, &STYLE);
        assert_eq!(plan.map(|plan| plan.shape), Some(CursorShape::BlockHollow));
        let plan = plan_cursor(&grid, true, true, None, &STYLE);
        assert_eq!(plan.map(|plan| plan.shape), Some(CursorShape::Bar));
    }

    #[::std::prelude::v1::test]
    fn a_hidden_cursor_draws_nothing() {
        let mut grid = FakeGrid::new(3, 1);
        grid.set_cursor(Cursor {
            x: 0,
            y: 0,
            visible: false,
        });
        assert_eq!(plan_cursor(&grid, true, true, None, &STYLE), None);
    }

    #[::std::prelude::v1::test]
    fn the_program_can_set_the_cursor_color() {
        let mut grid = FakeGrid::new(3, 1);
        grid.set_colors(Colors {
            cursor: Some(Rgb::hex(0xff8800)),
            ..Colors::default()
        });
        at(&mut grid, 0, 0);
        let plan = plan_cursor(&grid, true, true, None, &STYLE);
        assert_eq!(plan.map(|plan| plan.color), Some(Rgb::hex(0xff8800)));
    }
}
