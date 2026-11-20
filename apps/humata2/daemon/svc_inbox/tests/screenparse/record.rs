//! Runs a script against an agent CLI under a holder and writes the
//! snapshots it asks for.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_holder::{Control, Event, HolderError, HolderSpec, attach, shutdown, spawn};
use humata2_vt_authority::{Authority, AuthorityError, Config, Step as Answer};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;

use crate::script::{Kind, Step, parse_script};
use crate::snapshot::{Expect, Snapshot};
use crate::ScriptError;

const WAIT_LIMIT: Duration = Duration::from_secs(120);
const QUIET_LIMIT: Duration = Duration::from_secs(30);
const WIDTH_QUIET: Duration = Duration::from_millis(800);
const WIDTH_LIMIT: Duration = Duration::from_secs(5);
/// How long typed text gets to be taken before return is pressed.
const WRITE_LIMIT: Duration = Duration::from_secs(5);
/// The pause between typed text and its return, as a person's would be.
const BEFORE_RETURN: Duration = Duration::from_millis(400);
/// A cell's size in pixels, which the engine needs to be nonzero.
const CELL: (u32, u32) = (8, 16);

/// What to record.
#[derive(Clone, Debug)]
pub struct Options {
    pub agent: String,
    pub cli_version: String,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub script: PathBuf,
    pub out: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub widths: Vec<u16>,
    pub holder: PathBuf,
}

