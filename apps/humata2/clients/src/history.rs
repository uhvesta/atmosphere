//! Searching every session's conversation, and reading a hit's turns
//! (LD-23, LD-24).
//!
//! The host keeps a copy of each session's conversation and a full-text
//! index over it, so one search covers every machine, archived sessions
//! included, and still answers while a machine is offline. [`FakeHistory`]
//! is the only implementation until the live client serves it.

use crate::daemon::Reply;
use crate::model::{ChannelId, MachineId, SessionId};

pub mod fake;

pub use fake::{Call, FakeHistory, IndexedSession};

/// A turn that matched a search, and where it is (LD-24).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// The session.
    pub session: SessionId,
    /// Its channel.
    pub channel: ChannelId,
    /// The channel's name.
    pub channel_name: String,
    /// The session's title.
    pub title: String,
    /// The machine the session runs on.
    pub machine: MachineId,
    /// The machine's name.
    pub machine_name: String,
    /// The session's agent, such as `claude`.
    pub agent: String,
    /// The turn that matched, counted from 1; 0 is what came before the
    /// first prompt.
    pub turn: u32,
    /// The matching entry's place in the session's conversation, counted
    /// from 0: [`Turn::seq`] of the piece that matched (D186).
    pub seq: u64,
    /// The text around the match, as plain text.
    pub snippet: String,
    /// When the host last caught up with this session, RFC 3339: results
    /// from an offline machine are as of then (LD-23).
    pub as_of: String,
}

/// Who wrote a piece of a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TurnRole {
    /// The person's prompt.
    User,
    /// What the agent said.
    Agent,
    /// A tool call or its result.
    Tool,
}

/// One piece of a session's conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    /// Its place in the session, counted from 0.
    pub seq: u64,
    /// The turn it belongs to, counted from 1; 0 is what came before the
    /// first prompt.
    pub turn: u32,
    /// Who wrote it.
    pub role: TurnRole,
    /// What it says.
    pub text: String,
}

/// One page of hits, best first, and the token of the next page: empty on
/// the last one.
pub type SearchPage = (Vec<SearchHit>, String);

/// What the app asks the host about sessions' conversations.
pub trait History: Send + Sync + 'static {
    /// The turns that have every word of `query`, one hit for a turn
    /// however often it matches. `page_token` is empty for the first page,
    /// or what the page before returned.
    fn search(&self, query: &str, page_token: &str) -> Reply<SearchPage>;

    /// The session's conversation from turn `first_turn` to `last_turn`,
    /// both included, in order. 0 starts at the beginning and `u32::MAX`
    /// reads to the end.
    fn read(&self, session: &SessionId, first_turn: u32, last_turn: u32) -> Reply<Vec<Turn>>;
}
