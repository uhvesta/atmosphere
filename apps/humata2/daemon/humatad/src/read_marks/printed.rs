//! What a session's agent printed lately, as words, so a redraw can be told
//! from new output (CH-16; D393).
//!
//! A program that gets SIGWINCH repaints what was on its screen. Stripped of
//! escape codes and cut into words, a repaint says nothing the session
//! hadn't printed before; new output does. This keeps the last
//! [`WINDOW`] bytes of words, and [`Printed::hear`] says whether some output
//! brought a word that wasn't among them.

use std::collections::{HashMap, VecDeque};

/// How many bytes of words are kept.
const WINDOW: usize = 64 << 10;

/// The longest word kept whole; a longer one is cut, the same way each time.
const WORD: usize = 1024;

/// The words of a session's recent output.
#[derive(Default)]
pub(crate) struct Printed {
    /// Oldest first, for evicting.
    order: VecDeque<Box<str>>,
    /// How many times each word is in `order`.
    counts: HashMap<Box<str>, usize>,
    /// Bytes of the words in `order`.
    bytes: usize,
    /// Where the escape stripper is, across chunks.
    scan: Scan,
    /// The word cut by the end of the last chunk, not yet known whole.
    word: Vec<u8>,
}

/// The stripper's position in an escape sequence.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Scan {
    #[default]
    Text,
    /// After ESC.
    Escape,
    /// In a CSI sequence, up to its final byte.
    Csi,
    /// In a string sequence (OSC, DCS, SOS, PM, APC), up to BEL or ST.
    Str,
    /// In a string sequence, after an ESC that may start its ST.
    StrEscape,
    /// After ESC and an intermediate byte, up to the final one.
    Intermediate,
}

impl Printed {
    /// Takes in `data`, output of the agent, and says whether it had a
    /// word not among those printed before it. A word cut by the end of
    /// `data` is judged with the next output.
    pub(crate) fn hear(&mut self, data: &[u8]) -> bool {
        let mut new = false;
        for &byte in data {
            match self.scan {
                Scan::Text => match byte {
                    0x1b => {
                        self.scan = Scan::Escape;
                        new |= self.end_word();
                    }
                    0x00..=0x20 | 0x7f => new |= self.end_word(),
                    _ => {
                        if self.word.len() < WORD {
                            self.word.push(byte);
                        }
                    }
                },
                Scan::Escape => {
                    self.scan = match byte {
                        b'[' => Scan::Csi,
                        b']' | b'P' | b'X' | b'^' | b'_' => Scan::Str,
                        0x20..=0x2f => Scan::Intermediate,
                        // An ESC inside an ESC starts over.
                        0x1b => Scan::Escape,
                        _ => Scan::Text,
                    };
                }
                Scan::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        self.scan = Scan::Text;
                    } else if byte == 0x1b {
                        self.scan = Scan::Escape;
                    }
                }
                Scan::Str => match byte {
                    0x07 => self.scan = Scan::Text,
                    0x1b => self.scan = Scan::StrEscape,
                    _ => {}
                },
                Scan::StrEscape => {
                    self.scan = match byte {
                        b'\\' => Scan::Text,
                        0x1b => Scan::StrEscape,
                        _ => Scan::Str,
                    };
                }
                Scan::Intermediate => {
                    if !(0x20..=0x2f).contains(&byte) {
                        self.scan = Scan::Text;
                    }
                }
            }
        }
        new
    }

    /// The word being read is whole: keeps it, and says whether it was new.
    /// A word with no letter or digit (box drawing, a spinner's glyph) is
    /// nothing.
    fn end_word(&mut self) -> bool {
        if self.word.is_empty() {
            return false;
        }
        let word = String::from_utf8_lossy(&self.word).into_owned();
        self.word.clear();
        if !word.chars().any(char::is_alphanumeric) {
            return false;
        }
        let word: Box<str> = word.into();
        let new = !self.counts.contains_key(&word);
        self.bytes += word.len();
        *self.counts.entry(word.clone()).or_default() += 1;
        self.order.push_back(word);
        while self.bytes > WINDOW
            && let Some(old) = self.order.pop_front()
        {
            self.bytes -= old.len();
            if let Some(count) = self.counts.get_mut(&old) {
                *count -= 1;
                if *count == 0 {
                    self.counts.remove(&old);
                }
            }
        }
        new
    }

    #[cfg(test)]
    fn kept_bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_word_is_new_and_the_same_word_again_is_not() {
        let mut printed = Printed::default();
        assert!(printed.hear(b"hello world\n"));
        assert!(!printed.hear(b"world hello\n"));
        assert!(printed.hear(b"hello there\n"));
    }

    #[test]
    fn escape_codes_are_not_words_and_do_not_split_the_text_around_them() {
        let mut printed = Printed::default();
        assert!(printed.hear(b"\x1b[1;32mready\x1b[0m \x1b]0;a title\x07done\n"));
        // The same text with other codes, and a title in the other end.
        assert!(!printed.hear(b"\x1b[2J\x1b[Hready \x1b[38;5;9mdone\x1b]0;a title\x1b\\\n"));
        // A cursor move and a charset choice are no text either.
        assert!(!printed.hear(b"\x1b(Bready\x1b7 done\x1b8\n"));
    }

    #[test]
    fn a_redraw_of_the_same_words_wrapped_differently_is_not_new() {
        let mut printed = Printed::default();
        assert!(printed.hear(b"the quick brown fox jumps over the lazy dog\r\n"));
        assert!(!printed.hear(b"the quick brown\r\nfox jumps over\r\nthe lazy dog\r\n"));
    }

    #[test]
    fn glyphs_without_letters_or_digits_are_not_words() {
        let mut printed = Printed::default();
        assert!(!printed.hear("\u{250c}\u{2500}\u{2500}\u{2510} \u{280b} \u{2502}\n".as_bytes()));
        assert!(printed.hear("\u{2502} saved \u{2502}\n".as_bytes()));
    }

    #[test]
    fn an_escape_or_word_cut_between_outputs_is_read_whole() {
        let mut printed = Printed::default();
        assert!(printed.hear(b"alphabet \n"));
        assert!(!printed.hear(b"\x1b[3"));
        assert!(!printed.hear(b"1malpha"));
        assert!(!printed.hear(b"bet\n"));
    }

    #[test]
    fn multibyte_words_cut_between_outputs_are_read_whole() {
        let mut printed = Printed::default();
        let word = "caf\u{e9}".as_bytes();
        assert!(printed.hear(&[word, b"\n"].concat()));
        assert!(!printed.hear(&word[..4]));
        assert!(!printed.hear(&[&word[4..], b"\n"].concat()));
    }

    #[test]
    fn a_word_that_fell_out_of_the_window_is_new_again_and_the_window_is_bounded() {
        let mut printed = Printed::default();
        assert!(printed.hear(b"first\n"));
        let filler: String = (0..20_000).map(|n| format!("w{n} ")).collect();
        printed.hear(filler.as_bytes());
        printed.hear(b"\n");
        assert!(printed.kept_bytes() <= WINDOW);
        assert!(printed.hear(b"first\n"));
    }

    #[test]
    fn a_huge_word_is_cut_and_still_matches_itself() {
        let mut printed = Printed::default();
        let huge = "x".repeat(10 * WORD);
        assert!(printed.hear(format!("{huge}\n").as_bytes()));
        assert!(!printed.hear(format!("{huge}\n").as_bytes()));
    }
}
