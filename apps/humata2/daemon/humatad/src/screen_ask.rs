//! D767: a question an agent's CLI asks on its own screen, such as
//! Copilot's folder trust, becomes an Inbox question. No hook fires for it,
//! so each time the agent's output goes quiet its watch looks at the screen
//! ([`quiet`]); [`screen_question::asked_on`] says whether it asks.
//!
//! - The card shows the question and every choice the screen lists, takes
//!   only one of those (or Deny), and is held while the screen asks: the
//!   session needs you meanwhile.
//! - Humata never answers it itself. Only the person's reply picks a
//!   choice, by moving the screen's highlight to it and pressing Enter, and
//!   only if the screen still asks the same question.
//! - Deny (the app's Dismiss, D781) types nothing. It looks at the screen
//!   once: a question no longer there is withdrawn as resolved; one still
//!   there is denied, and the session goes on needing you, with no card,
//!   until a look sees it gone.
//! - A question the screen no longer shows (answered in the terminal) is
//!   withdrawn as resolved; one whose agent exited, as its call ended.
//! - The card names the agent's tab, so the app shows that screen's live
//!   terminal under it (D781).
//! - While a card is up, the agent's output also has the screen read again
//!   ([`output`]), at most every [`RELOOK_EVERY`]: a box drawn in steps
//!   gets its last choices, and one answered in the terminal is withdrawn,
//!   though the output never goes quiet (Copilot's animated logo). A reply
//!   is checked against the screen as it comes: one to a question the
//!   screen no longer asks withdraws the card instead (D767 amendment).
//! - A list the screen's height cuts off, on a tab no one who can type is
//!   attached to, grows the screen to [`START_ROWS`] rows once, and the
//!   look after raises the card (D786, [`grows`]).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::Duration;

use humata2_store::{AgentState, SessionRecord, Store, StoreError, TabKind, TabRecord};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::holders::{Holders, START_ROWS};
use crate::identity::{AgentCaller, Caller};
use crate::inbox::Inbox;
use crate::inbox::permission::{Decision, Question, StillAsked};
use crate::screen_question::{self, ScreenQuestion};
use crate::status;
use crate::terminal::Writers;

/// Between the highlight's moves and the Enter that picks it, so a
/// program reading keys one at a time sees them apart.
const KEY_GAP: Duration = Duration::from_millis(150);
/// One row up and one row down, as a terminal's arrow keys send them.
const UP: &[u8] = b"\x1b[A";
const DOWN: &[u8] = b"\x1b[B";
/// The marks a screen puts before its highlighted choice.
const HIGHLIGHT: &[char] = &['❯', '›', '>', '●', '▶'];
/// The most often an agent's output has its screen read again while its
/// card is up. A throttle, not a wait for quiet: output that never stops
/// still gets a read this often.
const RELOOK_EVERY: Duration = Duration::from_millis(300);
/// How long a reply waits to read the screen it is checked against.
const CHECK_LIMIT: Duration = Duration::from_secs(3);
/// After a grow, when the screen is read again whether or not the program
/// redrew (D786: its card comes within 3 s of the grow).
const LOOK_AFTER_GROW: Duration = Duration::from_secs(1);

static ASKS: OnceLock<Arc<Asks>> = OnceLock::new();

/// What looks at agents' screens, for the daemon's life.
struct Asks {
    holders: Arc<Holders>,
    /// Each tab's attaches that can type: a grow waits for none (D786).
    writers: Arc<Writers>,
    inbox: Arc<Inbox>,
    store: Arc<Store>,
    machine: String,
    /// The question each session's screen asks, by session.
    open: Mutex<HashMap<String, Open>>,
    /// Which sessions' output reads their screen again. Never held across
    /// an await.
    watching: std::sync::Mutex<Watching>,
}

/// Sessions whose card is up, so their output reads the screen again, and
/// those with a read already due.
#[derive(Default)]
struct Watching {
    asking: HashSet<String>,
    due: HashSet<String>,
}

/// A question a session's screen asks, and the card's wait for it.
struct Open {
    holder_pid: u32,
    question: ScreenQuestion,
    /// What the card shows: a later look that reads more of the screen
    /// (a box drawn in steps) raises the card again with it.
    asked: Asked,
    /// The card's request id; `None` for D782's card-less hold.
    card: Option<String>,
    /// D786: a card-less hold a grow left while the screen redraws. It is
    /// no card shown: the next look raises the card.
    grew: bool,
    /// Holds the card and the session's Needs you; ends with the person's
    /// answer, except a Deny, which leaves the session needing you while
    /// the screen asks (D781). Ending it early withdraws a pending card.
    task: JoinHandle<()>,
}

