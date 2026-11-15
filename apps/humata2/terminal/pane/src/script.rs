//! Deterministic terminal output for tests and benchmarks: a [`Script`] is
//! the bytes a program would write, cut into the chunks a pty would hand
//! over, with the same chunks every run.

/// Bytes a program writes, in the chunks they arrive in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script {
    chunks: Vec<Vec<u8>>,
}

impl Script {
    pub fn new() -> Self {
        Script::default()
    }

    /// Adds `text` as one chunk. `\n` is sent as `\r\n`, as a pty in its
    /// usual mode turns a program's new line into both.
    pub fn text(self, text: &str) -> Self {
        self.bytes(text.replace('\n', "\r\n").as_bytes())
    }

    /// Adds `bytes` as one chunk, exactly as given.
    pub fn bytes(mut self, bytes: &[u8]) -> Self {
        if !bytes.is_empty() {
            self.chunks.push(bytes.to_vec());
        }
        self
    }

    /// The same bytes cut into chunks of at most `size` bytes, so escape
    /// sequences and UTF-8 characters arrive split, as they do from a
    /// real pty under load. A `size` of 0 is taken as 1.
    pub fn split_every(self, size: usize) -> Self {
        let size = size.max(1);
        let all = self.into_bytes();
        Script {
            chunks: all.chunks(size).map(<[u8]>::to_vec).collect(),
        }
    }

    pub fn chunks(&self) -> impl Iterator<Item = &[u8]> {
        self.chunks.iter().map(Vec::as_slice)
    }

    pub fn len(&self) -> usize {
        self.chunks.iter().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.chunks.concat()
    }

    /// A shell prompt, a command and its output, and the next prompt.
    pub fn shell_session() -> Self {
        Script::new()
            .text("\x1b]2;zsh\x07")
            .text("\x1b[1;32mme@mac\x1b[0m:\x1b[1;34m~/repo\x1b[0m$ ")
            .text("ls -la\n")
            .text("total 24\n")
            .text("drwxr-xr-x  5 me staff  160 Nov 14 09:00 \x1b[1;34m.\x1b[0m\n")
            .text("-rw-r--r--  1 me staff  812 Nov 14 09:00 Cargo.toml\n")
            .text("-rw-r--r--  1 me staff 1042 Nov 14 09:00 README.md\n")
            .text("\x1b[1;32mme@mac\x1b[0m:\x1b[1;34m~/repo\x1b[0m$ ")
    }

    /// Output that is hard to draw right: wide characters, emoji, combining
    /// marks, box drawing, true color, underline styles and a bar cursor.
    pub fn tricky_rendering() -> Self {
        Script::new()
            .text("wide: 日本語の文字 | 한국어\n")
            .text("emoji: 👍 🧑‍💻 🇮🇷 ❤️\n")
            .text("combining: e\u{301} a\u{308} n\u{303}\n")
            .text("box: ┌──┬──┐\n     │  │  │\n     └──┴──┘\n")
            .text("\x1b[38;2;255;100;0mtrue color\x1b[0m \x1b[48;2;0;80;160mon blue\x1b[0m\n")
            .text("\x1b[4:3mcurly\x1b[0m \x1b[9mstrike\x1b[0m \x1b[1;3mbold italic\x1b[0m\n")
            .text("\x1b[5 q")
    }

    /// An agent streaming `lines` numbered lines of output, as an agent CLI
    /// writing a long answer does.
    pub fn agent_stream(lines: usize) -> Self {
        let mut script = Script::new();
        for n in 0..lines {
            script = script.text(&format!(
                "\x1b[2m{n:>5}\x1b[0m streaming \x1b[33manswer\x1b[0m text for line {n}\n"
            ));
        }
        script
    }

    /// A full-screen program: the alternate screen, the cursor hidden, a
    /// box drawn with cursor moves, then back to the normal screen.
    pub fn full_screen_app() -> Self {
        Script::new()
            .bytes(b"\x1b[?1049h\x1b[?25l\x1b[H\x1b[2J")
            .bytes("\x1b[1;1H┌────────┐\x1b[2;1H│ editor │\x1b[3;1H└────────┘".as_bytes())
            .bytes(b"\x1b[?1049l\x1b[?25h")
    }
}
