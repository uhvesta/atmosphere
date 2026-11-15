//! What the screen holds after a program writes to it.

use ghostty_vt::{Cell, CellWidth, Color, Cursor, Grid, Size, Terminal, Underline};

fn terminal(cols: u16, rows: u16) -> Terminal {
    Terminal::new(cols, rows, 1000).expect("a terminal")
}

/// The row's text, a space for each empty cell, with trailing spaces cut.
fn text(cells: &[Cell]) -> String {
    let mut text = String::new();
    for cell in cells {
        match cell.width {
            CellWidth::SpacerTail | CellWidth::SpacerHead => {}
            _ if cell.text.is_empty() => text.push(' '),
            _ => text.push_str(&cell.text),
        }
    }
    text.trim_end().to_owned()
}

fn viewport_text(terminal: &Terminal, y: u16) -> String {
    text(&terminal.viewport_row(y).expect("a row").cells)
}

#[test]
fn plain_text_wraps_at_the_right_edge() {
    let mut terminal = terminal(5, 3);
    terminal.feed(b"abcdefg");

    let first = terminal.viewport_row(0).expect("row 0");
    assert_eq!(text(&first.cells), "abcde");
    assert!(first.wrapped);
    let second = terminal.viewport_row(1).expect("row 1");
    assert_eq!(text(&second.cells), "fg");
    assert!(!second.wrapped);
    assert_eq!(
        terminal.cursor(),
        Cursor {
            x: 2,
            y: 1,
            visible: true
        }
    );
}

#[test]
fn a_new_line_does_not_mark_the_row_wrapped() {
    let mut terminal = terminal(10, 3);
    terminal.feed(b"one\r\ntwo");

    assert!(!terminal.viewport_row(0).expect("row 0").wrapped);
    assert_eq!(viewport_text(&terminal, 0), "one");
    assert_eq!(viewport_text(&terminal, 1), "two");
}

#[test]
fn the_grid_trait_reads_the_viewport() {
    let mut terminal = terminal(4, 2);
    terminal.feed(b"hi");
    terminal.sync();

    assert_eq!(Grid::size(&terminal), Size { cols: 4, rows: 2 });
    assert_eq!(text(&terminal.row(0).expect("row 0").cells), "hi");
    assert!(terminal.row(5).is_none());
    assert_eq!(
        Grid::cursor(&terminal),
        Cursor {
            x: 2,
            y: 0,
            visible: true
        }
    );
}

#[test]
fn a_hidden_cursor_reports_hidden() {
    let mut terminal = terminal(10, 3);
    terminal.feed(b"\x1b[?25l");

    assert!(!terminal.cursor().visible);
}

#[test]
fn cjk_characters_take_two_cells() {
    let mut terminal = terminal(10, 2);
    terminal.feed("日本".as_bytes());

    let cells = terminal.viewport_row(0).expect("row 0").cells;
    assert_eq!(cells[0].text, "日");
    assert_eq!(cells[0].width, CellWidth::Wide);
    assert_eq!(cells[0].columns(), 2);
    assert_eq!(cells[1].width, CellWidth::SpacerTail);
    assert_eq!(cells[1].text, "");
    assert_eq!(cells[2].text, "本");
    assert_eq!(cells[2].width, CellWidth::Wide);
    assert_eq!(terminal.cursor().x, 4);
}

#[test]
fn a_wide_character_that_does_not_fit_moves_to_the_next_row() {
    let mut terminal = terminal(3, 2);
    terminal.feed("ab日".as_bytes());

    let first = terminal.viewport_row(0).expect("row 0").cells;
    assert_eq!(first[2].width, CellWidth::SpacerHead);
    let second = terminal.viewport_row(1).expect("row 1").cells;
    assert_eq!(second[0].text, "日");
    assert_eq!(second[0].width, CellWidth::Wide);
}

