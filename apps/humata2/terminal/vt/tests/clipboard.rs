//! What a program asks of the clipboard (OSC 52) and how it is answered.
//! The terminal itself never touches a clipboard.

use ghostty_vt::{ClipboardTarget, Event, Terminal, clipboard_read_reply};

fn terminal() -> Terminal {
    Terminal::new(20, 5, 100).expect("a terminal")
}

#[test]
fn a_clipboard_write_is_raised_for_the_host() {
    let mut terminal = terminal();
    terminal.feed(b"\x1b]52;c;aGVsbG8=\x07");

    assert_eq!(
        terminal.drain_events(),
        vec![Event::ClipboardWrite {
            target: ClipboardTarget::Standard,
            text: "hello".to_owned()
        }]
    );
}

#[test]
fn a_write_names_its_clipboard() {
    let mut terminal = terminal();
    terminal.feed(b"\x1b]52;p;aGk=\x1b\\");

    assert_eq!(
        terminal.drain_events(),
        vec![Event::ClipboardWrite {
            target: ClipboardTarget::Primary,
            text: "hi".to_owned()
        }]
    );
}

#[test]
fn a_clipboard_read_is_raised_and_not_answered() {
    let mut terminal = terminal();
    terminal.feed(b"\x1b]52;c;?\x07");

    let events = terminal.drain_events();
    assert_eq!(
        events,
        vec![Event::ClipboardReadRequested {
            target: ClipboardTarget::Standard
        }]
    );
    assert!(
        !events.iter().any(|event| matches!(event, Event::Reply(_))),
        "the program gets no answer until the host gives one"
    );
}

#[test]
fn other_replies_still_go_out_after_a_clipboard_read() {
    let mut terminal = terminal();
    terminal.feed(b"\x1b]52;c;?\x07\x1b[6n");

    let events = terminal.drain_events();
    assert_eq!(
        events.first(),
        Some(&Event::ClipboardReadRequested {
            target: ClipboardTarget::Standard
        })
    );
    assert_eq!(events.get(1), Some(&Event::Reply(b"\x1b[1;1R".to_vec())));
}

#[test]
fn the_host_answers_a_read_with_text_or_an_empty_clipboard() {
    assert_eq!(
        clipboard_read_reply(ClipboardTarget::Standard, Some("hello")),
        b"\x1b]52;c;aGVsbG8=\x1b\\"
    );
    assert_eq!(
        clipboard_read_reply(ClipboardTarget::Selection, None),
        b"\x1b]52;s;\x1b\\"
    );
}
