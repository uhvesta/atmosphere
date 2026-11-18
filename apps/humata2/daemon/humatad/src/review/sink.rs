//! Where a batch of the user's comments is typed (SPEC SM-9, D138): the
//! session's main agent terminal. The review talks to a [`CommentSink`], so
//! it can tell "nothing was typed" from "can't tell" without knowing about
//! terminals.

use std::sync::Arc;
use std::time::Duration;

use tonic::Status;

use crate::terminal::{CommentsArrived, CommentsTarget, CommentsTyped, Core};

/// Where a batch of comments is typed: the session's main agent terminal
/// (SM-9).
#[tonic::async_trait]
pub(crate) trait CommentSink: Send + Sync + 'static {
    /// Where the session's batches are typed now, or why there is nowhere.
    async fn target(&self, session: &str) -> Result<Target, Status>;

    /// Types `paste` (already a bracketed paste) into `target`'s terminal.
    /// `key` names the batch: a holder that can remember keys types it once
    /// however often it is sent.
    async fn paste(&self, target: &Target, key: &str, paste: &[u8]) -> Pasted;

    /// After a restart: whether the batch `key` reached `target`'s
    /// terminal. `No` only when the same holder process that held the
    /// terminal when the batch was begun says it never came; a holder
    /// started again, or one that can't remember keys, answers `Unknown`.
    async fn arrived(&self, target: &Target, key: &str) -> Arrived;
}

/// Where a batch goes: the tab and the holder process that held its
/// terminal when the batch was begun (0 if it can't confirm what it
/// typed).
pub(crate) struct Target {
    pub(crate) tab: String,
    pub(crate) holder: u64,
}

/// What became of typing a batch.
pub(crate) enum Pasted {
    /// All of it reached the terminal.
    Typed,
    /// None of it was typed; Humata holds the comments.
    NotTyped(Status),
    /// Some or all of it may have been typed.
    Unknown(Status),
}

/// Whether a batch reached the terminal.
pub(crate) enum Arrived {
    Yes,
    No,
    Unknown,
}

/// The pause between a batch's paste showing and the return that sends
/// it. D671 (Provisional): 100ms was too short for Copilot's terminal.
const RETURN_PAUSE: Duration = Duration::from_millis(400);
/// How long the paste gets to show before return is pressed anyway.
const SHOWN_WAIT: Duration = Duration::from_secs(2);

/// The real sink: the terminals' shared state.
pub(crate) struct TerminalSink {
    core: Arc<Core>,
}

impl TerminalSink {
    pub(crate) fn new(core: Arc<Core>) -> Self {
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

    /// Types the paste, then presses return so the agent gets it as one
    /// message (SM-9), each once for its own key. A return that fails
    /// leaves the batch in doubt: sending it again presses only return.
    async fn paste(&self, target: &Target, key: &str, paste: &[u8]) -> Pasted {
        let target = core_target(target);
        let mut watch = match self.core.watch_for_comments(&target).await {
            Ok(watch) => watch,
            Err(refused) => return Pasted::NotTyped(refused),
        };
        match self.core.type_comments(&target, key, paste).await {
            CommentsTyped::Yes => {}
            CommentsTyped::No(status) => return Pasted::NotTyped(status),
            CommentsTyped::Unknown(status) => return Pasted::Unknown(status),
        }
        // BUG-T1: a return that reaches the agent before it has taken the
        // paste is part of it. A long paste shown as a summary is never
        // seen, so the wait is short.
        if let Some(watch) = watch.as_mut() {
            let _ = watch.shown(paste, SHOWN_WAIT).await;
        }
        // An agent CLI reads a return right behind a paste as part of it.
        tokio::time::sleep(RETURN_PAUSE).await;
        match self.core.type_comments(&target, &format!("{key}/return"), b"\r").await {
            CommentsTyped::Yes => Pasted::Typed,
            CommentsTyped::No(status) | CommentsTyped::Unknown(status) => Pasted::Unknown(status),
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