#[test]
fn an_emoji_zwj_sequence_is_one_grapheme() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let mut terminal = terminal(10, 2);
    terminal.feed(format!("{family}x").as_bytes());

    let cells = terminal.viewport_row(0).expect("row 0").cells;
    assert_eq!(cells[0].text, family);
    assert_eq!(cells[0].width, CellWidth::Wide);
    assert_eq!(cells[1].width, CellWidth::SpacerTail);
    assert_eq!(cells[2].text, "x");
}

#[test]
fn a_flag_is_one_grapheme() {
    let flag = "\u{1F1EF}\u{1F1F5}";
    let mut terminal = terminal(10, 2);
    terminal.feed(flag.as_bytes());

    let cells = terminal.viewport_row(0).expect("row 0").cells;
    assert_eq!(cells[0].text, flag);
    assert_eq!(cells[0].width, CellWidth::Wide);
}

#[test]
fn combining_marks_stay_with_their_base() {
    let mut terminal = terminal(10, 2);
    terminal.feed("e\u{301}a\u{308}z".as_bytes());

    let cells = terminal.viewport_row(0).expect("row 0").cells;
    assert_eq!(cells[0].text, "e\u{301}");
    assert_eq!(cells[1].text, "a\u{308}");
    assert_eq!(cells[2].text, "z");
    assert_eq!(terminal.cursor().x, 3);
}

#[test]
fn hebrew_is_kept_in_logical_order() {
    let mut terminal = terminal(10, 2);
    terminal.feed("שלום".as_bytes());

    let cells = terminal.viewport_row(0).expect("row 0").cells;
    let letters: Vec<&str> = cells[..4].iter().map(|c| c.text.as_str()).collect();
    assert_eq!(letters, ["ש", "ל", "ו", "ם"]);
}

#[test]
fn arabic_is_kept_in_logical_order_without_shaping() {
    let mut terminal = terminal(10, 2);
    terminal.feed("سلام".as_bytes());

    let cells = terminal.viewport_row(0).expect("row 0").cells;
    let letters: Vec<&str> = cells[..4].iter().map(|c| c.text.as_str()).collect();
    assert_eq!(letters, ["س", "ل", "ا", "م"]);
}

#[test]
fn sgr_sets_colors_and_attributes() {
    let mut terminal = terminal(10, 2);
    terminal.feed(b"\x1b[1;3;4;7;31;42mX\x1b[0m\x1b[38;2;10;20;30;48;5;200;2;9mY\x1b[0mZ");

    let cells = terminal.viewport_row(0).expect("row 0").cells;
    let x = &cells[0];
    assert_eq!(x.text, "X");
    assert!(x.bold && x.italic && x.inverse);
    assert_eq!(x.underline, Underline::Single);
    assert_eq!(x.fg, Color::Palette(1));
    assert_eq!(x.bg, Color::Palette(2));

    let y = &cells[1];
    assert_eq!(
        y.fg,
        Color::Rgb {
            r: 10,
            g: 20,
            b: 30
        }
    );
    assert_eq!(y.bg, Color::Palette(200));
    assert!(y.faint && y.strikethrough);
    assert!(!y.bold && !y.italic && !y.inverse);
    assert_eq!(y.underline, Underline::None);

    let z = &cells[2];
    assert_eq!(z.fg, Color::Default);
    assert_eq!(z.bg, Color::Default);
    assert!(!z.faint && !z.strikethrough);
}

#[test]
fn curly_underline_is_read_from_its_sub_parameter() {
    let mut terminal = terminal(10, 2);
    terminal.feed(b"\x1b[4:3mU");

    let cells = terminal.viewport_row(0).expect("row 0").cells;
    assert_eq!(cells[0].underline, Underline::Curly);
}

#[test]
fn erasing_with_a_background_paints_empty_cells() {
    let mut terminal = terminal(4, 2);
    terminal.feed(b"\x1b[44m\x1b[2K\x1b[0m");

    let cells = terminal.viewport_row(0).expect("row 0").cells;
    assert!(cells.iter().all(|cell| cell.bg == Color::Palette(4)));
    assert!(cells.iter().all(|cell| cell.text.is_empty()));
}

