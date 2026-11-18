//! Runs invocations as processes on this computer, with a cleared
//! environment, in a process group of their own that a timeout kills.

use std::io::{self, Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal};

use crate::error::{HostVariablesError, RunError};
use crate::name::BASELINE;
use crate::runner::{EnvValue, Exit, Invocation, Output, Runner};

/// How long output may keep arriving after the command ends, from children
/// it left behind holding its pipes.
const DRAIN: Duration = Duration::from_secs(2);
/// The most of each stream kept; the rest is dropped.
const MAX_OUTPUT: usize = 8 << 20;

/// Where the runner looks up host variables. The values stay in memory: the
/// runner sets them on the child and replaces them in what it printed.
pub trait HostVariables {
    /// `name`'s value, or `None` when it isn't set. A store that can't be
    /// read is an error, never "not set".
    fn value(&self, name: &str) -> Result<Option<String>, HostVariablesError>;
}

/// No host variables at all.
impl HostVariables for () {
    fn value(&self, _name: &str) -> Result<Option<String>, HostVariablesError> {
        Ok(None)
    }
}

/// Runs each invocation as a child process of this one.
pub struct SystemRunner<'a> {
    variables: &'a dyn HostVariables,
}

impl<'a> SystemRunner<'a> {
    pub fn new(variables: &'a dyn HostVariables) -> Self {
        Self { variables }
    }
}

#[derive(Clone, Copy)]
enum Stream {
    Out,
    Err,
}

/// What the runner waits for.
enum Event {
    Chunk(Stream, Vec<u8>),
    /// One of the pipes closed.
    Closed,
    Exited(io::Result<ExitStatus>),
}

