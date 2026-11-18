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
//! - Deny types nothing: the question stays on the screen for the person.
//! - A question the screen no longer shows (answered in the terminal) is
//!   withdrawn, and so is one whose agent exited.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use humata2_store::{AgentState, SessionRecord, Store, StoreError, TabKind, TabRecord};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::holders::Holders;
use crate::identity::{AgentCaller, Caller};
use crate::inbox::Inbox;
use crate::inbox::permission::{Decision, Question};
use crate::screen_question::{self, ScreenQuestion};
use crate::status;

/// Between the highlight's moves and the Enter that picks it, so a
/// program reading keys one at a time sees them apart.
const KEY_GAP: Duration = Duration::from_millis(150);
/// One row up and one row down, as a terminal's arrow keys send them.
const UP: &[u8] = b"\x1b[A";
const DOWN: &[u8] = b"\x1b[B";
/// The marks a screen puts before its highlighted choice.
const HIGHLIGHT: &[char] = &['❯', '›', '>', '●', '▶'];

static ASKS: OnceLock<Arc<Asks>> = OnceLock::new();

/// What looks at agents' screens, for the daemon's life.
struct Asks {
    holders: Arc<Holders>,
    inbox: Arc<Inbox>,
    store: Arc<Store>,
    machine: String,
    /// The question each session's screen asks, by session.
    open: Mutex<HashMap<String, Open>>,
}

/// A question a session's screen asks, and the card's wait for it.
struct Open {
    holder_pid: u32,
    question: ScreenQuestion,
    /// What the card shows: a later look that reads more of the screen
    /// (a box drawn in steps) raises the card again with it.
    asked: Asked,
    /// Holds the card and the session's Needs you; ends with the person's
    /// answer. Ending it early withdraws the card.
    task: JoinHandle<()>,
}

impl Drop for Open {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Starts looking at agents' screens. Called once, when the daemon serves.
pub(crate) fn start(holders: Arc<Holders>, inbox: Arc<Inbox>, store: Arc<Store>, machine: String) {
    let _ = ASKS.set(Arc::new(Asks {
        holders,
        inbox,
        store,
        machine,
        open: Mutex::new(HashMap::new()),
    }));
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
        asks.open.lock().await.remove(&session);
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
}

impl Asks {
    async fn look(self: &Arc<Self>, session: &str, holder_pid: u32) {
        let Some((record, tab)) = self.agent_of(session, holder_pid).await else {
            return;
        };
        let Some(seen) = self.screen(&tab).await else {
            return;
        };
        let mut open = self.open.lock().await;
        let Some((question, asked)) = seen else {
            open.remove(session);
            return;
        };
        let shown = open.get(session).is_some_and(|now| {
            (now.holder_pid, now.question) == (holder_pid, question) && now.asked.same_card(&asked)
        });
        if shown {
            return;
        }
        // A card read off a screen still drawing is withdrawn first.
        open.remove(session);
        let Some(task) = self.raise(&record, tab, question, asked.clone()).await else {
            return;
        };
        open.insert(
            session.to_owned(),
            Open {
                holder_pid,
                question,
                asked,
                task,
            },
        );
    }

    /// The question `tab`'s screen asks now: `Some(None)` when it asks
    /// none, `None` when the screen can't be read.
    async fn screen(&self, tab: &TabRecord) -> Option<Option<(ScreenQuestion, Asked)>> {
        let screen = crate::holders::screen_at(&tab.holder_socket).await.ok()?;
        Some(screen_question::asked_on(&screen).and_then(|question| {
            let asked = read_asked(&screen)?;
            Some((question, asked))
        }))
    }

    /// Raises the card and holds it until the person answers.
    async fn raise(
        self: &Arc<Self>,
        record: &SessionRecord,
        tab: TabRecord,
        question: ScreenQuestion,
        asked: Asked,
    ) -> Option<JoinHandle<()>> {
        // D199: the user keeps asks in the agent's own screen.
        if self.inbox.asks_in_agent().await.unwrap_or(false) {
            return None;
        }
        let agent = AgentCaller::of(record, &self.machine);
        let caller = Caller::Agent(agent.clone());
        let card = Question {
            question: card_text(&record.agent, question, &asked),
            choices: asked.choices.clone(),
            // Its choices too, so a card raised again with more of them is
            // a new card, not the one being withdrawn.
            key: format!("screen/{question:?}/{}", asked.choices.join("\u{1f}")),
            only_choices: true,
        };
        let wait = match self.inbox.raise_question(&caller, &agent, card).await {
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
        let asking = crate::session_service::Asking::hold(&record.id);
        let this = Arc::clone(self);
        Some(tokio::spawn(async move {
            let mut wait = wait;
            let decision = wait.decided().await;
            drop(asking);
            if let Decision::Reply { text } = decision {
                this.pick(&tab, question, &text).await;
            }
            drop(wait);
        }))
    }

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

    /// The session's record and its agent's tab, while it runs under
    /// `holder_pid`.
    async fn agent_of(&self, session: &str, holder_pid: u32) -> Option<(SessionRecord, TabRecord)> {
        let (store, key) = (self.store.clone(), session.to_owned());
        let read = tokio::task::spawn_blocking(
            move || -> Result<Option<(SessionRecord, TabRecord)>, StoreError> {
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
                Ok(store.session(&key)?.map(|record| (record, tab)))
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
        self.words == other.words && self.choices == other.choices
    }
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

/// The card's words for `question`, asked by `agent`.
fn card_text(agent: &str, question: ScreenQuestion, asked: &Asked) -> String {
    let note = match question {
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
/// `1. …`, `2. …` rows after it. `None` if it lists no choices.
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
    for row in &rows[asks + 1..] {
        if row.is_empty() && choices.is_empty() {
            continue;
        }
        let marked = row.starts_with(HIGHLIGHT);
        let rest = row.trim_start_matches(HIGHLIGHT).trim_start();
        let Some((number, text)) = rest.split_once(". ") else {
            break;
        };
        if number.parse::<usize>().ok() != Some(choices.len() + 1) {
            break;
        }
        if marked {
            highlighted = choices.len();
        }
        choices.push(without_key(text.trim()).to_owned());
    }
    (!choices.is_empty()).then_some(Asked {
        words,
        choices,
        highlighted,
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

    #[test]
    fn a_reply_to_a_card_read_off_a_box_still_drawing_picks_its_row_on_the_full_box() {
        let early = read_asked(
            "│ Do you trust the files in this folder? │\n│                │\n│ ❯ 1. Yes       │",
        )
        .expect("a question");
        assert_eq!(early.choices, ["Yes"]);
        let full = read_asked(COPILOT_TRUST_MOVED).expect("a question");
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
}
