//! Drops the replies a terminal sends when a program asks it something: a
//! cursor position, device attributes, a colour. `humata attach` reads the
//! user's terminal in raw mode, so those replies arrive on its stdin looking
//! like keystrokes. The daemon has already answered the same questions, so
//! they must not reach the remote shell as typed text. Every other byte passes
//! through unchanged, including keystrokes and bracketed paste content.

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// The most bytes held back while a sequence is still undecided. Longer
/// input is typed as it is.
const MAX_HELD: usize = 256;

/// Bracketed paste markers: `ESC [ 200 ~` and `ESC [ 201 ~`.
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// What follows `ESC P` in the XTVERSION reply: `ESC P > | name ESC \`.
const XTVERSION_PREFIX: &[u8] = b">|";

/// Removes terminal replies from the bytes a person types, across reads.
#[derive(Debug, Default)]
pub(super) struct ReplyFilter {
    /// The start of a sequence not yet known to be a reply or a keystroke.
    held: Vec<u8>,
    /// Inside bracketed paste, where every byte passes unchanged.
    in_paste: bool,
    /// How much of `PASTE_END` the paste content has matched so far.
    paste_end_matched: usize,
}

impl ReplyFilter {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// The bytes to type for `bytes`, the next read. A sequence that could
    /// still turn out to be a reply is held back until the next read or
    /// until `flush`.
    pub(super) fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.run(bytes, false)
    }

    /// Whether bytes are held back, so the caller knows to call `flush` if
    /// no more input comes soon.
    pub(super) fn holding(&self) -> bool {
        !self.held.is_empty()
    }

    /// The held bytes, typed as they are. Called when input has gone idle:
    /// they are keystrokes after all (a lone Esc key is one).
    pub(super) fn flush(&mut self) -> Vec<u8> {
        self.run(&[], true)
    }

    fn run(&mut self, bytes: &[u8], last: bool) -> Vec<u8> {
        let mut input = std::mem::take(&mut self.held);
        input.extend_from_slice(bytes);
        let mut out = Vec::with_capacity(input.len());
        let mut at = 0;
        while at < input.len() {
            if self.in_paste {
                let byte = input[at];
                out.push(byte);
                self.step_paste_end(byte);
                at += 1;
                continue;
            }
            match classify(&input[at..]) {
                Verdict::Drop(len) => at += len,
                Verdict::Keep(len) => {
                    let sequence = &input[at..at + len];
                    if sequence == PASTE_START {
                        self.in_paste = true;
                        self.paste_end_matched = 0;
                    }
                    out.extend_from_slice(sequence);
                    at += len;
                }
                Verdict::Pending if !last => {
                    self.held = input[at..].to_vec();
                    break;
                }
                Verdict::Pending | Verdict::Plain => {
                    out.push(input[at]);
                    at += 1;
                }
            }
        }
        out
    }

    /// Follows the paste end marker through the paste content, byte by byte.
    fn step_paste_end(&mut self, byte: u8) {
        self.paste_end_matched = if PASTE_END.get(self.paste_end_matched) == Some(&byte) {
            self.paste_end_matched + 1
        } else {
            usize::from(byte == ESC)
        };
        if self.paste_end_matched == PASTE_END.len() {
            self.in_paste = false;
            self.paste_end_matched = 0;
        }
    }
}

/// What the bytes at the start of the input are.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// A reply: these bytes are dropped.
    Drop(usize),
    /// A complete sequence that is not a reply: forwarded whole.
    Keep(usize),
    /// Could still become a reply or a keystroke with more bytes.
    Pending,
    /// Not the start of any reply: the first byte is forwarded on its own.
    Plain,
}

fn classify(bytes: &[u8]) -> Verdict {
    if bytes.first() != Some(&ESC) {
        return Verdict::Plain;
    }
    match bytes.get(1).copied() {
        None => pending(bytes),
        Some(b'[') => csi(bytes),
        Some(b']') => osc(bytes),
        Some(b'P') => dcs(bytes),
        Some(_) => Verdict::Plain,
    }
}

/// Undecided, unless the bytes are already too long to be a reply.
fn pending(bytes: &[u8]) -> Verdict {
    if bytes.len() < MAX_HELD {
        Verdict::Pending
    } else {
        Verdict::Plain
    }
}

/// `ESC [`, then parameter bytes, intermediate bytes and a final byte.
fn csi(bytes: &[u8]) -> Verdict {
    let mut end = 2;
    while bytes.get(end).is_some_and(|byte| (0x30..=0x3f).contains(byte)) {
        end += 1;
    }
    while bytes.get(end).is_some_and(|byte| (0x20..=0x2f).contains(byte)) {
        end += 1;
    }
    match bytes.get(end).copied() {
        None => pending(bytes),
        Some(0x40..=0x7e) => {
            let len = end + 1;
            if bytes.get(2..len).is_some_and(is_csi_reply) {
                Verdict::Drop(len)
            } else {
                Verdict::Keep(len)
            }
        }
        Some(_) => Verdict::Plain,
    }
}

