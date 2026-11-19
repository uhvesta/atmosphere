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

/// D798: `session`'s structured channel dropped (the Codex link lost its
/// thread). The screen fallback reads its asks again until it is heard.
pub(crate) fn unheard(session: &str) {
    let heard = HEARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(sender) = heard.get(session) {
        sender.send_replace(false);
    }
}

/// `session`'s agent exited: the next one launched must be heard again
/// before the screen fallback goes quiet or a first message is sent.
pub(crate) fn forget(session: &str) {
    let mut heard = HEARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    heard.remove(session);
    drop(heard);
    release_sends(session);
}

/// D797: how long one of Humata's own sends waits behind a held first
/// message before it is refused.
const SEND_WAITS: Duration = Duration::from_secs(30);

/// Sessions whose held first message hasn't been typed yet, as a watch
/// each that turns true once it has.
static PENDING: LazyLock<Mutex<HashMap<String, watch::Sender<bool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn hold_sends(session: &str) {
    let mut pending = PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    pending.insert(session.to_owned(), watch::channel(false).0);
}

fn release_sends(session: &str) {
    let mut pending = PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(sender) = pending.remove(session) {
        sender.send_replace(true);
    }
}

/// One of Humata's own sends to `session` waited [`SEND_WAITS`] behind the
/// first message, which still waits for the agent. Nothing was typed.
#[derive(Debug, thiserror::Error)]
#[error("session {session}'s agent hasn't said it is ready, and its start message still waits for it")]
pub(crate) struct StillStarting {
    session: String,
}

/// The refusal a caller gets for `still`.
pub(crate) fn refusal(still: &StillStarting) -> tonic::Status {
    status::failure(
        tonic::Code::FailedPrecondition,
        humata2_grpc::humata::v1::Error {
            message: format!(
                "{still}, so nothing was typed: the text would have gone in ahead of the start message, or into a screen the agent shows before it is up"
            ),
            fix: "look at the session's terminal: if the agent waits there (a trust or login screen), answer it, or pick Send it now on its \"waiting in its terminal\" card in the Inbox. Then send again".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// D797: waits, at most [`SEND_WAITS`], while `session`'s first message is
/// held for the ready signal, so one of Humata's own sends goes after it.
/// A session with nothing held passes at once.
pub(crate) async fn behind_start(session: &str) -> Result<(), StillStarting> {
    let rx = {
        let pending = PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.get(session).map(watch::Sender::subscribe)
    };
    let Some(mut rx) = rx else {
        return Ok(());
    };
    match tokio::time::timeout(SEND_WAITS, rx.wait_for(|sent| *sent)).await {
        Ok(_) => Ok(()),
        Err(_) => Err(StillStarting {
            session: session.to_owned(),
        }),
    }
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
        let id_for_sends = session.to_owned();
        Box::pin(async move {
            let found = tokio::task::spawn_blocking(move || {
                let record = store.session(&id).ok().flatten()?;
                agents.get(&record.agent).ok().map(|kind| kind.conversation_format)
            })
            .await;
            let expects = matches!(found, Ok(Some(format)) if has_signal(format));
            // Set before the first message is held, so a send that comes
            // right after the start already waits behind it.
            if expects {
                hold_sends(&id_for_sends);
            }
            expects
        })
    }

    fn delivered(&self, session: &str) {
        release_sends(session);
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
            let agent = crate::identity::agent_caller_of(&record, &machine);
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
