use std::path::PathBuf;

use crate::Format;

/// Why a conversation couldn't be read, forked, packed or placed. Nothing a
/// refusal names was written: the parent's files are as they were.
#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    /// The adapter can't read this conversation (CV-3, CV-7): its format
    /// isn't proven against the real CLI yet, or a record holds a kind or a
    /// path field the adapter's strict parse doesn't know. `version` is the
    /// CLI version seen, as recorded in the conversation or at launch.
    /// Nothing is read further or written.
    #[error(
        "Humata can't read this {format} conversation (CLI version {}) yet: this version of Humata doesn't know its format or some of its records, so its turns can't be listed and it can't be forked with its conversation (at a turn or from now) or carried to another machine. Check for a Humata update that knows this CLI version. Until then, fork it code-only for a fresh agent on the same code, or start fresh with a summary. Copying the files as they are isn't offered, because a rewrite of records Humata can't read could damage the conversation without saying so",
        version.as_deref().unwrap_or("not recorded")
    )]
    Unsupported { format: Format, version: Option<String> },

    /// A conversation id that isn't one plain file name.
    #[error(
        "{id:?} isn't a conversation id this CLI could have written: an id is one plain name, with no `/` and not `.` or `..`. Check the session's pinned conversation id"
    )]
    BadId { id: String },

    /// The conversation's transcript isn't where its CLI keeps it.
    #[error(
        "there is no conversation file at {}; the agent may not have saved its first turn yet, or it ran with another home or folder. Send the agent a prompt and try again, or check the session's agent settings",
        path.display()
    )]
    NotFound { path: PathBuf },

    /// A complete line of a transcript isn't a record of its format.
    #[error(
        "line {line} of {} isn't a conversation record Humata can read, so nothing was copied. The CLI may have been cut off while writing it; resume the agent once so it repairs the file, or fork from now as a handover",
        path.display()
    )]
    Malformed {
        path: PathBuf,
        /// The line's number, from 1.
        line: u64,
        /// Why the line doesn't parse, when it isn't JSON at all.
        #[source]
        source: Option<serde_json::Error>,
    },

    /// A fork before turn 1 would hold no conversation.
    #[error("a fork before the first turn would hold no conversation; pick a later turn, fork from now, or start a fresh agent")]
    NothingBefore,

    /// The conversation has no such turn.
    #[error(
        "the conversation has {turns} turns, so there is no turn {turn} to fork before; reopen the fork dialog to see its turns as they are now"
    )]
    NoSuchTurn { turn: u32, turns: u32 },

    /// A file is over the size a carry rewrites.
    #[error(
        "{} is {size} bytes, over the {max} bytes Humata rewrites when it carries a conversation, so nothing was copied. Start fresh with a summary on the new machine instead",
        path.display()
    )]
    TooBig { path: PathBuf, size: u64, max: u64 },

    /// A file of the conversation isn't text, so the old folder's path in it
    /// can't be rewritten.
    #[error(
        "{} isn't text, so Humata can't rewrite the old folder's path in it and copied nothing. Copying it as it is could leave the carried conversation pointing at the old folder; start fresh with a summary instead, or remove the file if the agent doesn't need it",
        path.display()
    )]
    NotText { path: PathBuf },

    /// A path among the conversation's files is a link or something other
    /// than a file or folder.
    #[error(
        "{} is a link or special file, not a plain file, so Humata won't follow or copy it. Replace it with the file it points to, or remove it, then try again",
        path.display()
    )]
    NotAFile { path: PathBuf },

    /// Something is already where a new file would go.
    #[error(
        "{} is already there, and Humata never overwrites a conversation. If it is left over from an earlier attempt that didn't finish, move it aside and try again; otherwise pick another id or folder",
        path.display()
    )]
    Exists { path: PathBuf },

    /// A packed file's name would place it outside its folder.
    #[error(
        "the carried file name {rel:?} would land outside the agent's folder, so nothing was placed. The move manifest is damaged or wasn't written by Humata; start the move again from the old machine"
    )]
    BadPath { rel: String },

    /// The CLI's own index couldn't be read or updated (D336).
    #[error(
        "couldn't update the CLI's thread index {}: {source}. The conversation's file was already rewritten for its new folder, so the CLI may not list it there. Close the CLI if it is running, check that the file is writable, then resume the conversation once from its new folder",
        path.display()
    )]
    State {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    /// The CLI's turn history of a conversation about to be rewritten where
    /// it is couldn't be dropped (D347, X2).
    #[error(
        "couldn't drop this conversation's turn history from the CLI's file {}: {source}. The conversation's files weren't rewritten. Close the CLI if it is running and check that the file is writable, then move the session again. If the file has no such table, this CLI keeps its history in a layout this version of Humata doesn't know: check for a Humata update before moving, because a rewrite would leave that history pointing at the wrong places in the conversation",
        path.display()
    )]
    History {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    /// The file system refused a read or write.
    #[error(
        "couldn't {doing} {}; check that it is readable by this user, that its folder is writable and that the disk has free space, then try again. The parent conversation is unchanged",
        path.display()
    )]
    Io {
        doing: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
