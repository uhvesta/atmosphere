//! A session's first message, typed into its agent once the agent's input
//! is clear (BUG-T1b, D763). An agent's CLI may ask a question of its own
//! first, such as Copilot's folder trust: the message waits for the person
//! to answer it, is never typed into it, and a return never answers it.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use humata2_holder::{HeldReturn, HolderError, Ready, Typed};
pub use humata2_revive::{GateRefusal, WriteGate};

/// How long a first message waits for the agent's input to clear and show
/// it before the person is told it wasn't sent.
const ANSWER_WAIT: Duration = Duration::from_secs(30 * 60); // D763 (Provisional)

/// Where a first message that waits is told.
pub trait HeldMessages: Send + Sync {
    /// Kept while `session`'s first message waits: the session needs the
    /// person meanwhile.
    fn waiting(&self, session: &str) -> Box<dyn Send>;
    /// `text`, `session`'s first message, wasn't sent in time: the person
    /// is told, with the text, so they can send it themselves or have
    /// `resend` try again.
    fn unsent(&self, session: &str, text: &str, resend: Resend);
}

/// Tries a first message that wasn't sent again, as a new start would.
pub type Resend = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// How a start's first message reaches its agent.
#[derive(Clone)]
pub struct FirstMessage {
    gate: Option<Arc<dyn WriteGate>>,
    held: Option<Arc<dyn HeldMessages>>,
    wait: Duration,
}

impl Default for FirstMessage {
    fn default() -> Self {
        Self {
            gate: None,
            held: None,
            wait: ANSWER_WAIT,
        }
    }
}

impl FirstMessage {
    /// Typing waits for `gate` to say the agent's input is clear.
    pub fn with_gate(mut self, gate: Arc<dyn WriteGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    /// A message that waits is told to `held`.
    pub fn with_held(mut self, held: Arc<dyn HeldMessages>) -> Self {
        self.held = Some(held);
        self
    }

    /// How long a message waits before the person is told it wasn't sent.
    pub fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    fn ready(&self, socket: &Path) -> Option<Ready> {
        let gate = self.gate.clone()?;
        let socket = socket.to_owned();
        Some(Arc::new(move || {
            let guarded = gate.guard(&socket);
            Box::pin(async move { guarded.await.is_ok() })
        }))
    }

    /// Types `message` into `session`'s agent on `socket` and presses
    /// return once the agent shows it. When the input isn't clear, or the
    /// agent doesn't show it, the rest goes on in the background, and the
    /// start doesn't wait for it.
    pub(crate) async fn send(
        &self,
        session: &str,
        socket: &Path,
        message: &str,
    ) -> Result<(), HolderError> {
        let ready = self.ready(socket);
        let clear = match &ready {
            Some(ready) => ready().await,
            None => true,
        };
        let held = if clear {
            match humata2_holder::type_then_return(socket, message.as_bytes(), TYPE_PAUSE).await? {
                Typed::Submitted => {
                    // Looked at in the background: the start doesn't wait.
                    if let Some(gate) = self.gate.clone() {
                        let (socket, message) = (socket.to_owned(), message.to_owned());
                        tokio::spawn(async move { confirm_logged(&*gate, &socket, &message).await });
                    }
                    return Ok(());
                }
                Typed::Held(held) => Some(held),
            }
        } else {
            None
        };
        let follow = Follow {
            session: session.to_owned(),
            socket: socket.to_owned(),
            message: message.to_owned(),
            held,
            ready,
            first: self.clone(),
        };
        tokio::spawn(follow.run());
        Ok(())
    }
}

/// The pause before the return (D671).
pub(crate) const TYPE_PAUSE: Duration = Duration::from_millis(400); // D671 (Provisional): own write after a longer pause

/// A first message still on its way.
struct Follow {
    session: String,
    socket: PathBuf,
    message: String,
    held: Option<HeldReturn>,
    ready: Option<Ready>,
    first: FirstMessage,
}

impl Follow {
    // Boxed with its `Send` spelled out: what it tells the person can start
    // another follow.
    fn run(self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(self.follow())
    }

