//! Taking over a terminal the person logged in through (MP-14). Once they
//! say they're in, Humata types into their shell: a line that reports the
//! platform and what is installed, the install of each file that differs
//! (sent with the far terminal in raw mode), and finally the bridge, whose
//! sync marker means the terminal now carries the gRPC stream.
//!
//! It does no I/O: the daemon feeds it what the terminal printed and does
//! what it answers.

use std::collections::BTreeMap;

use crate::error::TakeoverError;
use crate::install::{self, Builds, InstallProblem, InstalledFile, Payload};
use crate::link::BIN_DIR;
use crate::platform::{Arch, Os, Platform};
use crate::quote::sh_command;
use crate::runner::Exit;

/// What the daemon does next with the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeoverAction {
    /// Type these bytes into the terminal.
    Write(Vec<u8>),
    /// Put this end of the terminal in raw mode, so the bytes that follow
    /// reach the machine unchanged.
    MakeLocalRaw,
    /// Write the payload's bytes, exactly, and nothing else until the next
    /// answer.
    SendFile(Payload),
    /// The bridge is running: from here the terminal carries the gRPC
    /// stream, starting with `leftover`.
    Ready {
        platform: Platform,
        leftover: Vec<u8>,
    },
    Failed(TakeoverError),
}

/// One takeover of one terminal.
#[derive(Debug)]
pub struct Takeover {
    nonce: String,
    state: State,
    /// What arrived since the last whole line (or, at the end, since the
    /// bridge started).
    buffer: Vec<u8>,
    platform: Option<Platform>,
    pending: Vec<Payload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// The person is still logging in; what the terminal prints is theirs.
    LoggingIn,
    Facts,
    /// Waiting for the far terminal to be raw before sending a file.
    Raw,
    /// Waiting for the install's exit status.
    Installed(InstalledFile),
    /// Waiting for the bridge's sync marker.
    Sync,
    Over,
}

impl Default for Takeover {
    fn default() -> Self {
        Self::new()
    }
}

impl Takeover {
    pub fn new() -> Self {
        Self {
            nonce: install::nonce(),
            state: State::LoggingIn,
            buffer: Vec::new(),
            platform: None,
            pending: Vec::new(),
        }
    }

    /// The person is logged in: start by asking for the facts. Ctrl-U
    /// clears anything half-typed first.
    pub fn logged_in(&mut self) -> Vec<TakeoverAction> {
        if self.state != State::LoggingIn {
            return Vec::new();
        }
        self.buffer.clear();
        self.state = State::Facts;
        match self.typed(&facts_script(&self.nonce)) {
            Ok(write) => vec![write],
            Err(error) => self.fail(error),
        }
    }

    /// Whether the takeover is finished, ready or failed.
    pub fn is_over(&self) -> bool {
        self.state == State::Over
    }

    /// Feeds what the terminal printed.
    pub fn output(&mut self, bytes: &[u8], builds: &dyn Builds) -> Vec<TakeoverAction> {
        match self.state {
            State::LoggingIn | State::Over => return Vec::new(),
            _ => {}
        }
        self.buffer.extend_from_slice(bytes);
        if self.state == State::Sync {
            return self.sync();
        }
        let Some(end) = self.buffer.iter().rposition(|&b| b == b'\n') else {
            return Vec::new();
        };
        let whole: Vec<u8> = self.buffer.drain(..=end).collect();
        let text = plain(&whole);
        let mut actions = Vec::new();
        for line in text.lines() {
            actions.extend(self.line(line, builds));
            if self.state == State::Sync {
                // The bridge's bytes follow; nothing after this is a line.
                actions.extend(self.sync());
                break;
            }
            if matches!(self.state, State::Over) || !actions.is_empty() {
                // An action was asked for; what follows on this read was
                // printed before it took effect.
                self.buffer.clear();
                break;
            }
        }
        actions
    }