#[test]
fn the_alternate_screen_hides_and_restores_the_primary() {
    let mut terminal = terminal(10, 3);
    terminal.feed(b"primary");
    assert!(!terminal.is_alternate_screen());

    terminal.feed(b"\x1b[?1049h");
    assert!(terminal.is_alternate_screen());
    assert_eq!(viewport_text(&terminal, 0), "");
    // The alternate screen keeps the cursor where it was, so go home first.
    terminal.feed(b"\x1b[Halt");
    assert_eq!(viewport_text(&terminal, 0), "alt");

    terminal.feed(b"\x1b[?1049l");
    assert!(!terminal.is_alternate_screen());
    assert_eq!(viewport_text(&terminal, 0), "primary");
}

#[test]
fn lines_that_scroll_off_stay_in_scrollback() {
    let mut terminal = terminal(20, 10);
    let lines: Vec<String> = (0..100).map(|n| format!("line {n}")).collect();
    terminal.feed(lines.join("\r\n").as_bytes());

    assert_eq!(terminal.scrollback_rows(), 90);
    assert_eq!(terminal.total_rows(), 100);
    assert_eq!(
        text(&terminal.screen_row(0).expect("oldest").cells),
        "line 0"
    );
    assert_eq!(
        text(&terminal.screen_row(99).expect("newest").cells),
        "line 99"
    );
    assert_eq!(viewport_text(&terminal, 0), "line 90");
    assert!(terminal.screen_row(100).is_err());
}

#[test]
fn the_viewport_scrolls_through_history() {
    let mut terminal = terminal(20, 10);
    let lines: Vec<String> = (0..100).map(|n| format!("line {n}")).collect();
    terminal.feed(lines.join("\r\n").as_bytes());

    terminal
        .scroll_viewport(ghostty_vt::Scroll::Top)
        .expect("scroll to the top");
    assert_eq!(viewport_text(&terminal, 0), "line 0");
    terminal
        .scroll_viewport(ghostty_vt::Scroll::Delta(5))
        .expect("scroll down");
    assert_eq!(viewport_text(&terminal, 0), "line 5");
    terminal
        .scroll_viewport(ghostty_vt::Scroll::Bottom)
        .expect("scroll to the bottom");
    assert_eq!(viewport_text(&terminal, 0), "line 90");
}

#[test]
fn resizing_reflows_wrapped_lines() {
    let mut terminal = terminal(10, 5);
    terminal.feed(b"abcdefghijklmno\r\nnext");
    assert_eq!(viewport_text(&terminal, 0), "abcdefghij");
    assert_eq!(viewport_text(&terminal, 1), "klmno");

    terminal.resize(20, 5).expect("a wider terminal");
    assert_eq!(terminal.size(), Size { cols: 20, rows: 5 });
    assert_eq!(viewport_text(&terminal, 0), "abcdefghijklmno");
    assert_eq!(viewport_text(&terminal, 1), "next");

    terminal.resize(5, 5).expect("a narrower terminal");
    assert_eq!(viewport_text(&terminal, 0), "abcde");
    assert_eq!(viewport_text(&terminal, 1), "fghij");
    assert_eq!(viewport_text(&terminal, 2), "klmno");
    assert_eq!(viewport_text(&terminal, 3), "next");
}

#[test]
fn a_zero_size_is_refused() {
    assert!(matches!(
        Terminal::new(0, 10, 100),
        Err(ghostty_vt::Error::Create {
            cols: 0,
            rows: 10,
            ..
        })
    ));
    let mut terminal = terminal(10, 10);
    assert!(matches!(
        terminal.resize(10, 0),
        Err(ghostty_vt::Error::Resize { rows: 0, .. })
    ));
    assert_eq!(terminal.size(), Size { cols: 10, rows: 10 });
}