impl Runner for SystemRunner<'_> {
    fn run(&self, invocation: &Invocation) -> Result<Output, RunError> {
        let mut command = Command::new(&invocation.program);
        command.args(&invocation.args).env_clear();
        for name in BASELINE {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut secrets: Vec<(String, String)> = Vec::new();
        for (name, value) in &invocation.env {
            match value {
                EnvValue::Value(value) => {
                    command.env(name, value);
                }
                EnvValue::HostVariable(variable) => {
                    let value = self
                        .variables
                        .value(variable)
                        .map_err(|source| RunError::UnreadableHostVariable {
                            name: variable.clone(),
                            source,
                        })?
                        .ok_or_else(|| RunError::MissingHostVariable {
                            name: variable.clone(),
                        })?;
                    command.env(name, &value);
                    secrets.push((variable.clone(), value));
                }
            }
        }
        command
            .stdin(if invocation.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // A group of its own, so a timeout stops everything it started.
            .process_group(0);

        let mut child = command.spawn().map_err(|source| RunError::Spawn {
            program: invocation.program.clone(),
            source,
        })?;
        let group = i32::try_from(child.id()).ok().and_then(Pid::from_raw);

        if let (Some(bytes), Some(mut stdin)) = (invocation.stdin.clone(), child.stdin.take()) {
            // A command that stops reading gets EPIPE here, or is killed by
            // the timeout; either way this thread ends.
            thread::spawn(move || {
                let _ = stdin.write_all(&bytes);
            });
        }
        let (sender, receiver) = mpsc::channel();
        let mut open = 0;
        if let Some(stdout) = child.stdout.take() {
            read_into(Stream::Out, stdout, sender.clone());
            open += 1;
        }
        if let Some(stderr) = child.stderr.take() {
            read_into(Stream::Err, stderr, sender.clone());
            open += 1;
        }
        // The child's end comes as a message too, from a thread blocked in
        // wait(): nothing here wakes up to ask (TR-9).
        let waiter = sender;
        thread::spawn(move || {
            let _ = waiter.send(Event::Exited(child.wait()));
        });

        let deadline = invocation.timeout.map(|timeout| Instant::now() + timeout);
        let mut timed_out = false;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut ended: Option<(Exit, Instant)> = None;
        loop {
            if let Some((exit, drain_until)) = ended {
                if open == 0 || Instant::now() >= drain_until {
                    redact(&mut stdout, &secrets);
                    redact(&mut stderr, &secrets);
                    return Ok(Output {
                        exit,
                        stdout,
                        stderr,
                    });
                }
            }
            // Wakes only for a message, the timeout, or the end of draining.
            let wake_at = match ended {
                Some((_, drain_until)) => Some(drain_until),
                None if timed_out => None,
                None => deadline,
            };
            let event = match wake_at {
                Some(at) => receiver.recv_timeout(at.saturating_duration_since(Instant::now())),
                None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match event {
                Ok(Event::Chunk(stream, chunk)) => {
                    let buffer = match stream {
                        Stream::Out => &mut stdout,
                        Stream::Err => &mut stderr,
                    };
                    let room = MAX_OUTPUT.saturating_sub(buffer.len());
                    buffer.extend_from_slice(&chunk[..chunk.len().min(room)]);
                }
                Ok(Event::Closed) => open -= 1,
                Ok(Event::Exited(Ok(status))) => {
                    let exit = if timed_out { Exit::TimedOut } else { exit_of(status) };
                    ended = Some((exit, Instant::now() + DRAIN));
                }
                Ok(Event::Exited(Err(source))) => {
                    kill(group);
                    return Err(RunError::Wait {
                        program: invocation.program.clone(),
                        source,
                    });
                }
                Err(RecvTimeoutError::Timeout) => {
                    if ended.is_none() && !timed_out {
                        // The waiter reports the end once the kill lands.
                        kill(group);
                        timed_out = true;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    // Every sender is gone, the waiter's included, so its
                    // message was already handled.
                    open = 0;
                    if ended.is_none() {
                        ended = Some((if timed_out { Exit::TimedOut } else { Exit::Code(-1) }, Instant::now()));
                    }
                }
            }
        }
    }
}

/// Sends what `reader` yields, then [`Event::Closed`] when it ends.
fn read_into(
    stream: Stream,
    mut reader: impl Read + Send + 'static,
    sender: Sender<Event>,
) {
    thread::spawn(move || {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if sender.send(Event::Chunk(stream, buffer[..n].to_vec())).is_err() {
                        return;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let _ = sender.send(Event::Closed);
    });
}

fn exit_of(status: ExitStatus) -> Exit {
    match (status.code(), status.signal()) {
        (Some(code), _) => Exit::Code(code),
        (None, Some(signal)) => Exit::Signal(signal),
        (None, None) => Exit::Code(-1),
    }
}

fn kill(group: Option<Pid>) {
    if let Some(group) = group {
        let _ = rustix::process::kill_process_group(group, Signal::KILL);
    }
}

/// Replaces each host variable's value in `bytes` with `‹NAME›`, longest
/// value first so one that contains another is caught whole.
fn redact(bytes: &mut Vec<u8>, secrets: &[(String, String)]) {
    let mut secrets: Vec<&(String, String)> =
        secrets.iter().filter(|(_, value)| !value.is_empty()).collect();
    secrets.sort_by_key(|(_, value)| std::cmp::Reverse(value.len()));
    for (name, value) in secrets {
        let needle = value.as_bytes();
        if !bytes.windows(needle.len()).any(|window| window == needle) {
            continue;
        }
        let marker = format!("‹{name}›");
        let mut out = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index..].starts_with(needle) {
                out.extend_from_slice(marker.as_bytes());
                index += needle.len();
            } else {
                out.push(bytes[index]);
                index += 1;
            }
        }
        *bytes = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_copy_of_a_value_is_replaced_by_its_name() {
        let mut bytes = b"token=s3cret and s3cret again; s3cre".to_vec();
        redact(&mut bytes, &[("TOKEN".to_owned(), "s3cret".to_owned())]);
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "token=‹TOKEN› and ‹TOKEN› again; s3cre"
        );
    }

    #[test]
    fn a_value_containing_another_is_replaced_whole() {
        let mut bytes = b"abcdef".to_vec();
        redact(
            &mut bytes,
            &[
                ("SHORT".to_owned(), "cd".to_owned()),
                ("LONG".to_owned(), "bcde".to_owned()),
            ],
        );
        assert_eq!(String::from_utf8(bytes).unwrap(), "a‹LONG›f");
    }
}
