//! Turning pasted text into what the program reads.

use ghostty_vt::{Error, Terminal};

fn terminal() -> Terminal {
    Terminal::new(20, 5, 100).expect("a terminal")
}

#[test]
fn a_bracketed_paste_is_marked_and_keeps_its_new_lines() {
    let mut terminal = terminal();
    terminal.feed(b"\x1b[?2004h");
    assert!(terminal.is_bracketed_paste());

    let bytes = terminal.paste("ls\n", false).expect("a safe paste");
    assert_eq!(bytes, b"\x1b[200~ls\n\x1b[201~");
}

#[test]
fn a_plain_paste_of_one_line_is_sent_as_typed() {
    let terminal = terminal();
    assert!(!terminal.is_bracketed_paste());
    assert_eq!(terminal.paste("ls -la", false).expect("a safe paste"), b"ls -la");
}

#[test]
fn a_plain_paste_with_a_line_break_needs_confirming() {
    let terminal = terminal();
    assert!(matches!(
        terminal.paste("rm -rf build\nls", false),
        Err(Error::UnsafePaste { line_breaks: 1 })
    ));

    let bytes = terminal
        .paste("rm -rf build\nls", true)
        .expect("a confirmed paste");
    assert_eq!(bytes, b"rm -rf build\rls");
}

#[test]
fn pasted_text_cannot_end_a_bracketed_paste_early() {
    let mut terminal = terminal();
    terminal.feed(b"\x1b[?2004h");

    let bytes = terminal
        .paste("a\x1b[201~rm -rf ~\n", false)
        .expect("a bracketed paste");

    let end = b"\x1b[201~";
    let ends = bytes.windows(end.len()).filter(|w| w == end).count();
    assert_eq!(ends, 1, "only the closing marker remains");
    assert!(bytes.ends_with(end));
}

#[test]
fn turning_bracketed_paste_off_goes_back_to_plain() {
    let mut terminal = terminal();
    terminal.feed(b"\x1b[?2004h\x1b[?2004l");
    assert!(!terminal.is_bracketed_paste());
    assert!(terminal.paste("ls\n", false).is_err());
}
