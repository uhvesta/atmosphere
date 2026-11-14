//! What changed in a chat since Swift was last told: the rows that came,
//! went or changed, what the streaming reply gained, and the status when
//! it changed. A long chat costs a pointer check per row when nothing
//! changed, and a streaming reply sends only its new text.

use std::sync::Arc;

use tca::{Change, IdentifiedVec};

use crate::chat::{Body, Chat, Entry, Queued};

/// One change to the rows, in the order to make them: removals first,
/// then, by index in the new rows, what came and what changed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum RowChange {
    Removed { id: String },
    Inserted { index: u64, row: Row },
    Updated { index: u64, row: Row },
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Row {
    pub id: String,
    pub body: Body,
}

impl From<&Entry> for Row {
    fn from(entry: &Entry) -> Self {
        Self {
            id: entry.id.to_string(),
            body: Body::clone(&entry.body),
        }
    }
}

/// What happened to the reply that streams below the rows.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ReplyChange {
    /// A reply started streaming, with what it has so far.
    Started { id: String, text: String },
    /// It gained `text` at its end.
    Appended { text: String },
    /// It's done; its whole text is a row now, or the turn was stopped.
    Ended,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ReadingProgress {
    pub read: u64,
    /// 0 when it isn't known.
    pub total: u64,
}

/// Everything about the chat besides its rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ChatStatus {
    pub session: Option<String>,
    /// Set while the history is read.
    pub reading: Option<ReadingProgress>,
    pub working: bool,
    pub waiting: Option<String>,
    pub queued: Vec<QueuedMessage>,
    pub sending: bool,
    pub stopping: bool,
    pub unsent: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct QueuedMessage {
    pub id: String,
    pub text: String,
}

impl From<&Queued> for QueuedMessage {
    fn from(queued: &Queued) -> Self {
        Self {
            id: queued.id.clone(),
            text: queued.text.clone(),
        }
    }
}

/// What changed since the last update.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ChatUpdate {
    /// Counts the updates that changed something, so two alike still
    /// differ.
    pub number: u64,
    /// Drop every row and the reply first: another session shows.
    pub reset: bool,
    pub rows: Vec<RowChange>,
    pub reply: Option<ReplyChange>,
    /// Set when it changed.
    pub status: Option<ChatStatus>,
}

impl ChatUpdate {
    /// Whether there's nothing to tell.
    pub fn is_empty(&self) -> bool {
        !self.reset && self.rows.is_empty() && self.reply.is_none() && self.status.is_none()
    }
}

/// Remembers what it last told, to tell only what changed since.
#[derive(Default)]
pub struct Feed {
    session: Option<String>,
    rows: IdentifiedVec<Entry>,
    /// The streaming reply's id and how much of it was told.
    reply: Option<(Arc<str>, usize)>,
    status: ChatStatus,
    number: u64,
    /// Nothing told yet: the first update tells the status even if it's
    /// the default.
    started: bool,
}

impl Feed {
    pub fn new() -> Self {
        Self::default()
    }

    /// What changed in `chat` since the last call. The first call tells
    /// all of it.
    pub fn next(&mut self, chat: &Chat) -> ChatUpdate {
        let mut update = ChatUpdate::default();
        if self.session != chat.session {
            self.session = chat.session.clone();
            if !self.rows.is_empty() || self.reply.is_some() {
                update.reset = true;
            }
            self.rows = IdentifiedVec::new();
            self.reply = None;
        }

        let shown = &chat.shown;
        // Rows are shared, so this compares pointers, not text.
        if shown.entries != self.rows {
            update.rows = shown
                .entries
                .changes(&self.rows)
                .into_iter()
                .map(|change| row_change(&shown.entries, change))
                .collect();
            self.rows = shown.entries.clone();
        }

        update.reply = match (&self.reply, &shown.streaming) {
            (None, None) => None,
            (Some(_), None) => {
                self.reply = None;
                Some(ReplyChange::Ended)
            }
            (Some((id, told)), Some(streaming))
                if *id == streaming.id && streaming.text.len() >= *told =>
            {
                let told = *told;
                (streaming.text.len() > told).then(|| {
                    self.reply = Some((streaming.id.clone(), streaming.text.len()));
                    ReplyChange::Appended {
                        text: streaming.text[told..].to_owned(),
                    }
                })
            }
            (_, Some(streaming)) => {
                self.reply = Some((streaming.id.clone(), streaming.text.len()));
                Some(ReplyChange::Started {
                    id: streaming.id.to_string(),
                    text: streaming.text.clone(),
                })
            }
        };

        let status = status(chat);
        if !self.started || status != self.status {
            self.started = true;
            self.status = status.clone();
            update.status = Some(status);
        }

        if !update.is_empty() {
            self.number += 1;
        }
        update.number = self.number;
        update
    }
}

fn row_change(rows: &IdentifiedVec<Entry>, change: Change<Arc<str>>) -> RowChange {
    let row = |index: usize| Row::from(&rows[index]);
    match change {
        Change::Removed { id } => RowChange::Removed { id: id.to_string() },
        Change::Inserted { index, .. } => RowChange::Inserted {
            index: index as u64,
            row: row(index),
        },
        Change::Updated { index, .. } => RowChange::Updated {
            index: index as u64,
            row: row(index),
        },
    }
}

fn status(chat: &Chat) -> ChatStatus {
    ChatStatus {
        session: chat.session.clone(),
        reading: chat.reading.as_ref().map(|reading| ReadingProgress {
            read: reading.read,
            total: reading.total,
        }),
        working: chat.shown.working,
        waiting: chat.shown.waiting.clone(),
        queued: chat.shown.queued.iter().map(QueuedMessage::from).collect(),
        sending: chat.sending,
        stopping: chat.stopping,
        unsent: chat.unsent.clone(),
        error: chat.error.clone(),
    }
}