/// Why a recording stopped.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("couldn't read the script {path}. Check the path given to --script.")]
    ReadScript {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Script(#[from] ScriptError),
    #[error("the script {path} has no steps. Add at least one line.")]
    EmptyScript { path: PathBuf },
    #[error("the holder program {path} doesn't exist. Build it with `bazel build //apps/humata2/daemon/holder:humata2_holder` or pass --holder.")]
    HolderMissing { path: PathBuf },
    #[error("couldn't find the program {program} to run. Give --program a full path or put it on PATH.")]
    ProgramMissing { program: String },
    #[error("couldn't start the holder. Check --program and --cwd, then run again.")]
    Spawn(#[source] HolderError),
    #[error("couldn't connect to the holder it started. Run again.")]
    Attach(#[source] HolderError),
    #[error("lost the holder while running the script. Run again.")]
    Holder(#[source] HolderError),
    #[error("the holder refused a request: {message}. Run again.")]
    Refused { message: String },
    #[error("the terminal engine couldn't read the program's screen. Run again.")]
    Screen(#[source] AuthorityError),
    #[error("couldn't write the snapshot {path}. Check that --out is a folder you can write to.")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("script line {line}: step {step} (`{text}`) waited {seconds}s and never saw it. A `timeout-{step}` snapshot was written; read it to see where the program stopped.")]
    TimedOut {
        step: usize,
        line: usize,
        text: String,
        seconds: u64,
    },
    #[error("script line {line}: step {step} (`{text}`) was still waiting when the program ended. A `timeout-{step}` snapshot was written; check the program's own output for why it quit.")]
    ProgramExited {
        step: usize,
        line: usize,
        text: String,
    },
}

/// Ends the holder this run started, only while its pid is still that
/// holder, so a reused pid is left alone.
struct HolderGuard {
    pid: u32,
    socket: PathBuf,
}

impl Drop for HolderGuard {
    fn drop(&mut self) {
        let pid = self.pid.to_string();
        let ours = std::process::Command::new("/bin/ps")
            .args(["-ww", "-o", "args=", "-p", &pid])
            .output()
            .is_ok_and(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .contains(&format!("--socket {} ", self.socket.display()))
            });
        if ours {
            let _ = std::process::Command::new("/bin/kill")
                .args(["-TERM", &pid])
                .status();
        }
    }
}

enum Next {
    Event,
    Timeout,
    Closed,
}

struct Session {
    events: UnboundedReceiver<Event>,
    control: Control,
    authority: Authority,
    /// The last sequence number given to the holder, for typing and for
    /// answers alike.
    seq: u64,
    /// The holder's sequence of each answer sent, to the engine's own.
    answers: HashMap<u64, u64>,
    written: u64,
    cols: u16,
    rows: u16,
    /// The program ended, so no later step can see anything new.
    exited: bool,
}

impl Session {
    async fn handle(&mut self, event: Event) -> Result<(), RecordError> {
        match event {
            Event::Output { offset, data } => {
                let steps = self
                    .authority
                    .feed(offset, &data)
                    .map_err(RecordError::Screen)?;
                for step in steps {
                    // Answers the program's queries the way the daemon does:
                    // sent as input under a sequence of its own, then marked
                    // written once the holder says so.
                    if let Answer::Reply { seq, bytes } = step {
                        self.seq += 1;
                        self.control
                            .input(self.seq, &bytes)
                            .await
                            .map_err(RecordError::Holder)?;
                        self.answers.insert(self.seq, seq);
                    }
                }
            }
            Event::InputWritten { seq } => {
                self.written = self.written.max(seq);
                if let Some(answer) = self.answers.remove(&seq) {
                    self.authority.written(answer);
                }
            }
            Event::Refused { message } => return Err(RecordError::Refused { message }),
            Event::Exited { .. } => self.exited = true,
            Event::Trimmed { .. } => {}
        }
        Ok(())
    }

    async fn next(&mut self, deadline: Instant) -> Result<Next, RecordError> {
        match tokio::time::timeout_at(deadline, self.events.recv()).await {
            Ok(Some(event)) => {
                self.handle(event).await?;
                Ok(Next::Event)
            }
            Ok(None) => {
                self.exited = true;
                Ok(Next::Closed)
            }
            Err(_) => Ok(Next::Timeout),
        }
    }

    /// Keeps up with the program for `time`.
    async fn settle(&mut self, time: Duration) -> Result<(), RecordError> {
        let deadline = Instant::now() + time;
        while let Next::Event = self.next(deadline).await? {}
        Ok(())
    }

    /// Waits until no output has come for `quiet`, or `cap` has passed.
    async fn quiet(&mut self, quiet: Duration, cap: Duration) -> Result<(), RecordError> {
        let end = Instant::now() + cap;
        loop {
            let deadline = (Instant::now() + quiet).min(end);
            if !matches!(self.next(deadline).await?, Next::Event) {
                return Ok(());
            }
        }
    }

    /// Types `data` and waits until the holder has written it.
    async fn send(&mut self, data: &[u8]) -> Result<(), RecordError> {
        self.seq += 1;
        let seq = self.seq;
        self.control
            .input(seq, data)
            .await
            .map_err(RecordError::Holder)?;
        let deadline = Instant::now() + WRITE_LIMIT;
        while self.written < seq {
            if !matches!(self.next(deadline).await?, Next::Event) {
                break;
            }
        }
        Ok(())
    }

    fn screen(&self) -> Result<String, RecordError> {
        self.authority.screen_text().map_err(RecordError::Screen)
    }

    async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), RecordError> {
        // The engine first, so output after the new size is parsed at it.
        self.authority
            .resize(cols, rows, CELL.0, CELL.1)
            .map_err(RecordError::Screen)?;
        self.control
            .resize(cols, rows)
            .await
            .map_err(RecordError::Holder)?;
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    /// Waits until the screen shows `text` (or, with `gone`, stops
    /// showing it). `Err(None)` is a timeout, `Err(Some)` a real error.
    async fn wait(&mut self, text: &str, gone: bool) -> Result<Waited, RecordError> {
        let wanted = normalize(text);
        let deadline = Instant::now() + WAIT_LIMIT;
        loop {
            if normalize(&self.screen()?).contains(&wanted) != gone {
                return Ok(Waited::Done);
            }
            match self.next(deadline).await? {
                Next::Event => {}
                Next::Timeout => return Ok(Waited::TimedOut),
                Next::Closed => return Ok(Waited::Exited),
            }
        }
    }
}

enum Waited {
    Done,
    TimedOut,
    Exited,
}

/// The screen's text the way a wait compares it: lowercase, box-drawing
/// characters as spaces, runs of white space as one space.
fn normalize(text: &str) -> String {
    let mapped: String = text
        .chars()
        .map(|c| if ('\u{2500}'..='\u{257f}').contains(&c) { ' ' } else { c })
        .collect();
    mapped
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn find_program(program: &Path) -> Result<PathBuf, RecordError> {
    if program.components().count() > 1 {
        return Ok(program.to_owned());
    }
    let missing = || RecordError::ProgramMissing {
        program: program.display().to_string(),
    };
    let path = std::env::var_os("PATH").ok_or_else(missing)?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
        .ok_or_else(missing)
}

/// Runs the script and writes its snapshots under `options.out`.
pub async fn record(options: &Options) -> Result<(), RecordError> {
    let text = std::fs::read_to_string(&options.script).map_err(|source| RecordError::ReadScript {
        path: options.script.clone(),
        source,
    })?;
    let steps = parse_script(&text)?;
    if steps.is_empty() {
        return Err(RecordError::EmptyScript {
            path: options.script.clone(),
        });
    }
    let stem = options
        .script
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "script".into());
    if !options.holder.exists() {
        return Err(RecordError::HolderMissing {
            path: options.holder.clone(),
        });
    }
    let program = find_program(&options.program)?;

    // Socket paths must be short, so the run's folder is under /tmp.
    let dir = humata2_tmp_folder::folder("h2sp");
    let spec = HolderSpec {
        holder: options.holder.clone(),
        socket: dir.path().join("t.sock"),
        program,
        args: options.args.clone(),
        cwd: options.cwd.clone(),
        env: options.env.clone(),
        cols: options.cols,
        rows: options.rows,
        scrollback_bytes: 8 << 20,
        log: dir.path().join("holder.log"),
    };
    let spawned = spawn(&spec).map_err(RecordError::Spawn)?;
    let _guard = HolderGuard {
        pid: spawned.pid,
        socket: spec.socket.clone(),
    };
    let attachment = attach(&spec.socket, 0).await.map_err(RecordError::Attach)?;
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut holder_events = attachment.events;
    // Reading isn't cancel-safe, so a task of its own feeds a channel.
    tokio::spawn(async move {
        while let Ok(Some(event)) = holder_events.next().await {
            let exited = matches!(event, Event::Exited { .. });
            if sender.send(event).is_err() || exited {
                return;
            }
        }
    });
    let authority = Authority::new(&Config {
        cols: options.cols,
        rows: options.rows,
        scrollback_lines: 10_000,
        scrollback_bytes: 8 << 20,
        checkpoint_limit: 16 << 20,
    })
    .map_err(RecordError::Screen)?;
    let mut session = Session {
        events: receiver,
        control: attachment.control,
        authority,
        seq: 0,
        answers: HashMap::new(),
        written: 0,
        cols: options.cols,
        rows: options.rows,
        exited: false,
    };

    let result = run_steps(options, &stem, &steps, &mut session).await;
    // Ends the program with its holder; the guard covers a holder that
    // doesn't answer.
    let _ = shutdown(&spec.socket).await;
    result
}

async fn run_steps(
    options: &Options,
    stem: &str,
    steps: &[Step],
    session: &mut Session,
) -> Result<(), RecordError> {
    let mut snaps = 0usize;
    for (index, step) in steps.iter().enumerate() {
        let number = index + 1;
        let started = std::time::Instant::now();
        let outcome = run_step(options, stem, step, number, &mut snaps, session).await;
        eprintln!(
            "step {number} line {} {} {}ms",
            step.line,
            step.kind.name(),
            started.elapsed().as_millis()
        );
        outcome?;
    }
    Ok(())
}

async fn run_step(
    options: &Options,
    stem: &str,
    step: &Step,
    number: usize,
    snaps: &mut usize,
    session: &mut Session,
) -> Result<(), RecordError> {
    // A step after the program ended would record a screen it no longer
    // draws, so the run stops at the first one.
    if session.exited {
        return Err(RecordError::ProgramExited {
            step: number,
            line: step.line,
            text: step.kind.name().to_owned(),
        });
    }
    match &step.kind {
        Kind::Wait(text) | Kind::WaitGone(text) => {
            let gone = matches!(step.kind, Kind::WaitGone(_));
            match session.wait(text, gone).await? {
                Waited::Done => Ok(()),
                waited => {
                    let snap = snapshot(
                        options,
                        stem,
                        &format!("timeout-{number}"),
                        Expect::Unknown,
                        session,
                    )?;
                    write(options, &snap, *snaps + 1)?;
                    let text = text.clone();
                    Err(match waited {
                        Waited::TimedOut => RecordError::TimedOut {
                            step: number,
                            line: step.line,
                            text,
                            seconds: WAIT_LIMIT.as_secs(),
                        },
                        _ => RecordError::ProgramExited {
                            step: number,
                            line: step.line,
                            text,
                        },
                    })
                }
            }
        }
        Kind::Quiet(ms) => {
            session
                .quiet(Duration::from_millis(*ms), QUIET_LIMIT)
                .await
        }
        Kind::Type(text) => {
            session.send(text.as_bytes()).await?;
            // Return is its own key, pressed after the program took the
            // text, as a person's is.
            session.settle(BEFORE_RETURN).await?;
            session.send(b"\r").await
        }
        Kind::Keys(bytes) => session.send(bytes).await,
        Kind::Resize { cols, rows } => session.resize(*cols, *rows).await,
        Kind::Snap { label, expect } => {
            *snaps += 1;
            let index = *snaps;
            let snap = snapshot(options, stem, label, expect.clone(), session)?;
            write(options, &snap, index)?;
            let (cols, rows) = (session.cols, session.rows);
            for width in options.widths.iter().copied().filter(|w| *w != cols) {
                session.resize(width, rows).await?;
                session.quiet(WIDTH_QUIET, WIDTH_LIMIT).await?;
                let snap = snapshot(options, stem, label, expect.clone(), session)?;
                write(options, &snap, index)?;
            }
            if session.cols != cols {
                session.resize(cols, rows).await?;
                session.quiet(WIDTH_QUIET, WIDTH_LIMIT).await?;
            }
            Ok(())
        }
    }
}

fn snapshot(
    options: &Options,
    stem: &str,
    label: &str,
    expect: Expect,
    session: &Session,
) -> Result<Snapshot, RecordError> {
    Ok(Snapshot {
        agent: options.agent.clone(),
        cli: options.cli_version.clone(),
        script: stem.to_owned(),
        label: label.to_owned(),
        expect,
        cols: session.cols,
        rows: session.rows,
        screen: session.screen()?,
    })
}

fn write(options: &Options, snap: &Snapshot, index: usize) -> Result<(), RecordError> {
    snap.write(&options.out, index)
        .map_err(|source| RecordError::Write {
            path: options.out.join(snap.file_name(index)),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn waits_compare_case_blind_with_box_lines_and_runs_of_space_flattened() {
        assert_eq!(
            normalize("│  Do   you\n│ WANT it?  │"),
            "do you want it?"
        );
    }
}
