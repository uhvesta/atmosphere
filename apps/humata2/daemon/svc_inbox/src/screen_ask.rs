//! D845 (replacing D767, D781, D785 and D786's grow): a dialog an agent's CLI
//! asks on its own screen, such as Copilot's folder trust, never raises an
//! Inbox card, whatever the agent's "Ask in the Inbox" switch says (D782). No
//! hook fires for it, so each time the agent's output goes quiet its watch
//! looks at the screen ([`ScreenAsk::quiet`]); [`screen_question::asked_on`]
//! says whether it shows a dialog.
//!
//! - While the screen shows one, the session needs you (NEEDS_INPUT) with
//!   no card: a card-less hold, kept until a look sees the dialog gone.
//! - Humata never answers it and types nothing. The person answers in the
//!   agent's terminal.
//! - While the hold stands, the agent's output also has the screen read
//!   again ([`ScreenAsk::output`]), at most every [`RELOOK_EVERY`], so a
//!   dialog answered in the terminal ends the hold though the output never
//!   goes quiet (Copilot's animated logo).
//! - A dialog whose agent exited ends with it ([`ScreenAsk::gone`]).
//! - A session whose agent was heard on its structured channel since its
//!   launch (Claude's hooks, Codex's link) asks there: its screen is not
//!   read for a dialog (D798).
//!
//! The input gate that keeps typed text out of a dialog (D762) and the held
//! first message (D763) don't live here.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::Duration;

use humata2_daemon_runtime::screen_question::{self, ScreenQuestion};
use humata2_store::{AgentState, Store, StoreError, TabKind, TabRecord};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::ports::{AskingGuardPort, ScreenHoldersPort};

/// The most often an agent's output has its screen read again while its
/// dialog is up. A throttle, not a wait for quiet: output that never stops
/// still gets a read this often.
const RELOOK_EVERY: Duration = Duration::from_millis(300);

static ASKS: OnceLock<ScreenAsk> = OnceLock::new();

/// What looks at agents' screens, for the daemon's life.
#[derive(Clone)]
pub struct ScreenAsk(Arc<Asks>);

struct Asks {
    holders: Arc<dyn ScreenHoldersPort>,
    asking: Arc<dyn AskingGuardPort>,
    store: Arc<Store>,
    is_heard: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    forget: Arc<dyn Fn(&str) + Send + Sync>,
    /// The dialog each session's screen shows, by session.
    open: Mutex<HashMap<String, Open>>,
    /// Which sessions' output reads their screen again. Never held across
    /// an await.
    watching: std::sync::Mutex<Watching>,
}

/// Sessions whose dialog is up, so their output reads the screen again, and
/// those with a read already due.
#[derive(Default)]
struct Watching {
    asking: HashSet<String>,
    due: HashSet<String>,
}

/// A dialog a session's screen shows, and the hold on its Needs you.
struct Open {
    holder_pid: u32,
    question: ScreenQuestion,
    /// Holds the session's Needs you; aborted (and so let go) when a look
    /// sees the dialog gone or the agent is.
    task: JoinHandle<()>,
}

impl Drop for Open {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ScreenAsk {
    /// Creates a new [`ScreenAsk`] watcher.
    pub fn new(
        holders: Arc<dyn ScreenHoldersPort>,
        asking: Arc<dyn AskingGuardPort>,
        store: Arc<Store>,
        is_heard: Arc<dyn Fn(&str) -> bool + Send + Sync>,
        forget: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        Self(Arc::new(Asks {
            holders,
            asking,
            store,
            is_heard,
            forget,
            open: Mutex::new(HashMap::new()),
            watching: std::sync::Mutex::new(Watching::default()),
        }))
    }

