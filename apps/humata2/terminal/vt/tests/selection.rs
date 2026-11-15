//! Copying text out of the screen and finding it.

use ghostty_vt::{Match, Point, Terminal};

#[test]
fn a_selection_across_a_wrapped_line_copies_as_one_line() {
    let mut terminal = Terminal::new(10, 5, 100).expect("a terminal");
    terminal.feed(b"hello world, wrapped\r\nnext line");

    let text = terminal
        .selection_text(Point { x: 0, y: 0 }, Point { x: 9, y: 1 })
        .expect("the selected text");
    assert_eq!(text, "hello world, wrapped");
}

#[test]
fn a_selection_across_a_new_line_keeps_it() {
    let mut terminal = Terminal::new(10, 5, 100).expect("a terminal");
    terminal.feed(b"one\r\ntwo\r\nthree");

    let text = terminal
        .selection_text(Point { x: 0, y: 0 }, Point { x: 2, y: 1 })
        .expect("the selected text");
    assert_eq!(text, "one\ntwo");
}

#[test]
fn a_selection_inside_a_line_copies_just_that_part() {
    let mut terminal = Terminal::new(20, 3, 100).expect("a terminal");
    terminal.feed(b"cargo build --release");

    let text = terminal
        .selection_text(Point { x: 6, y: 0 }, Point { x: 10, y: 0 })
        .expect("the selected text");
    assert_eq!(text, "build");
}

#[test]
fn a_selection_can_reach_into_scrollback() {
    let mut terminal = Terminal::new(10, 2, 100).expect("a terminal");
    terminal.feed(b"first\r\nsecond\r\nthird\r\nfourth");
    assert!(terminal.scrollback_rows() >= 2);

    let text = terminal
        .selection_text(Point { x: 0, y: 0 }, Point { x: 5, y: 1 })
        .expect("the selected text");
    assert_eq!(text, "first\nsecond");
}

#[test]
fn a_point_off_the_screen_is_refused() {
    let mut terminal = Terminal::new(10, 2, 100).expect("a terminal");
    terminal.feed(b"text");

    assert!(matches!(
        terminal.selection_text(Point { x: 0, y: 0 }, Point { x: 0, y: 50 }),
        Err(ghostty_vt::Error::Point { y: 50, .. })
    ));
}

#[test]
fn search_finds_every_match_oldest_first() {
    let mut terminal = Terminal::new(40, 3, 100).expect("a terminal");
    terminal.feed(b"error: one\r\nok\r\nERROR: two\r\nfine\r\nan error here");

    let matches = terminal.search("error").expect("a search");
    assert_eq!(
        matches,
        [
            Match {
                start: Point { x: 0, y: 0 },
                end: Point { x: 4, y: 0 },
            },
            Match {
                start: Point { x: 0, y: 2 },
                end: Point { x: 4, y: 2 },
            },
            Match {
                start: Point { x: 3, y: 4 },
                end: Point { x: 7, y: 4 },
            },
        ]
    );
}

#[test]
fn search_with_no_match_finds_nothing() {
    let mut terminal = Terminal::new(40, 3, 100).expect("a terminal");
    terminal.feed(b"all good");

    assert!(terminal.search("panic").expect("a search").is_empty());
}

#[test]
fn a_match_can_be_copied() {
    let mut terminal = Terminal::new(40, 3, 100).expect("a terminal");
    terminal.feed(b"see https://example.com/x for more");

    let found = terminal.search("https://").expect("a search");
    assert_eq!(found.len(), 1);
    let text = terminal
        .selection_text(found[0].start, Point { x: 24, y: 0 })
        .expect("the selected text");
    assert_eq!(text, "https://example.com/x");
}
