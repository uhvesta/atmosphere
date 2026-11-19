//! The opening prompt of an agent opened from New agent tab (D857). The
//! agent opens as a new shell tab; once the shell has painted and gone
//! quiet, the agent's command is typed, and once the agent has painted and
//! gone quiet, the prompt is typed. Nothing is typed if the person typed
//! first or the whole sequence passes its deadline.

use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

use humata2_clients::model::{Agent, SessionId};
use humata2_clients::daemon::ServerFrame;
use humata2_clients::{Clients, DaemonError, TerminalLink};

/// How long the tab must stay silent, after it painted, before it is taken
/// to be waiting for input.
pub(crate) const QUIET: Duration = Duration::from_millis(500);

/// How long the whole sequence has, from the tab opening.
pub(crate) const DEADLINE: Duration = Duration::from_secs(15);

/// A prompt waiting for its new tab to be ready.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpeningPrompt {
    /// The session whose new tab gets it.
    pub(crate) session: SessionId,
    /// The agent to start in the new shell tab.
    pub(crate) agent: Agent,
    /// What to type once the agent is ready.
    pub(crate) text: String,
    /// The new tab, once it is known.
    pub(crate) tab: Option<usize>,
}

impl OpeningPrompt {
    /// A prompt for the next tab opened in `session`.
    pub(crate) fn new(session: SessionId, agent: Agent, text: String) -> Self {
        Self {
            session,
            agent,
            text,
            tab: None,
        }
    }

    /// Whether it waits for tab `tab` of `session`.
    pub(crate) fn waits_for(&self, session: &SessionId, tab: usize) -> bool {
        self.session == *session && self.tab == Some(tab)
    }
}

type Sleep = Pin<Box<dyn Future<Output = ()> + Send>>;

/// What ended one step of a wait.
enum Step {
    Frame(Option<ServerFrame>),
    Quiet,
    Deadline,
}

/// Whether `frame` shows the tab has drawn something.
fn paints(frame: &ServerFrame) -> bool {
    match frame {
        ServerFrame::Snapshot { offset, .. } => *offset > 0,
        ServerFrame::Output { .. } => true,
        _ => false,
    }
}

/// A view on one tab that waits for it to paint and go quiet, on the
/// clients' clock.
struct Waiter {
    clients: Clients,
    link: TerminalLink,
    deadline: Sleep,
    quiet: Option<Sleep>,
}

impl Waiter {
    /// Waits until the tab has painted and been silent for [`QUIET`]
    /// (`true`), or the deadline passed, it exited, failed or closed
    /// (`false`).
    async fn quiet(&mut self) -> bool {
        loop {
            let step = std::future::poll_fn(|cx| {
                if let Poll::Ready(frame) = self.link.frames.poll_recv(cx) {
                    return Poll::Ready(Step::Frame(frame));
                }
                if self.deadline.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(Step::Deadline);
                }
                if let Some(quiet) = self.quiet.as_mut()
                    && quiet.as_mut().poll(cx).is_ready()
                {
                    return Poll::Ready(Step::Quiet);
                }
                Poll::Pending
            })
            .await;
            match step {
                Step::Quiet => return true,
                Step::Deadline | Step::Frame(None) => return false,
                Step::Frame(Some(frame)) => match frame {
                    ServerFrame::Exited { .. } | ServerFrame::Error { .. } => return false,
                    _ if paints(&frame) || self.quiet.is_some() => {
                        self.quiet = Some(self.clients.clock.sleep(QUIET));
                    }
                    _ => {}
                },
            }
        }
    }

    /// Forgets the screen so far: what follows must paint afresh.
    fn start_over(&mut self) {
        self.quiet = None;
    }
}

/// Runs the sequence in tab `tab` of `prompt.session`: the shell's first
/// screen, the agent's command, the agent's first screen, the prompt. Gives
/// up quietly (`Ok`) when the tab fails or time runs out; typing that is
/// refused is an error.
pub(crate) async fn run(
    clients: Clients,
    prompt: OpeningPrompt,
    tab: usize,
    keys: [String; 2],
) -> Result<(), DaemonError> {
    let deadline = clients.clock.sleep(DEADLINE);
    let Ok(link) = clients.terminals.link_unsized(&prompt.session, tab).await else {
        return Ok(());
    };
    let mut waiter = Waiter {
        clients: clients.clone(),
        link,
        deadline,
        quiet: None,
    };
    let [command_key, prompt_key] = keys;
    if !waiter.quiet().await {
        return Ok(());
    }
    clients
        .daemon
        .send_to_tab(&prompt.session, tab, prompt.agent.command(), &command_key)
        .await?;
    waiter.start_over();
    if !waiter.quiet().await {
        return Ok(());
    }
    clients
        .daemon
        .send_to_tab(&prompt.session, tab, &prompt.text, &prompt_key)
        .await
}
