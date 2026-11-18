//! A checkpoint over its limit trims the oldest history instead of failing.

use ghostty_vt::{Error, Point, Terminal};

const LINES: usize = 1000;

fn filled() -> Terminal {
    let mut terminal = Terminal::new(40, 5, LINES).unwrap();
    for n in 0..400 {
        terminal.feed(format!("line {n:04} {}\r\n", "abcdefghij".repeat(3)).as_bytes());
    }
    terminal
}

fn size_of(terminal: &Terminal) -> usize {
    terminal.checkpoint(usize::MAX / 2).unwrap().len()
}

#[test]
fn a_history_over_the_limit_trims_to_fit_and_reports_how_many_rows_went() {
    let mut terminal = filled();
    let before = terminal.scrollback_rows();
    assert!(before > 100);
    let full = size_of(&terminal);
    let mut bare = Terminal::new(40, 5, LINES).unwrap();
    bare.feed(b"line 0399\r\n");
    let limit = (full + size_of(&bare)) / 2;
    assert!(matches!(
        terminal.checkpoint(limit),
        Err(Error::CheckpointTooLarge { .. })
    ));

    let (bytes, trimmed) = terminal.checkpoint_trimmed(limit, LINES, 4 << 20).unwrap();

    assert!(bytes.len() <= limit);
    assert!(trimmed > 0);
    assert_eq!(terminal.scrollback_rows(), before - trimmed);
    let restored = Terminal::from_checkpoint(&bytes).unwrap();
    assert_eq!(restored.scrollback_rows(), before - trimmed);
}

#[test]
fn the_newest_rows_stay_when_the_oldest_go() {
    let mut terminal = filled();
    let full = size_of(&terminal);
    let mut bare = Terminal::new(40, 5, LINES).unwrap();
    bare.feed(b"x\r\n");
    let limit = (full + size_of(&bare)) / 2;
    terminal.checkpoint_trimmed(limit, LINES, 4 << 20).unwrap();
    let row = |y: u32| {
        let at = Point { x: 0, y };
        let end = Point { x: 39, y };
        terminal.selection_text(at, end).unwrap()
    };
    let newest = u32::try_from(terminal.total_rows()).unwrap() - 2;
    assert!(row(newest).contains("line 0399"), "the newest line is gone");
    assert!(!row(0).contains("line 0000"), "the oldest line is still first");
}

#[test]
fn a_checkpoint_that_fits_is_not_trimmed() {
    let mut terminal = filled();
    let (_, trimmed) = terminal.checkpoint_trimmed(16 << 20, LINES, 4 << 20).unwrap();
    assert_eq!(trimmed, 0);
}

#[test]
fn it_is_an_error_only_when_even_an_empty_history_does_not_fit() {
    let mut terminal = filled();
    assert!(matches!(
        terminal.checkpoint_trimmed(32, LINES, 4 << 20),
        Err(Error::CheckpointTooLarge { .. })
    ));
}
