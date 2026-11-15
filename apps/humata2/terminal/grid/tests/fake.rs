use terminal_grid::{
    Cell, CellWidth, Color, Colors, Dirty, FakeGrid, Grid, Rgb, Row, Scrollbar, Span,
    ViewportPoint,
};

fn text(grid: &FakeGrid, y: u16) -> String {
    grid.row(y)
        .map(|row| {
            row.cells
                .iter()
                .map(|cell| if cell.text.is_empty() { " " } else { cell.text.as_str() })
                .collect::<String>()
        })
        .unwrap_or_default()
}

#[test]
fn a_new_grid_is_blank_and_wholly_dirty() {
    let grid = FakeGrid::new(4, 2);
    assert_eq!(grid.dirty(), Dirty::Full);
    assert!(grid.is_row_dirty(0) && grid.is_row_dirty(1));
    assert_eq!(text(&grid, 1), "    ");
    assert!(grid.row(2).is_none());
}

#[test]
fn writing_dirties_only_its_row() {
    let mut grid = FakeGrid::new(6, 3);
    grid.clear_dirty();
    assert_eq!(grid.dirty(), Dirty::Clean);

    grid.write(1, 1, "hi", &Cell::default());

    assert_eq!(grid.dirty(), Dirty::Partial);
    assert!(!grid.is_row_dirty(0));
    assert!(grid.is_row_dirty(1));
    assert!(!grid.is_row_dirty(2));
    assert_eq!(text(&grid, 1), " hi   ");
}

#[test]
fn a_wide_character_takes_a_cell_and_a_spacer() {
    let mut grid = FakeGrid::new(5, 1);
    let next = grid.write(0, 0, "日a", &Cell::default());
    assert_eq!(next, 3);
    let row = grid.row(0).cloned().unwrap_or_default();
    assert_eq!(row.cells[0].width, CellWidth::Wide);
    assert_eq!(row.cells[0].text, "日");
    assert_eq!(row.cells[1].width, CellWidth::SpacerTail);
    assert_eq!(row.cells[2].text, "a");
}

#[test]
fn a_wide_character_at_the_edge_leaves_a_spacer_head() {
    let mut grid = FakeGrid::new(3, 1);
    grid.write(0, 0, "ab日", &Cell::default());
    let row = grid.row(0).cloned().unwrap_or_default();
    assert_eq!(row.cells[2].width, CellWidth::SpacerHead);
    assert!(row.cells[2].text.is_empty());
}

#[test]
fn an_emoji_sequence_stays_in_one_cell() {
    let mut grid = FakeGrid::new(6, 1);
    grid.write(0, 0, "👩\u{200d}💻e\u{301}", &Cell::default());
    let row = grid.row(0).cloned().unwrap_or_default();
    assert_eq!(row.cells[0].text, "👩\u{200d}💻");
    assert_eq!(row.cells[0].width, CellWidth::Wide);
    assert_eq!(row.cells[2].text, "e\u{301}");
}

#[test]
fn writes_keep_the_style_they_were_given() {
    let mut grid = FakeGrid::new(3, 1);
    let style = Cell {
        fg: Color::Palette(1),
        bold: true,
        ..Cell::default()
    };
    grid.write(0, 0, "x", &style);
    let cell = &grid.row(0).cloned().unwrap_or_default().cells[0];
    assert_eq!(cell.fg, Color::Palette(1));
    assert!(cell.bold);
}

#[test]
fn scrolling_moves_rows_up_and_dirties_everything() {
    let mut grid = FakeGrid::new(3, 2);
    grid.write(0, 1, "abc", &Cell::default());
    grid.clear_dirty();

    grid.scroll_up(Row::default());

    assert_eq!(text(&grid, 0), "abc");
    assert_eq!(text(&grid, 1), "   ");
    assert_eq!(grid.dirty(), Dirty::Full);
}

#[test]
fn a_selection_spans_rows_in_reading_order() {
    let mut grid = FakeGrid::new(5, 3);
    grid.clear_dirty();

    grid.select(ViewportPoint { x: 1, y: 2 }, ViewportPoint { x: 3, y: 0 });

    assert_eq!(grid.selection(0), Some(Span { start: 3, end: 4 }));
    assert_eq!(grid.selection(1), Some(Span { start: 0, end: 4 }));
    assert_eq!(grid.selection(2), Some(Span { start: 0, end: 1 }));
    assert!(grid.is_row_dirty(0) && grid.is_row_dirty(2));

    grid.clear_dirty();
    grid.select(ViewportPoint { x: 0, y: 2 }, ViewportPoint { x: 1, y: 2 });
    assert!(grid.is_row_dirty(0), "the row that lost its selection redraws");
    assert!(!grid.is_row_dirty(2), "the row whose span didn't change doesn't");
}

#[test]
fn changing_colors_dirties_everything() {
    let mut grid = FakeGrid::new(2, 2);
    grid.clear_dirty();
    grid.set_colors(Colors {
        background: Rgb::hex(0x000000),
        ..Colors::default()
    });
    assert_eq!(grid.dirty(), Dirty::Full);
}

#[test]
fn resizing_keeps_what_fits() {
    let mut grid = FakeGrid::new(4, 2);
    grid.write(0, 0, "abcd", &Cell::default());
    grid.resize(2, 3);
    assert_eq!(text(&grid, 0), "ab");
    assert_eq!(text(&grid, 2), "  ");
    assert_eq!(grid.dirty(), Dirty::Full);
}

#[test]
fn palette_colors_resolve_through_the_palette() {
    let colors = Colors::default();
    assert_eq!(colors.resolve(Color::Default), None);
    assert_eq!(colors.resolve(Color::Palette(16)), Some(Rgb::hex(0x000000)));
    assert_eq!(colors.resolve(Color::Palette(231)), Some(Rgb::hex(0xffffff)));
    assert_eq!(colors.resolve(Color::Palette(232)), Some(Rgb::hex(0x080808)));
    assert_eq!(
        colors.resolve(Color::Rgb { r: 1, g: 2, b: 3 }),
        Some(Rgb { r: 1, g: 2, b: 3 })
    );
}

#[test]
fn a_rectangular_selection_takes_the_same_columns_of_each_row() {
    let mut grid = FakeGrid::new(6, 4);
    grid.clear_dirty();

    grid.select_rectangle(ViewportPoint { x: 4, y: 2 }, ViewportPoint { x: 1, y: 1 });

    assert_eq!(grid.selection(0), None);
    assert_eq!(grid.selection(1), Some(Span { start: 1, end: 4 }));
    assert_eq!(grid.selection(2), Some(Span { start: 1, end: 4 }));
    assert_eq!(grid.selection(3), None);
}

#[test]
fn a_scrollbar_change_redraws_everything() {
    let mut grid = FakeGrid::new(4, 2);
    assert_eq!(grid.scrollbar(), None, "a fake has no history until given one");
    grid.clear_dirty();

    let scrolled = Scrollbar { total: 10, offset: 3, len: 2 };
    grid.set_scrollbar(Some(scrolled));

    assert_eq!(grid.scrollbar(), Some(scrolled));
    assert_eq!(grid.dirty(), Dirty::Full);
    assert!(!scrolled.at_bottom());
    assert!(Scrollbar { total: 10, offset: 8, len: 2 }.at_bottom());
    assert!(scrolled.scrollable());
    assert!(!Scrollbar { total: 2, offset: 0, len: 2 }.scrollable());
}
