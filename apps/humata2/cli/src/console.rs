//! The terminal `humata` runs in, behind a trait so tests can type into it
//! and read what it printed.

use std::fs::OpenOptions;
use std::io::{self, BufRead, IsTerminal, Read, Write};

use rustix::termios::{self, LocalModes, OptionalActions, Termios};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

pub trait Console {
    /// Text for stdout: results.
    fn print(&self, text: &str);
    /// Text for stderr: errors, prompts and progress.
    fn eprint(&self, text: &str);
    /// Bytes for stdout as they are: file contents, terminal output.
    fn write(&self, bytes: &[u8]);
    /// Whether a person is likely at the keyboard: stdin and stdout are
    /// both terminals.
    fn interactive(&self) -> bool;
    /// A line typed with echo off, read from the terminal itself rather than
    /// stdin, so a pipe can't answer it (CLI.md §3).
    fn secret(&self, prompt: &str) -> io::Result<String>;
    /// A line typed with echo on, read from the terminal itself rather than
    /// stdin, so a pipe can't answer it: for a count the person can see.
    fn line(&self, prompt: &str) -> io::Result<String>;
    /// The CLI's own "Continue?" after a plan. Never stands in for presence.
    fn confirm(&self, prompt: &str) -> io::Result<bool>;
    /// All of stdin.
    fn stdin(&self) -> io::Result<Vec<u8>>;
    /// The keyboard and the terminal's size, for `humata attach`.
    fn keyboard(&self) -> io::Result<Keyboard>;
}

/// Raw keystrokes and size changes while attached. The terminal goes back
/// to how it was when this is dropped.
pub struct Keyboard {
    pub input: mpsc::Receiver<Vec<u8>>,
    pub resizes: mpsc::Receiver<(u32, u32)>,
    pub size: (u32, u32),
    _raw: Option<Raw>,
}

impl Keyboard {
    pub fn new(
        input: mpsc::Receiver<Vec<u8>>,
        resizes: mpsc::Receiver<(u32, u32)>,
        size: (u32, u32),
    ) -> Self {
        Self {
            input,
            resizes,
            size,
            _raw: None,
        }
    }
}

/// The process's own stdin, stdout and terminal.
pub struct StdConsole;

impl Console for StdConsole {
    fn print(&self, text: &str) {
        let mut stdout = io::stdout().lock();
        let _ = writeln!(stdout, "{text}");
    }

    fn eprint(&self, text: &str) {
        let mut stderr = io::stderr().lock();
        let _ = writeln!(stderr, "{text}");
    }

    fn write(&self, bytes: &[u8]) {
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(bytes);
        let _ = stdout.flush();
    }

    fn interactive(&self) -> bool {
        io::stdin().is_terminal() && io::stdout().is_terminal()
    }

    fn line(&self, prompt: &str) -> io::Result<String> {
        let mut tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
        write!(tty, "{prompt}")?;
        tty.flush()?;
        let mut line = String::new();
        io::BufReader::new(&tty).read_line(&mut line)?;
        Ok(line.trim_end_matches(['\r', '\n']).to_owned())
    }

    fn secret(&self, prompt: &str) -> io::Result<String> {
        let mut tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
        let saved = termios::tcgetattr(&tty)?;
        let mut quiet = saved.clone();
        quiet.local_modes.remove(LocalModes::ECHO);
        quiet.local_modes.insert(LocalModes::ECHONL);
        termios::tcsetattr(&tty, OptionalActions::Now, &quiet)?;
        let typed = (|| -> io::Result<String> {
            write!(tty, "{prompt}")?;
            tty.flush()?;
            let mut line = String::new();
            io::BufReader::new(&tty).read_line(&mut line)?;
            Ok(line)
        })();
        termios::tcsetattr(&tty, OptionalActions::Now, &saved)?;
        Ok(typed?.trim_end_matches(['\r', '\n']).to_owned())
    }

    fn confirm(&self, prompt: &str) -> io::Result<bool> {
        let mut stderr = io::stderr().lock();
        write!(stderr, "{prompt} [y/N] ")?;
        stderr.flush()?;
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes"))
    }

    fn stdin(&self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        io::stdin().lock().read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn keyboard(&self) -> io::Result<Keyboard> {
        let raw = Raw::enter();
        let (input_sender, input) = mpsc::channel(64);
        std::thread::spawn(move || {
            let mut stdin = io::stdin().lock();
            let mut buffer = [0u8; 4096];
            loop {
                match stdin.read(&mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(read) => {
                        if input_sender.blocking_send(buffer[..read].to_vec()).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        let mut changes = signal(SignalKind::window_change())?;
        let (resize_sender, resizes) = mpsc::channel(8);
        tokio::spawn(async move {
            while changes.recv().await.is_some() {
                let Some(size) = size() else { continue };
                if resize_sender.send(size).await.is_err() {
                    return;
                }
            }
        });
        Ok(Keyboard {
            input,
            resizes,
            size: size().unwrap_or((80, 24)),
            _raw: Some(raw),
        })
    }
}

/// Columns and rows of stdin's terminal, if it is one.
fn size() -> Option<(u32, u32)> {
    let size = termios::tcgetwinsize(io::stdin()).ok()?;
    (size.ws_row > 0 && size.ws_col > 0)
        .then_some((u32::from(size.ws_col), u32::from(size.ws_row)))
}

/// Stdin's terminal in raw mode until dropped; nothing when it isn't one.
struct Raw {
    saved: Option<Termios>,
}

impl Raw {
    fn enter() -> Self {
        let stdin = io::stdin();
        let Ok(saved) = termios::tcgetattr(&stdin) else {
            return Self { saved: None };
        };
        let mut raw = saved.clone();
        raw.make_raw();
        if termios::tcsetattr(&stdin, OptionalActions::Now, &raw).is_err() {
            return Self { saved: None };
        }
        Self { saved: Some(saved) }
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        if let Some(saved) = &self.saved {
            let _ = termios::tcsetattr(io::stdin(), OptionalActions::Now, saved);
        }
    }
}
