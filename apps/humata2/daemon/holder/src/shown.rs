//! Whether a program's output shows some typed text, as a person reading
//! its screen would see it: escape sequences, control characters and
//! white space don't count, so text a program redraws with colours or
//! wraps over lines still matches.

/// The most characters of the typed text looked for: its end, which a
/// prompt shows once all of it was taken.
const TAIL: usize = 24;

/// What agent CLIs show a long paste as in place of its text: Copilot's
/// `[Paste #1 - 31 lines]`, Claude Code's `[Pasted text #1 +30 lines]`.
const PASTE_SUMMARY: &[u8] = b"[Paste";

/// Looks for typed text in output fed to it a piece at a time: its end, or
/// the summary a CLI shows a long paste as instead.
pub(crate) struct Shown {
    text: Seen,
    summary: Seen,
}

impl Shown {
    /// `None` when `text` has nothing visible to look for.
    pub(crate) fn new(text: &[u8]) -> Option<Self> {
        Some(Self {
            text: Seen::new(text)?,
            summary: Seen::new(PASTE_SUMMARY)?,
        })
    }

    /// Takes the next piece of output. Returns whether the text was shown.
    pub(crate) fn push(&mut self, output: &[u8]) -> bool {
        // Both see every piece, so neither loses its place.
        let text = self.text.push(output);
        let summary = self.summary.push(output);
        text || summary
    }
}

/// Looks for the end of some text in output fed to it a piece at a time.
pub(crate) struct Seen {
    needle: Vec<u8>,
    /// The visible bytes since the last miss, at most a needle long.
    window: Vec<u8>,
    escape: Escape,
}

#[derive(Clone, Copy)]
enum Escape {
    None,
    /// After ESC.
    Start,
    /// ESC and an intermediate byte, as in a character set choice.
    Intermediate,
    /// In a control sequence (ESC `[`).
    Control,
    /// In a string (OSC, DCS, APC, PM, SOS), ended by BEL or ST.
    Text,
    /// ESC inside a string: ST if `\` comes next.
    TextEscape,
}

impl Seen {
    /// `None` when `text` has nothing visible to look for.
    pub(crate) fn new(text: &[u8]) -> Option<Self> {
        let mut visible = Vec::new();
        Escape::None.visible(text, &mut visible);
        let mut start = visible.len().saturating_sub(TAIL * 4);
        // Counted in characters, cut on a character's first byte.
        let chars = visible[start..].iter().filter(|byte| !is_continuation(**byte)).count();
        for _ in TAIL..chars {
            start += 1;
            while visible.get(start).is_some_and(|byte| is_continuation(*byte)) {
                start += 1;
            }
        }
        let needle = visible.split_off(start);
        (!needle.is_empty()).then(|| Self {
            needle,
            window: Vec::new(),
            escape: Escape::None,
        })
    }

    /// Takes the next piece of output. Returns whether the text was seen.
    pub(crate) fn push(&mut self, output: &[u8]) -> bool {
        self.escape = self.escape.visible(output, &mut self.window);
        if self
            .window
            .windows(self.needle.len())
            .any(|seen| seen == self.needle)
        {
            return true;
        }
        let keep = self.needle.len().saturating_sub(1);
        let drop = self.window.len().saturating_sub(keep);
        self.window.drain(..drop);
        false
    }
}

fn is_continuation(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}

impl Escape {
    /// Appends what's visible in `bytes` to `out`, starting in `self`.
    /// Returns the state at the end.
    fn visible(self, bytes: &[u8], out: &mut Vec<u8>) -> Self {
        let mut state = self;
        for &byte in bytes {
            state = match (state, byte) {
                (Self::None, 0x1b) => Self::Start,
                (Self::None, byte) if byte <= b' ' || byte == 0x7f => Self::None,
                (Self::None, byte) => {
                    out.push(byte);
                    Self::None
                }
                (Self::Start, b'[') => Self::Control,
                (Self::Start, b']' | b'P' | b'_' | b'^' | b'X') => Self::Text,
                (Self::Start | Self::Intermediate, 0x20..=0x2f) => Self::Intermediate,
                (Self::Start | Self::Intermediate, _) => Self::None,
                (Self::Control, 0x1b) => Self::Start,
                (Self::Control, 0x40..=0x7e) => Self::None,
                (Self::Control, _) => Self::Control,
                (Self::Text, 0x07) => Self::None,
                (Self::Text, 0x1b) => Self::TextEscape,
                (Self::Text, _) => Self::Text,
                (Self::TextEscape, b'\\') => Self::None,
                (Self::TextEscape, _) => Self::Text,
            };
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::{Seen, Shown};

    #[test]
    fn a_long_paste_shown_as_a_summary_counts_as_shown() {
        let mut shown = Shown::new(b"one\ntwo\nthree").expect("visible text");
        assert!(shown.push(b"\x1b[2K\r> [Paste #1 - 3 lines]"));
        let mut shown = Shown::new(b"one\ntwo\nthree").expect("visible text");
        assert!(shown.push(b"> [Pasted text #1 +2 lines]"));
    }

    #[test]
    fn a_question_of_the_programs_own_is_not_the_text_shown() {
        let mut shown = Shown::new(b"create hello.txt").expect("visible text");
        assert!(!shown.push(b"Confirm folder trust\r\n> 1. Yes, proceed\r\n  2. No, exit"));
    }

    fn seen_in(text: &str, pieces: &[&[u8]]) -> bool {
        let mut seen = Seen::new(text.as_bytes()).expect("visible text");
        pieces.iter().any(|piece| seen.push(piece))
    }

    #[test]
    fn text_drawn_with_colours_and_cursor_moves_is_seen() {
        assert!(seen_in(
            "say hi",
            &[b"\x1b[2K\r\x1b[1;34m> \x1b[0msay \x1b[7mh\x1b[27mi\x1b[?25h"]
        ));
    }

    #[test]
    fn text_split_across_pieces_and_escapes_is_seen() {
        assert!(seen_in("hello", &[b"> he", b"\x1b[3", b"8;5;2m", b"llo"]));
    }

    #[test]
    fn text_wrapped_over_lines_is_seen() {
        assert!(seen_in("a long line", &[b"> a long\r\n  line"]));
    }

    #[test]
    fn titles_and_other_strings_dont_count() {
        assert!(!seen_in("hello", &[b"\x1b]0;hello\x07\x1b]8;;hello\x1b\\", b"> hel"]));
    }

    #[test]
    fn other_text_is_not_seen() {
        assert!(!seen_in("hello", &[b"> help", b"lo"]));
    }

    #[test]
    fn only_the_end_of_long_text_is_looked_for() {
        let text = format!("{}the end of what was typed", "x".repeat(500));
        assert!(seen_in(&text, &["\u{2026}xxxxxx the end of what was typed".as_bytes()]));
    }

    #[test]
    fn the_end_is_cut_on_a_character() {
        let text = "é".repeat(40);
        assert!(seen_in(&text, &[text.as_bytes()]));
        assert!(Seen::new(text.as_bytes()).is_some_and(|seen| seen.needle.len() == 48));
    }

    #[test]
    fn text_with_nothing_visible_is_not_looked_for() {
        assert!(Seen::new(b" \r\n\t").is_none());
    }
}
