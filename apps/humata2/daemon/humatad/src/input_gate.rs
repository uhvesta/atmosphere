//! D762: the one check before humatad2 writes text into an agent's tab.
//! Every path that types into an agent (Type, the review comments' paste,
//! a rebuilt session's note, the Desk seed, and through `WriteGate` the
//! start message and revive's note) asks [`guard`] first. It reads the
//! screen the holder shows now and refuses while the agent asks a question
//! of its own there (text would go into the question, and its Enter answer
//! it), and refuses when the screen can't be read: nothing is typed blind.

use std::path::Path;
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use tonic::Status;

use crate::holders::{self, ScreenUnread};
use crate::screen_question::{self, ScreenQuestion};
use crate::status;

/// How long the gate waits for the holder's screen before it refuses.
const READ_LIMIT: Duration = Duration::from_secs(3);

/// Why text wasn't written into an agent's tab. Nothing was typed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum InputRefused {
    #[error("the agent is asking a question of its own on its screen")]
    Question(ScreenQuestion),
    #[error("humatad2 couldn't read the agent's screen, so it typed nothing rather than type blind")]
    Unreadable(#[source] ScreenUnread),
}

/// Whether text may be written into the agent's tab whose holder answers
/// on `socket` now: `Ok` when its screen shows no question of its own.
pub(crate) async fn guard(socket: &Path) -> Result<(), InputRefused> {
    let screen = tokio::time::timeout(READ_LIMIT, holders::screen_at(socket))
        .await
        .map_err(|_| InputRefused::Unreadable(ScreenUnread::TimedOut))?
        .map_err(InputRefused::Unreadable)?;
    match screen_question::asked_on(&screen) {
        Some(question) => Err(InputRefused::Question(question)),
        None => Ok(()),
    }
}

/// The refusal a caller of `session` gets for `refused`.
pub(crate) fn refusal(session: &str, refused: &InputRefused) -> Status {
    let (code, message, fix) = match refused {
        InputRefused::Question(ScreenQuestion::FolderTrust) => (
            tonic::Code::FailedPrecondition,
            format!(
                "the agent in session {session} is asking on its screen whether to trust the session's folder, so nothing was typed: the text would have gone into that question instead of the agent's input, and its Enter would have answered it"
            ),
            format!(
                "answer the folder-trust question in the Inbox, or on the session's screen, then type again. Humata never answers it for you, because answering grants the agent access to the folder. To accept it for this session only (the highlighted answer), press Enter alone with `humata session type {session} \"\"`"
            ),
        ),
        InputRefused::Unreadable(source) => (
            tonic::Code::Unavailable,
            format!(
                "humatad2 couldn't read the screen of session {session}'s agent ({}), so nothing was typed: it doesn't type into an agent without seeing that the agent's input is what shows",
                status::chain(source)
            ),
            "type again in a moment. If it keeps failing, close the session's view and open it again, or restart humatad2; the agent keeps running in its holder".to_owned(),
        ),
    };
    status::failure(
        code,
        pb::Error {
            message,
            fix,
            retry: None,
            kind: None,
        },
    )
}

/// The gate as a start's first message asks it (D763), from the sessions
/// crate, which has no screen of its own.
#[derive(Debug)]
pub(crate) struct StartGate;

impl humata2_sessions::WriteGate for StartGate {
    fn guard(
        &self,
        socket: &Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), humata2_sessions::GateRefusal>> + Send + 'static>>
    {
        let socket = socket.to_owned();
        Box::pin(async move {
            guard(&socket).await.map_err(|refused| match refused {
                InputRefused::Question(_) => humata2_sessions::GateRefusal::Question,
                InputRefused::Unreadable(_) => humata2_sessions::GateRefusal::Unreadable,
            })
        })
    }

    fn unsent(
        &self,
        socket: &Path,
        text: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'static>> {
        let (socket, text) = (socket.to_owned(), text.to_owned());
        Box::pin(async move {
            match tokio::time::timeout(READ_LIMIT, holders::screen_at(&socket)).await {
                Ok(Ok(screen)) => input_shows(&screen, &text),
                // Can't tell: nothing is pressed again.
                Ok(Err(_)) | Err(_) => false,
            }
        })
    }
}

/// The marks an agent's CLI puts before its input row.
const PROMPT: &[char] = &['❯', '>'];

/// Whether `screen`'s agent input still shows the last line of `text`: the
/// last row that starts with a prompt mark, read with the rows below it in
/// case the text wraps. An input showed empty, or another prompt (a list
/// of choices) below the message, is not the message.
fn input_shows(screen: &str, text: &str) -> bool {
    let Some(last) = text.lines().map(str::trim).filter(|line| !line.is_empty()).last() else {
        return false;
    };
    let rows: Vec<&str> = screen.lines().map(unboxed).collect();
    let Some(at) = rows.iter().rposition(|row| row.starts_with(PROMPT)) else {
        return false;
    };
    let input = rows[at..].join(" ");
    let input = input.trim_start_matches(PROMPT);
    spaced(input).contains(&spaced(last))
}

/// `row` without the box drawing around it.
fn unboxed(row: &str) -> &str {
    row.trim_matches(|c: char| c.is_whitespace() || ('\u{2500}'..='\u{257f}').contains(&c))
}

/// `text` with each run of white space as one space.
fn spaced(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Copilot 1.0.113 with a first message typed into its input, and the
    /// same screen once the message was taken (its words above, input
    /// empty).
    const TYPED: &str = "\
 ● MCP Servers reloaded: 2 servers connected
 ~/work/demo
──────────────────────────────────────────────────────────
❯ reply with the single word pong
──────────────────────────────────────────────────────────
 ← open sidebar · Interactive · Manual Approval · / commands";
    const TAKEN: &str = "\
 ❯ reply with the single word pong                         18:16
 ● pong
 ~/work/demo
──────────────────────────────────────────────────────────
❯
──────────────────────────────────────────────────────────
 ← open sidebar · Interactive · Manual Approval · / commands";

    #[test]
    fn a_message_left_in_the_input_is_unsent_and_one_taken_is_not() {
        assert!(input_shows(TYPED, "reply with the single word pong"));
        assert!(!input_shows(TAKEN, "reply with the single word pong"));
    }

    #[test]
    fn a_message_wrapped_over_rows_in_the_input_is_still_seen() {
        let narrow = "│ ❯ reply with the │\n│   single word pong │\n╰────╯";
        assert!(input_shows(narrow, "reply with the single word pong"));
    }

    #[test]
    fn a_list_of_choices_below_the_message_is_not_its_input() {
        let asking = format!("{TYPED}\n│ ❯ 1. Yes │\n│   2. No  │");
        assert!(!input_shows(&asking, "reply with the single word pong"));
    }

    #[tokio::test]
    async fn a_holder_that_cant_be_reached_refuses_the_text() {
        let dir = tempfile::tempdir().unwrap();
        let refused = guard(&dir.path().join("gone.sock")).await.unwrap_err();
        assert!(matches!(refused, InputRefused::Unreadable(_)), "{refused:?}");
        assert_eq!(refusal("s1", &refused).code(), tonic::Code::Unavailable);
    }

    #[tokio::test]
    async fn a_socket_that_never_sends_a_screen_refuses_the_text() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("mute.sock");
        // Takes the connection and never answers.
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let _held = tokio::spawn(async move {
            let mut kept = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                kept.push(stream);
            }
        });
        let refused = guard(&socket).await.unwrap_err();
        assert!(matches!(refused, InputRefused::Unreadable(_)), "{refused:?}");
    }
}
