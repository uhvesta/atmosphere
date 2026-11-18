use super::FakeDaemon;
use crate::daemon::{DaemonError, ImageKind, Reply};
use crate::fixture;
use crate::model::{Agent, Session, SessionId, Speaker, Status};

/// A new tab running `agent` in `folder`; the fake keeps the folder only in
/// its call log.
pub(super) fn open_in(
    daemon: &FakeDaemon,
    session: &SessionId,
    agent: Agent,
    folder: &str,
) -> Reply<(usize, Session)> {
    open_as(
        daemon,
        session,
        agent,
        format!("open_in {session} {} {folder}", agent.command()),
        true,
    )
}

pub(super) fn open(
    daemon: &FakeDaemon,
    session: &SessionId,
    agent: Agent,
) -> Reply<(usize, Session)> {
    open_as(
        daemon,
        session,
        agent,
        format!("open {session} {}", agent.command()),
        false,
    )
}

fn open_as(
    daemon: &FakeDaemon,
    session: &SessionId,
    agent: Agent,
    call: String,
    always_new: bool,
) -> Reply<(usize, Session)> {
    let id = session.clone();
    daemon.reply(
        None,
        call,
        move |inner| {
            let session = inner.session_mut(&id)?;
            let existing = if agent == Agent::Shell || always_new {
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

/// Saves a pasted image as the daemon does and returns where: a name of its
/// kind under the session's folder. Bytes that are not of `kind` are
/// refused, as the daemon refuses them.
pub(super) fn paste_image(
    daemon: &FakeDaemon,
    session: &SessionId,
    kind: ImageKind,
    bytes: Vec<u8>,
) -> Reply<String> {
    let id = session.clone();
    daemon.reply(
        None,
        format!("paste-image {session} {kind:?} {}", bytes.len()),
        move |inner| {
            inner.session_mut(&id)?;
            let Some(extension) = image_extension(kind, &bytes) else {
                return Err(DaemonError::Refused {
                    what: "The pasted image".into(),
                    reason: format!(
                        "its bytes are not a {kind:?} image. Copy the image again, then paste it"
                    ),
                });
            };
            inner.pasted_images += 1;
            Ok(format!(
                "/tmp/humata-pastes/{id}/paste-{}.{extension}",
                inner.pasted_images
            ))
        },
    )
}

/// The extension an image of `kind` is saved with, when `bytes` start as
/// one does.
fn image_extension(kind: ImageKind, bytes: &[u8]) -> Option<&'static str> {
    let (extension, matches) = match kind {
        ImageKind::Png => ("png", bytes.starts_with(b"\x89PNG\r\n\x1a\n")),
        ImageKind::Jpeg => ("jpg", bytes.starts_with(b"\xff\xd8\xff")),
        ImageKind::Gif => (
            "gif",
            bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        ),
        ImageKind::Webp => (
            "webp",
            bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()),
        ),
        ImageKind::Tiff => (
            "tiff",
            bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*"),
        ),
        ImageKind::Heic => ("heic", bytes.get(4..8) == Some(b"ftyp".as_slice())),
    };
    matches.then_some(extension)
}

/// What typing `bytes` into a tab does, for a view's link.
pub(super) fn apply_input(
    inner: &mut super::Inner,
    id: &SessionId,
    tab: usize,
    bytes: Vec<u8>,
) -> Result<(), DaemonError> {
    {
        {
            let session = inner.session_mut(id)?;
            if tab > session.shells.len() {
                return Err(DaemonError::NotFound {
                    what: format!("Terminal {tab} of {id}"),
                });
            }
            if tab == 0 && session.status.agent_ended() {
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
            if inner.exited_tabs.contains_key(&(id.clone(), tab)) {
                return Err(DaemonError::Refused {
                    what: "Typing into this terminal".into(),
                    reason: "its shell has exited. Open a new terminal to run more commands"
                        .into(),
                });
            }
            super::desk::used(inner, &id);
            let mut output = Vec::new();
            let mut exited = None;
            // This is a deterministic fake PTY, never a local command executor.
            let pending = inner.terminal_input.entry((id.clone(), tab)).or_default();
            for byte in bytes {
                match byte {
                    b'\r' | b'\n' => {
                        let text = String::from_utf8_lossy(pending).into_owned();
                        pending.clear();
                        output.extend_from_slice(b"\r\n");
                        // A shell tab's shell ends at `exit`; what follows
                        // it reaches nothing.
                        if tab != 0 && agent == Agent::Shell && text.trim() == "exit" {
                            exited = Some(0);
                            break;
                        }
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
            let printed = !output.is_empty();
            inner.terminal_data(id, tab, output);
            if printed {
                inner.heard(id, tab);
            }
            if let Some(code) = exited {
                inner.exited_tabs.insert((id.clone(), tab), code);
                super::terminal_link::exit(&inner.links, id, tab, code);
            }
            Ok(())
        }
    }
}

/// What a new size does to a tab, for a view's link.
pub(super) fn apply_resize(
    inner: &mut super::Inner,
    id: &SessionId,
    tab: usize,
    columns: u16,
    rows: u16,
) -> Result<(), DaemonError> {
    {
        {
            let session = inner.session_mut(id)?;
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
                inner.terminal_data(id, tab, screen);
            }
            Ok(())
        }
    }
}
