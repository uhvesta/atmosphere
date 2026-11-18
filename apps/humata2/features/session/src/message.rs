//! Persisted waiting messages.
use humata2_clients::model::Comment;
use humata2_clients::model::SessionId;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};

/// A message waiting to be typed into the session's main agent. It saves as
/// plain text, as the oldest builds did.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "Saved")]
pub enum QueuedMessage {
    /// Plain queued typing, from builds that saved no key.
    Text(String),
    /// Typing with the key its delivery is deduplicated by. The key is saved
    /// with the message, so sending it again after the app quit mid-send
    /// does not type it twice.
    Keyed {
        /// Stable for this message; never reused for another.
        key: String,
        /// What to type.
        text: String,
    },
}

/// What older builds may have saved: text, or a batch of review comments.
/// Comments now belong to the daemon, so a saved batch becomes the text it
/// would have typed and nothing the person sent is lost.
#[derive(Deserialize)]
#[serde(untagged)]
enum Saved {
    Text(String),
    Keyed { key: String, text: String },
    Comments { comments: Vec<Comment> },
}

impl From<Saved> for QueuedMessage {
    fn from(saved: Saved) -> Self {
        match saved {
            Saved::Text(text) => Self::Text(text),
            Saved::Keyed { key, text } => Self::Keyed { key, text },
            Saved::Comments { comments } => Self::Text(Comment::message(&comments)),
        }
    }
}

impl Serialize for QueuedMessage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text(text) => serializer.serialize_str(text),
            Self::Keyed { key, text } => {
                let mut saved = serializer.serialize_struct("Keyed", 2)?;
                saved.serialize_field("key", key)?;
                saved.serialize_field("text", text)?;
                saved.end()
            }
        }
    }
}

impl QueuedMessage {
    /// The complete message shown in the queue and delivered together.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Text(text) | Self::Keyed { text, .. } => text.clone(),
        }
    }

    /// The key a delivery of this message is deduplicated by, if it has one.
    #[must_use]
    pub fn key(&self) -> Option<&str> {
        match self {
            Self::Text(_) => None,
            Self::Keyed { key, .. } => Some(key),
        }
    }

    /// A new message for `session`, in queue slot `position`, made at `now`
    /// on the app's clock as the `serial`th message of this run. The serial
    /// keeps two messages made at the same instant apart; the text's hash
    /// keeps a later run (whose clock and serial start over) from repeating
    /// the key of a different message the daemon already delivered.
    pub(crate) fn new(
        session: &SessionId,
        position: usize,
        text: String,
        now: std::time::Duration,
        serial: u64,
    ) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        let nanos = now.as_nanos();
        let digest = hasher.finish();
        Self::Keyed {
            key: format!("queued-{session}-{nanos}-{serial}-{position}-{digest:x}"),
            text,
        }
    }
}
