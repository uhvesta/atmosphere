//! `humata terminal attach`: a host's terminal as this process's own, for a
//! terminal app to run in its pty. Humata's terminal view runs it in
//! Ghostty, so Ghostty draws the host's shell while the bytes go through
//! humatad as every other client's do.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use humata_wire::{Client, ClientError, pb};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// Written before a replay, which is everything the terminal printed that
/// the host kept: RIS resets the screen, and ED 3 clears its scrollback, so
/// the replay draws it all again from nothing.
pub const RESET: &[u8] = b"\x1bc\x1b[3J";

/// How long sizes have to stay the same before the last one is sent, so
/// dragging a window's edge sends one resize rather than dozens.
const RESIZE_SETTLE: Duration = Duration::from_millis(100);

/// The longest wait between attempts to reattach.
const LONGEST_WAIT: Duration = Duration::from_secs(30);

/// How much of what's typed is read at once.
const READ_SIZE: usize = 64 * 1024;

/// A terminal's size in character cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub rows: u32,
    pub columns: u32,
}

/// The terminal to attach to.
pub struct Target {
    /// The host it runs on; empty for this Mac.
    pub host: String,
    pub terminal: String,
}

/// How an attach ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    /// The terminal's shell exited with this code.
    Exited(i32),
    /// What's typed ended: the terminal app closed its pty.
    InputClosed,
}

#[derive(Debug, thiserror::Error)]
pub enum AttachError {
    #[error(
        "there is no terminal {terminal} on {}; it was closed, or it runs on another host. List them with `humata terminal list --host <host>`",
        place(host)
    )]
    Gone { terminal: String, host: String },

    #[error("humatad wouldn't attach to terminal {terminal}")]
    Refused {
        terminal: String,
        #[source]
        source: ClientError,
    },

    #[error("couldn't show the terminal's output; the terminal app may have closed")]
    Output {
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't read what's typed; the terminal app may have closed")]
    Input {
        #[source]
        source: std::io::Error,
    },
}

fn place(host: &str) -> &str {
    if host.is_empty() { "this Mac" } else { host }
}

/// The code the attach exits with when the shell exited with `code`: the
/// same, when it fits in an exit status.
pub fn exit_status(code: i32) -> u8 {
    u8::try_from(code).unwrap_or(1)
}

/// Attaches to `target` through the connections `connect` makes, writing
/// its output to `output` and sending what's read from `input` and the
/// sizes `sizes` holds. A dropped connection is said on `output`, then
/// reattached after `first_wait`, twice that the next time, up to every 30
/// seconds; the replay that follows clears the note.
pub async fn attach<C, F, E, I, O>(
    target: &Target,
    mut connect: C,
    mut input: I,
    mut output: O,
    mut sizes: watch::Receiver<Option<Size>>,
    first_wait: Duration,
) -> Result<Ended, AttachError>
where
    C: FnMut() -> F,
    F: Future<Output = Result<Client, E>>,
    E: std::error::Error,
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
{
    let mut retries = 0u32;
    let mut buffer = vec![0; READ_SIZE];
    loop {
        let lost = match connect().await {
            Err(error) => humata_wire::describe(&error),
            Ok(client) => {
                let mut session = Session {
                    target,
                    client: Arc::new(client),
                    input: &mut input,
                    output: &mut output,
                    sizes: &mut sizes,
                    buffer: &mut buffer,
                    retries: &mut retries,
                };
                match session.run().await? {
                    Outcome::Ended(ended) => return Ok(ended),
                    Outcome::Lost(reason) => reason,
                }
            }
        };
        let wait = first_wait
            .saturating_mul(1 << retries.min(4))
            .min(LONGEST_WAIT);
        retries += 1;
        let note = format!(
            "\r\n\x1b[2m[humata] Lost the connection to the terminal ({lost}). Reconnecting in {}s…\x1b[0m\r\n",
            wait.as_secs().max(1)
        );
        write(&mut output, note.as_bytes()).await?;
        tokio::time::sleep(wait).await;
    }
}

