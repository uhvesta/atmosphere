//! Humata 2's features, on tca-rust: each one's state, actions and reducer,
//! and what they ask of `humatad`. Swift lays the state out and sends the
//! actions through what the `ffi` feature exports.

mod chat;
mod daemon;
mod feed;
#[cfg(feature = "ffi")]
mod ffi;
mod sessions;

pub use chat::{
    Body, Chat, ChatAction, ChatFeature, ChatItem, Entry, EventKind, PermissionAnswer, Queued,
    Reading, Streaming, ToolOutcome, Transcript,
};
pub use daemon::{ChatItems, Daemon, DaemonError, LiveDaemon, Session, SessionState};
pub use feed::{
    ChatStatus, ChatUpdate, Feed, QueuedMessage, ReadingProgress, ReplyChange, Row, RowChange,
};
pub use sessions::{Sessions, SessionsAction, SessionsFeature};

#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!("humata2");
