//! Where a batch of the user's comments is typed (SPEC SM-9, D138): the
//! session's main agent terminal. The review talks to a [`CommentSink`], so
//! it can tell "nothing was typed" from "can't tell" without knowing about
//! terminals.

use std::sync::Arc;
use std::time::Duration;

use tonic::Status;

use crate::ports::{CommentsArrived, CommentsTarget, CommentsTyped, ReviewTerminalCore};

/// Where a batch of comments is typed: the session's main agent terminal (SM-9).
#[tonic::async_trait]
pub trait CommentSink: Send + Sync + 'static {
    /// Where the session's batches are typed now, or why there is nowhere.
    async fn target(&self, session: &str) -> Result<Target, Status>;

    /// Types `paste` (already a bracketed paste) into `target`'s terminal.
    async fn paste(&self, target: &Target, key: &str, paste: &[u8]) -> Pasted;

    /// After a restart: whether the batch `key` reached `target`'s terminal.
    async fn arrived(&self, target: &Target, key: &str) -> Arrived;
}

/// Where a batch goes: the tab and the holder process that held its terminal.
pub struct Target {
    pub tab: String,
    pub holder: u64,
}

/// What became of typing a batch.
pub enum Pasted {
    /// All of it reached the terminal.
    Typed,
    /// None of it was typed; Humata holds the comments.
    NotTyped(Status),
    /// Some or all of it may have been typed.
    Unknown(Status),
}

/// Whether a batch reached the terminal.
pub enum Arrived {
    Yes,
    No,
    Unknown,
}

/// The pause between a batch's paste showing and the return that sends it.
const RETURN_PAUSE: Duration = Duration::from_millis(400);
/// How long the paste gets to show before return is pressed anyway.
const SHOWN_WAIT: Duration = Duration::from_secs(2);

/// The real sink: the terminals' shared state behind [`ReviewTerminalCore`].
pub struct TerminalSink {
    core: Arc<dyn ReviewTerminalCore>,
}

impl TerminalSink {
    pub fn new(core: Arc<dyn ReviewTerminalCore>) -> Self {
        Self { core }
    }
}

fn core_target(target: &Target) -> CommentsTarget {
    CommentsTarget {
        tab: target.tab.clone(),
        holder: target.holder,
    }
}

#[tonic::async_trait]
impl CommentSink for TerminalSink {
    async fn target(&self, session: &str) -> Result<Target, Status> {
        let CommentsTarget { tab, holder } = self.core.comments_target(session).await?;
        Ok(Target { tab, holder })
    }

    async fn paste(&self, target: &Target, key: &str, paste: &[u8]) -> Pasted {
        match self
            .core
            .type_comments(&core_target(target), key, paste, SHOWN_WAIT, RETURN_PAUSE)
            .await
        {
            CommentsTyped::Typed => Pasted::Typed,
            CommentsTyped::NotTyped(status) => Pasted::NotTyped(status),
            CommentsTyped::Unknown(status) => Pasted::Unknown(status),
        }
    }

    async fn arrived(&self, target: &Target, key: &str) -> Arrived {
        match self.core.comments_arrived(&core_target(target), key).await {
            CommentsArrived::Yes => Arrived::Yes,
            CommentsArrived::No => Arrived::No,
            CommentsArrived::Unknown => Arrived::Unknown,
        }
    }
}
