use super::FakeDaemon;
use crate::daemon::{DaemonError, Reply};
use crate::fixture;
use crate::model::{Agent, Session, SessionId, Speaker, Status};

pub(super) fn open(
    daemon: &FakeDaemon,
    session: &SessionId,
    agent: Agent,
) -> Reply<(usize, Session)> {
    let id = session.clone();
    daemon.reply(
        None,
        format!("open {session} {}", agent.command()),
        move |inner| {
            let session = inner.session_mut(&id)?;
            let existing = if agent == Agent::Shell {
                None
            } else if session.agent == agent {
                Some(0)
            } else {
                session
                    .terminal_agents
                    .iter()
                    .find_map(|(tab, found)| (*found == agent).then_some(*tab))
            };
            let tab = existing.unwrap_or_else(|| {
                session.shells.push(agent.command().to_owned());
                let tab = session.shells.len();
                if agent != Agent::Shell {
                    session.terminal_agents.insert(tab, agent);
                }
                tab
            });
            let snapshot = session.clone();
            inner.changed(&id);
            if existing.is_none() {
                for line in fixture::extra_terminal(agent, &snapshot.repo, &snapshot.branch) {
                    inner.output(&id, tab, line.from, &line.text);
                }
            }
            Ok((tab, snapshot))
        },
    )
}

pub(super) fn write(daemon: &FakeDaemon, session: &SessionId, tab: usize, text: &str) -> Reply<()> {
    let id = session.clone();
    let text = text.to_owned();
    daemon.reply(
        None,
        format!("write {session} {tab} {text:?}"),
        move |inner| {
            inner.session_mut(&id)?;
            super::desk::used(inner, &id);
            inner.output(&id, tab, Speaker::You, &format!("$ {text}"));
            let echo = match text.trim() {
                "ls" => "notes.md  todo.txt".to_owned(),
                "pwd" => "~/humata".to_owned(),
                "" => String::new(),
                other => format!("zsh: command not found: {other}"),
            };
            if !echo.is_empty() {
                inner.output(&id, tab, Speaker::Tool, &echo);
            }
            Ok(())
        },
    )
}

pub(super) fn input(
    daemon: &FakeDaemon,
    session: &SessionId,
    tab: usize,
    bytes: Vec<u8>,
) -> Reply<()> {
    let id = session.clone();
    daemon.reply(
        None,
        format!("terminal input {session} {tab} {} bytes", bytes.len()),
        move |inner| {
            let session = inner.session_mut(&id)?;
            if tab > session.shells.len() {
                return Err(DaemonError::NotFound {
                    what: format!("Terminal {tab} of {id}"),
                });
            }
            if tab == 0 && matches!(session.status, Status::Exited { .. }) {
                return Err(DaemonError::Refused {
                    what: "Typing into this terminal".into(),
                    reason: "the program has exited. Resume it first".into(),
                });
            }
            let agent = if tab == 0 {
                session.agent
            } else {
                session
                    .terminal_agents
                    .get(&tab)
                    .copied()
                    .unwrap_or(Agent::Shell)
            };
            super::desk::used(inner, &id);
            let mut output = Vec::new();
            // This is a deterministic fake PTY, never a local command executor.
            let pending = inner.terminal_input.entry((id.clone(), tab)).or_default();
            for byte in bytes {
                match byte {
                    b'\r' | b'\n' => {
                        let text = String::from_utf8_lossy(pending).into_owned();
                        pending.clear();
                        output.extend_from_slice(b"\r\n");
                        let result = if agent == Agent::Shell {
                            match text.trim() {
                                "pwd" => "~/humata".into(),
                                "ls" => "notes.md  todo.txt".into(),
                                "" => String::new(),
                                value => format!("zsh: command not found: {value}"),
                            }
                        } else {
                            format!("Message received by the fake {} terminal.", agent.name())
                        };
                        output.extend_from_slice(result.as_bytes());
                        output.extend_from_slice(if agent == Agent::Shell {
                            b"\r\n$ "
                        } else {
                            b"\r\n> "
                        });
                    }
                    8 | 127 => {
                        if !pending.is_empty() {
                            while let Some(byte) = pending.pop() {
                                if byte & 0xc0 != 0x80 {
                                    break;
                                }
                            }
                            output.extend_from_slice(b"\x08 \x08");
                        }
                    }
                    3 => {
                        pending.clear();
                        output.extend_from_slice(b"^C\r\n> ");
                    }
                    32..=255 => {
                        pending.push(byte);
                        output.push(byte);
                    }
                    _ => {}
                }
            }
            inner.terminal_data(&id, tab, output);
            Ok(())
        },
    )
}

pub(super) fn resize(
    daemon: &FakeDaemon,
    session: &SessionId,
    tab: usize,
    columns: u16,
    rows: u16,
) -> Reply<()> {
    let id = session.clone();
    daemon.reply(
        None,
        format!("terminal resize {session} {tab} {columns}x{rows}"),
        move |inner| {
            let session = inner.session_mut(&id)?;
            if tab > session.shells.len() {
                return Err(DaemonError::NotFound {
                    what: format!("Terminal {tab} of {id}"),
                });
            }
            if columns == 0 || rows == 0 {
                return Err(DaemonError::Refused {
                    what: "Resizing the terminal".into(),
                    reason: "it must have at least one row and column. Enlarge the window".into(),
                });
            }
            // An agent redraws its screen when the terminal changes size, as a
            // real TUI does on SIGWINCH, so its input box stays on the last rows.
            let redraw = (tab == 0
                && session.agent != Agent::Shell
                && !matches!(session.status, Status::Exited { .. } | Status::NeedsYou))
            .then(|| session.clone());
            let previous = inner
                .workspace
                .terminal_sizes
                .insert((id.clone(), tab), (columns, rows));
            if let Some(session) = redraw
                && previous != Some((columns, rows))
            {
                let mut screen = b"\x1b[H\x1b[2J".to_vec();
                screen.extend(fixture::terminal_screen(&session, tab));
                inner.terminal_data(&id, tab, screen);
            }
            Ok(())
        },
    )
}
