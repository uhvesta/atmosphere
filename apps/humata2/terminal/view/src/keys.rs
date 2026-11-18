//! What a key press means to the terminal: copy, paste, a key for the
//! program, or nothing the view handles (so the app's own shortcuts, such
//! as Cmd+T, still work).

use gpui_kit::{Keystroke, Modifiers, PlatformInfo};
use terminal_grid::{Key, Mods};

use crate::host::ScrollTo;

/// What a key press does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyIntent {
    Copy,
    Paste,
    /// Select the whole screen and history: Cmd+A on a Mac,
    /// Ctrl+Shift+A elsewhere.
    SelectAll,
    /// Move through the history: Cmd with Page Up, Page Down, Home or End
    /// on a Mac, Shift with them elsewhere, as in `Ghostty`.
    Scroll(ScrollTo),
    /// A key for the program, with the text it types.
    Send {
        key: Key,
        mods: Mods,
        consumed_mods: Mods,
        text: String,
        unshifted: Option<char>,
    },
    /// Bytes a key stands for whatever the program's key encoding, such as
    /// Esc b for ⌥← on a Mac.
    Text(&'static str),
    /// Not the terminal's: leave it to the app.
    Ignore,
}

/// Which system's shortcuts apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    /// Cmd+C copies and Cmd+V pastes; any other Cmd shortcut is the app's.
    Mac,
    /// Ctrl+Shift+C copies, and Ctrl+Shift+V or Shift+Insert pastes, so
    /// Ctrl+C still interrupts the program.
    Other,
}

impl Platform {
    /// The shortcuts of the system GPUI reports, which in a browser is the
    /// visitor's system rather than the one the app was built for.
    pub(crate) fn of(info: PlatformInfo) -> Platform {
        if info.is_mac() {
            Platform::Mac
        } else {
            Platform::Other
        }
    }
}

/// What `keystroke` does on `platform`.
pub(crate) fn intent(keystroke: &Keystroke, platform: Platform) -> KeyIntent {
    let m = keystroke.modifiers;
    let key = keystroke.key.as_str();
    match platform {
        Platform::Mac if m.platform && !m.control && !m.alt => {
            return match key {
                "c" if !m.shift => KeyIntent::Copy,
                "v" if !m.shift => KeyIntent::Paste,
                "a" if !m.shift => KeyIntent::SelectAll,
                _ if !m.shift => scroll_key(key).map_or(KeyIntent::Ignore, KeyIntent::Scroll),
                _ => KeyIntent::Ignore,
            };
        }
        Platform::Mac if m.platform => return KeyIntent::Ignore,
        Platform::Mac if m.alt && !m.control && !m.shift => {
            if let Some(bytes) = word_key(key) {
                return KeyIntent::Text(bytes);
            }
        }
        Platform::Other if m.control && m.shift && !m.alt && !m.platform => match key {
            "c" => return KeyIntent::Copy,
            "v" => return KeyIntent::Paste,
            "a" => return KeyIntent::SelectAll,
            _ => {}
        },
        Platform::Other if m.shift && !m.control && !m.alt && key == "insert" => {
            return KeyIntent::Paste;
        }
        Platform::Other if m.shift && !m.control && !m.alt && !m.platform => {
            if let Some(to) = scroll_key(key) {
                return KeyIntent::Scroll(to);
            }
        }
        _ => {}
    }
    send(keystroke, platform)
}

/// The key as the program gets it.
fn send(keystroke: &Keystroke, platform: Platform) -> KeyIntent {
    let m = keystroke.modifiers;
    let key = keystroke.key.as_str();
    let Some(physical) = physical_key(key) else {
        return KeyIntent::Ignore;
    };
    let mods = mods(m);
    let text = keystroke
        .key_char
        .as_deref()
        .filter(|text| !text.chars().any(char::is_control))
        .unwrap_or_default()
        .to_owned();
    let mut chars = key.chars();
    let unshifted = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c.to_ascii_lowercase()),
        _ => None,
    };
    // The layout used Shift (or Option on a Mac) when the text differs
    // from the key's own character.
    let mut consumed_mods = Mods::NONE;
    if !text.is_empty() && unshifted.is_none_or(|c| text != c.to_string()) {
        if m.shift {
            consumed_mods |= Mods::SHIFT;
        }
        if m.alt && platform == Platform::Mac {
            consumed_mods |= Mods::ALT;
        }
    }
    KeyIntent::Send {
        key: physical,
        mods,
        consumed_mods,
        text,
        unshifted,
    }
}

/// A history key on a screen that keeps no history, a full-screen
/// program's: the program gets the key without the modifier that made it
/// a history key, so ⌘PgUp pages up in `htop`.
pub(crate) fn program_key(keystroke: &Keystroke, platform: Platform) -> KeyIntent {
    let mut keystroke = keystroke.clone();
    match platform {
        Platform::Mac => keystroke.modifiers.platform = false,
        Platform::Other => keystroke.modifiers.shift = false,
    }
    send(&keystroke, platform)
}