    /// `session`'s agent, under holder `holder_pid`, wrote output: while its
    /// dialog is up, reads the screen again soon. Returns at once.
    pub fn output(&self, session: &str, holder_pid: u32) {
        let asks = &self.0;
        {
            let mut watching = asks.watching();
            if !watching.asking.contains(session) || !watching.due.insert(session.to_owned()) {
                return;
            }
        }
        let asks = Arc::clone(asks);
        let session = session.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(RELOOK_EVERY).await;
            // Due no more before the read, so output during it asks again.
            asks.watching().due.remove(&session);
            asks.look(&session, holder_pid).await;
        });
    }

    /// `session`'s agent, under holder `holder_pid`, went quiet: looks at its
    /// screen for a dialog. Returns at once.
    pub fn quiet(&self, session: &str, holder_pid: u32) {
        let asks = Arc::clone(&self.0);
        let session = session.to_owned();
        tokio::spawn(async move { asks.look(&session, holder_pid).await });
    }

    /// `session`'s agent exited or its holder is gone: ends its hold.
    pub fn gone(&self, session: &str) {
        (self.0.forget)(session);
        let asks = Arc::clone(&self.0);
        let session = session.to_owned();
        tokio::spawn(async move {
            let mut open = asks.open.lock().await;
            open.remove(&session);
            asks.watching().asking.remove(&session);
        });
    }
}

/// Starts looking at agents' screens. Called once, when the daemon serves.
pub fn start(
    holders: Arc<dyn ScreenHoldersPort>,
    asking: Arc<dyn AskingGuardPort>,
    store: Arc<Store>,
    is_heard: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    forget: Arc<dyn Fn(&str) + Send + Sync>,
) {
    let _ = ASKS.set(ScreenAsk::new(holders, asking, store, is_heard, forget));
}

/// `session`'s agent, under holder `holder_pid`, wrote output: while its
/// dialog is up, reads the screen again soon. Returns at once.
pub fn output(session: &str, holder_pid: u32) {
    if let Some(asks) = ASKS.get() {
        asks.output(session, holder_pid);
    }
}

/// `session`'s agent, under holder `holder_pid`, went quiet: looks at its
/// screen for a dialog. Returns at once.
pub fn quiet(session: &str, holder_pid: u32) {
    if let Some(asks) = ASKS.get() {
        asks.quiet(session, holder_pid);
    }
}

/// `session`'s agent exited or its holder is gone: ends its hold.
pub fn gone(session: &str) {
    if let Some(asks) = ASKS.get() {
        asks.gone(session);
    }
}

/// Keeps `asking` (the session's Needs you) until the task running it is
/// aborted.
async fn hold(asking: Box<dyn Send + Sync>) {
    let _asking = asking;
    std::future::pending::<()>().await;
}

impl Asks {
    fn watching(&self) -> std::sync::MutexGuard<'_, Watching> {
        self.watching.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn look(self: &Arc<Self>, session: &str, holder_pid: u32) {
        // D798: an agent heard on its structured channel asks there; its
        // screen holds nothing. A hold taken while the channel was down
        // stays until its screen stops showing the dialog.
        let heard = (self.is_heard)(session);
        if heard && !self.open.lock().await.contains_key(session) {
            return;
        }
        let Some(tab) = self.agent_tab(session, holder_pid).await else {
            return;
        };
        let Ok(screen) = self.holders.screen_at(&tab.holder_socket).await else {
            return;
        };
        // D864: an offer (Claude's auto-mode tip) only stops the input
        // gate; it holds no Needs you.
        let seen = screen_question::asked_on(&screen).filter(|question| question.needs_you());
        let mut open = self.open.lock().await;
        let Some(question) = seen.filter(|_| !heard) else {
            // Gone from the screen: the hold ends with its entry.
            open.remove(session);
            self.watching().asking.remove(session);
            return;
        };
        if open
            .get(session)
            .is_some_and(|now| (now.holder_pid, now.question) == (holder_pid, question))
        {
            return;
        }
        open.insert(
            session.to_owned(),
            Open {
                holder_pid,
                question,
                task: tokio::spawn(hold(self.asking.hold(session))),
            },
        );
        self.watching().asking.insert(session.to_owned());
    }

    /// The session's agent tab, while it runs under `holder_pid`.
    async fn agent_tab(&self, session: &str, holder_pid: u32) -> Option<TabRecord> {
        let (store, key) = (self.store.clone(), session.to_owned());
        let read = tokio::task::spawn_blocking(move || -> Result<Option<TabRecord>, StoreError> {
            let Some(agent) = store.agent(&key)? else {
                return Ok(None);
            };
            if agent.state != (AgentState::Running { holder_pid }) {
                return Ok(None);
            }
            Ok(store.tab(&agent.tab)?.filter(|tab| tab.kind == TabKind::Agent))
        })
        .await;
        read.ok()?.ok()?
    }
}
