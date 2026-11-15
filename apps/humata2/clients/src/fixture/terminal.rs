use crate::model::{Agent, Line, Session, Speaker, Status};

/// Deterministic output from a second program in the sample session.
pub(crate) fn extra_terminal(agent: Agent, repo: &str, branch: &str) -> Vec<Line> {
    let mut lines = vec![Line {
        from: Speaker::System,
        text: format!("~/src/{repo} ({branch}) $ {}", agent.command()),
    }];
    if agent == Agent::Codex {
        lines.extend(
            [
                (
                    Speaker::You,
                    "> Review the migration Claude is writing; don't edit files.",
                ),
                (Speaker::Agent, ""),
                (Speaker::Agent, "• Read migrations/0042_accounts.sql"),
                (Speaker::Agent, "• Read api/src/client.rs"),
                (Speaker::Agent, ""),
                (
                    Speaker::Agent,
                    "  The dual write isn't in a transaction: a crash between",
                ),
                (
                    Speaker::Agent,
                    "  insert_entry and insert_legacy leaves them out of step.",
                ),
                (Speaker::Agent, "• Working (41s • esc to interrupt)"),
            ]
            .into_iter()
            .map(|(from, text)| Line {
                from,
                text: text.into(),
            }),
        );
    }
    lines
}

/// Deterministic fake-program output; the UI consumes the same ANSI stream as a PTY.
pub(crate) fn terminal_line(line: &Line) -> Vec<u8> {
    let color = match line.from {
        Speaker::System => "\x1b[90m",
        Speaker::Tool => "\x1b[32m",
        _ => "\x1b[0m",
    };
    format!("{color}{}\x1b[0m\r\n", line.text.replace('\n', "\r\n")).into_bytes()
}

/// Initial output of a fake attached program, also used for new sessions/forks.
pub(crate) fn terminal_screen(session: &Session, tab: usize) -> Vec<u8> {
    let lines = if tab == 0 {
        session.transcript.clone()
    } else {
        extra_terminal(
            session
                .terminal_agents
                .get(&tab)
                .copied()
                .unwrap_or(Agent::Shell),
            &session.repo,
            &session.branch,
        )
    };
    let mut bytes = Vec::new();
    for line in lines {
        bytes.extend(terminal_line(&line));
    }
    if tab != 0 || !matches!(session.status, Status::Exited { .. } | Status::NeedsYou) {
        bytes.extend_from_slice(if tab == 0 && session.agent != Agent::Shell {
            b"\r\n> "
        } else {
            b"\r\n$ "
        });
    }
    bytes
}
