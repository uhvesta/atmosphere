//! The bytes a program reads for keys and mouse events.

use ghostty_vt::{
    Key, KeyAction, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent, OptionAsAlt, Terminal,
};

fn terminal() -> Terminal {
    Terminal::new(80, 24, 100).expect("a terminal")
}

fn key(terminal: &mut Terminal, event: KeyEvent<'_>) -> Vec<u8> {
    terminal.encode_key(&event).expect("an encoded key")
}

#[test]
fn enter_sends_a_carriage_return() {
    let mut terminal = terminal();
    assert_eq!(key(&mut terminal, KeyEvent::press(Key::Enter)), b"\r");
}

#[test]
fn arrows_follow_the_cursor_key_mode() {
    let mut terminal = terminal();
    assert_eq!(key(&mut terminal, KeyEvent::press(Key::ArrowUp)), b"\x1b[A");
    assert_eq!(
        key(&mut terminal, KeyEvent::press(Key::ArrowDown)),
        b"\x1b[B"
    );
    assert_eq!(
        key(&mut terminal, KeyEvent::press(Key::ArrowRight)),
        b"\x1b[C"
    );
    assert_eq!(
        key(&mut terminal, KeyEvent::press(Key::ArrowLeft)),
        b"\x1b[D"
    );

    // DECCKM: application cursor keys, as full-screen programs ask for.
    terminal.feed(b"\x1b[?1h");
    assert_eq!(key(&mut terminal, KeyEvent::press(Key::ArrowUp)), b"\x1bOA");
    assert_eq!(
        key(&mut terminal, KeyEvent::press(Key::ArrowLeft)),
        b"\x1bOD"
    );

    terminal.feed(b"\x1b[?1l");
    assert_eq!(key(&mut terminal, KeyEvent::press(Key::ArrowUp)), b"\x1b[A");
}

#[test]
fn ctrl_c_sends_etx() {
    let mut terminal = terminal();
    let event = KeyEvent::press(Key::C).with_mods(Mods::CTRL).with_text("c");
    assert_eq!(key(&mut terminal, event), [0x03]);
}

#[test]
fn alt_and_a_letter_sends_escape_first_when_option_is_alt() {
    let mut terminal = terminal();
    terminal.set_option_as_alt(OptionAsAlt::On);
    let event = KeyEvent::press(Key::X).with_mods(Mods::ALT).with_text("x");
    assert_eq!(key(&mut terminal, event), b"\x1bx");
}

#[test]
fn a_letter_sends_its_text() {
    let mut terminal = terminal();
    let event = KeyEvent::press(Key::A)
        .with_mods(Mods::SHIFT)
        .with_text("A");
    let event = KeyEvent {
        consumed_mods: Mods::SHIFT,
        ..event
    };
    assert_eq!(key(&mut terminal, event), b"A");
}

#[test]
fn releasing_a_key_sends_nothing_by_default() {
    let mut terminal = terminal();
    let event = KeyEvent {
        action: KeyAction::Release,
        ..KeyEvent::press(Key::A).with_text("a")
    };
    assert_eq!(key(&mut terminal, event), b"");
}

#[test]
fn mods_combine() {
    let mods = Mods::CTRL | Mods::SHIFT;
    assert!(mods.contains(Mods::CTRL));
    assert!(mods.contains(Mods::SHIFT));
    assert!(!mods.contains(Mods::ALT));
    assert!(mods.contains(Mods::NONE));
}

fn left_press_at(x: f32, y: f32) -> MouseEvent {
    MouseEvent {
        action: MouseAction::Press,
        button: Some(MouseButton::Left),
        x,
        y,
        ..MouseEvent::default()
    }
}

#[test]
fn the_mouse_sends_nothing_until_the_program_asks() {
    let mut terminal = terminal();
    terminal
        .resize_with_cell_size(80, 24, 10, 20)
        .expect("a cell size");

    assert!(!terminal.is_mouse_tracking());
    let bytes = terminal
        .encode_mouse(&left_press_at(55.0, 45.0))
        .expect("an encoded event");
    assert!(bytes.is_empty());
}

#[test]
fn an_sgr_mouse_press_reports_the_cell_under_the_pointer() {
    let mut terminal = terminal();
    terminal
        .resize_with_cell_size(80, 24, 10, 20)
        .expect("a cell size");
    terminal.feed(b"\x1b[?1000h\x1b[?1006h");
    assert!(terminal.is_mouse_tracking());

    // Pixel (55, 45) is in column 5, row 2, which SGR counts from 1.
    let bytes = terminal
        .encode_mouse(&left_press_at(55.0, 45.0))
        .expect("an encoded event");
    assert_eq!(bytes, b"\x1b[<0;6;3M");
}
