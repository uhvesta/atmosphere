//! D797: a start's first message to an agent with a structured ready signal
//! waits for that signal, not for the agent's screen. Claude's signal is a
//! SessionStart hook (it runs only once workspace trust is accepted) and
//! Codex's is the wrapper's thread/started; both reach the daemon as a
//! `conversation` hook, and any such hook from a session counts as hearing
//! it. There is no deadline: twenty seconds after the launch, a session not
//! heard from raises an Inbox card saying its agent is waiting in its
//! terminal, with the held message and one action, "Send it now". The
//! signal withdraws the card. Keys the person types are never gated.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

use humata2_sessions::{Agents, Boxed, ConversationFormat, StartSignal};
use humata2_store::Store;
use tokio::sync::watch;

use crate::identity::{AgentCaller, Caller};
use crate::inbox::Inbox;
use crate::inbox::permission::Decision;
use crate::session_service::Asking;
use crate::status;

/// D797: how long after its launch a session not heard from raises the
/// "waiting in its terminal" card.
const CARD_AFTER: Duration = Duration::from_secs(20);

/// Which sessions have been heard from, as a watch each, so a hook that
/// comes before the first message asks still counts.
static HEARD: LazyLock<Mutex<HashMap<String, watch::Sender<bool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `session`'s watch, made if it has none.
fn watching(session: &str) -> watch::Sender<bool> {
    let mut heard = HEARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    heard
        .entry(session.to_owned())
        .or_insert_with(|| watch::channel(false).0)
        .clone()
}

/// A conversation hook of `session`'s agent reached the daemon: the agent
/// is up and takes its first message.
pub(crate) fn heard(session: &str) {
    watching(session).send_replace(true);
}

/// D798: whether `session`'s agent has been heard on its structured
/// channel since it was launched. While it has, the screen fallback
/// (`screen_ask`) raises nothing for it.
pub(crate) fn is_heard(session: &str) -> bool {
    let heard = HEARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    heard.get(session).is_some_and(|sender| *sender.borrow())
}

/// `session`'s agent exited: the next one launched must be heard again
/// before the screen fallback goes quiet or a first message is sent.
pub(crate) fn forget(session: &str) {
    let mut heard = HEARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    heard.remove(session);
}

/// Resolves once `rx` says the session was heard.
async fn until_heard(rx: &mut watch::Receiver<bool>) {
    let _ = rx.wait_for(|heard| *heard).await;
}

/// Where the card is raised.
struct Told {
    inbox: Arc<Inbox>,
    store: Arc<Store>,
    machine: String,
    agents: Agents,
}

/// The ready signal as a start's first message asks for it.
#[derive(Default)]
pub(crate) struct Ready {
    /// Set once the Inbox is made, after the starter.
    told: OnceLock<Told>,
}

impl Ready {
    pub(crate) fn tell(&self, inbox: Arc<Inbox>, store: Arc<Store>, machine: String, agents: Agents) {
        let _ = self.told.set(Told { inbox, store, machine, agents });
    }
}

/// Whether `format`'s agent gives a ready signal: Claude's SessionStart
/// and Codex's thread/started. Copilot and the rest keep the screen check.
fn has_signal(format: ConversationFormat) -> bool {
    matches!(format, ConversationFormat::ClaudeJsonl | ConversationFormat::CodexRollout)
}

impl StartSignal for Ready {
    fn expects(&self, session: &str) -> Boxed<bool> {
        let Some(told) = self.told.get() else {
            return Box::pin(async { false });
        };
        let (store, agents, id) = (told.store.clone(), told.agents.clone(), session.to_owned());
        Box::pin(async move {
            let found = tokio::task::spawn_blocking(move || {
                let record = store.session(&id).ok().flatten()?;
                agents.get(&record.agent).ok().map(|kind| kind.conversation_format)
            })
            .await;
            matches!(found, Ok(Some(format)) if has_signal(format))
        })
    }

    fn released(&self, session: &str, text: &str) -> Boxed<()> {
        let session = session.to_owned();
        let text = text.to_owned();
        let told = self.told.get().map(|told| {
            (told.inbox.clone(), told.store.clone(), told.machine.clone(), told.agents.clone())
        });
        Box::pin(async move {
            let mut rx = watching(&session).subscribe();
            if tokio::time::timeout(CARD_AFTER, until_heard(&mut rx)).await.is_ok() {
                return;
            }
            let Some((inbox, store, machine, agents)) = told else {
                until_heard(&mut rx).await;
                return;
            };
            let id = session.clone();
            let found = tokio::task::spawn_blocking(move || store.session(&id)).await;
            let Ok(Ok(Some(record))) = found else {
                crate::log::say!(
                    "humatad2: session {session}'s agent hasn't been heard from and the session couldn't be read to say so in the Inbox; its start message waits for the agent, or type it into the session yourself: {text}"
                );
                until_heard(&mut rx).await;
                return;
            };
            // The session needs you while the card waits.
            let _asking = Asking::hold(&session);
            let agent = AgentCaller::of(&record, &machine);
            let caller = Caller::Agent(agent.clone());
            let name = agents.person_name(&record.agent);
            let mut wait = match inbox.raise_waiting(&caller, &agent, &name, text).await {
                Ok(wait) => wait,
                Err(error) => {
                    crate::log::say!(
                        "humatad2: couldn't say in the Inbox that session {session}'s agent is waiting in its terminal ({}); its start message waits for the agent",
                        status::chain(&error)
                    );
                    until_heard(&mut rx).await;
                    return;
                }
            };
            // Only the person's Send it now sends; a Deny or a withdrawn
            // card leaves the message waiting for the signal.
            let sent_now = tokio::select! {
                biased;
                () = until_heard(&mut rx) => true,
                decision = wait.decided() => matches!(decision, Decision::Allow),
            };
            if !sent_now {
                drop(wait);
                until_heard(&mut rx).await;
            }
        })
    }
}
