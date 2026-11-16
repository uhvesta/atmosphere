//! Search over every session's conversation (LD-23, LD-24): the query, its
//! hits, and opening a hit's turn.

use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::history::{History, SearchHit, SearchPage};
use humata2_clients::model::SessionId;
use tca::{Effect, Reducer};

/// The fewest characters, spaces not counted, a query is searched for.
const SHORTEST_QUERY: usize = 2;

/// The search's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FindState {
    /// The query as it was typed.
    pub query: String,
    /// The hits of the last search that answered, best first. They stay
    /// while the next search is on its way.
    pub hits: Arc<Vec<SearchHit>>,
    /// The next page's token, empty when there's no more.
    pub next_page: String,
    /// Whether a search is on its way.
    pub loading: bool,
    /// What went wrong with the last search.
    pub error: Option<String>,
    /// Counts the queries: one more with each [`FindAction::Query`], so a
    /// reply is known to be for the query that is there now.
    pub generation: u64,
}

/// What search asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindDelegate {
    /// Open a session at a turn (LD-24).
    OpenTurn {
        /// The session.
        session: SessionId,
        /// The turn, counted from 1.
        turn: u32,
        /// The matching entry's place in the session's conversation (D186).
        seq: u64,
    },
}

/// What happens in search.
#[derive(Clone, Debug, PartialEq)]
pub enum FindAction {
    /// The query changed. With fewer than two characters, spaces not
    /// counted, nothing is searched and the hits, the next page and the
    /// error are cleared; otherwise its first page is asked for, and the
    /// error is cleared.
    Query(String),
    /// A search answered. A reply for another generation than the state's
    /// is dropped, so a slow reply never replaces a newer query's hits. A
    /// failure shows as the error and clears the hits.
    Loaded {
        /// The generation of the query it answers.
        generation: u64,
        /// The hits and the next page's token, or why there are none.
        result: Result<SearchPage, DaemonError>,
    },
    /// Ask for the next page. It does nothing when [`FindState::next_page`]
    /// is empty or a search is on its way (`loading`). Otherwise the page
    /// is asked for and answers with [`FindAction::MoreLoaded`].
    More,
    /// The next page answered. A reply for another generation than the
    /// state's is dropped. On success its hits are added after the hits
    /// shown; on failure the error shows and the hits and the next page's
    /// token are kept, so the same page can be asked for again.
    MoreLoaded {
        /// The generation of the query it answers.
        generation: u64,
        /// The hits and the next page's token, or why there are none.
        result: Result<SearchPage, DaemonError>,
    },
    /// The hit at this place in [`FindState::hits`] was picked: its turn
    /// opens. Past the end, nothing happens.
    Open(usize),
    /// For the parent.
    Delegate(FindDelegate),
}

/// Search's reducer.
#[derive(Clone)]
pub struct Find {
    history: Arc<dyn History>,
}

impl Find {
    /// A search that asks `history`.
    #[must_use]
    pub fn new(history: Arc<dyn History>) -> Self {
        Self { history }
    }
}

impl Reducer for Find {
    type State = FindState;
    type Action = FindAction;

    fn reduce(&self, state: &mut FindState, action: FindAction) -> Effect<FindAction> {
        match action {
            FindAction::Query(query) => {
                let long_enough = query.chars().filter(|c| !c.is_whitespace()).count() >= SHORTEST_QUERY;
                state.generation += 1;
                state.error = None;
                if !long_enough {
                    state.query = query;
                    state.hits = Arc::new(Vec::new());
                    state.next_page = String::new();
                    state.loading = false;
                    return Effect::none();
                }
                state.loading = true;
                let generation = state.generation;
                let search = self.history.search(&query, "");
                state.query = query;
                Effect::future(async move {
                    FindAction::Loaded { generation, result: search.await }
                })
            }
            FindAction::Loaded { generation, result } => {
                if generation != state.generation {
                    return Effect::none();
                }
                state.loading = false;
                match result {
                    Ok((hits, next)) => {
                        state.hits = Arc::new(hits);
                        state.next_page = next;
                    }
                    Err(error) => {
                        state.error = Some(error.to_string());
                        state.hits = Arc::new(Vec::new());
                        state.next_page = String::new();
                    }
                }
                Effect::none()
            }
            FindAction::More => {
                if state.next_page.is_empty() || state.loading {
                    return Effect::none();
                }
                state.loading = true;
                let generation = state.generation;
                let search = self.history.search(&state.query, &state.next_page);
                Effect::future(async move {
                    FindAction::MoreLoaded { generation, result: search.await }
                })
            }
            FindAction::MoreLoaded { generation, result } => {
                if generation != state.generation {
                    return Effect::none();
                }
                state.loading = false;
                match result {
                    Ok((hits, next)) => {
                        let all: Vec<SearchHit> = state.hits.iter().cloned().chain(hits).collect();
                        state.hits = Arc::new(all);
                        state.next_page = next;
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
                Effect::none()
            }
            FindAction::Open(index) => match state.hits.get(index) {
                Some(hit) => Effect::send(FindAction::Delegate(FindDelegate::OpenTurn {
                    session: hit.session.clone(),
                    turn: hit.turn,
                    seq: hit.seq,
                })),
                None => Effect::none(),
            },
            FindAction::Delegate(_) => Effect::none(),
        }
    }
}
