//! A scripted [`History`]: it keeps the conversations a test gives it and
//! searches them the simple way, so results are easy to predict. No
//! timers, no real IO.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::watch;

use super::{History, SearchHit, SearchPage, Turn, TurnRole};
use crate::daemon::{DaemonError, Reply};
use crate::model::{ChannelId, MachineId, SessionId};

/// What the fake says about a session in its hits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedSession {
    /// The session.
    pub session: SessionId,
    /// Its channel.
    pub channel: ChannelId,
    /// The channel's name.
    pub channel_name: String,
    /// The session's title.
    pub title: String,
    /// The machine it runs on.
    pub machine: MachineId,
    /// The machine's name.
    pub machine_name: String,
    /// Its agent.
    pub agent: String,
    /// When the host last caught up with it, RFC 3339.
    pub as_of: String,
}

/// A call a test can read back, one per trait method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// `search`.
    Search {
        /// The query as it was asked.
        query: String,
        /// The page asked for.
        page_token: String,
    },
    /// `read`.
    Read {
        /// The session.
        session: SessionId,
        /// The first turn asked for.
        first_turn: u32,
        /// The last turn asked for.
        last_turn: u32,
    },
}

/// Hits to a page unless a test says otherwise.
const DEFAULT_PAGE_SIZE: usize = 50;

/// One piece of a conversation, as it was pushed.
struct Piece {
    session: SessionId,
    turn: u32,
    role: TurnRole,
    text: String,
}

struct Inner {
    sessions: HashMap<SessionId, IndexedSession>,
    pieces: Vec<Piece>,
    page_size: usize,
    failure: Option<DaemonError>,
    calls: Vec<Call>,
    /// The next search waits.
    hold_next: bool,
    /// Opens the gates of the searches that wait.
    gates: Vec<watch::Sender<bool>>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            pieces: Vec::new(),
            page_size: DEFAULT_PAGE_SIZE,
            failure: None,
            calls: Vec::new(),
            hold_next: false,
            gates: Vec::new(),
        }
    }
}

impl Inner {
    fn push(&mut self, session: &SessionId, turn: u32, role: TurnRole, text: &str) {
        self.sessions.entry(session.clone()).or_insert_with(|| IndexedSession {
            session: session.clone(),
            channel: ChannelId::new(""),
            channel_name: String::new(),
            title: String::new(),
            machine: MachineId::new(""),
            machine_name: String::new(),
            agent: String::new(),
            as_of: String::new(),
        });
        self.pieces.push(Piece {
            session: session.clone(),
            turn,
            role,
            text: text.to_owned(),
        });
    }

    fn search(&self, query: &str, page_token: &str) -> Result<SearchPage, DaemonError> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Err(refused("This search", "it has no words; type something to look for"));
        }
        let mut seen = HashSet::new();
        let mut hits = Vec::new();
        for piece in self.pieces.iter().rev() {
            let text = piece.text.to_lowercase();
            if !words.iter().all(|word| text.contains(word.as_str())) {
                continue;
            }
            if !seen.insert((piece.session.clone(), piece.turn)) {
                continue;
            }
            let Some(about) = self.sessions.get(&piece.session) else {
                continue;
            };
            hits.push(SearchHit {
                session: about.session.clone(),
                channel: about.channel.clone(),
                channel_name: about.channel_name.clone(),
                title: about.title.clone(),
                machine: about.machine.clone(),
                machine_name: about.machine_name.clone(),
                agent: about.agent.clone(),
                turn: piece.turn,
                snippet: piece.text.clone(),
                as_of: about.as_of.clone(),
            });
        }
        let start = self.offset(page_token, hits.len())?;
        let end = hits.len().min(start.saturating_add(self.page_size));
        let next = if end < hits.len() { format!("page-{end}") } else { String::new() };
        Ok((hits[start..end].to_vec(), next))
    }

    /// Where the page of `token` starts; only tokens this fake gave work.
    fn offset(&self, token: &str, total: usize) -> Result<usize, DaemonError> {
        if token.is_empty() {
            return Ok(0);
        }
        let start = token
            .strip_prefix("page-")
            .and_then(|digits| digits.parse::<usize>().ok())
            .filter(|start| *start > 0 && *start < total && start % self.page_size == 0);
        start.ok_or_else(|| {
            refused(
                "This page",
                "the page token is not one this search gave; search again from the first page",
            )
        })
    }

    fn read(&self, session: &SessionId, first_turn: u32, last_turn: u32) -> Result<Vec<Turn>, DaemonError> {
        if !self.sessions.contains_key(session) {
            return Err(DaemonError::NotFound { what: "This session's conversation".to_owned() });
        }
        let turns = self
            .pieces
            .iter()
            .filter(|piece| &piece.session == session)
            .enumerate()
            .filter(|(_, piece)| piece.turn >= first_turn && piece.turn <= last_turn)
            .map(|(seq, piece)| Turn {
                seq: u64::try_from(seq).unwrap_or(u64::MAX),
                turn: piece.turn,
                role: piece.role,
                text: piece.text.clone(),
            })
            .collect();
        Ok(turns)
    }
}