/// Whether the bytes after `ESC [` (parameters and final byte) are a reply.
fn is_csi_reply(seq: &[u8]) -> bool {
    let Some((&last, body)) = seq.split_last() else {
        return false;
    };
    match last {
        // Device attributes, primary (`?`) and secondary (`>`).
        b'c' => body
            .strip_prefix(b"?")
            .or_else(|| body.strip_prefix(b">"))
            .is_some_and(numbers),
        // Cursor position report. Shift+F3 has the same shape, `ESC [ 1 ; 2 R`,
        // so that one is a keystroke.
        b'R' => pair(body).is_some_and(|(row, col)| !(row == b"1" && matches!(col, [b'2'..=b'8']))),
        // Device status report: `ESC [ 0 n`.
        b'n' => body == b"0",
        // Mode report: `ESC [ ? mode ; value $ y`.
        b'y' => body.strip_suffix(b"$").is_some_and(|mode| {
            let mode = mode.strip_prefix(b"?").unwrap_or(mode);
            pair(mode).is_some()
        }),
        // Kitty keyboard flags: `ESC [ ? flags u`.
        b'u' => body.strip_prefix(b"?").is_some_and(digits),
        _ => false,
    }
}

/// `ESC ] text BEL` or `ESC ] text ESC \`: a colour or palette reply when
/// the text is `number ; ...`.
fn osc(bytes: &[u8]) -> Verdict {
    match read_text(bytes, 2) {
        Text::Open => pending(bytes),
        Text::Invalid => Verdict::Plain,
        Text::Closed { text_end, len } => {
            if bytes.get(2..text_end).is_some_and(is_osc_reply) {
                Verdict::Drop(len)
            } else {
                Verdict::Keep(len)
            }
        }
    }
}

fn is_osc_reply(text: &[u8]) -> bool {
    let mut fields = text.split(|byte| *byte == b';');
    fields.next().is_some_and(digits) && fields.next().is_some()
}

/// `ESC P > |` then a name and `ESC \`: the XTVERSION reply. Any other
/// `ESC P` is typed as it is.
fn dcs(bytes: &[u8]) -> Verdict {
    let body = bytes.get(2..).unwrap_or_default();
    if body.len() < XTVERSION_PREFIX.len() {
        return if XTVERSION_PREFIX.starts_with(body) {
            pending(bytes)
        } else {
            Verdict::Plain
        };
    }
    if !body.starts_with(XTVERSION_PREFIX) {
        return Verdict::Plain;
    }
    match read_text(bytes, 2 + XTVERSION_PREFIX.len()) {
        Text::Open => pending(bytes),
        Text::Invalid => Verdict::Plain,
        Text::Closed { len, .. } => Verdict::Drop(len),
    }
}

/// The result of reading printable text up to a string terminator.
enum Text {
    /// Ran out of bytes before the terminator.
    Open,
    /// The text ends at `text_end`; the whole sequence is `len` bytes.
    Closed { text_end: usize, len: usize },
    /// A byte that cannot be in the text or its terminator.
    Invalid,
}

/// Reads printable text from `from` up to a BEL or an `ESC \`.
fn read_text(bytes: &[u8], from: usize) -> Text {
    let mut at = from;
    loop {
        match bytes.get(at).copied() {
            None => return Text::Open,
            Some(BEL) => return Text::Closed { text_end: at, len: at + 1 },
            Some(ESC) => {
                return match bytes.get(at + 1).copied() {
                    None => Text::Open,
                    Some(b'\\') => Text::Closed { text_end: at, len: at + 2 },
                    Some(_) => Text::Invalid,
                };
            }
            Some(0x20..=0x7e) => at += 1,
            Some(_) => return Text::Invalid,
        }
    }
}

fn digits(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)
}

/// `;`-separated fields, each of digits.
fn numbers(bytes: &[u8]) -> bool {
    bytes.split(|byte| *byte == b';').all(digits)
}

/// Exactly two `;`-separated fields, each of digits.
fn pair(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut fields = bytes.split(|byte| *byte == b';');
    let first = fields.next()?;
    let second = fields.next()?;
    let no_more = fields.next().is_none();
    (no_more && digits(first) && digits(second)).then_some((first, second))
}

#[cfg(test)]
mod tests {
    use super::ReplyFilter;

