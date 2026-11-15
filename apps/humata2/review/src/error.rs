//! What can go wrong in a review, said to the person who has to act on it.

use crate::model::{CommentId, DeliveryId, FileKey, LineRange, NoteId, SessionId, Switch};

/// A review action that couldn't be done.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReviewError {
    #[error("The comment is empty. Write what the agent should do, or press Escape to cancel.")]
    EmptyComment,

    #[error("Comment {id} isn't in this session's review any more; it may have been dropped.")]
    NoComment { id: CommentId },

    #[error(
        "Comment {id} is being typed into the agent's terminal right now, so it can't be \
         changed. Wait for it to be sent, then reopen it to change it."
    )]
    Sending { id: CommentId },

    #[error(
        "Comment {id} was already sent to the agent, so changing it here wouldn't reach the \
         agent. Reopen it to send a new version."
    )]
    AlreadySent { id: CommentId },

    #[error("There are no unsent comments to send. Add a comment on a line first.")]
    NothingToSend,

    #[error("Send {delivery} isn't queued any more; it may have been sent or cancelled.")]
    NoDelivery { delivery: DeliveryId },

    #[error(
        "Send {delivery} is being typed into the agent's terminal right now, so it can't be \
         cancelled. Wait for it to finish."
    )]
    DeliveryInFlight { delivery: DeliveryId },

    #[error("Note {note} isn't in this review any more; it may have been dropped or undone.")]
    NoNote { note: NoteId },
}

/// Why the review chat's agent couldn't change the view. It's the tool's
/// answer, so it's said to the agent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    #[error(
        "The user turned off {switch} for this review (\"Agent may: {switch}\" in the review's \
         Info panel). Don't change their view that way; say in the chat where to look instead, \
         or ask them to turn {switch} on."
    )]
    TurnedOff { switch: Switch },

    #[error(
        "{file} isn't in this review's changes. Call review_context for the files it has, then \
         try again with one of those."
    )]
    NoFile { file: FileKey },

    #[error(
        "Lines {}-{} aren't in the diff of {file} on that side. Call review_context for the \
         lines shown, then try again.", lines.first, lines.last
    )]
    NoLines { file: FileKey, lines: LineRange },

    #[error("{file} has no hunk {}. Call review_context for its hunks, then try again.", hunk + 1)]
    NoHunk { file: FileKey, hunk: usize },

    #[error("There's no change {id} to undo; it may have been undone already.")]
    NoChange { id: String },

    #[error(
        "Session {session}'s review isn't open in the app, so its view can't be changed now. \
         Ask the user to open the session's review, then try again."
    )]
    NotOpen { session: SessionId },
}

/// Why the terminal didn't take a message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeliveryError {
    #[error(
        "The agent in session {session} has exited, so nothing was typed. Resume the agent \
         from the session, then press Retry; the comments are kept until then."
    )]
    Exited { session: SessionId },

    #[error(
        "Session {session}'s machine is offline, so nothing was typed yet. The comments are \
         kept and sent when it reconnects."
    )]
    Disconnected { session: SessionId },

    #[error(
        "The link to session {session} dropped while the comments were being sent, so it isn't \
         known whether they arrived. They are sent again with the same key when it's back, \
         which the terminal types at most once."
    )]
    Interrupted { session: SessionId },

    #[error("Session {session}'s terminal refused the comments: {reason}. Fix that, then press Retry.")]
    Refused { session: SessionId, reason: String },
}

/// Saving or loading a session's review failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(
        "Couldn't save session {session}'s review comments: {source}. Free disk space or fix \
         the data folder's permissions; until then nothing more is sent, so no comment is sent \
         without being recorded."
    )]
    Write {
        session: SessionId,
        #[source]
        source: std::io::Error,
    },

    #[error("Couldn't read session {session}'s saved review: {source}. Check the data folder's permissions.")]
    Read {
        session: SessionId,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "Session {session}'s saved review can't be read ({reason}). It was left as it is; \
         deleting it loses that session's drafts and unsent comments."
    )]
    Corrupt { session: SessionId, reason: String },
}

/// The session's review history couldn't be read or added to.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryError {
    #[error(
        "Couldn't record what you reviewed in session {session}: {reason}. Nothing was marked; \
         press Mark reviewed again once the session's machine is reachable."
    )]
    Mark { session: SessionId, reason: String },

    #[error(
        "Review {number} of session {session} isn't kept any more, so there's nothing to compare \
         with. Pick a later review, or compare with the session's base."
    )]
    Gone { session: SessionId, number: u32 },

    #[error(
        "Couldn't load session {session}'s changes: {reason}. The review shows what it had; \
         pick the base again once the session's machine is reachable."
    )]
    Load { session: SessionId, reason: String },
}
