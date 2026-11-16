//! Persisted waiting messages.
use humata2_clients::model::Comment;
use serde::{Deserialize, Serialize, Serializer};

/// A message waiting to be typed into the session's main agent. It saves as
/// plain text, as the oldest builds did.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "Saved")]
pub enum QueuedMessage {
    /// Plain queued typing.
    Text(String),
}

/// What older builds may have saved: text, or a batch of review comments.
/// Comments now belong to the daemon, so a saved batch becomes the text it
/// would have typed and nothing the person sent is lost.
#[derive(Deserialize)]
#[serde(untagged)]
enum Saved {
    Text(String),
    Comments { comments: Vec<Comment> },
}

impl From<Saved> for QueuedMessage {
    fn from(saved: Saved) -> Self {
        match saved {
            Saved::Text(text) => Self::Text(text),
            Saved::Comments { comments } => Self::Text(Comment::message(&comments)),
        }
    }
}

impl Serialize for QueuedMessage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text(text) => serializer.serialize_str(text),
        }
    }
}

impl QueuedMessage {
    /// The complete message shown in the queue and delivered together.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
        }
    }
}