    /// What is typed for these reads, then for the idle flush.
    fn typed(reads: &[&[u8]]) -> Vec<u8> {
        let mut filter = ReplyFilter::new();
        let mut out = Vec::new();
        for read in reads {
            out.extend(filter.push(read));
        }
        out.extend(filter.flush());
        out
    }

    /// Replies a terminal sends to the queries a program makes.
    const REPLIES: &[&[u8]] = &[
        b"\x1b[12;40R",
        b"\x1b[?62;22c",
        b"\x1b[>0;276;0c",
        b"\x1b[0n",
        b"\x1b[?2026;2$y",
        b"\x1b[?0u",
        b"\x1bP>|ghostty 1.2\x1b\\",
        b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\",
        b"\x1b]11;rgb:1e1e/1e1e/1e1e\x07",
        b"\x1b]4;1;rgb:ffff/0000/0000\x1b\\",
    ];

    /// Keystrokes a person types, which reach the remote shell as they are.
    const KEYSTROKES: &[&[u8]] = &[
        b"\x1b[A",
        b"\x1b[15~",
        b"\x1bOP",
        b"\x1bx",
        b"\x1bP",
        b"\x1b[1;2R",
        b"\x1b[97u",
        b"\x1b[200~",
        b"\x1d",
        b"ls -la\r",
        b"\x1b",
    ];

    #[test]
    fn every_reply_kind_is_dropped() {
        for &reply in REPLIES {
            assert_eq!(typed(&[reply]), Vec::<u8>::new(), "reply {reply:?}");
        }
    }

    #[test]
    fn every_keystroke_kind_passes_unchanged() {
        for &keys in KEYSTROKES {
            assert_eq!(typed(&[keys]), keys.to_vec(), "keystroke {keys:?}");
        }
    }

    #[test]
    fn a_reply_split_at_any_byte_is_dropped() {
        for &reply in REPLIES {
            for at in 1..reply.len() {
                let (head, tail) = reply.split_at(at);
                assert_eq!(typed(&[head, tail]), Vec::<u8>::new(), "{reply:?} split at {at}");
            }
        }
    }

    #[test]
    fn a_reply_between_two_keystrokes_leaves_both() {
        let reads: &[&[u8]] = &[b"\x1b[A", b"\x1b[?62c", b"\x1b[B"];
        assert_eq!(typed(reads), b"\x1b[A\x1b[B".to_vec());
    }

    #[test]
    fn a_lone_esc_is_typed_when_nothing_follows() {
        let mut filter = ReplyFilter::new();
        assert!(filter.push(b"\x1b").is_empty());
        assert!(filter.holding());
        assert_eq!(filter.flush(), b"\x1b".to_vec());
    }

    #[test]
    fn an_esc_then_an_arrow_across_reads_is_the_arrow() {
        assert_eq!(typed(&[b"\x1b", b"[A"]), b"\x1b[A".to_vec());
    }

    #[test]
    fn an_esc_before_a_reply_is_typed_and_the_reply_dropped() {
        assert_eq!(typed(&[b"\x1b\x1b[?62c"]), b"\x1b".to_vec());
    }

    #[test]
    fn an_incomplete_reply_is_typed_on_flush() {
        let mut filter = ReplyFilter::new();
        assert!(filter.push(b"\x1b[?6").is_empty());
        assert_eq!(filter.flush(), b"\x1b[?6".to_vec());
    }

    #[test]
    fn bracketed_paste_content_that_looks_like_a_reply_passes() {
        let paste: &[u8] = b"\x1b[200~\x1b[?62c\x1b[12;40R\x1b[201~";
        assert_eq!(typed(&[paste]), paste.to_vec());
    }

    #[test]
    fn a_reply_after_bracketed_paste_is_dropped() {
        assert_eq!(
            typed(&[b"\x1b[200~ok\x1b[201~\x1b[?62c"]),
            b"\x1b[200~ok\x1b[201~".to_vec()
        );
    }

    #[test]
    fn a_stream_gives_the_same_bytes_however_it_is_split() {
        let stream: &[u8] =
            b"ls\x1b[A\x1b[?62;22c\x1b[200~\x1b[0n\x1b[201~\x1b]11;rgb:0/0/0\x07\x1bx\x1b";
        let expected = b"ls\x1b[A\x1b[200~\x1b[0n\x1b[201~\x1bx\x1b".to_vec();
        assert_eq!(typed(&[stream]), expected);
        for at in 0..=stream.len() {
            let (head, tail) = stream.split_at(at);
            assert_eq!(typed(&[head, tail]), expected, "split at {at}");
        }
    }

    #[test]
    fn a_sequence_that_never_ends_is_typed_as_it_is() {
        let mut long = b"\x1b[".to_vec();
        long.extend(vec![b'1'; 400]);
        assert_eq!(typed(&[long.as_slice()]), long);
    }
}
