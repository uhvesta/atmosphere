//! Ask (PR-19, D59, D200): a question about one line, range, hunk or file of a
//! pull request, typed into the review chat with where it points.
//!
//! The review chat is the session's agent terminal, so the question goes the
//! way a sent batch of comments does: the daemon's terminal sink types it as
//! one message and presses return, keyed once so a retry types nothing more.
//! It is never stored as a comment and never goes to GitHub.

use std::sync::Arc;

use humata2_clients::comments::{Anchor, Side};
use humata2_clients::model::SessionId;
use humata2_clients::{Clock, Daemon, DaemonError};
use tca::{Effect, Reducer};

/// What Ask holds: only what went wrong last.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AskState {
    /// The last failure, as text for the person; cleared by the next Ask.
    pub error: Option<String>,
    /// Questions asked so far, so keys made at one instant differ.
    asked: u64,
    /// The review the last question went to, so its failure shows there
    /// and nowhere else.
    asked_in: Option<SessionId>,
}

impl AskState {
    /// The last failure, if the last question went to `session`.
    #[must_use]
    pub fn error_for(&self, session: &SessionId) -> Option<&str> {
        self.error
            .as_deref()
            .filter(|_| self.asked_in.as_ref() == Some(session))
    }
}

/// What happens to a question.
#[derive(Clone, Debug, PartialEq)]
pub enum AskAction {
    /// Type this question about `anchor` into the review chat.
    Ask {
        /// The review session.
        session: SessionId,
        /// What it is about: repo, file, lines, side and the quoted code.
        anchor: Anchor,
        /// What the person asks.
        question: String,
    },
    /// The terminal answered.
    Typed(Result<(), DaemonError>),
    /// Forget the last failure.
    DismissError,
}

/// The message the review chat gets: where the question points, the quoted
/// code, then the question.
#[must_use]
pub fn question_text(anchor: &Anchor, question: &str) -> String {
    let lines = if anchor.first == anchor.last {
        format!("line {}", anchor.first)
    } else {
        format!("lines {}-{}", anchor.first, anchor.last)
    };
    let side = match anchor.side {
        Side::Old => "before the change",
        Side::New => "after the change",
    };
    let mut text = format!("A question about {} {} {lines} ({side}).", anchor.repo, anchor.path);
    for line in &anchor.quoted {
        text.push_str("\n> ");
        text.push_str(line);
    }
    text.push('\n');
    text.push_str(question.trim());
    text
}

/// The Ask reducer.
#[derive(Clone)]
pub struct Ask {
    daemon: Arc<dyn Daemon>,
    clock: Arc<dyn Clock>,
}

impl Ask {
    /// A reducer that types through `daemon` and keys on `clock`.
    #[must_use]
    pub fn new(daemon: Arc<dyn Daemon>, clock: Arc<dyn Clock>) -> Self {
        Self { daemon, clock }
    }
}

impl Reducer for Ask {
    type State = AskState;
    type Action = AskAction;

    fn reduce(&self, state: &mut AskState, action: AskAction) -> Effect<AskAction> {
        match action {
            AskAction::Ask { session, anchor, question } => {
                if question.trim().is_empty() {
                    return Effect::none();
                }
                state.error = None;
                state.asked += 1;
                state.asked_in = Some(session.clone());
                let key = format!("ask/{session}/{}-{}", self.clock.now().as_millis(), state.asked);
                let call = self.daemon.send_keyed(&session, &question_text(&anchor, &question), &key);
                Effect::future(async move { AskAction::Typed(call.await) })
            }
            AskAction::Typed(Ok(())) => Effect::none(),
            AskAction::Typed(Err(error)) => {
                state.error = Some(error.to_string());
                Effect::none()
            }
            AskAction::DismissError => {
                state.error = None;
                Effect::none()
            }
        }
    }
}