    fn line(&mut self, line: &str, builds: &dyn Builds) -> Vec<TakeoverAction> {
        let n = &self.nonce;
        match self.state {
            State::Facts => {
                let Some(rest) = after(line, &format!("HUMATA_FACTS_{n} ")) else {
                    return Vec::new();
                };
                let words: Vec<&str> = rest.split_whitespace().collect();
                let platform = match words.as_slice() {
                    [os, arch, _, _] => Os::parse(os)
                        .zip(Arch::parse(arch))
                        .map(|(os, arch)| Platform { os, arch }),
                    _ => None,
                };
                let Some(platform) = platform else {
                    return self.fail(TakeoverError::BadAnswer {
                        line: line.to_owned(),
                    });
                };
                let installed: BTreeMap<InstalledFile, &str> =
                    InstalledFile::ALL.into_iter().zip(words[2..].iter().copied()).collect();
                let Some(payloads) = builds.payloads(platform) else {
                    return self.fail(TakeoverError::NoBuild { platform });
                };
                self.platform = Some(platform);
                self.pending = payloads
                    .into_iter()
                    .filter(|payload| installed.get(&payload.file()) != Some(&payload.sha256()))
                    .collect();
                self.next()
            }
            State::Raw => {
                if after(line, &format!("HUMATA_RAW_{n}")).is_none() {
                    return Vec::new();
                }
                let Some(payload) = self.pending.first().cloned() else {
                    return self.next();
                };
                self.state = State::Installed(payload.file());
                vec![TakeoverAction::MakeLocalRaw, TakeoverAction::SendFile(payload)]
            }
            State::Installed(file) => {
                let Some(rest) = after(line, &format!("HUMATA_DONE_{n} ")) else {
                    return Vec::new();
                };
                match rest.trim().parse::<i32>() {
                    Ok(0) => {
                        self.pending.retain(|payload| payload.file() != file);
                        self.next()
                    }
                    Ok(code) => self.fail(TakeoverError::Install {
                        file,
                        problem: InstallProblem::from_exit(Exit::Code(code)),
                    }),
                    Err(_) => self.fail(TakeoverError::BadAnswer {
                        line: line.to_owned(),
                    }),
                }
            }
            State::LoggingIn | State::Sync | State::Over => Vec::new(),
        }
    }

    /// Installs the next file, or starts the bridge when none is left.
    fn next(&mut self) -> Vec<TakeoverAction> {
        let n = self.nonce.clone();
        let (script, state) = match self.pending.first() {
            Some(payload) => (install_script(payload, &n), State::Raw),
            None => (
                format!(
                    "stty raw -echo -iexten 2>/dev/null; exec \"$HOME/{BIN_DIR}/humatad2\" bridge --sync {n}"
                ),
                State::Sync,
            ),
        };
        // The bridge line is typed into the person's shell as it is, so
        // `exec` replaces it; an install runs in a shell of its own.
        let line = if state == State::Sync {
            Ok(script)
        } else {
            sh_command(&script)
        };
        let write = line.map(|line| {
            let mut bytes = line.into_bytes();
            bytes.push(b'\n');
            TakeoverAction::Write(bytes)
        });
        match write {
            Ok(write) => {
                self.state = state;
                let mut actions = Vec::new();
                if state == State::Sync {
                    actions.push(TakeoverAction::MakeLocalRaw);
                }
                actions.push(write);
                actions
            }
            Err(error) => self.fail(error.into()),
        }
    }

    /// Looks for the bridge's `HUMATA_SYNC_<nonce>` line in the raw bytes.
    fn sync(&mut self) -> Vec<TakeoverAction> {
        let marker = format!("HUMATA_SYNC_{}", self.nonce);
        let Some(start) = find(&self.buffer, marker.as_bytes()) else {
            // Keep only what could still begin the marker.
            let keep = self.buffer.len().saturating_sub(marker.len() + 1);
            self.buffer.drain(..keep);
            return Vec::new();
        };
        let mut end = start + marker.len();
        if self.buffer.get(end) == Some(&b'\r') {
            end += 1;
        }
        match self.buffer.get(end) {
            Some(b'\n') => end += 1,
            Some(_) => {
                // Something else that merely starts like the marker.
                self.buffer.drain(..=start);
                return self.sync();
            }
            None => return Vec::new(),
        }
        let leftover = self.buffer.split_off(end);
        self.buffer.clear();
        self.state = State::Over;
        match self.platform {
            Some(platform) => vec![TakeoverAction::Ready { platform, leftover }],
            None => Vec::new(),
        }
    }