enum Outcome {
    Ended(Ended),
    /// The connection dropped, for this reason.
    Lost(String),
}

/// One connection's attach.
struct Session<'a, I, O> {
    target: &'a Target,
    client: Arc<Client>,
    input: &'a mut I,
    output: &'a mut O,
    sizes: &'a mut watch::Receiver<Option<Size>>,
    buffer: &'a mut Vec<u8>,
    retries: &'a mut u32,
}

impl<I, O> Session<'_, I, O>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
{
    async fn run(&mut self) -> Result<Outcome, AttachError> {
        let request = pb::request::Kind::AttachTerminal(pb::AttachTerminal {
            host: self.target.host.clone(),
            terminal_id: self.target.terminal.clone(),
        });
        let mut stream = match self.client.stream(request.clone()).await {
            Ok((_, stream)) => stream,
            Err(error) => return self.refused(error),
        };
        let size = *self.sizes.borrow_and_update();
        if let Some(size) = size
            && let Err(lost) = self.resize(size).await
        {
            return Ok(lost);
        }
        // Keys typed while those before them are on their way, sent next in
        // one go, so they arrive in the order they were typed.
        let mut pending = Vec::new();
        let mut typing: Option<JoinHandle<Result<(), ClientError>>> = None;
        let mut resize_at: Option<Instant> = None;
        let mut sizes_open = true;
        loop {
            // Set when this side read too slowly and the stream was let go:
            // the connection is fine, so attaching again on it replays the
            // screen with what was missed.
            let mut behind = false;
            tokio::select! {
                item = stream.next() => match item {
                    None if stream.fell_behind() => behind = true,
                    None => return Ok(Outcome::Lost("humatad stopped sending its output".to_owned())),
                    Some(Err(error)) if error.code() == pb::ErrorCode::UnknownTerminal => {
                        return Err(self.gone());
                    }
                    Some(Err(error)) => return Ok(Outcome::Lost(error.message)),
                    Some(Ok(pb::StreamItem {
                        kind: Some(pb::stream_item::Kind::TerminalOutput(printed)),
                    })) => {
                        *self.retries = 0;
                        if printed.replay {
                            write(self.output, RESET).await?;
                        }
                        write(self.output, &printed.data).await?;
                        if printed.exited {
                            return Ok(Outcome::Ended(Ended::Exited(printed.exit_code)));
                        }
                    }
                    Some(Ok(_)) => {}
                },
                read = self.input.read(self.buffer) => match read {
                    Ok(0) => return Ok(Outcome::Ended(Ended::InputClosed)),
                    Ok(count) => {
                        pending.extend_from_slice(&self.buffer[..count]);
                        if typing.is_none() {
                            typing = Some(self.send(std::mem::take(&mut pending)));
                        }
                    }
                    Err(source) => return Err(AttachError::Input { source }),
                },
                sent = finished(&mut typing) => {
                    typing = None;
                    match sent {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => return Ok(Outcome::Lost(humata_wire::describe(&error))),
                        Err(error) => return Ok(Outcome::Lost(error.to_string())),
                    }
                    if !pending.is_empty() {
                        typing = Some(self.send(std::mem::take(&mut pending)));
                    }
                },
                changed = self.sizes.changed(), if sizes_open => match changed {
                    Ok(()) => resize_at = Some(Instant::now() + RESIZE_SETTLE),
                    Err(_) => sizes_open = false,
                },
                () = settled(resize_at) => {
                    resize_at = None;
                    let size = *self.sizes.borrow_and_update();
                    if let Some(size) = size
                        && let Err(lost) = self.resize(size).await
                    {
                        return Ok(lost);
                    }
                },
            }
            if behind {
                stream = match self.client.stream(request.clone()).await {
                    Ok((_, stream)) => stream,
                    Err(error) => return self.refused(error),
                };
            }
        }
    }

