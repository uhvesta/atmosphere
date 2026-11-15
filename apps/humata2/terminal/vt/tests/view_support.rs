//! What an embedded view reads beyond the cells: the scrollbar, the
//! viewport staying put while output streams, modes, a rectangular
//! selection and focus reports.

use ghostty_vt::{DecMode, Grid, Scroll, Scrollbar, Span, Terminal, ViewportPoint};

/// Row `y` of the viewport as text, trailing blanks trimmed.
fn viewport_text(terminal: &Terminal, y: u16) -> String {
    let row = terminal.viewport_row(y).expect("a viewport row");
    let text: String = row.cells.iter().map(|cell| cell.text.as_str()).collect();
    text.trim_end().to_owned()
}

fn lines(terminal: &mut Terminal, from: u32, to: u32) {
    for n in from..to {
        terminal.feed(format!("line {n}\r\n").as_bytes());
    }
}

#[test]
fn the_scrollbar_follows_the_viewport_through_history() {
    let mut terminal = Terminal::new(20, 5, 1000).expect("a terminal");
    lines(&mut terminal, 0, 20);

    let bottom = terminal.scrollbar().expect("a scrollbar");
    assert_eq!(bottom.len, 5);
    assert!(bottom.total > bottom.len, "20 lines on 5 rows leave history");
    assert!(bottom.at_bottom());

    terminal.scroll_viewport(Scroll::Delta(-3)).expect("scrolled up");
    let up = terminal.scrollbar().expect("a scrollbar");
    assert_eq!(up.offset + 3, bottom.offset);
    assert!(!up.at_bottom());

    terminal.scroll_viewport(Scroll::Top).expect("scrolled to the top");
    assert_eq!(terminal.scrollbar().map(|bar| bar.offset), Some(0));
}

#[test]
fn output_doesnt_move_a_viewport_scrolled_up() {
    let mut terminal = Terminal::new(20, 4, 1000).expect("a terminal");
    lines(&mut terminal, 0, 30);
    terminal.scroll_viewport(Scroll::Delta(-10)).expect("scrolled up");
    let before: Vec<String> = (0..4).map(|y| viewport_text(&terminal, y)).collect();

    lines(&mut terminal, 30, 60);

    let after: Vec<String> = (0..4).map(|y| viewport_text(&terminal, y)).collect();
    assert_eq!(after, before, "the rows being read stay in view");
    assert!(!terminal.scrollbar().is_some_and(Scrollbar::at_bottom));
}

#[test]
fn output_keeps_a_viewport_at_the_bottom_following() {
    let mut terminal = Terminal::new(20, 4, 1000).expect("a terminal");
    lines(&mut terminal, 0, 30);

    lines(&mut terminal, 30, 40);

    assert!(terminal.scrollbar().is_some_and(Scrollbar::at_bottom));
    assert_eq!(viewport_text(&terminal, 2), "line 39");
}

#[test]
fn a_grid_reports_the_same_scrollbar() {
    let mut terminal = Terminal::new(20, 4, 1000).expect("a terminal");
    lines(&mut terminal, 0, 10);
    terminal.scroll_viewport(Scroll::Delta(-2)).expect("scrolled up");
    Grid::sync(&mut terminal);

    assert_eq!(Grid::scrollbar(&terminal), terminal.scrollbar());
}

#[test]
fn modes_follow_what_the_program_set() {
    let mut terminal = Terminal::new(20, 4, 100).expect("a terminal");
    for mode in [
        DecMode::ApplicationCursorKeys,
        DecMode::FocusReporting,
        DecMode::AlternateScroll,
        DecMode::BracketedPaste,
    ] {
        let before = terminal.is_mode_set(mode);
        let number = match mode {
            DecMode::ApplicationCursorKeys => 1,
            DecMode::FocusReporting => 1004,
            DecMode::AlternateScroll => 1007,
            DecMode::BracketedPaste => 2004,
        };
        terminal.feed(format!("\x1b[?{number}h").as_bytes());
        assert!(terminal.is_mode_set(mode), "{mode:?} is on after DECSET");
        terminal.feed(format!("\x1b[?{number}l").as_bytes());
        assert!(!terminal.is_mode_set(mode), "{mode:?} is off after DECRST (was {before})");
    }
}

#[test]
fn focus_is_reported_only_when_the_program_asked() {
    let mut terminal = Terminal::new(20, 4, 100).expect("a terminal");
    assert_eq!(terminal.focus_report(true).expect("encoded"), None);

    terminal.feed(b"\x1b[?1004h");

    assert_eq!(terminal.focus_report(true).expect("encoded"), Some(b"\x1b[I".to_vec()));
    assert_eq!(terminal.focus_report(false).expect("encoded"), Some(b"\x1b[O".to_vec()));
}

#[test]
fn a_rectangular_selection_takes_a_block_of_columns() {
    let mut terminal = Terminal::new(10, 3, 100).expect("a terminal");
    terminal.feed(b"abcdefgh\r\nijklmnop\r\nqrstuvwx");

    terminal
        .select_rectangle(ViewportPoint { x: 5, y: 2 }, ViewportPoint { x: 2, y: 0 })
        .expect("selected");
    Grid::sync(&mut terminal);

    for y in 0..3 {
        assert_eq!(Grid::selection(&terminal, y), Some(Span { start: 2, end: 5 }), "row {y}");
    }
    assert_eq!(
        terminal.selected_text().expect("copied").as_deref(),
        Some("cdef\nklmn\nstuv")
    );
}

#[test]
fn with_option_as_alt_option_and_a_letter_send_escape_and_the_letter() {
    use ghostty_vt::{Key, KeyEvent, Mods, OptionAsAlt};
    let mut terminal = Terminal::new(20, 5, 0).expect("a terminal");
    terminal.set_option_as_alt(OptionAsAlt::On);
    assert_eq!(terminal.option_as_alt(), OptionAsAlt::On);
    // The text is the key's own letter: the pane asks for it when Option
    // acts as Alt, instead of the character macOS would type.
    let event = KeyEvent {
        key: Key::F,
        mods: Mods::ALT,
        text: "f",
        unshifted: Some('f'),
        ..KeyEvent::default()
    };
    assert_eq!(terminal.encode_key(&event).expect("encoded"), b"\x1bf".to_vec());
}