impl Drop for Open {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Starts looking at agents' screens. Called once, when the daemon serves.
pub(crate) fn start(
    holders: Arc<Holders>,
    writers: Arc<Writers>,
    inbox: Arc<Inbox>,
    store: Arc<Store>,
    machine: String,
) {
    let _ = ASKS.set(Arc::new(Asks {
        holders,
        writers,
        inbox,
        store,
        machine,
        open: Mutex::new(HashMap::new()),
        watching: std::sync::Mutex::new(Watching::default()),
    }));
}

/// `session`'s agent, under holder `holder_pid`, wrote output: while its
/// screen's card is up, reads the screen again soon. Returns at once.
pub(crate) fn output(session: &str, holder_pid: u32) {
    let Some(asks) = ASKS.get() else {
        return;
    };
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
/// screen for a question. Returns at once.
pub(crate) fn quiet(session: &str, holder_pid: u32) {
    let Some(asks) = ASKS.get().cloned() else {
        return;
    };
    let session = session.to_owned();
    tokio::spawn(async move { asks.look(&session, holder_pid).await });
}

/// `session`'s agent exited or its holder is gone: withdraws its question.
pub(crate) fn gone(session: &str) {
    let Some(asks) = ASKS.get().cloned() else {
        return;
    };
    let session = session.to_owned();
    tokio::spawn(async move {
        let mut open = asks.open.lock().await;
        // Dropping the wait withdraws a pending card as its call ended.
        open.remove(&session);
        asks.watching().asking.remove(&session);
    });
}

/// A question on the screen, as the card shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Asked {
    /// The question's own words, and the folder it names, if it shows one.
    words: Vec<String>,
    choices: Vec<String>,
    /// The choice the screen highlights now, from 0.
    highlighted: usize,
    /// Whether `choices` is the whole list (D785). A list cut off by a
    /// screen too short for it (Copilot scrolls it, so 24 rows show only
    /// `1. Yes`) is not: its card offers no choice, only the terminal.
    whole: bool,
    /// Whether the screen's height cuts the list off: it starts past `1.`
    /// or a row says it scrolls (D786). A list that only lacks an answer
    /// may still be drawing, so that alone is not.
    cut_by_height: bool,
}

impl Asks {
    fn watching(&self) -> std::sync::MutexGuard<'_, Watching> {
        self.watching.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `session`'s screen no longer asks its card's question (answered in
    /// the terminal, or the agent moved on): withdraws the card as resolved
    /// (D781), ends its hold on the session's Needs you and stops following
    /// the screen for it. `open` is the locked map of open cards.
    async fn withdraw_gone(&self, open: &mut HashMap<String, Open>, session: &str) {
        if let Some(card) = open.remove(session).and_then(|mut gone| gone.card.take()) {
            self.inbox.withdraw_screen_resolved(&card).await;
        }
        self.watching().asking.remove(session);
    }

    async fn look(self: &Arc<Self>, session: &str, holder_pid: u32) {
        let Some((record, tab, index)) = self.agent_of(session, holder_pid).await else {
            return;
        };
        let Some((seen, (cols, rows))) = read_screen_sized(&tab.holder_socket).await else {
            return;
        };
        let mut open = self.open.lock().await;
        let Some((question, asked)) = seen else {
            self.withdraw_gone(&mut open, session).await;
            return;
        };
        let now = open
            .get(session)
            .filter(|now| (now.holder_pid, now.question) == (holder_pid, question));
        let grown = now.is_some_and(|now| now.grew);
        if now.is_some_and(|now| !now.grew && now.asked.same_card(&asked)) {
            return;
        }
        // A card read off a screen still drawing is withdrawn first. A
        // grow's hold is kept until the card takes over, so the session
        // needs you throughout.
        let held = open.remove(session).filter(|before| before.grew);
        self.watching().asking.remove(session);
        let grow = !grown
            && grows(
                asked.cut_by_height,
                self.writers.count(&tab.id),
                rows,
                !self.inbox.asks_in_agent(&record.agent),
            );
        if grow {
            drop(held);
            self.grow(&mut open, session, holder_pid, question, asked, &tab, cols).await;
            return;
        }
        let Some((task, card)) = self.raise(&record, tab, index, question, asked.clone()).await else {
            return;
        };
        self.watching().asking.insert(session.to_owned());
        open.insert(
            session.to_owned(),
            Open {
                holder_pid,
                question,
                asked,
                card,
                grew: false,
                task,
            },
        );
        drop(held);
    }

    /// D786: grows `tab`'s screen to [`START_ROWS`] rows at its `cols`,
    /// once. The read that asked it raises no card, but the session needs
    /// you from it on: a card-less hold stands until the next look, which
    /// comes with the program's redraw or [`LOOK_AFTER_GROW`] later, and
    /// raises the card. Nothing is typed. A grow that fails is not tried
    /// again: the next look raises the card the screen allows.
    #[allow(clippy::too_many_arguments)]
    async fn grow(
        self: &Arc<Self>,
        open: &mut HashMap<String, Open>,
        session: &str,
        holder_pid: u32,
        question: ScreenQuestion,
        asked: Asked,
        tab: &TabRecord,
        cols: u16,
    ) {
        open.insert(
            session.to_owned(),
            Open {
                holder_pid,
                question,
                asked,
                card: None,
                grew: true,
                task: hold_without_card(session),
            },
        );
        self.watching().asking.insert(session.to_owned());
        if let Err(error) = crate::attach::grow_rows(tab, &self.holders, cols, START_ROWS).await {
            crate::log::say!(
                "humatad2: session {session}'s agent asks a question its screen is too short to show whole, and growing the screen to {START_ROWS} rows failed, so its card offers no choice; open its terminal to answer: {}",
                status::chain(&error)
            );
        }
        self.look_after_grow(session, holder_pid);
    }

    /// Looks at `session`'s screen again [`LOOK_AFTER_GROW`] after a grow,
    /// so a program that doesn't redraw still gets its card. Not async, so
    /// a look's future doesn't hold another look's.
    fn look_after_grow(self: &Arc<Self>, session: &str, holder_pid: u32) {
        let (asks, session) = (Arc::clone(self), session.to_owned());
        let again: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> = Box::pin(async move {
            tokio::time::sleep(LOOK_AFTER_GROW).await;
            asks.look(&session, holder_pid).await;
        });
        tokio::spawn(again);
    }

    /// The question `tab`'s screen asks now: `Some(None)` when it asks
    /// none, `None` when the screen can't be read.
    async fn screen(&self, tab: &TabRecord) -> Option<Option<(ScreenQuestion, Asked)>> {
        read_screen(&tab.holder_socket).await
    }

    /// Raises the card for `tab`, the session's tab at `index`, and holds
    /// it until the person answers. Returns the hold and the card's id. For
    /// an agent whose asks stay in its own terminal (D782) it raises no
    /// card, but still holds the session's Needs you, card-less, until a
    /// look sees the dialog gone and drops the [`Open`] entry.
    async fn raise(
        self: &Arc<Self>,
        record: &SessionRecord,
        tab: TabRecord,
        index: u32,
        question: ScreenQuestion,
        asked: Asked,
    ) -> Option<(JoinHandle<()>, Option<String>)> {
        if self.inbox.asks_in_agent(&record.agent) {
            return Some((hold_without_card(&record.id), None));
        }
        let agent = AgentCaller::of(record, &self.machine);
        let caller = Caller::Agent(agent.clone());
        let card = Question {
            question: card_text(&record.agent, question, &asked),
            // None of a cut-off list (D785): with no choices and only
            // choices taken, no reply is taken, so the card leads to the
            // terminal and Dismiss only.
            choices: asked.offered(),
            // Its choices too, so a card raised again with more of them is
            // a new card, not the one being withdrawn.
            key: format!(
                "screen/{question:?}/{}/{}",
                asked.whole,
                asked.choices.join("\u{1f}")
            ),
            only_choices: true,
        };
        let still_asked = still_asks(tab.holder_socket.clone(), question);
        let wait = match self
            .inbox
            .raise_screen_question(&caller, &agent, card, index, still_asked)
            .await
        {
            Ok(wait) => wait,
            Err(error) => {
                crate::log::say!(
                    "humatad2: session {}'s agent asks a question on its screen, but raising it in the Inbox failed, so only its terminal shows it: {}",
                    record.id,
                    status::chain(&error)
                );
                return None;
            }
        };
        let id = wait.id().to_owned();
        let asking = crate::session_service::Asking::hold(&record.id);
        let this = Arc::clone(self);
        let task = tokio::spawn(async move {
            let mut wait = wait;
            let decision = wait.decided().await;
            if let Decision::Deny { .. } = decision {
                // D781: dismissed while the screen still asks (or couldn't
                // be read). The agent is still blocked on it, so this task
                // becomes D782's card-less hold: Needs you stays, the
                // screen is still followed, and the look that sees the
                // question gone drops the entry, which aborts this task.
                // The same screen raises no second card.
                drop(wait);
                hold_card_less(asking).await;
                return;
            }
            drop(asking);
            // Settled: the screen needn't be followed for this card.
            if let Some(session) = tab.session.as_deref() {
                this.watching().asking.remove(session);
            }
            if let Decision::Reply { text } = decision {
                this.pick(&tab, question, &text).await;
            }
            drop(wait);
        });
        Some((task, Some(id)))
    }

}

/// Holds `session`'s Needs you with no Inbox card, while its screen still
/// asks (D782; D781's Dismiss reuses it). Kept as an [`Open`] entry's task,
/// so the hold ends when a look sees the question gone and `withdraw_gone`
/// drops the entry, which aborts the task and lets go of the hold.
pub(crate) fn hold_without_card(session: &str) -> JoinHandle<()> {
    tokio::spawn(hold_card_less(crate::session_service::Asking::hold(session)))
}

/// Keeps `asking` (the session's Needs you) until the task running it is
/// aborted: the one card-less hold, for D782's agents that ask in their
/// own terminal and for a card dismissed while its screen still asks
/// (D781).
async fn hold_card_less(asking: crate::session_service::Asking) {
    let _asking = asking;
    std::future::pending::<()>().await;
}

impl Asks {
    /// Picks `choice` on `tab`'s screen, if the screen still asks
    /// `question` and still lists that choice. Nothing else is typed.
    async fn pick(&self, tab: &TabRecord, question: ScreenQuestion, choice: &str) {
        let session = tab.session.as_deref().unwrap_or("?");
        let Some(now) = self.screen(tab).await else {
            crate::log::say!(
                "humatad2: couldn't read session {session}'s screen to pick the Inbox answer, so the question is still there for you to answer in its terminal"
            );
            return;
        };
        let moves = match keys_for(question, choice, now.as_ref()) {
            Ok(moves) => moves,
            Err(not) => {
                crate::log::say!("humatad2: didn't pick the Inbox answer on session {session}'s screen: {not}");
                return;
            }
        };
        let link = match self.holders.link(&tab.id, &tab.holder_socket).await {
            Ok(link) => link,
            Err(error) => {
                crate::log::say!(
                    "humatad2: couldn't reach session {session}'s holder to pick the Inbox answer, so the question is still there for you to answer in its terminal: {}",
                    status::chain(&error)
                );
                return;
            }
        };
        if !moves.is_empty() {
            if let Err(error) = link.type_text(&moves).await {
                crate::log::say!(
                    "humatad2: couldn't pick the Inbox answer on session {session}'s screen, so the question is still there for you to answer in its terminal: {error}"
                );
                return;
            }
            tokio::time::sleep(KEY_GAP).await;
        }
        if let Err(error) = link.type_text(b"\r").await {
            crate::log::say!(
                "humatad2: couldn't press Enter for the Inbox answer on session {session}'s screen, so the question is still there for you to answer in its terminal: {error}"
            );
        }
    }