    /// What a refused attach means: a terminal that's gone ends the attach,
    /// another refusal is an error, and a dropped connection is retried.
    fn refused(&self, error: ClientError) -> Result<Outcome, AttachError> {
        match error {
            ClientError::Refused {
                code: pb::ErrorCode::UnknownTerminal,
                ..
            } => Err(self.gone()),
            ClientError::Refused { .. } => Err(AttachError::Refused {
                terminal: self.target.terminal.clone(),
                source: error,
            }),
            other => Ok(Outcome::Lost(humata_wire::describe(&other))),
        }
    }

    fn gone(&self) -> AttachError {
        AttachError::Gone {
            terminal: self.target.terminal.clone(),
            host: self.target.host.clone(),
        }
    }

    fn send(&self, data: Vec<u8>) -> JoinHandle<Result<(), ClientError>> {
        let client = self.client.clone();
        let request = pb::request::Kind::TerminalInput(pb::TerminalInput {
            host: self.target.host.clone(),
            terminal_id: self.target.terminal.clone(),
            data,
        });
        tokio::spawn(async move { client.call(request).await.map(|_| ()) })
    }

    /// Sends `size`. A refusal, such as from a shell that just exited, is
    /// left for the output to explain; a dropped connection is lost.
    async fn resize(&self, size: Size) -> Result<(), Outcome> {
        let request = pb::request::Kind::ResizeTerminal(pb::ResizeTerminal {
            host: self.target.host.clone(),
            terminal_id: self.target.terminal.clone(),
            rows: size.rows,
            columns: size.columns,
        });
        match self.client.call(request).await {
            Ok(_) | Err(ClientError::Refused { .. }) => Ok(()),
            Err(error) => Err(Outcome::Lost(humata_wire::describe(&error))),
        }
    }
}

/// Waits for the keys on their way to arrive, or forever when none are.
async fn finished(
    typing: &mut Option<JoinHandle<Result<(), ClientError>>>,
) -> Result<Result<(), ClientError>, tokio::task::JoinError> {
    match typing {
        Some(task) => task.await,
        None => std::future::pending().await,
    }
}

/// Waits until `at`, or forever when there's nothing to wait for.
async fn settled(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

async fn write<O: AsyncWrite + Unpin>(output: &mut O, data: &[u8]) -> Result<(), AttachError> {
    output
        .write_all(data)
        .await
        .map_err(|source| AttachError::Output { source })?;
    output
        .flush()
        .await
        .map_err(|source| AttachError::Output { source })
}

/// This process's terminal, for the attach to run in: raw while attached,
/// so every key goes to the host as typed, and its size as it changes.
#[cfg(unix)]
pub mod pty {

    use rustix::termios::{self, OptionalActions, Termios};
    use tokio::signal::unix::{SignalKind, signal};
    use tokio::sync::watch;

    use super::Size;

    /// Puts standard input's terminal in raw mode, and back as it was when
    /// dropped. Nothing when standard input isn't a terminal.
    pub struct Raw {
        saved: Option<Termios>,
    }

    impl Raw {
        pub fn enter() -> Self {
            let stdin = stdin();
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
                let _ = termios::tcsetattr(stdin(), OptionalActions::Now, saved);
            }
        }
    }

    fn stdin() -> std::io::Stdin {
        std::io::stdin()
    }

    /// The terminal's size now, if standard input is one.
    fn size() -> Option<Size> {
        let size = termios::tcgetwinsize(stdin()).ok()?;
        (size.ws_row > 0 && size.ws_col > 0).then_some(Size {
            rows: u32::from(size.ws_row),
            columns: u32::from(size.ws_col),
        })
    }

    /// The terminal's size, updated on each SIGWINCH.
    pub fn sizes() -> std::io::Result<watch::Receiver<Option<Size>>> {
        let mut changes = signal(SignalKind::window_change())?;
        let (sender, receiver) = watch::channel(size());
        tokio::spawn(async move {
            while changes.recv().await.is_some() {
                if sender.send(size()).is_err() {
                    return;
                }
            }
        });
        Ok(receiver)
    }
}

#[cfg(test)]
#[path = "attach_tests.rs"]
mod tests;