fn refused(what: &str, reason: &str) -> DaemonError {
    DaemonError::Refused { what: what.to_owned(), reason: reason.to_owned() }
}

/// A history that holds what a test pushed into it. Clones share it all.
///
/// A search splits the query at spaces and finds the pieces whose text has
/// every word somewhere in it, whatever the case ("ledg" finds "Ledger").
/// A turn is one hit however many of its pieces match: the hit's snippet is
/// the text of the newest piece that matched. Hits come newest first: the
/// one whose matching piece was pushed last leads. A query with no words is
/// refused, as is a page token the fake didn't give. Reading a session the
/// fake has nothing of is not found.
#[derive(Clone, Default)]
pub struct FakeHistory {
    shared: Arc<Mutex<Inner>>,
}

impl FakeHistory {
    /// A history with no sessions, 50 hits to a page.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Hits to a page from now on; at least 1.
    #[must_use]
    pub fn with_page_size(self, size: usize) -> Self {
        self.inner().page_size = size.max(1);
        self
    }

    /// Names a session for its hits and adds `turns` to it, oldest first,
    /// each a turn number, who wrote it and the text. Naming a session
    /// again renames it and keeps its turns.
    #[must_use]
    pub fn with_turns(self, about: &IndexedSession, turns: &[(u32, TurnRole, &str)]) -> Self {
        {
            let mut inner = self.inner();
            inner.sessions.insert(about.session.clone(), about.clone());
            for (turn, role, text) in turns {
                inner.push(&about.session, *turn, *role, text);
            }
        }
        self
    }

    /// Adds one piece to a session's conversation: the newest there is, in
    /// any session. A session the fake wasn't told about has empty names.
    pub fn push_turn(&self, session: &SessionId, turn: u32, role: TurnRole, text: &str) {
        self.inner().push(session, turn, role, text);
    }

    /// Makes the next call fail with `error`, whichever method it is. The
    /// call is still recorded.
    pub fn fail_next(&self, error: DaemonError) {
        self.inner().failure = Some(error);
    }

    /// The calls made so far, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.inner().calls.clone()
    }

    /// Makes the next search wait: its reply doesn't come until
    /// [`release`](Self::release). It answers from what the fake held when
    /// it was asked. Searches after that one answer at once, so a test can
    /// let a newer query's reply overtake an older one's.
    pub fn hold(&self) {
        self.inner().hold_next = true;
    }

    /// Lets every search that waits answer.
    pub fn release(&self) {
        let mut inner = self.inner();
        inner.hold_next = false;
        for gate in inner.gates.drain(..) {
            gate.send_replace(true);
        }
    }
}

impl History for FakeHistory {
    fn search(&self, query: &str, page_token: &str) -> Reply<SearchPage> {
        let mut inner = self.inner();
        inner.calls.push(Call::Search { query: query.to_owned(), page_token: page_token.to_owned() });
        if let Some(error) = inner.failure.take() {
            return Box::pin(std::future::ready(Err(error)));
        }
        let result = inner.search(query, page_token);
        if !std::mem::take(&mut inner.hold_next) {
            return Box::pin(std::future::ready(result));
        }
        let (gate, mut opened) = watch::channel(false);
        inner.gates.push(gate);
        Box::pin(async move {
            // A gate dropped without opening answers too: nothing is left to wait for.
            let _ = opened.wait_for(|open| *open).await;
            result
        })
    }

    fn read(&self, session: &SessionId, first_turn: u32, last_turn: u32) -> Reply<Vec<Turn>> {
        let mut inner = self.inner();
        inner.calls.push(Call::Read { session: session.clone(), first_turn, last_turn });
        let result = match inner.failure.take() {
            Some(error) => Err(error),
            None => inner.read(session, first_turn, last_turn),
        };
        Box::pin(std::future::ready(result))
    }
}