    /// The session's record, its agent's tab and that tab's index among
    /// the session's tabs, as ListTabs orders them, while it runs under
    /// `holder_pid`.
    async fn agent_of(&self, session: &str, holder_pid: u32) -> Option<(SessionRecord, TabRecord, u32)> {
        let (store, key) = (self.store.clone(), session.to_owned());
        let read = tokio::task::spawn_blocking(
            move || -> Result<Option<(SessionRecord, TabRecord, u32)>, StoreError> {
                let Some(agent) = store.agent(&key)? else {
                    return Ok(None);
                };
                if agent.state != (AgentState::Running { holder_pid }) {
                    return Ok(None);
                }
                let Some(tab) = store.tab(&agent.tab)? else {
                    return Ok(None);
                };
                if tab.kind != TabKind::Agent {
                    return Ok(None);
                }
                let Some(index) = store
                    .tabs()?
                    .iter()
                    .filter(|each| each.session.as_deref() == Some(key.as_str()))
                    .position(|each| each.id == tab.id)
                    .and_then(|index| u32::try_from(index).ok())
                else {
                    return Ok(None);
                };
                Ok(store.session(&key)?.map(|record| (record, tab, index)))
            },
        )
        .await;
        read.ok()?.ok()?
    }
}

impl Asked {
    /// Whether the card for `self` is the card for `other`: the same words
    /// and choices, wherever the highlight is.
    fn same_card(&self, other: &Asked) -> bool {
        self.words == other.words && self.choices == other.choices && self.whole == other.whole
    }