/// Where a history key scrolls to.
fn scroll_key(key: &str) -> Option<ScrollTo> {
    match key {
        "pageup" => Some(ScrollTo::PageUp),
        "pagedown" => Some(ScrollTo::PageDown),
        "home" => Some(ScrollTo::Top),
        "end" => Some(ScrollTo::Bottom),
        _ => None,
    }
}

/// On a Mac, ⌥←, ⌥→ and ⌥⌫ move and delete by word whatever Option does
/// to other keys (D420): Esc b, Esc f and Esc DEL, which shells and line
/// editors bind.
fn word_key(key: &str) -> Option<&'static str> {
    match key {
        "left" => Some("\x1bb"),
        "right" => Some("\x1bf"),
        "backspace" => Some("\x1b\x7f"),
        _ => None,
    }
}

/// The terminal's modifiers for `GPUI`'s.
pub(crate) fn mods(modifiers: Modifiers) -> Mods {
    let mut mods = Mods::NONE;
    if modifiers.shift {
        mods |= Mods::SHIFT;
    }
    if modifiers.control {
        mods |= Mods::CTRL;
    }
    if modifiers.alt {
        mods |= Mods::ALT;
    }
    if modifiers.platform {
        mods |= Mods::SUPER;
    }
    mods
}

/// The physical key `GPUI` names `key`: the character printed on it on a
/// US layout (which `GPUI` gives even on other layouts), its shifted
/// character, or a name such as `"enter"`.
fn physical_key(key: &str) -> Option<Key> {
    let named = match key {
        "backspace" => Key::Backspace,
        "enter" => Key::Enter,
        "space" | " " => Key::Space,
        "tab" => Key::Tab,
        "delete" => Key::Delete,
        "end" => Key::End,
        "home" => Key::Home,
        "insert" => Key::Insert,
        "pagedown" => Key::PageDown,
        "pageup" => Key::PageUp,
        "down" => Key::ArrowDown,
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        "up" => Key::ArrowUp,
        "escape" => Key::Escape,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        _ => {
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            return printed_key(c);
        }
    };
    Some(named)
}

