//! What a key press means to the terminal: copy, paste, a key for the
//! program, or nothing the view handles (so the app's own shortcuts, such
//! as Cmd+T, still work).

use gpui_kit::{Keystroke, Modifiers, PlatformInfo};
use terminal_grid::{Key, Mods};

/// What a key press does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyIntent {
    Copy,
    Paste,
    /// A key for the program, with the text it types.
    Send {
        key: Key,
        mods: Mods,
        consumed_mods: Mods,
        text: String,
        unshifted: Option<char>,
    },
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
                _ => KeyIntent::Ignore,
            };
        }
        Platform::Mac if m.platform => return KeyIntent::Ignore,
        Platform::Other if m.control && m.shift && !m.alt && !m.platform => match key {
            "c" => return KeyIntent::Copy,
            "v" => return KeyIntent::Paste,
            _ => {}
        },
        Platform::Other if m.shift && !m.control && !m.alt && key == "insert" => {
            return KeyIntent::Paste;
        }
        _ => {}
    }
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