    /// The choices the card offers: every one, or none when the screen
    /// shows only part of the list (D785). A part offered as the whole
    /// would let one tap on Yes trust a folder the person could not see
    /// how to refuse.
    fn offered(&self) -> Vec<String> {
        if self.whole { self.choices.clone() } else { Vec::new() }
    }
}

/// The question the screen of the holder at `socket` asks now:
/// `Some(None)` when it asks none, `None` when it can't be read.
async fn read_screen(socket: &Path) -> Option<Option<(ScreenQuestion, Asked)>> {
    read_screen_sized(socket).await.map(|(asked, _)| asked)
}

/// [`read_screen`], and the screen's size, `(cols, rows)`.
async fn read_screen_sized(socket: &Path) -> Option<(Option<(ScreenQuestion, Asked)>, (u16, u16))> {
    let (screen, size) = crate::holders::screen_and_size_at(socket).await.ok()?;
    Some((asked_of(&screen), size))
}

/// D786: whether a look grows the screen to [`START_ROWS`] rows: only for
/// a list its height cuts off, while no attach that can type is attached,
/// the screen is shorter than that, and the agent's questions come to the
/// Inbox (D782). It never shrinks the screen nor changes its columns.
fn grows(cut_by_height: bool, writers: u32, rows: u16, asks_in_inbox: bool) -> bool {
    cut_by_height && writers == 0 && rows < START_ROWS && asks_in_inbox
}

/// The question `screen` asks and what its card shows, if it asks one.
fn asked_of(screen: &str) -> Option<(ScreenQuestion, Asked)> {
    let question = screen_question::asked_on(screen)?;
    let mut asked = read_asked(screen)?;
    asked.whole &= answers_complete(question, &asked.choices);
    Some((question, asked))
}

/// Checks an answer to the card for `question` against the screen of the
/// holder at `socket` as it comes: whether it still asks `question` and,
/// for a reply, lists the reply's choice.
fn still_asks(socket: std::path::PathBuf, question: ScreenQuestion) -> StillAsked {
    Arc::new(move |choice: Option<String>| {
        let socket = socket.clone();
        Box::pin(async move {
            let read = tokio::time::timeout(CHECK_LIMIT, read_screen(&socket)).await;
            let Ok(Some(now)) = read else {
                return Err(status::failure(
                    tonic::Code::Unavailable,
                    status::error(
                        "couldn't read the agent's screen to check that it still asks this question, so the reply wasn't taken and nothing was typed",
                        "reply again in a moment; if it keeps failing, answer the question in the session's terminal",
                        humata2_grpc::humata::v1::error::Kind::StepFailed(Box::new(
                            humata2_grpc::humata::v1::StepFailed {
                                step: "read the agent's screen".to_owned(),
                                left_as_is: "the card is still pending".to_owned(),
                                ..Default::default()
                            },
                        )),
                    ),
                ));
            };
            Ok(match choice {
                Some(choice) => keys_for(question, &choice, now.as_ref()).is_ok(),
                None => now.is_some_and(|(asking, _)| asking == question),
            })
        })
    })
}

/// Why a reply picked nothing on the screen.
#[derive(Debug, thiserror::Error)]
enum NotPicked {
    #[error("the screen no longer asks that question (it was answered in the terminal, or the agent moved on), so nothing was typed")]
    NoLongerAsked,
    #[error("the screen no longer lists the choice \"{0}\", so nothing was typed; answer the question in the session's terminal")]
    ChoiceGone(String),
}

/// The arrow keys that move the highlight of the question the screen asks
/// `now` onto `choice`, to be followed by Enter: only while it still asks
/// `question` and lists `choice`. The card may have been read off fewer
/// rows than the screen shows now, so the row is found on the screen now.
fn keys_for(
    question: ScreenQuestion,
    choice: &str,
    now: Option<&(ScreenQuestion, Asked)>,
) -> Result<Vec<u8>, NotPicked> {
    let Some((_, shown)) = now.filter(|(asking, _)| *asking == question) else {
        return Err(NotPicked::NoLongerAsked);
    };
    let row = shown
        .choices
        .iter()
        .position(|offered| offered == choice)
        .ok_or_else(|| NotPicked::ChoiceGone(choice.to_owned()))?;
    Ok(moves(shown.highlighted, row))
}

/// Whether `choices` holds every answer `question` must offer, whatever
/// the screen's footer says (D785): folder trust always offers a way to
/// refuse, so a list without a No is a part of one.
fn answers_complete(question: ScreenQuestion, choices: &[String]) -> bool {
    match question {
        ScreenQuestion::FolderTrust => {
            choices.len() >= 2
                && choices
                    .iter()
                    .any(|choice| choice == "No" || choice.starts_with("No,") || choice.starts_with("No "))
        }
    }
}

/// The card's words for `question`, asked by `agent`.
fn card_text(agent: &str, question: ScreenQuestion, asked: &Asked) -> String {
    let note = match question {
        _ if !asked.whole => {
            "Its screen is too short to show every choice, so none is offered here: open its terminal to see them all and answer there. Humata never answers this for you."
        }
        ScreenQuestion::FolderTrust => {
            "Yes lets the agent read, change and run the files in that folder. Humata never answers this for you: it picks only what you reply."
        }
    };
    format!(
        "{agent} is asking on its screen, and waits until it is answered:\n{}\n{note}",
        asked.words.join("\n")
    )
}

/// The keys that move the highlight from row `from` to row `to`.
fn moves(from: usize, to: usize) -> Vec<u8> {
    let (key, count) = if to >= from {
        (DOWN, to - from)
    } else {
        (UP, from - to)
    };
    key.repeat(count)
}

/// The question and its numbered choices as `screen` shows them: the line
/// that asks (one ending in `?`), a folder path just above it, and the
/// `1. …`, `2. …` rows after it. `None` if it lists no choices. The list
/// is not whole when it starts past `1.` or a row below it says the list
/// scrolls (D785); [`answers_complete`] checks the choices themselves.
fn read_asked(screen: &str) -> Option<Asked> {
    let rows: Vec<String> = screen.lines().map(unboxed).collect();
    let asks = rows.iter().rposition(|row| row.ends_with('?'))?;
    let mut words = Vec::new();
    // The nearest folder above the question: Copilot puts its own lines
    // between them.
    if let Some(folder) = rows[..asks]
        .iter()
        .rev()
        .find(|row| row.starts_with('/') || row.starts_with('~'))
    {
        words.push(folder.clone());
    }
    words.push(rows[asks].clone());
    let mut choices = Vec::new();
    let mut highlighted = 0;
    // The number the first row shown carries: a list scrolled down starts
    // past 1.
    let mut first = 1;
    let mut after = rows.len();
    for (at, row) in rows.iter().enumerate().skip(asks + 1) {
        if row.is_empty() && choices.is_empty() {
            continue;
        }
        let marked = row.starts_with(HIGHLIGHT);
        let rest = row.trim_start_matches(HIGHLIGHT).trim_start();
        let numbered = rest
            .split_once(". ")
            .and_then(|(number, text)| Some((number.parse::<usize>().ok()?, text)));
        let Some((number, text)) = numbered else {
            after = at;
            break;
        };
        if choices.is_empty() && number >= 1 {
            first = number;
        }
        if number != first + choices.len() {
            after = at;
            break;
        }
        if marked {
            highlighted = choices.len();
        }
        choices.push(without_key(text.trim()).to_owned());
    }
    let scrolls = rows[after..]
        .iter()
        .any(|row| row.to_lowercase().contains("to scroll"));
    (!choices.is_empty()).then_some(Asked {
        words,
        choices,
        highlighted,
        whole: first == 1 && !scrolls,
        cut_by_height: first != 1 || scrolls,
    })
}

/// A choice's words without the key the screen says picks it, as in
/// `No (Esc)`: a reply of `No` names it.
fn without_key(choice: &str) -> &str {
    let Some(open) = choice.rfind(" (") else {
        return choice;
    };
    let key = &choice[open + 2..];
    match key.strip_suffix(')') {
        Some(key) if !key.is_empty() && key.len() <= 10 && !key.contains(char::is_whitespace) => {
            choice[..open].trim_end()
        }
        _ => choice,
    }
}

/// `row` without box drawing and the space around it.
fn unboxed(row: &str) -> String {
    row.chars()
        .map(|c| if ('\u{2500}'..='\u{257f}').contains(&c) { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Copilot 1.0.113's folder trust, as a fresh or moved session shows it.
    const COPILOT_TRUST: &str = "\
 [Current]  Sessions   Issues   Pull requests   Gists
╭──────────────────────────────────────────────────────────╮
│ Confirm folder trust                                     │
│ ╭──────────────────────────────────────────────────────╮ │
│ │ /home/vremote/Humata/remote-flows/sessions/rr-flow2b │ │
│ ╰──────────────────────────────────────────────────────╯ │
│ Do you trust the files in this folder?                   │
│                                                          │
│ ❯ 1. Yes                                                 │
│   2. Yes, and remember this folder for future sessions   │
│   3. No (Esc)                                            │
╰──────────────────────────────────────────────────────────╯";

    /// Copilot 1.0.113's folder trust box as VERIFY captured it on a 120
    /// column screen: a fresh remote session, and a local session moved to
    /// a machine. The folder sits well above the question, and the third
    /// choice carries the key that picks it. Folder paths are made up.
    const COPILOT_TRUST_FRESH: &str = r#"╭──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Confirm folder trust                                                                                                 │
│ ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────── │
│ ╭──────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮ │
│ │ /home/dev/work/fresh                                                                                             │ │
│ ╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯ │
│                                                                                                                      │
│ Copilot can read files in this folder and, with your permission, edit them or run code and shell commands. It will   │
│ remember your permissions for the rest of this session.                                                              │
│                                                                                                                      │
│ Git commands run in this folder may use configuration from the enclosing repository and execute programs configured  │
│ by that repository.                                                                                                  │
│                                                                                                                      │
│ Do you trust the files in this folder?                                                                               │
│                                                                                                                      │
│ ❯ 1. Yes                                                                                                             │
│   2. Yes, and remember this folder for future sessions                                                               │
│   3. No (Esc)                                                                                                        │
│                                                                                                                      │
│ ↑/↓ to navigate · enter to select · esc to cancel                                                                    │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯"#;
    const COPILOT_TRUST_MOVED: &str = r#"╭──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Confirm folder trust                                                                                                 │
│ ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────── │
│ ╭──────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮ │
│ │ /home/dev/work/moved                                                                                             │ │
│ ╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯ │
│                                                                                                                      │
│ Copilot can read files in this folder and, with your permission, edit them or run code and shell commands. It will   │
│ remember your permissions for the rest of this session.                                                              │
│                                                                                                                      │
│ Git commands run in this folder may use configuration from the enclosing repository and execute programs configured  │
│ by that repository.                                                                                                  │
│                                                                                                                      │
│ Do you trust the files in this folder?                                                                               │
│                                                                                                                      │
│ ❯ 1. Yes                                                                                                             │
│   2. Yes, and remember this folder for future sessions                                                               │
│   3. No (Esc)                                                                                                        │
│                                                                                                                      │
│ ↑/↓ to navigate · enter to select · esc to cancel                                                                    │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯"#;

    /// D767 amendment: both captured screens give the folder, the question,
    /// all three choices (the No included, without its key hint) and the
    /// highlighted first row.
    #[test]
    fn copilots_captured_trust_boxes_give_every_choice_and_the_highlight() {
        for (screen, folder) in [
            (COPILOT_TRUST_FRESH, "/home/dev/work/fresh"),
            (COPILOT_TRUST_MOVED, "/home/dev/work/moved"),
        ] {
            let asked = read_asked(screen).expect("a question");
            assert_eq!(asked.words, [folder, "Do you trust the files in this folder?"]);
            assert_eq!(
                asked.choices,
                ["Yes", "Yes, and remember this folder for future sessions", "No"]
            );
            assert_eq!(asked.highlighted, 0);
        }
    }

    #[test]
    fn the_screen_gives_the_question_its_folder_and_every_choice() {
        let asked = read_asked(COPILOT_TRUST).expect("a question");
        assert_eq!(
            asked.words,
            [
                "/home/vremote/Humata/remote-flows/sessions/rr-flow2b",
                "Do you trust the files in this folder?"
            ]
        );
        assert_eq!(
            asked.choices,
            [
                "Yes",
                "Yes, and remember this folder for future sessions",
                "No"
            ]
        );
        assert_eq!(asked.highlighted, 0);
    }

    #[test]
    fn the_highlight_is_found_where_the_screen_marks_it() {
        let moved = COPILOT_TRUST
            .replace("│ ❯ 1. Yes ", "│   1. Yes ")
            .replace("│   3. No (Esc)  ", "│ ❯ 3. No (Esc)  ");
        assert_eq!(read_asked(&moved).expect("a question").highlighted, 2);
    }

    #[test]
    fn a_question_with_no_choices_listed_is_not_read() {
        assert_eq!(read_asked("Do you trust the files in this folder?\n❯ "), None);
    }

    #[test]
    fn a_reply_moves_the_highlight_to_its_row_on_the_screen_now() {
        let now = (
            ScreenQuestion::FolderTrust,
            read_asked(COPILOT_TRUST_FRESH).expect("a question"),
        );
        let keys = |choice| keys_for(ScreenQuestion::FolderTrust, choice, Some(&now));
        assert_eq!(keys("Yes").expect("picked"), b"");
        assert_eq!(keys("Yes, and remember this folder for future sessions").expect("picked"), DOWN);
        assert_eq!(keys("No").expect("picked"), [DOWN, DOWN].concat());
    }

    /// Copilot's trust box as VERIFY 0.2.3 captured it on a headless 80x24
    /// pty (e2e023 c6c): too short for the box, Copilot shows only `1. Yes`
    /// and scrolls the rest.
    const COPILOT_TRUST_CLIPPED: &str = r#" [Current]  Sessions   Issues   Pull requests   Gists

                                                                               ┃
╭──────────────────────────────────────────────────────────────────────────────╮
│ Confirm folder trust                                                         │
│ ──────────────────────────────────────────────────────────────────────────── │
│ │ /private/tmp/claude-501/-Users-avestabarzegar-oss-atmosphere-khodr/b04a6 │ │
│ │ 34e-8063-4fd8-9208-8df4fee17944/scratchpad/verify/lanes/e2/root/Humata/c │ │
│ │ 6/sessions/trust3                                                        │ │
│ ╰──────────────────────────────────────────────────────────────────────────╯ │
│                                                                              │
│ Copilot can read files in this folder and, with your permission, edit them   │
│ or run code and shell commands. It will remember your permissions for the    │
│ rest of this session.                                                        │
│                                                                              │
│ Git commands run in this folder may use configuration from the enclosing     │
│ repository and execute programs configured by that repository.               │
│                                                                              │
│ Do you trust the files in this folder?                                       │
│                                                                              │
│ ❯ 1. Yes                                                                     │
│                                                                              │
│ ↑/↓ to navigate · pgup/pgdn to scroll · enter to select · esc to cancel      │
╰──────────────────────────────────────────────────────────────────────────────╯"#;

    /// D785: a list the screen cuts off is never offered as the whole: the
    /// card for the clipped box has no choice to tap, so no one-tap Yes,
    /// and its words send the person to the terminal.
    #[test]
    fn a_trust_box_clipped_by_a_short_screen_offers_no_choice_only_the_terminal() {
        let (question, asked) = asked_of(COPILOT_TRUST_CLIPPED).expect("a question");
        assert_eq!(question, ScreenQuestion::FolderTrust);
        assert!(!asked.whole);
        assert!(asked.offered().is_empty());
        assert!(card_text("copilot", question, &asked).contains("terminal"));
    }

    /// D785: the same holds without the footer that says the list scrolls:
    /// a trust list with no way to refuse is cut off.
    #[test]
    fn a_trust_list_without_its_no_is_cut_off_whatever_the_footer_says() {
        let unfooted = COPILOT_TRUST_CLIPPED.replace("pgup/pgdn to scroll · ", "");
        let (_, asked) = asked_of(&unfooted).expect("a question");
        assert!(asked.offered().is_empty());
        let two = COPILOT_TRUST_FRESH.replace("│   3. No (Esc)  ", "│                ");
        let (_, asked) = asked_of(&two).expect("a question");
        assert!(asked.offered().is_empty());
    }

    #[test]
    fn a_list_scrolled_past_its_first_choice_is_cut_off() {
        let scrolled = COPILOT_TRUST_FRESH.replace("│ ❯ 1. Yes      ", "│                ");
        let (_, asked) = asked_of(&scrolled).expect("a question");
        assert!(asked.offered().is_empty());
    }

    #[test]
    fn a_reply_to_a_card_read_off_a_box_still_drawing_picks_its_row_on_the_full_box() {
        let (_, early) = asked_of(
            "│ Do you trust the files in this folder? │\n│                │\n│ ❯ 1. Yes       │",
        )
        .expect("a question");
        // D785: this test once took ["Yes"] as the early card's choices,
        // which offered a one-tap Yes with no way to say No. A box still
        // drawing shows a part of the list, so its card offers none.
        assert!(early.offered().is_empty());
        let (_, full) = asked_of(COPILOT_TRUST_MOVED).expect("a question");
        assert_eq!(full.offered().len(), 3);
        assert!(!early.same_card(&full));
        let now = (ScreenQuestion::FolderTrust, full);
        assert_eq!(
            keys_for(ScreenQuestion::FolderTrust, "Yes", Some(&now)).expect("picked"),
            b""
        );
    }

    #[test]
    fn a_reply_picks_nothing_once_the_screen_stops_asking_or_drops_the_choice() {
        assert!(matches!(
            keys_for(ScreenQuestion::FolderTrust, "Yes", None),
            Err(NotPicked::NoLongerAsked)
        ));
        let now = (
            ScreenQuestion::FolderTrust,
            read_asked(COPILOT_TRUST_FRESH).expect("a question"),
        );
        assert!(matches!(
            keys_for(ScreenQuestion::FolderTrust, "Maybe", Some(&now)),
            Err(NotPicked::ChoiceGone(_))
        ));
    }

    #[test]
    fn a_key_hint_is_left_off_a_choice_and_other_brackets_are_kept() {
        assert_eq!(without_key("No (Esc)"), "No");
        assert_eq!(without_key("Yes (for this session only)"), "Yes (for this session only)");
        assert_eq!(without_key("Yes"), "Yes");
    }

    #[test]
    fn the_highlight_moves_down_or_up_to_the_choice() {
        assert_eq!(moves(0, 0), b"");
        assert_eq!(moves(0, 2), b"\x1b[B\x1b[B");
        assert_eq!(moves(2, 1), b"\x1b[A");
    }

    /// D786: a box the screen's height clips or scrolls is cut by height;
    /// a whole box, and one that only lacks its No (still drawing), are not.
    #[test]
    fn only_a_list_the_screens_height_cuts_off_is_cut_by_height() {
        let clipped = |screen: &str| asked_of(screen).expect("a question").1.cut_by_height;
        assert!(clipped(COPILOT_TRUST_CLIPPED));
        assert!(clipped(&COPILOT_TRUST_FRESH.replace("│ ❯ 1. Yes      ", "│                ")));
        assert!(!clipped(COPILOT_TRUST_FRESH));
        assert!(!clipped(&COPILOT_TRUST_FRESH.replace("│   3. No (Esc)  ", "│                ")));
    }

    /// D786: a grow needs every condition; any one missing stops it.
    #[test]
    fn a_grow_needs_a_height_cut_no_writer_a_short_screen_and_the_inbox() {
        assert!(grows(true, 0, 24, true));
        assert!(grows(true, 0, START_ROWS - 1, true));
        assert!(!grows(false, 0, 24, true), "a list only still drawing");
        assert!(!grows(true, 1, 24, true), "a writer attached");
        assert!(!grows(true, 0, START_ROWS, true), "already tall enough");
        assert!(!grows(true, 0, 60, true), "never shrinks");
        assert!(!grows(true, 0, 24, false), "asks stay in the agent's terminal");
    }
}