    fn typed(&self, script: &str) -> Result<TakeoverAction, TakeoverError> {
        let mut bytes = vec![0x15];
        bytes.extend_from_slice(sh_command(script)?.as_bytes());
        bytes.push(b'\n');
        Ok(TakeoverAction::Write(bytes))
    }

    fn fail(&mut self, error: TakeoverError) -> Vec<TakeoverAction> {
        self.state = State::Over;
        self.buffer.clear();
        vec![TakeoverAction::Failed(error)]
    }
}

/// Prints `HUMATA_FACTS_<n> <os> <arch> <sha> <sha>`. Markers are put
/// together by `printf`, so the terminal's echo of the typed line never
/// matches them.
fn facts_script(n: &str) -> String {
    let files: Vec<String> = InstalledFile::ALL
        .iter()
        .map(|file| format!("\"$(s {})\"", file.file_name()))
        .collect();
    format!(
        r#"s() {{ f="$HOME/{BIN_DIR}/$1"; [ -f "$f" ] || {{ echo none; return; }}; {{ sha256sum "$f" 2>/dev/null || shasum -a 256 "$f" 2>/dev/null || echo none; }} | cut -d ' ' -f 1; }}; printf '%s_%s_%s %s %s %s %s\n' HUMATA FACTS {n} "$(uname -s)" "$(uname -m)" {}"#,
        files.join(" ")
    )
}

/// Turns the far terminal raw, says so, reads and installs one file, turns
/// it back and prints the install's exit status.
fn install_script(payload: &Payload, n: &str) -> String {
    format!(
        "stty raw -echo -iexten 2>/dev/null\nprintf '%s_%s_%s\\n' HUMATA RAW {n}\n(\n{}\n)\ns=$?\nstty sane 2>/dev/null\nprintf '%s_%s_%s %s\\n' HUMATA DONE {n} \"$s\"\n",
        install::script(payload, n)
    )
}

/// The rest of `line` after `marker`, wherever it starts (a prompt may come
/// first).
fn after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    line.find(marker).map(|index| &line[index + marker.len()..])
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Terminal output as text: escape sequences, carriage returns and other
/// control characters removed.
fn plain(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b => {
                i += 1;
                match bytes.get(i) {
                    Some(b'[') => {
                        i += 1;
                        while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                            i += 1;
                        }
                        i += 1;
                    }
                    Some(b']') => {
                        i += 1;
                        while i < bytes.len() {
                            if bytes[i] == 0x07 {
                                i += 1;
                                break;
                            }
                            if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    Some(_) => i += 1,
                    None => {}
                }
            }
            b'\n' | b'\t' => {
                out.push(bytes[i]);
                i += 1;
            }
            b if b < 0x20 || b == 0x7f => i += 1,
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_carriage_returns_are_not_text() {
        assert_eq!(
            plain(b"\x1b]0;title\x07\x1b[1;32m$ \x1b[0mHUMATA_RAW_1\r\n"),
            "$ HUMATA_RAW_1\n"
        );
    }

    #[test]
    fn typed_lines_stay_short_enough_for_a_terminal() {
        let payload = Payload::new(InstalledFile::Humatad, std::sync::Arc::from(&b"x"[..]));
        let lines = [
            sh_command(&facts_script("0123456789abcdef")).unwrap(),
            sh_command(&install_script(&payload, "0123456789abcdef")).unwrap(),
        ];
        for line in lines {
            for physical in line.lines() {
                assert!(physical.len() < 1000, "{}", physical.len());
            }
        }
    }
}