    async fn follow(self) {
        let _waiting = self.first.held.as_ref().map(|held| held.waiting(&self.session));
        let sent = humata2_holder::deliver(
            &self.socket,
            self.message.as_bytes(),
            TYPE_PAUSE,
            self.held,
            self.ready,
            self.first.wait,
        )
        .await;
        if matches!(sent, Ok(true)) {
            if let Some(gate) = &self.first.gate {
                confirm_logged(&**gate, &self.socket, &self.message).await;
            }
        }
        // Not sent for whatever reason, the agent's end included: the
        // person still has the text to send.
        if !matches!(sent, Ok(true)) {
            if let Some(held) = &self.first.held {
                let again = Follow {
                    session: self.session.clone(),
                    socket: self.socket.clone(),
                    message: self.message.clone(),
                    held: None,
                    ready: self.first.ready(&self.socket),
                    first: self.first.clone(),
                };
                held.unsent(&self.session, &self.message, Box::new(move || again.run()));
            }
        }
    }
}

/// How long after its return a first message's agent gets to take it before
/// its input is looked at (D763 amendment).
const SUBMIT_WAIT: Duration = Duration::from_secs(3); // D763 (Provisional)

/// What became of the return pressed after a first message the agent showed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Confirmed {
    /// The agent's input no longer shows the message: it took it.
    Taken,
    /// Its input still showed the message, so return was pressed once more.
    PressedAgain,
    /// Its input still showed the message, but a question of its own was up
    /// or the screen couldn't be read, so nothing more was pressed.
    LeftShown,
}

/// Sees that a first message the agent showed, and was pressed return
/// after, was taken: once `wait` has passed, if the agent's input still
/// shows `text`, return is pressed (`press`) once more, and only while the
/// gate says the input is clear, so it never answers a question.
async fn confirm<P, F>(
    gate: &dyn WriteGate,
    socket: &Path,
    text: &str,
    wait: Duration,
    press: P,
) -> Result<Confirmed, HolderError>
where
    P: FnOnce() -> F,
    F: Future<Output = Result<(), HolderError>>,
{
    tokio::time::sleep(wait).await;
    if !gate.unsent(socket, text).await {
        return Ok(Confirmed::Taken);
    }
    if gate.guard(socket).await.is_err() {
        return Ok(Confirmed::LeftShown);
    }
    press().await?;
    Ok(Confirmed::PressedAgain)
}

/// [`confirm`] on the holder at `socket`, saying what went wrong.
async fn confirm_logged(gate: &dyn WriteGate, socket: &Path, text: &str) {
    let press = || async {
        let humata2_holder::Attachment { mut control, .. } =
            humata2_holder::attach(socket, u64::MAX).await?;
        control.input(1, b"\r").await
    };
    match confirm(gate, socket, text, SUBMIT_WAIT, press).await {
        Ok(Confirmed::Taken | Confirmed::PressedAgain) => {}
        Ok(Confirmed::LeftShown) => eprintln!(
            "humatad2: a session's first message still shows in its agent's input, but the agent now asks a question of its own (or its screen can't be read), so return wasn't pressed again; press Enter in the session once you have answered it"
        ),
        Err(error) => eprintln!(
            "humatad2: couldn't press return again for a session's first message that still shows in its agent's input; press Enter in the session: {error}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// A gate that answers from scripts and counts nothing else.
    #[derive(Debug)]
    struct Scripted {
        unsent: bool,
        clear: bool,
    }

    impl WriteGate for Scripted {
        fn guard(
            &self,
            _socket: &Path,
        ) -> Pin<Box<dyn Future<Output = Result<(), GateRefusal>> + Send + 'static>> {
            let clear = self.clear;
            Box::pin(async move { if clear { Ok(()) } else { Err(GateRefusal::Question) } })
        }

        fn unsent(&self, _socket: &Path, _text: &str) -> Pin<Box<dyn Future<Output = bool> + Send + 'static>> {
            let unsent = self.unsent;
            Box::pin(async move { unsent })
        }
    }

    async fn presses(unsent: bool, clear: bool) -> (Confirmed, usize) {
        let pressed = Mutex::new(0);
        let confirmed = confirm(&Scripted { unsent, clear }, Path::new("/nowhere"), "hello", Duration::ZERO, || async {
            *pressed.lock().unwrap() += 1;
            Ok(())
        })
        .await
        .unwrap();
        (confirmed, pressed.into_inner().unwrap())
    }

    #[tokio::test]
    async fn a_message_the_agent_took_gets_no_second_return() {
        assert_eq!(presses(false, true).await, (Confirmed::Taken, 0));
    }

    #[tokio::test]
    async fn a_message_still_in_the_input_gets_one_more_return() {
        assert_eq!(presses(true, true).await, (Confirmed::PressedAgain, 1));
    }

    #[tokio::test]
    async fn a_message_still_in_the_input_behind_a_question_gets_no_return() {
        assert_eq!(presses(true, false).await, (Confirmed::LeftShown, 0));
    }
}