fn printed_key(c: char) -> Option<Key> {
    const LETTERS: [Key; 26] = [
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
    ];
    const DIGITS: [Key; 10] = [
        Key::Digit0,
        Key::Digit1,
        Key::Digit2,
        Key::Digit3,
        Key::Digit4,
        Key::Digit5,
        Key::Digit6,
        Key::Digit7,
        Key::Digit8,
        Key::Digit9,
    ];
    let c = c.to_ascii_lowercase();
    let key = match c {
        'a'..='z' => LETTERS[usize::from(c as u8 - b'a')],
        '0'..='9' => DIGITS[usize::from(c as u8 - b'0')],
        ')' => Key::Digit0,
        '!' => Key::Digit1,
        '@' => Key::Digit2,
        '#' => Key::Digit3,
        '$' => Key::Digit4,
        '%' => Key::Digit5,
        '^' => Key::Digit6,
        '&' => Key::Digit7,
        '*' => Key::Digit8,
        '(' => Key::Digit9,
        '`' | '~' => Key::Backquote,
        '\\' | '|' => Key::Backslash,
        '[' | '{' => Key::BracketLeft,
        ']' | '}' => Key::BracketRight,
        ',' | '<' => Key::Comma,
        '=' | '+' => Key::Equal,
        '-' | '_' => Key::Minus,
        '.' | '>' => Key::Period,
        '\'' | '"' => Key::Quote,
        ';' | ':' => Key::Semicolon,
        '/' | '?' => Key::Slash,
        // Keys of other layouts with no US position: the encoder works
        // from the text.
        _ => Key::Unidentified,
    };
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(source: &'static str) -> Keystroke {
        Keystroke::parse(source).unwrap_or_else(|error| panic!("{source}: {error}"))
    }

    #[::std::prelude::v1::test]
    fn the_copy_and_paste_shortcuts_follow_the_platform() {
        assert_eq!(intent(&stroke("cmd-c"), Platform::Mac), KeyIntent::Copy);
        assert_eq!(intent(&stroke("cmd-v"), Platform::Mac), KeyIntent::Paste);
        assert_eq!(intent(&stroke("ctrl-shift-c"), Platform::Other), KeyIntent::Copy);
        assert_eq!(intent(&stroke("ctrl-shift-v"), Platform::Other), KeyIntent::Paste);
        assert_eq!(intent(&stroke("shift-insert"), Platform::Other), KeyIntent::Paste);
    }

    #[::std::prelude::v1::test]
    fn select_all_follows_the_platform() {
        assert_eq!(intent(&stroke("cmd-a"), Platform::Mac), KeyIntent::SelectAll);
        assert_eq!(intent(&stroke("ctrl-shift-a"), Platform::Other), KeyIntent::SelectAll);
        assert_eq!(intent(&stroke("cmd-shift-a"), Platform::Mac), KeyIntent::Ignore);
        let KeyIntent::Send { key, mods, .. } = intent(&stroke("ctrl-a"), Platform::Mac) else {
            panic!("ctrl-a should go to the program");
        };
        assert_eq!((key, mods), (Key::A, Mods::CTRL));
    }

    #[::std::prelude::v1::test]
    fn a_browser_uses_the_shortcuts_of_the_system_it_runs_on() {
        use gpui_kit::{OperatingSystem, PlatformTarget};
        let on = |os| Platform::of(PlatformInfo { os, target: PlatformTarget::Web });
        assert_eq!(on(OperatingSystem::Mac), Platform::Mac);
        assert_eq!(on(OperatingSystem::Linux), Platform::Other);
        assert_eq!(on(OperatingSystem::Windows), Platform::Other);
        assert_eq!(on(OperatingSystem::Unknown), Platform::Other);
    }

    #[::std::prelude::v1::test]
    fn ctrl_c_goes_to_the_program_on_every_platform() {
        for platform in [Platform::Mac, Platform::Other] {
            let KeyIntent::Send { key, mods, .. } = intent(&stroke("ctrl-c"), platform) else {
                panic!("ctrl-c should go to the program");
            };
            assert_eq!(key, Key::C);
            assert_eq!(mods, Mods::CTRL);
        }
    }

    /// D420: on a Mac, ⌥←, ⌥→ and ⌥⌫ move and delete by word whatever the
    /// Option setting, with the bytes shells bind; with Shift, and off a
    /// Mac, they keep the program's own key encoding.
    #[::std::prelude::v1::test]
    fn option_arrows_and_backspace_move_by_word_on_a_mac() {
        assert_eq!(intent(&stroke("alt-left"), Platform::Mac), KeyIntent::Text("\x1bb"));
        assert_eq!(intent(&stroke("alt-right"), Platform::Mac), KeyIntent::Text("\x1bf"));
        assert_eq!(
            intent(&stroke("alt-backspace"), Platform::Mac),
            KeyIntent::Text("\x1b\x7f")
        );
        assert!(matches!(
            intent(&stroke("alt-shift-left"), Platform::Mac),
            KeyIntent::Send { .. }
        ));
        assert!(matches!(
            intent(&stroke("alt-left"), Platform::Other),
            KeyIntent::Send { .. }
        ));
    }

    #[::std::prelude::v1::test]
    fn other_cmd_shortcuts_are_left_to_the_app() {
        assert_eq!(intent(&stroke("cmd-t"), Platform::Mac), KeyIntent::Ignore);
        assert_eq!(intent(&stroke("cmd-shift-c"), Platform::Mac), KeyIntent::Ignore);
    }

    #[::std::prelude::v1::test]
    fn typed_text_carries_the_key_and_what_shift_did() {
        let KeyIntent::Send {
            key,
            text,
            consumed_mods,
            unshifted,
            ..
        } = intent(&stroke("shift-a->A"), Platform::Mac)
        else {
            panic!("a letter goes to the program");
        };
        assert_eq!(key, Key::A);
        assert_eq!(text, "A");
        assert_eq!(consumed_mods, Mods::SHIFT);
        assert_eq!(unshifted, Some('a'));
    }

    #[::std::prelude::v1::test]
    fn history_keys_scroll_the_view_instead_of_reaching_the_program() {
        let cases = [
            ("cmd-pageup", Platform::Mac, ScrollTo::PageUp),
            ("cmd-pagedown", Platform::Mac, ScrollTo::PageDown),
            ("cmd-home", Platform::Mac, ScrollTo::Top),
            ("cmd-end", Platform::Mac, ScrollTo::Bottom),
            ("shift-pageup", Platform::Other, ScrollTo::PageUp),
            ("shift-end", Platform::Other, ScrollTo::Bottom),
        ];
        for (source, platform, to) in cases {
            assert_eq!(intent(&stroke(source), platform), KeyIntent::Scroll(to), "{source}");
        }
        // On a screen with no history, a full-screen program's, the
        // program gets the key without the history modifier.
        for (source, platform) in [("cmd-pageup", Platform::Mac), ("shift-pageup", Platform::Other)] {
            let KeyIntent::Send { key, mods, .. } = program_key(&stroke(source), platform) else {
                panic!("{source} goes to the program");
            };
            assert_eq!((key, mods), (Key::PageUp, Mods::NONE), "{source}");
        }
        // Without the modifier the program gets the key, as `less` wants.
        assert!(matches!(
            intent(&stroke("pageup"), Platform::Mac),
            KeyIntent::Send { key: Key::PageUp, .. }
        ));
        assert!(matches!(
            intent(&stroke("shift-pageup"), Platform::Mac),
            KeyIntent::Send { key: Key::PageUp, .. }
        ));
    }

    #[::std::prelude::v1::test]
    fn named_keys_type_no_text() {
        let KeyIntent::Send { key, text, .. } = intent(&stroke("enter"), Platform::Other) else {
            panic!("enter goes to the program");
        };
        assert_eq!(key, Key::Enter);
        assert!(text.is_empty());
        let KeyIntent::Send { key, .. } = intent(&stroke("up"), Platform::Mac) else {
            panic!("arrows go to the program");
        };
        assert_eq!(key, Key::ArrowUp);
    }
}
