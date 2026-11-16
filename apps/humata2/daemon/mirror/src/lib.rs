//! The host's copy of every session's conversation (LD-8) and the full-text
//! index over it (LD-23 to LD-25).
//!
//! [`Mirror::catch_up`] reads an agent's conversation file from where the
//! session's mirror stopped ([`read_from`]), replaces the machine's host
//! variable values with their names ([`Scrubber`], LD-21), appends the new
//! entries to the session's mirror file and indexes them. [`Mirror::search`]
//! and [`Mirror::read`] answer `sessions_search` and `session_read` from the
//! host alone, so they work while a machine is offline.
//!
//! # On disk
//!
//! ```text
//! <root>/mirror/<session>/entries.log   the session's entries, append-only (user data)
//! <root>/mirror/<session>/...           the rest of the session's saved state (user data)
//! <root>/mirror/<session>/raw-<n>.jsonl the raw lines of a session on another machine (user data, D195)
//! <root>/search/index.db                the full-text index (derived, rebuilt from mirror/)
//! ```
//!
//! - `<session>` is the session id as it is when it holds only ASCII
//!   letters, digits, `_` and `-` (every id Humata mints does). Every other
//!   byte is written `%XX` (two upper-case hex digits), so no id names a
//!   path outside `<root>/mirror/`.
//! - `entries.log` only grows. One [`Mirror::catch_up`] adds its entries as
//!   one append, and the append counts once it is committed together with
//!   where reading the agent's file stopped and the [`OwnTools`] seen so far
//!   (a saved length beside the file or a checksummed frame; the format is
//!   this crate's own). Bytes after the last committed append are a cut-off
//!   append: they never come back as entries, [`Mirror::open`] or the next
//!   `catch_up` drops them, and the lines they came from are read again
//!   from the agent's file, once.
//! - Everything a hit shows is saved with the mirror: the names given to
//!   [`Mirror::describe`] and the time of the last catch-up as well as the
//!   entries. Nothing in `index.db` is the only copy (D62.5).
//! - `index.db` is SQLite with `PRAGMA user_version` = [`FORMAT_VERSION`].
//!   When the file is missing, unreadable as an index or carries another
//!   version, [`Mirror::open`] deletes it and builds it again from
//!   `<root>/mirror/`.

mod error;
mod index;
mod mirror;
mod raw;
mod scrub;
mod session;
mod source;
mod transcript;
mod turns;

pub use error::MirrorError;
pub use mirror::{
    Chunk, FORMAT_VERSION, Hit, Mirror, MirrorEntry, Page, PageAsk, Query, RawConversation,
    SessionMeta, entry_count,
};
pub use scrub::Scrubber;
pub use source::{Lines, MAX_CHUNK, read_lines};
pub use transcript::{Entry, OwnTools, Read, Role, parse_lines, read_from};
pub use turns::{TurnMark, turns_in, turns_of};
