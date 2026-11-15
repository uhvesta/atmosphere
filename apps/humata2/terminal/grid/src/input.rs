use std::ops::{BitOr, BitOrAssign};

/// A physical key, named after the US layout position it sits at (W3C
/// `KeyboardEvent.code`), so `Key::A` is the key left of S on any layout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Key {
    /// A key with no physical code, such as text from an input method.
    #[default]
    Unidentified = 0,
    Backquote = 1,
    Backslash = 2,
    BracketLeft = 3,
    BracketRight = 4,
    Comma = 5,
    Digit0 = 6,
    Digit1 = 7,
    Digit2 = 8,
    Digit3 = 9,
    Digit4 = 10,
    Digit5 = 11,
    Digit6 = 12,
    Digit7 = 13,
    Digit8 = 14,
    Digit9 = 15,
    Equal = 16,
    A = 20,
    B = 21,
    C = 22,
    D = 23,
    E = 24,
    F = 25,
    G = 26,
    H = 27,
    I = 28,
    J = 29,
    K = 30,
    L = 31,
    M = 32,
    N = 33,
    O = 34,
    P = 35,
    Q = 36,
    R = 37,
    S = 38,
    T = 39,
    U = 40,
    V = 41,
    W = 42,
    X = 43,
    Y = 44,
    Z = 45,
    Minus = 46,
    Period = 47,
    Quote = 48,
    Semicolon = 49,
    Slash = 50,
    Backspace = 53,
    Enter = 58,
    Space = 63,
    Tab = 64,
    Delete = 68,
    End = 69,
    Home = 71,
    Insert = 72,
    PageDown = 73,
    PageUp = 74,
    ArrowDown = 75,
    ArrowLeft = 76,
    ArrowRight = 77,
    ArrowUp = 78,
    Escape = 120,
    F1 = 121,
    F2 = 122,
    F3 = 123,
    F4 = 124,
    F5 = 125,
    F6 = 126,
    F7 = 127,
    F8 = 128,
    F9 = 129,
    F10 = 130,
    F11 = 131,
    F12 = 132,
}

/// Modifier keys held during a key or mouse event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Mods(u16);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const SHIFT: Mods = Mods(1 << 0);
    pub const CTRL: Mods = Mods(1 << 1);
    /// Alt, or Option on a Mac.
    pub const ALT: Mods = Mods(1 << 2);
    /// Command on a Mac.
    pub const SUPER: Mods = Mods(1 << 3);
    pub const CAPS_LOCK: Mods = Mods(1 << 4);
    pub const NUM_LOCK: Mods = Mods(1 << 5);

    /// Whether every modifier in `other` is held.
    pub fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    /// The bits libghostty-vt's `GhosttyMods` uses: Shift 1, Ctrl 2, Alt 4,
    /// Super 8, Caps Lock 16, Num Lock 32.
    pub fn bits(self) -> u16 {
        self.0
    }
}

impl BitOr for Mods {
    type Output = Mods;

    fn bitor(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }
}

impl BitOrAssign for Mods {
    fn bitor_assign(&mut self, other: Mods) {
        self.0 |= other.0;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum KeyAction {
    Release = 0,
    #[default]
    Press = 1,
    Repeat = 2,
}

/// Whether Option on a Mac acts as Alt (sending ESC before the key) or types
/// the character macOS gives it, as Ghostty's `macos-option-as-alt` setting.
/// Other systems ignore it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum OptionAsAlt {
    #[default]
    Off = 0,
    On = 1,
    LeftOnly = 2,
    RightOnly = 3,
}

/// A key event, as the window system reported it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyEvent<'a> {
    pub key: Key,
    pub action: KeyAction,
    pub mods: Mods,
    /// Modifiers the layout used to produce `text`, such as Shift for `"A"`.
    pub consumed_mods: Mods,
    /// The text the key types, after the layout and modifiers; empty for
    /// keys that type nothing. Control characters are ignored here, since
    /// the encoder works them out from the key and modifiers.
    pub text: &'a str,
    /// The character the key types with no modifiers held.
    pub unshifted: Option<char>,
    /// Whether an input method is composing, so the key belongs to it.
    pub composing: bool,
}

impl<'a> KeyEvent<'a> {
    /// A press of `key` with no modifiers and no text.
    pub fn press(key: Key) -> Self {
        KeyEvent {
            key,
            ..KeyEvent::default()
        }
    }

    pub fn with_mods(self, mods: Mods) -> Self {
        KeyEvent { mods, ..self }
    }

    /// Sets the text the key types and, if it is one character, the
    /// unshifted character too.
    pub fn with_text(self, text: &'a str) -> Self {
        let mut chars = text.chars();
        let unshifted = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c.to_ascii_lowercase()),
            _ => self.unshifted,
        };
        KeyEvent {
            text,
            unshifted,
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum MouseAction {
    #[default]
    Press = 0,
    Release = 1,
    Motion = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MouseButton {
    Left = 1,
    Right = 2,
    Middle = 3,
    /// Usually the scroll wheel up.
    Four = 4,
    /// Usually the scroll wheel down.
    Five = 5,
    Six = 6,
    Seven = 7,
    Eight = 8,
    Nine = 9,
    Ten = 10,
    Eleven = 11,
}

/// A mouse event over the terminal.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MouseEvent {
    pub action: MouseAction,
    /// The button pressed or released, or `None` for motion with no button.
    pub button: Option<MouseButton>,
    pub mods: Mods,
    /// Where the pointer is, in pixels from the grid's top-left corner.
    pub x: f32,
    pub y: f32,
}
