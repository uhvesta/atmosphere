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
    let orange = "\x1b[38;2;255;159;10m";
    let color = if line.text.contains("Backfill entries from")
        || line.text.contains("Backfilling entries")
        || line.text.contains("waiting for your approval")
        || line.text.starts_with("? ")
        || line.text.contains("Working (")
        // An agent's own progress line, whatever it says it is doing (frame 37).
        || line.text.starts_with("✻ ")
    {
        orange
    } else {
        color
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
    let agent = if tab == 0 {
        session.agent
    } else {
        session
            .terminal_agents
            .get(&tab)
            .copied()
            .unwrap_or(Agent::Shell)
    };
    if tab == 0
        && agent != Agent::Shell
        && !matches!(session.status, Status::Exited { .. } | Status::NeedsYou)
    {
        bytes.extend(agent_input_box());
    } else if tab != 0 && agent == Agent::Shell {
        bytes.extend_from_slice(b"\r\n$ ");
    } else if tab == 0
        && agent == Agent::Shell
        && !matches!(session.status, Status::Exited { .. } | Status::NeedsYou)
    {
        bytes.extend_from_slice(b"\r\n$ ");
    } else {
        // A program waiting on a menu, or one that has exited, shows no cursor
        // (CSI ?25l), as a real one would.
        bytes.extend_from_slice(b"\x1b[?25l");
    }
    bytes
}

/// The agent's own input box and hint pinned to the terminal's last rows.
/// The size is not known here, so it works from the bottom edge: autowrap is
/// off, so a line longer than the terminal ends in its last column, and the
/// right border is written there.
fn agent_input_box() -> Vec<u8> {
    let rule = "\u{2500}".repeat(400);
    format!(
        "\x1b[?7l\x1b[999;1H\x1b[3A\x1b[90m\x1b[2K╭{rule}╮\r\n\
         \x1b[2K│\x1b[0m > \x1b[90m\x1b[999G│\r\n\
         \x1b[2K╰{rule}╯\r\n\
         \x1b[2K  ⏵⏵ accept edits on (shift+tab to cycle)\x1b[0m\x1b[?7h\x1b[2A\x1b[5G"
    )
    .into_bytes()
}
