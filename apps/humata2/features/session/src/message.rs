//! Persisted waiting messages, including their comment metadata.
use humata2_clients::model::Comment;
use serde::{Deserialize, Serialize};

/// A message waiting to be typed into the session's main agent.
/// Untagged text retains compatibility with older saved string queues.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QueuedMessage {
    /// Plain queued typing.
    Text(String),
    /// Review comments sent together in one message.
    Comments { comments: Vec<Comment> },
}

impl QueuedMessage {
    /// The complete message shown in the queue and delivered together.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Comments { comments } => Comment::message(comments),
        }
    }
}
