//! What a program asks of the terminal besides drawing.

use ghostty_vt::{Event, Terminal};

fn terminal() -> Terminal {
    Terminal::new(20, 5, 100).expect("a terminal")
}

#[test]
fn osc_2_sets_the_title() {
    let mut terminal = terminal();
    assert_eq!(terminal.title(), None);

    terminal.feed(b"\x1b]2;build: ok\x07");

    assert_eq!(terminal.title().as_deref(), Some("build: ok"));
    assert_eq!(
        terminal.drain_events(),
        [Event::TitleChanged("build: ok".to_owned())]
    );
    assert!(terminal.drain_events().is_empty());
}

#[test]
fn osc_0_sets_the_title_too() {
    let mut terminal = terminal();
    terminal.feed("\x1b]0;café\x1b\\".as_bytes());

    assert_eq!(terminal.title().as_deref(), Some("café"));
    assert!(
        terminal
            .drain_events()
            .contains(&Event::TitleChanged("café".to_owned()))
    );
}

#[test]
fn a_title_split_across_writes_arrives_once_complete() {
    let mut terminal = terminal();
    terminal.feed(b"\x1b]2;hal");
    assert!(terminal.drain_events().is_empty());

    terminal.feed(b"f\x07");
    assert_eq!(
        terminal.drain_events(),
        [Event::TitleChanged("half".to_owned())]
    );
}

#[test]
fn the_bell_is_an_event() {
    let mut terminal = terminal();
    terminal.feed(b"ding\x07");

    assert_eq!(terminal.drain_events(), [Event::Bell]);
}

#[test]
fn a_cursor_position_query_is_answered() {
    let mut terminal = terminal();
    terminal.feed(b"abc\x1b[6n");

    assert_eq!(
        terminal.drain_events(),
        [Event::Reply(b"\x1b[1;4R".to_vec())]
    );
}

#[test]
fn a_reported_directory_is_kept() {
    let mut terminal = terminal();
    terminal.feed(b"\x1b]7;file://localhost/tmp/project\x07");

    let events = terminal.drain_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::PwdChanged(pwd) if pwd.ends_with("/tmp/project"))),
        "{events:?}"
    );
    assert!(
        terminal
            .pwd()
            .is_some_and(|pwd| pwd.ends_with("/tmp/project"))
    );
}

#[test]
fn terminals_on_other_threads_are_independent() {
    let handles: Vec<_> = (0..4)
        .map(|n| {
            std::thread::spawn(move || {
                let mut terminal = terminal();
                terminal.feed(format!("\x1b]2;terminal {n}\x07").as_bytes());
                terminal.title()
            })
        })
        .collect();

    for (n, handle) in handles.into_iter().enumerate() {
        let title = handle.join().expect("the thread finishes");
        assert_eq!(title, Some(format!("terminal {n}")));
    }
}

#[test]
fn a_terminal_moved_to_another_thread_keeps_its_screen() {
    let mut terminal = terminal();
    terminal.feed(b"\x1b]2;before\x07");

    let terminal = std::thread::spawn(move || {
        terminal.feed(b"\x1b]2;after\x07");
        terminal
    })
    .join()
    .expect("the thread finishes");

    assert_eq!(terminal.title().as_deref(), Some("after"));
}
