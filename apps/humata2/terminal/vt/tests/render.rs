//! The snapshot a renderer draws from: what changed since the last frame,
//! the cursor's shape, the colors and the selection.

use ghostty_vt::{
    CellWidth, Color, Colors, CursorShape, CursorStyle, Dirty, Grid, Rgb, Scroll, Span,
    Terminal, ViewportPoint,
};

fn terminal(cols: u16, rows: u16) -> Terminal {
    let mut terminal = Terminal::new(cols, rows, 1000).expect("a terminal");
    terminal.sync();
    terminal
}

/// Syncs, as a frame starts, and returns which rows are dirty.
fn dirty_rows(terminal: &mut Terminal) -> Vec<u16> {
    terminal.sync();
    (0..Grid::size(terminal).rows)
        .filter(|&y| terminal.is_row_dirty(y))
        .collect()
}

#[test]
fn the_first_frame_draws_everything() {
    let terminal = terminal(4, 3);
    assert_eq!(terminal.dirty(), Dirty::Full);
    assert!((0..3).all(|y| terminal.is_row_dirty(y)));
}

#[test]
fn a_frame_with_no_output_draws_nothing() {
    let mut terminal = terminal(4, 3);
    terminal.clear_dirty();
    terminal.sync();
    assert_eq!(terminal.dirty(), Dirty::Clean);
}

#[test]
fn writing_one_line_dirties_only_that_row() {
    let mut terminal = terminal(10, 5);
    terminal.feed(b"\x1b[3;1H");
    terminal.sync();
    terminal.clear_dirty();

    terminal.feed(b"hello");

    assert_eq!(dirty_rows(&mut terminal), vec![2]);
    assert_eq!(terminal.dirty(), Dirty::Partial);
    let row = terminal.row(2).expect("row 2");
    assert_eq!(row.cells[0].text, "h");
    assert_eq!(row.cells[4].text, "o");
}

#[test]
fn dirty_rows_add_up_until_cleared() {
    let mut terminal = terminal(10, 5);
    terminal.feed(b"\x1b[2;1H");
    terminal.sync();
    terminal.clear_dirty();
    terminal.feed(b"a");
    terminal.sync();
    terminal.feed(b"\x1b[4;1Hb");

    let dirty = dirty_rows(&mut terminal);
    assert!(dirty.contains(&1) && dirty.contains(&3));
    assert!(!dirty.contains(&0) && !dirty.contains(&4));
}

#[test]
fn resizing_redraws_everything() {
    let mut terminal = terminal(10, 3);
    terminal.clear_dirty();
    terminal.resize(12, 4).expect("a bigger terminal");
    terminal.sync();
    assert_eq!(terminal.dirty(), Dirty::Full);
    assert_eq!(Grid::size(&terminal).cols, 12);
    assert_eq!(terminal.row(3).expect("row 3").cells.len(), 12);
}

#[test]
fn scrolling_the_viewport_redraws_every_row_and_hides_the_cursor() {
    let mut terminal = terminal(10, 4);
    let lines: Vec<String> = (0..50).map(|n| format!("line {n}")).collect();
    terminal.feed(lines.join("\r\n").as_bytes());
    terminal.sync();
    terminal.clear_dirty();

    terminal.scroll_viewport(Scroll::Top).expect("scroll to the top");

    assert_eq!(dirty_rows(&mut terminal), vec![0, 1, 2, 3]);
    assert_eq!(terminal.row(0).expect("row 0").cells[5].text, "0");
    assert!(!Grid::cursor(&terminal).visible);
}

#[test]
fn styles_and_wide_characters_come_through() {
    let mut terminal = terminal(10, 2);
    terminal.feed("\x1b[1;31mX\x1b[0m日".as_bytes());
    terminal.sync();

    let row = terminal.row(0).expect("row 0");
    assert!(row.cells[0].bold);
    assert_eq!(row.cells[0].fg, Color::Palette(1));
    assert_eq!(row.cells[1].text, "日");
    assert_eq!(row.cells[1].width, CellWidth::Wide);
    assert_eq!(row.cells[2].width, CellWidth::SpacerTail);
}

#[test]
fn a_wrapped_row_says_so() {
    let mut terminal = terminal(4, 3);
    terminal.feed(b"abcdef");
    terminal.sync();
    assert!(terminal.row(0).expect("row 0").wrapped);
    assert!(!terminal.row(1).expect("row 1").wrapped);
}

#[test]
fn the_program_picks_the_cursor_shape() {
    let mut terminal = terminal(10, 2);
    terminal.feed(b"\x1b[5 q");
    terminal.sync();
    assert_eq!(
        terminal.cursor_style(),
        CursorStyle {
            shape: CursorShape::Bar,
            blinking: true
        }
    );

    terminal.feed(b"\x1b[4 q");
    terminal.sync();
    assert_eq!(
        terminal.cursor_style(),
        CursorStyle {
            shape: CursorShape::Underline,
            blinking: false
        }
    );
}

#[test]
fn default_colors_set_by_the_host_are_reported_and_redraw_everything() {
    let mut terminal = terminal(10, 2);
    terminal.clear_dirty();
    let colors = Colors {
        foreground: Rgb::hex(0x112233),
        background: Rgb::hex(0x445566),
        ..Colors::default()
    };

    terminal.set_default_colors(&colors).expect("colors set");
    terminal.sync();

    assert_eq!(terminal.colors().foreground, Rgb::hex(0x112233));
    assert_eq!(terminal.colors().background, Rgb::hex(0x445566));
    assert_eq!(terminal.dirty(), Dirty::Full);
}

#[test]
fn a_program_can_change_the_background() {
    let mut terminal = terminal(10, 2);
    // A host always sets its theme's colors first.
    terminal
        .set_default_colors(&Colors {
            background: Rgb::hex(0x101010),
            ..Colors::default()
        })
        .expect("colors set");
    terminal.feed(b"\x1b]11;rgb:00/80/ff\x07");
    terminal.sync();
    assert_eq!(terminal.colors().background, Rgb::hex(0x0080ff));
}

#[test]
fn a_selection_is_reported_per_row_and_redraws_its_rows() {
    let mut terminal = terminal(5, 4);
    terminal.feed(b"aaaaa\r\nbbbbb\r\nccccc");
    terminal.sync();
    terminal.clear_dirty();

    terminal
        .select(ViewportPoint { x: 1, y: 0 }, ViewportPoint { x: 2, y: 1 })
        .expect("a selection");

    let dirty = dirty_rows(&mut terminal);
    assert!(dirty.contains(&0) && dirty.contains(&1));
    assert_eq!(terminal.selection(0), Some(Span { start: 1, end: 4 }));
    assert_eq!(terminal.selection(1), Some(Span { start: 0, end: 2 }));
    assert_eq!(terminal.selection(2), None);

    terminal.clear_dirty();
    terminal.clear_selection().expect("no selection");
    let dirty = dirty_rows(&mut terminal);
    assert!(dirty.contains(&0) && dirty.contains(&1));
    assert_eq!(terminal.selection(0), None);
}
